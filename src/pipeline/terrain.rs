//! Terrain fill, geometry helpers, and per-tile processing.
//!
//! Holds the shared bounds/way-resolution helpers used by both the in-memory
//! preview pipeline ([`super::preview`]) and the streaming dispatch in
//! [`super`] (the parent module), plus [`process_tile`] — the deduplicated
//! tile body extracted by ARC-002 — and the terrain-only entry points
//! ([`run_terrain_only`] in-memory, [`run_terrain_only_to_disk`] streaming).

use anyhow::{Context, Result};
use rayon::prelude::*;
use std::collections::HashSet;

use crate::bedrock;
use crate::blocks::Block;
use crate::convert::CoordConverter;
use crate::elevation;
use crate::osm;
use crate::params::{ConvertParams, TerrainParams};
use crate::spatial::{HeightMap, ResolvedRelation, SpatialIndex, TILE_CHUNKS, compute_surface_y};
use crate::world::{self, ChunkData, Edition, MIN_Y, WorldWriter};

use super::render::{RenderContext, TileWays, render_osm_features};

/// Return type of [`fill_terrain_chunk`]: chunk coords, chunk data, and
/// a list of surface heights for each (bx, bz) column within the chunk.
type TerrainChunkResult = ((i32, i32), ChunkData, Vec<((i32, i32), i32)>);

/// Default surface thickness when elevation is active and the user didn't
/// override.  Thick enough to avoid hollow hillsides when digging sideways.
const ELEVATION_SURFACE_THICKNESS: i32 = 32;

// ── Geometry helpers (pipeline-local) ─────────────────────────────────────────

/// Compute the axis-aligned bounding box of the four map-corner block coordinates.
///
/// Converts the four corners of a geographic bounding box to block coordinates
/// and returns `(min_x, max_x, min_z, max_z)`.  The array always has exactly
/// four elements so `min`/`max` are guaranteed to return `Some`.
pub(super) fn bounding_box(corners: &[(i32, i32); 4]) -> (i32, i32, i32, i32) {
    let min_x = corners
        .iter()
        .map(|p| p.0)
        .min()
        .expect("corners is non-empty");
    let max_x = corners
        .iter()
        .map(|p| p.0)
        .max()
        .expect("corners is non-empty");
    let min_z = corners
        .iter()
        .map(|p| p.1)
        .min()
        .expect("corners is non-empty");
    let max_z = corners
        .iter()
        .map(|p| p.1)
        .max()
        .expect("corners is non-empty");
    (min_x, max_x, min_z, max_z)
}

// ── Terrain bounds computation ─────────────────────────────────────────────────

/// Compute the block-coordinate bounding box from all way nodes.
///
/// Uses 1st–99th percentile filtering and adds 10% padding so the terrain
/// extends slightly beyond the data boundary.  Returns `(-500, 500, -500, 500)`
/// when the way set is empty.
pub(super) fn compute_terrain_bounds(
    data: &osm::OsmData,
    conv: &CoordConverter,
) -> (i32, i32, i32, i32) {
    let mut xs: Vec<i32> = Vec::new();
    let mut zs: Vec<i32> = Vec::new();
    for way in data.ways() {
        for id in &way.node_refs {
            if let Some(node) = data.nodes().get(id) {
                let (bx, bz) = conv.to_block_xz(node.lat, node.lon);
                xs.push(bx);
                zs.push(bz);
            }
        }
    }
    xs.sort_unstable();
    zs.sort_unstable();

    if xs.is_empty() {
        return (-500, 500, -500, 500);
    }
    let p1 = xs.len() / 100;
    let p99 = xs.len().saturating_sub(1 + xs.len() / 100);
    let x_lo = xs[p1];
    let x_hi = xs[p99];
    let z_lo = zs[p1];
    let z_hi = zs[p99];
    let x_pad = ((x_hi - x_lo) / 10).max(32);
    let z_pad = ((z_hi - z_lo) / 10).max(32);
    (x_lo - x_pad, x_hi + x_pad, z_lo - z_pad, z_hi + z_pad)
}

/// Resolve multipolygon relations to block coordinates.
pub(super) fn resolve_relations<'a>(
    data: &'a osm::OsmData,
    conv: &CoordConverter,
) -> Vec<ResolvedRelation<'a>> {
    data.relations
        .iter()
        .filter_map(|rel| {
            let mut outers: Vec<Vec<(i32, i32)>> = Vec::new();
            let mut inners: Vec<Vec<(i32, i32)>> = Vec::new();
            for member in &rel.members {
                if let Some(&idx) = data.ways_by_id().get(&member.way_id) {
                    let way = &data.ways()[idx];
                    let pts: Vec<(i32, i32)> = way
                        .node_refs
                        .iter()
                        .filter_map(|id| data.nodes().get(id))
                        .map(|n| conv.to_block_xz(n.lat, n.lon))
                        .collect();
                    if pts.len() < 3 {
                        continue;
                    }
                    match member.role.as_str() {
                        "outer" | "" => outers.push(pts),
                        "inner" => inners.push(pts),
                        _ => {}
                    }
                }
            }
            if outers.is_empty() {
                return None;
            }
            Some(ResolvedRelation {
                tags: &rel.tags,
                outers,
                inners,
            })
        })
        .collect()
}

/// Resolve ways to (way, block_pts) pairs.
pub(super) fn resolve_ways<'a>(
    data: &'a osm::OsmData,
    conv: &CoordConverter,
) -> Vec<(&'a osm::OsmWay, Vec<(i32, i32)>)> {
    data.ways()
        .iter()
        .map(|way| {
            let pts: Vec<(i32, i32)> = way
                .node_refs
                .iter()
                .filter_map(|id| data.nodes().get(id))
                .map(|n| conv.to_block_xz(n.lat, n.lon))
                .collect();
            (way, pts)
        })
        .collect()
}

/// Compute spawn point from params, CoordConverter, and HeightMap.
pub(super) fn resolve_spawn(
    params: &ConvertParams,
    conv: &CoordConverter,
    height_map: &HeightMap,
    min_cx: i32,
    max_cx: i32,
    min_cz: i32,
    max_cz: i32,
) -> (i32, i32, i32) {
    let (spawn_x, spawn_z) = if let (Some(sx), Some(sz)) = (params.spawn_x, params.spawn_z) {
        (sx, sz)
    } else if let (Some(lat), Some(lon)) = (params.spawn_lat, params.spawn_lon) {
        conv.to_block_xz(lat, lon)
    } else {
        let total_cx = max_cx - min_cx + 1;
        let total_cz = max_cz - min_cz + 1;
        let avg_cx = min_cx + total_cx / 2;
        let avg_cz = min_cz + total_cz / 2;
        (avg_cx * 16 + 8, avg_cz * 16 + 8)
    };
    let spawn_y = params
        .spawn_y
        .unwrap_or_else(|| height_map.get(spawn_x, spawn_z) + 1);
    (spawn_x, spawn_y, spawn_z)
}

// ── Elevation helpers ─────────────────────────────────────────────────────────

/// Compute effective surface thickness: if elevation data is present and the
/// configured thickness is the compiled default (4), auto-increase to 32 so
/// hillsides aren't hollow.
pub(super) fn effective_thickness(configured: i32, has_elevation: bool) -> i32 {
    if has_elevation && configured == 4 {
        log::info!(
            "Elevation active — auto-increasing surface_thickness from {} to {}",
            configured,
            ELEVATION_SURFACE_THICKNESS,
        );
        ELEVATION_SURFACE_THICKNESS
    } else {
        configured
    }
}

/// Load elevation data from an optional path, logging the outcome.
pub(super) fn load_elevation(
    path: Option<&std::path::Path>,
    vertical_scale: f64,
) -> Option<elevation::ElevationData> {
    let path = path?;
    match elevation::ElevationData::from_path(path) {
        Ok(data) => {
            log::info!("Elevation enabled (vertical_scale={:.3})", vertical_scale);
            Some(data)
        }
        Err(e) => {
            log::warn!("Could not load elevation data: {e} — falling back to flat terrain");
            None
        }
    }
}

// ── Tile processing (ARC-002 dedup) ───────────────────────────────────────────

/// Process one tile's terrain fill + OSM-feature overlay into `world`.
///
/// Shared by the (now edition-agnostic) outer tile loop in
/// [`super::run_pipeline_streaming`]. The caller is responsible for:
/// 1. calling `world.set_tile_bounds(...)` with this tile's chunk rect
///    (so backends that enforce bounds write only in-tile blocks);
/// 2. calling `world.flush_tile()` after this returns (so streaming
///    backends drain to disk before the next tile).
///
/// The body is the ~300 LOC of terrain-fill rayon loop, spatial-filter
/// bucketing, relation bbox-overlap filtering, `RenderContext`/`TileWays`
/// assembly, and the `render_osm_features` call that previously existed
/// byte-for-byte duplicated in the Bedrock and Java branches.
#[allow(clippy::too_many_arguments)]
pub fn process_tile(
    world: &mut dyn WorldWriter,
    tile_cx0: i32,
    tile_cx1: i32,
    tile_cz0: i32,
    tile_cz1: i32,
    height_map: &HeightMap,
    surface: i32,
    surface_thickness: i32,
    spatial_index: &SpatialIndex,
    resolved_ways: &[(&osm::OsmWay, Vec<(i32, i32)>)],
    resolved_relations: &[ResolvedRelation],
    data: &osm::OsmData,
    params: &ConvertParams,
    conv: &CoordConverter,
) -> Result<()> {
    let tile_min_x = tile_cx0 * 16;
    let tile_max_x = (tile_cx1 + 1) * 16 - 1;
    let tile_min_z = tile_cz0 * 16;
    let tile_max_z = (tile_cz1 + 1) * 16 - 1;

    // Terrain fill (parallel rayon). Each chunk column gets a
    // bedrock → stone → dirt → grass layer stack capped at the surface Y.
    let tile_chunks: Vec<(i32, i32)> = (tile_cx0..=tile_cx1)
        .flat_map(|cx| (tile_cz0..=tile_cz1).map(move |cz| (cx, cz)))
        .collect();

    let filled: Vec<((i32, i32), ChunkData)> = tile_chunks
        .par_iter()
        .map(|&(cx, cz)| {
            let mut chunk = ChunkData::new();
            for lx in 0..16i32 {
                for lz in 0..16i32 {
                    let bx = cx * 16 + lx;
                    let bz = cz * 16 + lz;
                    let sy = height_map.get(bx, bz);
                    let base_y = (sy - surface_thickness).max(MIN_Y);
                    chunk.set(lx, base_y, lz, Block::Bedrock);
                    for y in (base_y + 1)..(sy - 1).max(base_y + 1) {
                        chunk.set(lx, y, lz, Block::Stone);
                    }
                    if sy > base_y + 1 {
                        chunk.set(lx, sy - 1, lz, Block::Dirt);
                    }
                    chunk.set(lx, sy, lz, Block::GrassBlock);
                }
            }
            ((cx, cz), chunk)
        })
        .collect();

    for ((cx, cz), chunk) in filled {
        world.insert_chunk(cx, cz, chunk);
    }

    // Spatial filter: find way indices intersecting this tile.
    let tile_idx_set: HashSet<usize> = spatial_index
        .query_rect(tile_min_x, tile_min_z, tile_max_x, tile_max_z)
        .into_iter()
        .collect();

    let filter_bucket = |bucket: &Vec<usize>| -> Vec<usize> {
        bucket
            .iter()
            .copied()
            .filter(|wi| tile_idx_set.contains(wi))
            .collect()
    };

    let tile_landuse = filter_bucket(&spatial_index.landuse);
    let tile_waterways = filter_bucket(&spatial_index.waterways);
    let tile_railways = filter_bucket(&spatial_index.railways);
    let tile_highways = filter_bucket(&spatial_index.highways);
    let tile_barriers = filter_bucket(&spatial_index.barriers);
    let tile_buildings = filter_bucket(&spatial_index.buildings);
    let tile_pois = filter_bucket(&spatial_index.pois);
    let tile_address = filter_bucket(&spatial_index.address);

    // Filter relations whose outer polygon bounding box overlaps this tile.
    //
    // Using bbox overlap (rather than checking whether any vertex lies
    // inside the tile) ensures that a large relation whose outer ring
    // spans multiple tiles is included in every tile it visually covers,
    // even when none of its vertices happen to fall inside a particular
    // tile.
    let tile_relations: Vec<&ResolvedRelation> = resolved_relations
        .iter()
        .filter(|rel| {
            rel.outers.iter().any(|outer| {
                // Compute the outer ring's axis-aligned bounding box.
                let (rel_min_x, rel_max_x, rel_min_z, rel_max_z) = outer.iter().fold(
                    (i32::MAX, i32::MIN, i32::MAX, i32::MIN),
                    |(mn_x, mx_x, mn_z, mx_z), &(x, z)| {
                        (mn_x.min(x), mx_x.max(x), mn_z.min(z), mx_z.max(z))
                    },
                );
                // Two axis-aligned boxes overlap iff they overlap on both axes.
                rel_min_x <= tile_max_x
                    && rel_max_x >= tile_min_x
                    && rel_min_z <= tile_max_z
                    && rel_max_z >= tile_min_z
            })
        })
        .collect();

    let ctx = RenderContext {
        resolved_ways,
        resolved_relations,
        data,
        params,
        height_map,
        conv,
        spatial_index,
        surface,
    };
    let tile_ways = TileWays {
        landuse: &tile_landuse,
        waterways: &tile_waterways,
        railways: &tile_railways,
        highways: &tile_highways,
        barriers: &tile_barriers,
        buildings: &tile_buildings,
        pois: &tile_pois,
        address: &tile_address,
        relations: &tile_relations,
        tile_bounds: Some((tile_min_x, tile_min_z, tile_max_x, tile_max_z)),
    };
    render_osm_features(world, &ctx, &tile_ways);

    Ok(())
}

// ── Terrain chunk fill (shared by in-memory + streaming terrain paths) ─────────

/// Fill a single terrain chunk with biome-appropriate blocks.
///
/// Shared by [`run_terrain_only`] (in-memory) and [`run_terrain_only_to_disk`]
/// (streaming).
#[allow(clippy::too_many_arguments)]
fn fill_terrain_chunk(
    cx: i32,
    cz: i32,
    elevation_data: &Option<elevation::ElevationData>,
    conv: &CoordConverter,
    sea: i32,
    snow_line: i32,
    vertical_scale: f64,
    surface_thickness: i32,
) -> TerrainChunkResult {
    let mut chunk = ChunkData::new();
    let mut local_heights: Vec<((i32, i32), i32)> = Vec::with_capacity(256);
    for lx in 0..16i32 {
        for lz in 0..16i32 {
            let bx = cx * 16 + lx;
            let bz = cz * 16 + lz;
            let sy = compute_surface_y(bx, bz, elevation_data, conv, sea, vertical_scale);

            if sy <= sea {
                let base_y = (sy - surface_thickness).max(MIN_Y);
                chunk.set(lx, base_y, lz, Block::Bedrock);
                for y in (base_y + 1)..sy {
                    chunk.set(lx, y, lz, Block::Stone);
                }
                chunk.set(lx, sy, lz, Block::Sand);
                for y in (sy + 1)..=sea {
                    chunk.set(lx, y, lz, Block::Water);
                }
                local_heights.push(((bx, bz), sea));
            } else if sy <= sea + 3 {
                let base_y = (sy - surface_thickness).max(MIN_Y);
                chunk.set(lx, base_y, lz, Block::Bedrock);
                for y in (base_y + 1)..(sy - 1).max(base_y + 1) {
                    chunk.set(lx, y, lz, Block::Stone);
                }
                if sy > base_y + 1 {
                    chunk.set(lx, sy - 1, lz, Block::Sand);
                }
                chunk.set(lx, sy, lz, Block::Sand);
                local_heights.push(((bx, bz), sy));
            } else if sy >= sea + snow_line {
                let base_y = (sy - surface_thickness).max(MIN_Y);
                chunk.set(lx, base_y, lz, Block::Bedrock);
                for y in (base_y + 1)..sy {
                    chunk.set(lx, y, lz, Block::Stone);
                }
                chunk.set(lx, sy, lz, Block::Stone);
                let snow_y = (sy + 1).min(world::MAX_Y);
                chunk.set(lx, snow_y, lz, Block::SnowLayer);
                local_heights.push(((bx, bz), snow_y));
            } else {
                let base_y = (sy - surface_thickness).max(MIN_Y);
                chunk.set(lx, base_y, lz, Block::Bedrock);
                for y in (base_y + 1)..(sy - 1).max(base_y + 1) {
                    chunk.set(lx, y, lz, Block::Stone);
                }
                if sy > base_y + 1 {
                    chunk.set(lx, sy - 1, lz, Block::Dirt);
                }
                chunk.set(lx, sy, lz, Block::GrassBlock);
                local_heights.push(((bx, bz), sy));
            }
        }
    }
    ((cx, cz), chunk, local_heights)
}

// ── Terrain-only entry points ─────────────────────────────────────────────────

/// Run the terrain-only pipeline: SRTM elevation → Bedrock world in memory.
///
/// Fills every block column in the bbox with biome-appropriate terrain:
/// - **underwater** (sy ≤ sea_level): stone fill → sand seafloor → water to sea_level
/// - **beach** (sea_level < sy ≤ sea_level + 3): sand
/// - **normal** (sea_level + 3 < sy < sea_level + snow_line): dirt + grass
/// - **alpine** (sy ≥ sea_level + snow_line): stone + thin snow layer
///
/// Returns `(world, spawn_x, spawn_y, spawn_z)`.
///
/// Note: for large inputs prefer [`run_terrain_only_to_disk`] which streams
/// tiles to LevelDB rather than accumulating all chunks in memory.
#[allow(dead_code)] // re-exported public API (pipeline::run_terrain_only); legacy in-memory terrain path
pub fn run_terrain_only(
    params: &TerrainParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(Box<dyn WorldWriter>, i32, i32, i32)> {
    let (min_lat, min_lon, max_lat, max_lon) = params.bbox;
    let origin_lat = (min_lat + max_lat) / 2.0;
    let origin_lon = (min_lon + max_lon) / 2.0;

    log::info!(
        "Terrain-only: bbox=({:.5},{:.5},{:.5},{:.5}) origin=({:.5},{:.5}) scale={} sea_level={}",
        min_lat,
        min_lon,
        max_lat,
        max_lon,
        origin_lat,
        origin_lon,
        params.scale,
        params.sea_level
    );

    let conv = CoordConverter::new(origin_lat, origin_lon, params.scale);
    let elevation_data = load_elevation(params.elevation.as_deref(), params.vertical_scale);

    let corners = [
        conv.to_block_xz(min_lat, min_lon),
        conv.to_block_xz(min_lat, max_lon),
        conv.to_block_xz(max_lat, min_lon),
        conv.to_block_xz(max_lat, max_lon),
    ];
    let (min_x, max_x, min_z, max_z) = bounding_box(&corners);

    let min_cx = min_x.div_euclid(16);
    let max_cx = max_x.div_euclid(16);
    let min_cz = min_z.div_euclid(16);
    let max_cz = max_z.div_euclid(16);

    let chunk_coords: Vec<(i32, i32)> = (min_cx..=max_cx)
        .flat_map(|cx| (min_cz..=max_cz).map(move |cz| (cx, cz)))
        .collect();

    log::info!(
        "Terrain bounds: x=[{}..{}] z=[{}..{}] ({} x {} blocks, {} chunks)",
        min_x,
        max_x,
        min_z,
        max_z,
        max_x - min_x,
        max_z - min_z,
        chunk_coords.len()
    );

    progress_cb(0.15, "Filling terrain");

    let sea = params.sea_level;
    let snow_line = params.snow_line;
    let vertical_scale = params.vertical_scale;
    let surface_thickness = effective_thickness(params.surface_thickness, elevation_data.is_some());

    type ChunkResult = ((i32, i32), ChunkData, Vec<((i32, i32), i32)>);
    let filled: Vec<ChunkResult> = chunk_coords
        .par_iter()
        .map(|&(cx, cz)| {
            fill_terrain_chunk(
                cx,
                cz,
                &elevation_data,
                &conv,
                sea,
                snow_line,
                vertical_scale,
                surface_thickness,
            )
        })
        .collect();

    progress_cb(0.85, "Building world");

    let mut world = params.edition.create_world(&params.output);
    let mut height_map = HeightMap::new(sea);
    for ((cx, cz), chunk, heights) in filled {
        world.insert_chunk(cx, cz, chunk);
        for ((bx, bz), sy) in heights {
            height_map.insert(bx, bz, sy);
        }
    }

    if params.elevation_smoothing > 0 {
        height_map.smooth(params.elevation_smoothing);
    }

    let (spawn_x, spawn_z) = if let (Some(sx), Some(sz)) = (params.spawn_x, params.spawn_z) {
        (sx, sz)
    } else if let (Some(lat), Some(lon)) = (params.spawn_lat, params.spawn_lon) {
        conv.to_block_xz(lat, lon)
    } else {
        (0, 0)
    };
    let spawn_y = params
        .spawn_y
        .unwrap_or_else(|| height_map.get(spawn_x, spawn_z) + 1);

    log::info!("Spawn: ({}, {}, {})", spawn_x, spawn_y, spawn_z);
    progress_cb(0.90, "Terrain complete");
    Ok((world, spawn_x, spawn_y, spawn_z))
}

/// Run the terrain-only pipeline and save the world to disk.
///
/// Uses tiled streaming to bound memory usage.
pub fn run_terrain_only_to_disk(
    params: &TerrainParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<()> {
    let (min_lat, min_lon, max_lat, max_lon) = params.bbox;
    let origin_lat = (min_lat + max_lat) / 2.0;
    let origin_lon = (min_lon + max_lon) / 2.0;

    log::info!(
        "Terrain-only (streaming): bbox=({:.5},{:.5},{:.5},{:.5}) scale={} sea_level={}",
        min_lat,
        min_lon,
        max_lat,
        max_lon,
        params.scale,
        params.sea_level
    );

    let conv = CoordConverter::new(origin_lat, origin_lon, params.scale);
    let elevation_data = load_elevation(params.elevation.as_deref(), params.vertical_scale);

    let corners = [
        conv.to_block_xz(min_lat, min_lon),
        conv.to_block_xz(min_lat, max_lon),
        conv.to_block_xz(max_lat, min_lon),
        conv.to_block_xz(max_lat, max_lon),
    ];
    let (min_x, max_x, min_z, max_z) = bounding_box(&corners);

    let min_cx = min_x.div_euclid(16);
    let max_cx = max_x.div_euclid(16);
    let min_cz = min_z.div_euclid(16);
    let max_cz = max_z.div_euclid(16);

    let total_chunks = ((max_cx - min_cx + 1) as u64) * ((max_cz - min_cz + 1) as u64);
    log::info!(
        "Terrain bounds: x=[{}..{}] z=[{}..{}] ({} chunks)",
        min_x,
        max_x,
        min_z,
        max_z,
        total_chunks
    );

    std::fs::create_dir_all(&params.output)
        .with_context(|| format!("creating output dir {}", params.output.display()))?;

    let sea = params.sea_level;
    let snow_line = params.snow_line;
    let vertical_scale = params.vertical_scale;
    let surface_thickness = effective_thickness(params.surface_thickness, elevation_data.is_some());

    let mut height_map = HeightMap::new(sea);

    let cx_tiles = ((max_cx - min_cx + TILE_CHUNKS) / TILE_CHUNKS) as u64;
    let cz_tiles = ((max_cz - min_cz + TILE_CHUNKS) / TILE_CHUNKS) as u64;
    let total_tiles = cx_tiles * cz_tiles;
    let mut tile_idx = 0u64;
    let mut last_logged_pct = 0u64;

    if params.edition == Edition::Bedrock {
        // Bedrock: stream tiles to LevelDB via ChunkWriter
        let db_path = params.output.join("db");
        std::fs::create_dir_all(&db_path)?;
        let chunk_writer = bedrock::ChunkWriter::open(db_path)?;
        let empty_signs: std::collections::HashMap<(i32, i32, i32), i32> =
            std::collections::HashMap::new();
        let empty_dirs: std::collections::HashMap<(i32, i32, i32), i32> =
            std::collections::HashMap::new();

        let mut tcx0 = min_cx;
        while tcx0 <= max_cx {
            let tcx1 = (tcx0 + TILE_CHUNKS - 1).min(max_cx);
            let mut tcz0 = min_cz;
            while tcz0 <= max_cz {
                let tcz1 = (tcz0 + TILE_CHUNKS - 1).min(max_cz);

                let progress = tile_idx as f32 / total_tiles as f32 * 0.90;
                progress_cb(
                    progress,
                    &format!("Filling terrain tile {}/{total_tiles}", tile_idx + 1),
                );

                let pct = tile_idx * 100 / total_tiles.max(1);
                if pct / 10 > last_logged_pct / 10 {
                    last_logged_pct = pct;
                    log::info!(
                        "Terrain tile progress: {pct}% ({}/{total_tiles})",
                        tile_idx + 1
                    );
                }

                let tile_coords: Vec<(i32, i32)> = (tcx0..=tcx1)
                    .flat_map(|cx| (tcz0..=tcz1).map(move |cz| (cx, cz)))
                    .collect();

                type ChunkResult = ((i32, i32), ChunkData, Vec<((i32, i32), i32)>);
                let filled: Vec<ChunkResult> = tile_coords
                    .par_iter()
                    .map(|&(cx, cz)| {
                        fill_terrain_chunk(
                            cx,
                            cz,
                            &elevation_data,
                            &conv,
                            sea,
                            snow_line,
                            vertical_scale,
                            surface_thickness,
                        )
                    })
                    .collect();

                for ((cx, cz), ref chunk, heights) in filled {
                    chunk_writer
                        .write_chunk(cx, cz, chunk, None, &empty_signs, &empty_dirs)
                        .with_context(|| format!("writing chunk ({cx},{cz})"))?;
                    for ((bx, bz), sy) in heights {
                        height_map.insert(bx, bz, sy);
                    }
                }

                tile_idx += 1;
                tcz0 += TILE_CHUNKS;
            }
            tcx0 += TILE_CHUNKS;
        }

        if params.elevation_smoothing > 0 {
            height_map.smooth(params.elevation_smoothing);
        }

        progress_cb(0.92, "Flushing to disk");
        chunk_writer.finish()?;

        let (spawn_x, spawn_z) = if let (Some(sx), Some(sz)) = (params.spawn_x, params.spawn_z) {
            (sx, sz)
        } else if let (Some(lat), Some(lon)) = (params.spawn_lat, params.spawn_lon) {
            conv.to_block_xz(lat, lon)
        } else {
            (0, 0)
        };
        let spawn_y = params
            .spawn_y
            .unwrap_or_else(|| height_map.get(spawn_x, spawn_z) + 1);

        log::info!("Spawn: ({}, {}, {})", spawn_x, spawn_y, spawn_z);

        bedrock::BedrockWorld::new(&params.output).write_level_dat(spawn_x, spawn_y, spawn_z)?;
    } else {
        // Java: stream tiles to lazily-written region files (ARC-001), so a
        // city-scale terrain bbox no longer accumulates the whole world in RAM.
        let mut world: Box<dyn WorldWriter> = Box::new(crate::anvil::JavaWorld::new_streaming(
            &params.output,
            min_cx,
            max_cx,
            min_cz,
            max_cz,
        )?);

        let mut tcx0 = min_cx;
        while tcx0 <= max_cx {
            let tcx1 = (tcx0 + TILE_CHUNKS - 1).min(max_cx);
            let mut tcz0 = min_cz;
            while tcz0 <= max_cz {
                let tcz1 = (tcz0 + TILE_CHUNKS - 1).min(max_cz);

                let progress = tile_idx as f32 / total_tiles as f32 * 0.90;
                progress_cb(
                    progress,
                    &format!("Filling terrain tile {}/{total_tiles}", tile_idx + 1),
                );

                let pct = tile_idx * 100 / total_tiles.max(1);
                if pct / 10 > last_logged_pct / 10 {
                    last_logged_pct = pct;
                    log::info!(
                        "Terrain tile progress: {pct}% ({}/{total_tiles})",
                        tile_idx + 1
                    );
                }

                // Scope the streaming writer to this tile before filling it.
                world.set_tile_bounds(tcx0, tcx1, tcz0, tcz1);

                let tile_coords: Vec<(i32, i32)> = (tcx0..=tcx1)
                    .flat_map(|cx| (tcz0..=tcz1).map(move |cz| (cx, cz)))
                    .collect();

                let filled: Vec<TerrainChunkResult> = tile_coords
                    .par_iter()
                    .map(|&(cx, cz)| {
                        fill_terrain_chunk(
                            cx,
                            cz,
                            &elevation_data,
                            &conv,
                            sea,
                            snow_line,
                            vertical_scale,
                            surface_thickness,
                        )
                    })
                    .collect();

                for ((cx, cz), chunk, heights) in filled {
                    world.insert_chunk(cx, cz, chunk);
                    for ((bx, bz), sy) in heights {
                        height_map.insert(bx, bz, sy);
                    }
                }

                // Drain the tile's chunks into region buffers and seal any
                // completed region files before the next tile.
                world.flush_tile()?;

                tile_idx += 1;
                tcz0 += TILE_CHUNKS;
            }
            tcx0 += TILE_CHUNKS;
        }

        if params.elevation_smoothing > 0 {
            height_map.smooth(params.elevation_smoothing);
        }

        let (spawn_x, spawn_z) = if let (Some(sx), Some(sz)) = (params.spawn_x, params.spawn_z) {
            (sx, sz)
        } else if let (Some(lat), Some(lon)) = (params.spawn_lat, params.spawn_lon) {
            conv.to_block_xz(lat, lon)
        } else {
            (0, 0)
        };
        let spawn_y = params
            .spawn_y
            .unwrap_or_else(|| height_map.get(spawn_x, spawn_z) + 1);

        log::info!("Spawn: ({}, {}, {})", spawn_x, spawn_y, spawn_z);

        progress_cb(0.92, "Saving world");
        world.save(spawn_x, spawn_y, spawn_z)?;
    }

    progress_cb(1.0, "Terrain world complete");
    log::info!(
        "Done! Streamed {} chunks to '{}'.",
        total_chunks,
        params.output.display()
    );
    Ok(())
}

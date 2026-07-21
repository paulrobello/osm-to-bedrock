//! In-memory preview pipeline entry points.
//!
//! Lightweight surface-only preview plus the in-memory full-feature pipeline
//! used by the server's preview endpoint. Distinct from the streaming
//! [`super::run_conversion`] / [`super::run_conversion_from_data`] entry
//! points: those tile through LevelDB/Anvil and never hold the whole world
//! in memory; the preview path allocates a `Box<dyn WorldWriter>` and returns
//! it to the caller.
//!
//! ## Variant map (ARC-003)
//!
//! | Function | Input | Output | Pipeline |
//! |----------|-------|--------|----------|
//! | [`run_conversion_preview`] | file path (`params.input`) | in-memory world | full feature |
//! | [`run_preview_from_data`] | pre-fetched `OsmData` | in-memory world | full feature |
//! | [`run_surface_preview`] | pre-fetched `OsmData` | flat `Vec<(x,z,y,name)>` grid | surface-only |
//!
//! The `*_preview` and `*_from_data` naming is preserved for API stability;
//! the distinctions are real (input source + output shape), not redundant.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use rayon::prelude::*;

use crate::blocks::{self, Block};
use crate::convert::{CoordConverter, rasterize_line, rasterize_polygon};
use crate::elevation;
use crate::geometry::road_perpendicular;
use crate::osm;
use crate::params::ConvertParams;
use crate::spatial::{HeightMap, ResolvedRelation, SpatialIndex, compute_surface_y};
use crate::world::{ChunkData, MIN_Y, WorldWriter};

use super::render::{RenderContext, TileWays, render_osm_features};
use super::terrain::{
    compute_terrain_bounds, effective_thickness, load_elevation, resolve_relations, resolve_spawn,
    resolve_ways,
};

/// Run the conversion pipeline and return the world in memory (preview).
///
/// Used by the preview endpoint (server) to inspect the world in memory.
/// Uses the same [`render_osm_features`] function as the streaming pipeline,
/// so preview results include signs, POI markers, and barriers.
pub fn run_conversion_preview(
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(Box<dyn WorldWriter>, i32, i32, i32)> {
    if params.scale <= 0.0 {
        bail!("scale must be positive");
    }
    progress_cb(0.0, "Parsing OSM data");
    let path = params.input.as_deref().ok_or_else(|| {
        anyhow::anyhow!("ConvertParams.input is required for file-based conversion")
    })?;
    log::info!("Reading {}", path.display());
    let data = crate::osm::parse_osm_file(path)?;
    if data.ways().is_empty() {
        bail!("No ways found in OSM file.");
    }
    run_pipeline(data, params, progress_cb)
}

/// Run the preview pipeline from pre-fetched `OsmData` (e.g. from Overpass cache).
///
/// Same as [`run_conversion_preview`] but takes `OsmData` directly instead of
/// reading from a file.
pub fn run_preview_from_data(
    data: osm::OsmData,
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(Box<dyn WorldWriter>, i32, i32, i32)> {
    if data.ways().is_empty() {
        bail!("No ways found in OSM data.");
    }
    run_pipeline(data, params, progress_cb)
}

/// Lightweight surface-only preview: computes height map + classifies each
/// (x, z) position by feature type without allocating any `ChunkData`.
///
/// Returns `Vec<(x, z, y, type_name)>` — the same shape as `BedrockWorld::surface_blocks()`
/// but orders of magnitude faster for large areas.
#[allow(clippy::type_complexity)]
pub fn run_surface_preview(
    data: osm::OsmData,
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(Vec<(i32, i32, i32, String)>, i32, i32, i32)> {
    if data.ways().is_empty() {
        bail!("No ways found in OSM data.");
    }

    let (origin_lat, origin_lon) = {
        let (min_lat, min_lon, max_lat, max_lon) = data
            .bounds()
            .ok_or_else(|| anyhow::anyhow!("OSM data has no bounds"))?;
        ((min_lat + max_lat) / 2.0, (min_lon + max_lon) / 2.0)
    };

    let conv = CoordConverter::new(origin_lat, origin_lon, params.scale);
    let elevation_data = load_elevation(params.elevation.as_deref(), params.vertical_scale);

    progress_cb(0.10, "Computing bounds");
    let (min_x, max_x, min_z, max_z) = compute_terrain_bounds(&data, &conv);
    let min_cx = min_x.div_euclid(16);
    let max_cx = max_x.div_euclid(16);
    let min_cz = min_z.div_euclid(16);
    let max_cz = max_z.div_euclid(16);

    // Compute height map (parallel)
    progress_cb(0.20, "Computing height map");
    let surface = params.sea_level;
    let height_map: HeightMap = {
        let all_cols: Vec<(i32, i32)> = (min_cx..=max_cx)
            .flat_map(|cx| {
                (min_cz..=max_cz).flat_map(move |cz| {
                    (0..16i32)
                        .flat_map(move |lx| (0..16i32).map(move |lz| (cx * 16 + lx, cz * 16 + lz)))
                })
            })
            .collect();

        let heights: Vec<((i32, i32), i32)> = all_cols
            .par_iter()
            .map(|&(bx, bz)| {
                let sy = compute_surface_y(
                    bx,
                    bz,
                    &elevation_data,
                    &conv,
                    surface,
                    params.vertical_scale,
                );
                ((bx, bz), sy)
            })
            .collect();

        let mut hm = HeightMap::with_bounds(
            min_cx * 16,
            min_cz * 16,
            max_cx * 16 + 15,
            max_cz * 16 + 15,
            surface,
        );
        for ((bx, bz), sy) in heights {
            hm.insert(bx, bz, sy);
        }
        if params.elevation_smoothing > 0 && elevation_data.is_some() {
            hm.smooth(params.elevation_smoothing);
        }
        hm
    };

    // Resolve ways + build spatial index
    progress_cb(0.40, "Classifying features");
    let resolved_ways = resolve_ways(&data, &conv);
    let spatial_index = SpatialIndex::build(&resolved_ways);

    // Classify each (x, z) by rasterizing features onto a 2D surface grid.
    // Priority (later overwrites earlier): grass < landuse < water < road < building
    let mut surface_type: HashMap<(i32, i32), &str> = HashMap::new();

    // Landuse polygons
    for &wi in &spatial_index.landuse {
        let (way, pts) = &resolved_ways[wi];
        if pts.len() >= 3 {
            let tag = way
                .tags
                .get("landuse")
                .or_else(|| way.tags.get("natural"))
                .or_else(|| way.tags.get("leisure"))
                .map(|s| s.as_str())
                .unwrap_or("grass");
            let label = match tag {
                "forest" | "wood" | "tree_row" => "forest",
                "water" | "wetland" | "reservoir" | "basin" => "water",
                "residential" | "commercial" | "industrial" | "retail" => "urban",
                "farmland" | "farm" | "meadow" | "grass" | "village_green" => "grass",
                "park" | "garden" | "recreation_ground" | "playground" => "park",
                _ => "landuse",
            };
            for (bx, bz) in rasterize_polygon(pts) {
                surface_type.insert((bx, bz), label);
            }
        }
    }

    // Waterways (lines with width)
    for &wi in &spatial_index.waterways {
        let (_way, pts) = &resolved_ways[wi];
        for seg in pts.windows(2) {
            let center = rasterize_line(seg[0].0, seg[0].1, seg[1].0, seg[1].1);
            for (cx, cz) in center {
                for dx in -2..=2 {
                    for dz in -2..=2 {
                        surface_type.insert((cx + dx, cz + dz), "water");
                    }
                }
            }
        }
    }

    // Roads (lines with perpendicular expansion)
    for &wi in &spatial_index.highways {
        let (way, pts) = &resolved_ways[wi];
        let hw_type = way
            .tags
            .get("highway")
            .map(|s| s.as_str())
            .unwrap_or("residential");
        let style = blocks::highway_to_style(hw_type, None);
        let hw = style.half_width;
        for seg in pts.windows(2) {
            let (x0, z0) = seg[0];
            let (x1, z1) = seg[1];
            let center = rasterize_line(x0, z0, x1, z1);
            let (px, pz) = road_perpendicular(x0, z0, x1, z1);
            for (cx, cz) in center {
                for d in -hw..=hw {
                    surface_type.insert((cx + px * d, cz + pz * d), "road");
                }
            }
        }
    }

    // Railways (narrow lines)
    for &wi in &spatial_index.railways {
        let (_way, pts) = &resolved_ways[wi];
        for seg in pts.windows(2) {
            for (bx, bz) in rasterize_line(seg[0].0, seg[0].1, seg[1].0, seg[1].1) {
                surface_type.insert((bx, bz), "railway");
            }
        }
    }

    // Buildings (filled polygons — mark footprint for 3D extrusion below)
    let mut building_footprints: Vec<Vec<(i32, i32)>> = Vec::new();
    for &wi in &spatial_index.buildings {
        let (_way, pts) = &resolved_ways[wi];
        if pts.len() >= 3 {
            let filled = rasterize_polygon(pts);
            for &(bx, bz) in &filled {
                surface_type.insert((bx, bz), "building");
            }
            building_footprints.push(filled);
        }
    }

    progress_cb(0.80, "Building surface grid");

    // Build the surface grid.  For large areas, sample grass blocks on a grid
    // while keeping ALL feature blocks at full resolution.
    let total_cols = (max_x - min_x + 1) as u64 * (max_z - min_z + 1) as u64;
    let max_grass: u64 = 2_000_000;
    let stride = if total_cols > max_grass {
        ((total_cols as f64 / max_grass as f64).sqrt().ceil() as i32).max(2)
    } else {
        1
    };
    if stride > 1 {
        log::info!("Surface preview: {total_cols} columns, sampling grass every {stride} blocks");
    }

    let bld_height = params.building_height;
    let mut result: Vec<(i32, i32, i32, String)> = Vec::new();

    // Emit all feature blocks at full resolution
    for (&(bx, bz), &typ) in &surface_type {
        if bx >= min_x && bx <= max_x && bz >= min_z && bz <= max_z {
            let y = height_map.get(bx, bz);
            let name = match typ {
                "road" => "SmoothStoneSlab",
                "building" => "StoneBrick", // floor — walls added below
                "water" => "Water",
                "forest" => "OakLeaves",
                "park" | "urban" | "grass" | "landuse" => "GrassBlock",
                "railway" => "IronBlock",
                other => other,
            };
            result.push((bx, bz, y, name.to_string()));
        }
    }

    // Extrude buildings: emit wall blocks at perimeter columns from Y+1 to Y+height
    for footprint in &building_footprints {
        let fp_set: HashSet<(i32, i32)> = footprint.iter().copied().collect();
        for &(bx, bz) in footprint {
            if bx < min_x || bx > max_x || bz < min_z || bz > max_z {
                continue;
            }
            let y = height_map.get(bx, bz);
            // Check if this block is on the perimeter (any neighbor not in footprint)
            let is_edge = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|&(dx, dz)| !fp_set.contains(&(bx + dx, bz + dz)));
            if is_edge {
                // Wall column
                for dy in 1..=bld_height {
                    result.push((bx, bz, y + dy, "StoneBrick".to_string()));
                }
            } else {
                // Roof at top
                result.push((bx, bz, y + bld_height, "StoneBrick".to_string()));
            }
        }
    }

    // Emit grass blocks on a grid (stride-sampled for large areas)
    let mut bx = min_x;
    while bx <= max_x {
        let mut bz = min_z;
        while bz <= max_z {
            if !surface_type.contains_key(&(bx, bz)) {
                let y = height_map.get(bx, bz);
                result.push((bx, bz, y, "GrassBlock".to_string()));
            }
            bz += stride;
        }
        bx += stride;
    }

    progress_cb(0.90, "Computing spawn");
    let (spawn_x, spawn_y, spawn_z) =
        resolve_spawn(params, &conv, &height_map, min_cx, max_cx, min_cz, max_cz);

    progress_cb(1.0, "Surface preview complete");
    Ok((result, spawn_x, spawn_y, spawn_z))
}

/// Inner in-memory pipeline: `OsmData` → world.
///
/// Used only by [`run_conversion_preview`] and [`run_preview_from_data`].
/// For large inputs this loads all chunk data into memory; the streaming
/// pipeline ([`super::run_pipeline_streaming`]) should be preferred for
/// production conversions. Now calls [`render_osm_features`] so it has full
/// feature parity with the streaming pipeline.
fn run_pipeline(
    data: osm::OsmData,
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(Box<dyn WorldWriter>, i32, i32, i32)> {
    let (origin_lat, origin_lon) = {
        let (min_lat, min_lon, max_lat, max_lon) = data
            .bounds()
            .ok_or_else(|| anyhow::anyhow!("OSM file has no nodes"))?;
        ((min_lat + max_lat) / 2.0, (min_lon + max_lon) / 2.0)
    };

    log::info!(
        "Origin: lat={:.6} lon={:.6}, scale={} m/block, surface y={}",
        origin_lat,
        origin_lon,
        params.scale,
        params.sea_level
    );

    let conv = CoordConverter::new(origin_lat, origin_lon, params.scale);

    // Load elevation data.
    let elevation_data: Option<elevation::ElevationData> =
        load_elevation(params.elevation.as_deref(), params.vertical_scale);
    let thickness = effective_thickness(params.surface_thickness, elevation_data.is_some());

    let (min_x, max_x, min_z, max_z) = compute_terrain_bounds(&data, &conv);
    progress_cb(0.10, "Computing terrain bounds");

    log::info!(
        "Terrain bounds: x=[{}..{}] z=[{}..{}] ({} x {} blocks)",
        min_x,
        max_x,
        min_z,
        max_z,
        max_x - min_x,
        max_z - min_z
    );

    let min_cx = min_x.div_euclid(16);
    let max_cx = max_x.div_euclid(16);
    let min_cz = min_z.div_euclid(16);
    let max_cz = max_z.div_euclid(16);

    let mut terrain_chunks: HashSet<(i32, i32)> = HashSet::new();
    for cx in min_cx..=max_cx {
        for cz in min_cz..=max_cz {
            terrain_chunks.insert((cx, cz));
        }
    }

    log::info!("Filling terrain for {} chunks...", terrain_chunks.len());

    // Pass 2: fill base terrain (parallel via rayon)
    progress_cb(0.20, "Filling base terrain");
    let surface = params.sea_level;
    let mut height_map = HeightMap::new(surface);
    {
        let chunk_coords: Vec<(i32, i32)> = terrain_chunks.iter().copied().collect();
        type ChunkResult = ((i32, i32), ChunkData, Vec<((i32, i32), i32)>);
        let filled: Vec<ChunkResult> = chunk_coords
            .par_iter()
            .map(|&(cx, cz)| {
                let mut chunk = ChunkData::new();
                let mut local_heights: Vec<((i32, i32), i32)> = Vec::with_capacity(256);
                for lx in 0..16i32 {
                    for lz in 0..16i32 {
                        let bx = cx * 16 + lx;
                        let bz = cz * 16 + lz;
                        let sy = compute_surface_y(
                            bx,
                            bz,
                            &elevation_data,
                            &conv,
                            surface,
                            params.vertical_scale,
                        );
                        local_heights.push(((bx, bz), sy));
                        let base_y = (sy - thickness).max(MIN_Y);
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
                ((cx, cz), chunk, local_heights)
            })
            .collect();

        let mut world = params.edition.create_world(&params.output);
        for ((cx, cz), chunk, heights) in filled {
            world.insert_chunk(cx, cz, chunk);
            for ((bx, bz), sy) in heights {
                height_map.insert(bx, bz, sy);
            }
        }
        // world is dropped here; we reassemble below after feature rendering
        let _ = world;
    }

    if params.elevation_smoothing > 0 && elevation_data.is_some() {
        height_map.smooth(params.elevation_smoothing);
    }

    // Pass 3: overlay OSM features
    progress_cb(0.40, "Processing OSM features");
    log::info!("Processing {} ways...", data.ways().len());

    let resolved_ways = resolve_ways(&data, &conv);
    let resolved_relations = resolve_relations(&data, &conv);
    let spatial_index = SpatialIndex::build(&resolved_ways);
    log::info!(
        "Spatial index: {} highway, {} building, {} landuse, {} water, {} railway, {} barrier, {} poi, {} address ways",
        spatial_index.highways.len(),
        spatial_index.buildings.len(),
        spatial_index.landuse.len(),
        spatial_index.waterways.len(),
        spatial_index.railways.len(),
        spatial_index.barriers.len(),
        spatial_index.pois.len(),
        spatial_index.address.len(),
    );

    // Rebuild world and re-fill terrain (in one pass this time for the preview path)
    let chunk_coords: Vec<(i32, i32)> = terrain_chunks.iter().copied().collect();
    type ChunkResult = ((i32, i32), ChunkData, Vec<((i32, i32), i32)>);
    let filled: Vec<ChunkResult> = chunk_coords
        .par_iter()
        .map(|&(cx, cz)| {
            let mut chunk = ChunkData::new();
            let mut local_heights: Vec<((i32, i32), i32)> = Vec::with_capacity(256);
            for lx in 0..16i32 {
                for lz in 0..16i32 {
                    let bx = cx * 16 + lx;
                    let bz = cz * 16 + lz;
                    let sy = compute_surface_y(
                        bx,
                        bz,
                        &elevation_data,
                        &conv,
                        surface,
                        params.vertical_scale,
                    );
                    local_heights.push(((bx, bz), sy));
                    let base_y = (sy - thickness).max(MIN_Y);
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
            ((cx, cz), chunk, local_heights)
        })
        .collect();

    let mut world = params.edition.create_world(&params.output);
    let mut height_map = HeightMap::new(surface);
    for ((cx, cz), chunk, heights) in filled {
        world.insert_chunk(cx, cz, chunk);
        for ((bx, bz), sy) in heights {
            height_map.insert(bx, bz, sy);
        }
    }

    let all_relations: Vec<&ResolvedRelation> = resolved_relations.iter().collect();
    let ctx = RenderContext {
        resolved_ways: &resolved_ways,
        resolved_relations: &resolved_relations,
        data: &data,
        params,
        height_map: &height_map,
        conv: &conv,
        spatial_index: &spatial_index,
        surface,
        block_overrides: params.block_overrides.as_ref(),
    };
    let tile = TileWays {
        landuse: &spatial_index.landuse,
        waterways: &spatial_index.waterways,
        railways: &spatial_index.railways,
        highways: &spatial_index.highways,
        barriers: &spatial_index.barriers,
        buildings: &spatial_index.buildings,
        pois: &spatial_index.pois,
        address: &spatial_index.address,
        relations: &all_relations,
        tile_bounds: None,
    };
    render_osm_features(&mut *world, &ctx, &tile);

    // Compute spawn point
    let (spawn_x, spawn_y, spawn_z) =
        resolve_spawn(params, &conv, &height_map, min_cx, max_cx, min_cz, max_cz);
    log::info!("Spawn point: ({}, {}, {})", spawn_x, spawn_y, spawn_z);

    progress_cb(0.85, "Conversion complete");
    Ok((world, spawn_x, spawn_y, spawn_z))
}

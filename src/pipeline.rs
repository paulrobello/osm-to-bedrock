//! OSM-to-Bedrock conversion pipeline.
//!
//! ## Pipeline variants
//!
//! | Function | Use case | Memory model |
//! |----------|----------|-------------|
//! | [`run_conversion`] | CLI `convert` subcommand | Streaming (tile-based) |
//! | [`run_conversion_from_data`] | Server / Overpass flow | Streaming (tile-based) |
//! | [`run_conversion_preview`] | Server preview endpoint | In-memory world |
//! | [`run_terrain_only`] | In-memory terrain (legacy) | In-memory world |
//! | [`run_terrain_only_to_disk`] | CLI `terrain-convert` / server | Streaming (tile-based) |
//!
//! ## Shared rendering
//!
//! Both the in-memory (`run_pipeline`) and streaming (`run_pipeline_streaming`)
//! paths call [`render_osm_features`] to avoid code duplication.  The only
//! difference between the two is how chunks are flushed: the streaming path
//! drains each tile to LevelDB before allocating the next; the preview path
//! accumulates everything in a single `BedrockWorld`.

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    bedrock,
    blocks::{self, Block},
    convert::{
        self, CoordConverter, rasterize_line, rasterize_polygon, rasterize_polygon_with_holes,
    },
    elevation,
    geometry::{
        draw_bridge, draw_building, draw_road, draw_roof, draw_tunnel, draw_waterway,
        road_perpendicular,
    },
    nbt::encode_sign_block_entity,
    osm,
    params::{ConvertParams, TerrainParams},
    sign::{format_poi_sign, format_sign_text, nearest_road_vector, vec_to_sign_dir},
    spatial::{HeightMap, ResolvedRelation, SpatialIndex, TILE_CHUNKS, compute_surface_y},
};

// ── Type aliases ──────────────────────────────────────────────────────────────

/// Return type of [`fill_terrain_chunk`]: chunk coords, chunk data, and
/// a list of surface heights for each (bx, bz) column within the chunk.
type TerrainChunkResult = ((i32, i32), bedrock::ChunkData, Vec<((i32, i32), i32)>);

// ── Helpers shared within this module ─────────────────────────────────────────

/// Returns true if a way's first and last node ref are the same (closed polygon).
pub fn is_closed_way(refs: &[i64]) -> bool {
    refs.len() >= 4 && refs.first() == refs.last()
}

/// Deterministic hash for coordinate-based procedural generation.
pub fn coord_hash(x: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(374761393);
    h = h.wrapping_add((z as u32).wrapping_mul(668265263));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

/// Extract the best POI type label from a tag set.
///
/// Tries standard OSM keys first (`amenity`, `shop`, `tourism`, `leisure`,
/// `historic`), then falls back to any non-metadata tag value that could serve
/// as a meaningful label.
fn resolve_poi_type(tags: &std::collections::HashMap<String, String>) -> &str {
    // Standard OSM POI keys
    const POI_KEYS: &[&str] = &["amenity", "shop", "tourism", "leisure", "historic"];
    for key in POI_KEYS {
        if let Some(v) = tags.get(*key) {
            return v.as_str();
        }
    }
    // Fallback: pick the first tag whose key isn't a metadata/structural field
    const SKIP_KEYS: &[&str] = &[
        "name",
        "building",
        "building:height",
        "building:levels",
        "highway",
        "surface",
        "bridge",
        "tunnel",
        "railway",
        "waterway",
        "natural",
        "landuse",
        "addr:housenumber",
        "addr:street",
        "barrier",
    ];
    for (k, v) in tags {
        if !SKIP_KEYS.contains(&k.as_str()) && !v.is_empty() {
            return v.as_str();
        }
    }
    "poi"
}

/// Zip a directory into a `.mcworld` file (which is just a zip archive).
pub fn zip_directory(dir: &Path, output_zip: &Path) -> Result<()> {
    use std::fs;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    // Count total files first for progress reporting.
    fn count_files(dir: &Path) -> usize {
        let mut n = 0;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    n += count_files(&path);
                } else {
                    n += 1;
                }
            }
        }
        n
    }

    let total_files = count_files(dir);
    log::info!("Zipping {total_files} files to {}", output_zip.display());

    let file = fs::File::create(output_zip)?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut files_done: usize = 0;
    let mut last_logged_pct: usize = 0;

    // Walk the directory recursively
    fn add_dir_to_zip(
        zip_writer: &mut zip::ZipWriter<std::fs::File>,
        base: &Path,
        current: &Path,
        options: SimpleFileOptions,
        files_done: &mut usize,
        last_logged_pct: &mut usize,
        total_files: usize,
    ) -> Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            let rel = path.strip_prefix(base)?;
            let name = rel.to_string_lossy().to_string();

            if path.is_dir() {
                zip_writer.add_directory(format!("{name}/"), options)?;
                add_dir_to_zip(
                    zip_writer,
                    base,
                    &path,
                    options,
                    files_done,
                    last_logged_pct,
                    total_files,
                )?;
            } else {
                zip_writer.start_file(&name, options)?;
                let mut f = fs::File::open(&path)?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)?;
                zip_writer.write_all(&buf)?;

                *files_done += 1;
                if total_files > 0 {
                    let pct = *files_done * 100 / total_files;
                    if pct / 10 > *last_logged_pct / 10 {
                        *last_logged_pct = pct;
                        log::info!("Zip progress: {pct}% ({}/{total_files} files)", *files_done);
                    }
                }
            }
        }
        Ok(())
    }

    add_dir_to_zip(
        &mut zip_writer,
        dir,
        dir,
        options,
        &mut files_done,
        &mut last_logged_pct,
        total_files,
    )?;
    zip_writer.finish()?;

    let zip_size = fs::metadata(output_zip).map(|m| m.len()).unwrap_or(0);
    log::info!("Zip complete: {}", format_bytes(zip_size));
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ── Shared OSM feature rendering ──────────────────────────────────────────────

/// Shared context passed to [`render_osm_features`].
///
/// Contains everything needed to render a set of OSM ways into a world,
/// independent of whether the world is in-memory or tile-bounded.
pub struct RenderContext<'a> {
    pub resolved_ways: &'a [(&'a osm::OsmWay, Vec<(i32, i32)>)],
    /// Stored for potential future use (e.g. cross-tile relation rendering).
    #[allow(dead_code)]
    pub resolved_relations: &'a [ResolvedRelation<'a>],
    pub data: &'a osm::OsmData,
    pub params: &'a ConvertParams,
    pub height_map: &'a HeightMap,
    pub conv: &'a CoordConverter,
    pub spatial_index: &'a SpatialIndex,
    pub surface: i32,
}

/// Per-tile way index sets, pre-filtered from the global `SpatialIndex`.
///
/// Pass `None` for all sets to render the global index without spatial filtering
/// (used by the in-memory preview pipeline).
pub struct TileWays<'a> {
    pub landuse: &'a [usize],
    pub waterways: &'a [usize],
    pub railways: &'a [usize],
    pub highways: &'a [usize],
    pub barriers: &'a [usize],
    pub buildings: &'a [usize],
    pub pois: &'a [usize],
    pub address: &'a [usize],
    pub relations: &'a [&'a ResolvedRelation<'a>],
    /// For tile-bounded address-node filtering — `None` means "all nodes".
    pub tile_bounds: Option<(i32, i32, i32, i32)>,
}

/// Render all OSM feature layers into `world` using the provided context.
///
/// This function is the single shared implementation called by both the
/// in-memory preview pipeline and the tile-based streaming pipeline.
/// Each layer is guarded by the corresponding `params.filter.*` flag.
#[allow(clippy::too_many_arguments)]
pub fn render_osm_features(
    world: &mut bedrock::BedrockWorld,
    ctx: &RenderContext,
    tile: &TileWays,
) {
    let params = ctx.params;
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;
    let data = ctx.data;
    let surface = ctx.surface;
    let conv = ctx.conv;
    let spatial_index = ctx.spatial_index;

    // ── Layer 1: Natural / landuse areas ────────────────────────────────────
    if params.filter.landuse {
        for &wi in tile.landuse {
            let (way, pts) = &resolved_ways[wi];
            if pts.is_empty() {
                continue;
            }

            let area_block = if let Some(natural) = way.tags.get("natural") {
                if natural == "water" {
                    continue;
                }
                Some(blocks::natural_to_block(natural))
            } else if let Some(lu) = way.tags.get("landuse") {
                if matches!(lu.as_str(), "reservoir" | "water" | "basin") {
                    continue;
                }
                Some(blocks::landuse_to_block(lu))
            } else {
                None
            };

            if let Some(block) = area_block
                && is_closed_way(&way.node_refs)
                && pts.len() >= 3
            {
                let filled = rasterize_polygon(pts);
                for (x, z) in filled {
                    let sy = height_map.get(x, z);
                    world.set_block(x, sy, z, block);
                    maybe_place_tree(world, x, z, sy, block);
                }
            }
        }

        // Layer 1b: Landuse from multipolygon relations
        for rel in tile.relations {
            let area_block = if let Some(natural) = rel.tags.get("natural") {
                if natural == "water" {
                    continue;
                }
                Some(blocks::natural_to_block(natural))
            } else if let Some(lu) = rel.tags.get("landuse") {
                if matches!(lu.as_str(), "reservoir" | "water" | "basin") {
                    continue;
                }
                Some(blocks::landuse_to_block(lu))
            } else {
                None
            };

            if let Some(block) = area_block {
                for outer in &rel.outers {
                    let filled = rasterize_polygon_with_holes(outer, &rel.inners);
                    for (x, z) in filled {
                        world.set_block(x, height_map.get(x, z), z, block);
                    }
                }
            }
        }
    } // end if params.filter.landuse

    // ── Layer 2: Water ───────────────────────────────────────────────────────
    if params.filter.water {
        for &wi in tile.waterways {
            let (way, pts) = &resolved_ways[wi];
            if pts.is_empty() {
                continue;
            }

            if let Some(ww) = way.tags.get("waterway") {
                let style = blocks::waterway_to_style(ww, &way.tags, params.scale);
                draw_waterway(world, pts, |x, z| height_map.get(x, z), &style);
                continue;
            }

            if (way.tags.get("natural").is_some_and(|v| v == "water")
                || way
                    .tags
                    .get("landuse")
                    .is_some_and(|v| matches!(v.as_str(), "reservoir" | "water" | "basin")))
                && is_closed_way(&way.node_refs)
                && pts.len() >= 3
            {
                // Water bodies (lakes, reservoirs) remain flat at sea level.
                let filled = rasterize_polygon(pts);
                for (x, z) in filled {
                    for dy in -2..=0 {
                        world.set_block(x, surface + dy, z, Block::Water);
                    }
                }
            }
        }

        // Layer 2a: Water from multipolygon relations
        for rel in tile.relations {
            let is_water = rel.tags.get("natural").is_some_and(|v| v == "water")
                || rel
                    .tags
                    .get("landuse")
                    .is_some_and(|v| matches!(v.as_str(), "reservoir" | "water" | "basin"));
            if is_water {
                for outer in &rel.outers {
                    let filled = rasterize_polygon_with_holes(outer, &rel.inners);
                    for (x, z) in filled {
                        for dy in -2..=0 {
                            world.set_block(x, surface + dy, z, Block::Water);
                        }
                    }
                }
            }
        }
    } // end if params.filter.water

    // ── Layer 2b: Railways ────────────────────────────────────────────────────
    if params.filter.railways {
        for &wi in tile.railways {
            let (way, pts) = &resolved_ways[wi];
            if pts.len() < 2 {
                continue;
            }
            if way.tags.get("railway").is_some_and(|v| v == "rail") {
                for w in pts.windows(2) {
                    let (x0, z0) = w[0];
                    let (x1, z1) = w[1];
                    let center = rasterize_line(x0, z0, x1, z1);
                    let dx = (x1 - x0).abs();
                    let dz = (z1 - z0).abs();
                    let rail_dir: i32 = if dz >= dx { 0 } else { 1 };
                    for (cx, cz) in &center {
                        let sy = height_map.get(*cx, *cz);
                        if rail_dir == 0 {
                            for d in -1..=1i32 {
                                world.set_block(
                                    cx + d,
                                    height_map.get(cx + d, *cz),
                                    *cz,
                                    Block::Gravel,
                                );
                            }
                        } else {
                            for d in -1..=1i32 {
                                world.set_block(
                                    *cx,
                                    height_map.get(*cx, cz + d),
                                    cz + d,
                                    Block::Gravel,
                                );
                            }
                        }
                        world.set_block(*cx, sy + 1, *cz, Block::Rail);
                        world.set_block_direction(*cx, sy + 1, *cz, rail_dir);
                    }
                }
            }
        }
    } // end if params.filter.railways

    // ── Layer 3: Roads ────────────────────────────────────────────────────────
    if params.filter.roads {
        for &wi in tile.highways {
            let (way, pts) = &resolved_ways[wi];
            if pts.is_empty() {
                continue;
            }
            if let Some(hw) = way.tags.get("highway") {
                let mut style = blocks::highway_to_style(hw);
                if let Some(lanes_str) = way.tags.get("lanes")
                    && let Ok(lanes) = lanes_str.parse::<i32>()
                {
                    style.half_width = (lanes - 1).max(1);
                }
                let is_bridge = way.tags.get("bridge").is_some_and(|v| v != "no");
                let is_tunnel = way.tags.get("tunnel").is_some_and(|v| v != "no");
                if is_bridge {
                    draw_bridge(world, pts, |x, z| height_map.get(x, z), &style);
                } else if is_tunnel {
                    draw_tunnel(world, pts, |x, z| height_map.get(x, z), &style);
                } else {
                    draw_road(world, pts, |x, z| height_map.get(x, z), &style);
                }
            }
        }
    } // end if params.filter.roads

    // ── Layer 3c: Barriers ────────────────────────────────────────────────────
    for &wi in tile.barriers {
        let (way, pts) = &resolved_ways[wi];
        if pts.len() < 2 {
            continue;
        }
        if let Some(barrier) = way.tags.get("barrier") {
            let (block, h) = match barrier.as_str() {
                "fence" | "guard_rail" => (Block::OakFence, 1),
                "wall" | "retaining_wall" => (Block::CobblestoneWall, 1),
                "hedge" => (Block::OakLeaves, 2),
                _ => continue,
            };
            for w in pts.windows(2) {
                for (x, z) in rasterize_line(w[0].0, w[0].1, w[1].0, w[1].1) {
                    let sy = height_map.get(x, z);
                    for dy in 1..=h {
                        world.set_block(x, sy + dy, z, block);
                    }
                }
            }
        }
    }

    // ── Layer 4: Buildings ────────────────────────────────────────────────────
    if params.filter.buildings {
        for &wi in tile.buildings {
            let (way, pts) = &resolved_ways[wi];
            if pts.is_empty() {
                continue;
            }
            if (way.tags.contains_key("building") || way.tags.contains_key("building:part"))
                && is_closed_way(&way.node_refs)
                && pts.len() >= 3
            {
                let building_surface =
                    pts.iter().map(|&(x, z)| height_map.get(x, z)).sum::<i32>() / pts.len() as i32;
                let n_pts = pts.len() as i64;
                let (sum_cx, sum_cz) = pts.iter().fold((0i64, 0i64), |(ax, az), &(x, z)| {
                    (ax + x as i64, az + z as i64)
                });
                let centroid_x = (sum_cx / n_pts) as i32;
                let centroid_z = (sum_cz / n_pts) as i32;
                let building_road_dir = nearest_road_vector(
                    centroid_x,
                    centroid_z,
                    &spatial_index.highways,
                    resolved_ways,
                    400,
                );
                let straight_pts =
                    convert::straighten_polygon(pts, params.wall_straighten_threshold);
                let pts = &straight_pts;
                draw_building(
                    world,
                    pts,
                    building_surface,
                    params.building_height,
                    &way.tags,
                    building_road_dir,
                );
                draw_roof(
                    world,
                    pts,
                    building_surface,
                    params.building_height,
                    &way.tags,
                );
            }
        }

        // Layer 4b: Buildings from multipolygon relations
        for rel in tile.relations {
            if rel.tags.contains_key("building") || rel.tags.contains_key("building:part") {
                let wall = blocks::building_block(rel.tags);
                for outer in &rel.outers {
                    let rel_surface = if outer.is_empty() {
                        surface
                    } else {
                        outer
                            .iter()
                            .map(|&(x, z)| height_map.get(x, z))
                            .sum::<i32>()
                            / outer.len() as i32
                    };
                    let filled = rasterize_polygon_with_holes(outer, &rel.inners);
                    for &(x, z) in &filled {
                        world.set_block(x, rel_surface, z, wall);
                        world.set_block(x, rel_surface + params.building_height, z, wall);
                    }
                    let n = outer.len();
                    for i in 0..n {
                        let j = (i + 1) % n;
                        for (x, z) in rasterize_line(outer[i].0, outer[i].1, outer[j].0, outer[j].1)
                        {
                            for dy in 1..params.building_height {
                                world.set_block(x, rel_surface + dy, z, wall);
                            }
                        }
                    }
                    for inner in &rel.inners {
                        let ni = inner.len();
                        for i in 0..ni {
                            let j = (i + 1) % ni;
                            for (x, z) in
                                rasterize_line(inner[i].0, inner[i].1, inner[j].0, inner[j].1)
                            {
                                for dy in 1..params.building_height {
                                    world.set_block(x, rel_surface + dy, z, wall);
                                }
                            }
                        }
                    }
                    draw_roof(world, outer, rel_surface, params.building_height, rel.tags);
                }
            }
        }
    } // end if params.filter.buildings

    // ── Layer 5: Street name signs ────────────────────────────────────────────
    if params.signs {
        for &wi in tile.highways {
            let (way, pts) = &resolved_ways[wi];
            if pts.len() < 2 {
                continue;
            }
            let name = match way.tags.get("name") {
                Some(n) if !n.is_empty() => n,
                _ => continue,
            };
            let sign_text = format_sign_text(name);
            let mut accum_dist = 0.0f64;
            let mut last_sign_dist = -50.0f64;
            for w in pts.windows(2) {
                let (x0, z0) = w[0];
                let (x1, z1) = w[1];
                let dx = (x1 - x0) as f64;
                let dz = (z1 - z0) as f64;
                let seg_len = (dx * dx + dz * dz).sqrt();
                if seg_len < 0.5 {
                    accum_dist += seg_len;
                    continue;
                }
                let angle = dz.atan2(dx);
                let dir_f = ((std::f64::consts::FRAC_PI_2 - angle) / (2.0 * std::f64::consts::PI)
                    * 16.0)
                    .rem_euclid(16.0);
                let direction = dir_f.round() as i32 % 16;
                if accum_dist + seg_len - last_sign_dist >= 50.0 {
                    let mut t = (last_sign_dist + 50.0 - accum_dist).max(0.0);
                    while t <= seg_len {
                        let frac = t / seg_len;
                        let sx = x0 + (dx * frac) as i32;
                        let sz = z0 + (dz * frac) as i32;
                        let sy = height_map.get(sx, sz) + 1;
                        world.set_block(sx, sy, sz, Block::OakSign);
                        world.set_sign_direction(sx, sy, sz, direction);
                        let sign_nbt = encode_sign_block_entity(sx, sy, sz, &sign_text);
                        world.add_block_entity(sx, sy, sz, sign_nbt);
                        last_sign_dist = accum_dist + t;
                        t += 50.0;
                    }
                }
                accum_dist += seg_len;
            }
        }
    }

    // ── Layer 5b: Address signs ───────────────────────────────────────────────
    if params.address_signs {
        let bounds = tile.tile_bounds;
        for addr in &data.addr_nodes {
            let housenumber = match addr.tags.get("addr:housenumber") {
                Some(n) if !n.is_empty() => n.as_str(),
                _ => continue,
            };
            let street = addr
                .tags
                .get("addr:street")
                .map(|s| s.as_str())
                .unwrap_or("");
            let addr_text = if street.is_empty() {
                housenumber.to_string()
            } else {
                format!("{}\n{}", housenumber, format_sign_text(street))
            };
            let (ax, az) = conv.to_block_xz(addr.lat, addr.lon);
            // If tile bounds provided, skip nodes outside this tile.
            if let Some((tx0, tz0, tx1, tz1)) = bounds
                && (ax < tx0 || ax > tx1 || az < tz0 || az > tz1)
            {
                continue;
            }
            let ay = height_map.get(ax, az) + 2;
            let addr_dir = nearest_road_vector(ax, az, &spatial_index.highways, resolved_ways, 300)
                .map(|(dx, dz)| vec_to_sign_dir(dx, dz))
                .unwrap_or(0);
            world.set_block(ax, ay, az, Block::CherryHangingSign);
            world.set_sign_direction(ax, ay, az, addr_dir);
            let sign_nbt = encode_sign_block_entity(ax, ay, az, &addr_text);
            world.add_block_entity(ax, ay, az, sign_nbt);
        }

        for &wi in tile.address {
            let (way, pts) = &resolved_ways[wi];
            if pts.len() < 3 {
                continue;
            }
            if !way.tags.contains_key("building") && !way.tags.contains_key("building:part") {
                continue;
            }
            let housenumber = match way.tags.get("addr:housenumber") {
                Some(n) if !n.is_empty() => n.as_str(),
                _ => continue,
            };
            let street = way
                .tags
                .get("addr:street")
                .map(|s| s.as_str())
                .unwrap_or("");
            let addr_text = if street.is_empty() {
                housenumber.to_string()
            } else {
                format!("{}\n{}", housenumber, format_sign_text(street))
            };

            let n = pts.len();
            let (sum_cx, sum_cz) = pts.iter().fold((0i64, 0i64), |(ax, az), &(x, z)| {
                (ax + x as i64, az + z as i64)
            });
            let centroid_x = (sum_cx / n as i64) as i32;
            let centroid_z = (sum_cz / n as i64) as i32;
            let road_vec = nearest_road_vector(
                centroid_x,
                centroid_z,
                &spatial_index.highways,
                resolved_ways,
                400,
            );
            let mut best_score = f64::NEG_INFINITY;
            let mut best_edge_idx = 0usize;
            let mut best_outward = (1.0f64, 0.0f64);
            for i in 0..n {
                let j = (i + 1) % n;
                let edge_dx = (pts[j].0 - pts[i].0) as f64;
                let edge_dz = (pts[j].1 - pts[i].1) as f64;
                let edge_len = (edge_dx * edge_dx + edge_dz * edge_dz).sqrt();
                if edge_len < 0.5 {
                    continue;
                }
                let mx = (pts[i].0 + pts[j].0) / 2;
                let mz = (pts[i].1 + pts[j].1) / 2;
                let n1 = (edge_dz / edge_len, -edge_dx / edge_len);
                let n2 = (-edge_dz / edge_len, edge_dx / edge_len);
                let out_dx = (mx - centroid_x) as f64;
                let out_dz = (mz - centroid_z) as f64;
                let outward = if n1.0 * out_dx + n1.1 * out_dz >= 0.0 {
                    n1
                } else {
                    n2
                };
                let score = if let Some((rdx, rdz)) = road_vec {
                    let road_len = (rdx * rdx + rdz * rdz).sqrt().max(1.0);
                    outward.0 * rdx / road_len + outward.1 * rdz / road_len
                } else {
                    edge_len
                };
                if score > best_score {
                    best_score = score;
                    best_edge_idx = i;
                    best_outward = outward;
                }
            }
            let j = (best_edge_idx + 1) % n;
            let mx = (pts[best_edge_idx].0 + pts[j].0) / 2;
            let mz = (pts[best_edge_idx].1 + pts[j].1) / 2;
            let sx = mx + best_outward.0.round() as i32;
            let sz = mz + best_outward.1.round() as i32;
            let sy = height_map.get(sx, sz) + 2;
            let direction = vec_to_sign_dir(best_outward.0, best_outward.1);
            world.set_block(sx, sy, sz, Block::CherryHangingSign);
            world.set_sign_direction(sx, sy, sz, direction);
            let sign_nbt = encode_sign_block_entity(sx, sy, sz, &addr_text);
            world.add_block_entity(sx, sy, sz, sign_nbt);
        }
    }

    // ── Layer 6: POI markers ─────────────────────────────────────────────────
    if params.poi_markers {
        let bounds = tile.tile_bounds;
        for poi in &data.poi_nodes {
            let (px, pz) = conv.to_block_xz(poi.lat, poi.lon);
            if let Some((tx0, tz0, tx1, tz1)) = bounds
                && (px < tx0 || px > tx1 || pz < tz0 || pz > tz1)
            {
                continue;
            }
            let py = height_map.get(px, pz) + 1;
            let poi_type = resolve_poi_type(&poi.tags);
            let name = poi.tags.get("name").map(|s| s.as_str()).unwrap_or("");
            let sign_text = format_poi_sign(name, poi_type);
            world.set_block(px, py, pz, Block::CherrySign);
            world.set_sign_direction(px, py, pz, 0);
            let sign_nbt = encode_sign_block_entity(px, py, pz, &sign_text);
            world.add_block_entity(px, py, pz, sign_nbt);
        }

        for &wi in tile.pois {
            let (way, pts) = &resolved_ways[wi];
            if pts.is_empty() {
                continue;
            }
            let poi_type = resolve_poi_type(&way.tags);
            let name = way.tags.get("name").map(|s| s.as_str()).unwrap_or("");
            let sign_text = format_poi_sign(name, poi_type);
            let (sum_x, sum_z) = pts.iter().fold((0i64, 0i64), |(sx, sz), &(x, z)| {
                (sx + x as i64, sz + z as i64)
            });
            let px = (sum_x / pts.len() as i64) as i32;
            let pz = (sum_z / pts.len() as i64) as i32;
            let is_building =
                way.tags.contains_key("building") || way.tags.contains_key("building:part");
            let py = if is_building {
                height_map.get(px, pz) + params.building_height + 1
            } else {
                height_map.get(px, pz) + 1
            };
            world.set_block(px, py, pz, Block::CherrySign);
            world.set_sign_direction(px, py, pz, 0);
            let sign_nbt = encode_sign_block_entity(px, py, pz, &sign_text);
            world.add_block_entity(px, py, pz, sign_nbt);
        }
    }

    // ── Layer 7: Individual tree nodes (OSM natural=tree / Overture land trees) ─
    if params.nature_decorations {
        let bounds = tile.tile_bounds;
        for tree in &data.tree_nodes {
            let (tx, tz) = conv.to_block_xz(tree.lat, tree.lon);
            if let Some((bx0, bz0, bx1, bz1)) = bounds
                && (tx < bx0 || tx > bx1 || tz < bz0 || tz > bz1)
            {
                continue;
            }
            let sy = height_map.get(tx, tz);
            place_tree(world, tx, tz, sy);
        }
    }

    // ── Layer 8: POI decorations ─────────────────────────────────────────────
    if params.poi_decorations {
        let bounds = tile.tile_bounds;
        for poi in &data.poi_nodes {
            let (px, pz) = conv.to_block_xz(poi.lat, poi.lon);
            if let Some((bx0, bz0, bx1, bz1)) = bounds
                && (px < bx0 || px > bx1 || pz < bz0 || pz > bz1)
            {
                continue;
            }
            let sy = height_map.get(px, pz);
            let poi_type = resolve_poi_type(&poi.tags);
            place_poi_decoration(world, px, sy, pz, poi_type);
        }
    }
}

/// Place a decorative block structure at a POI location.
fn place_poi_decoration(
    world: &mut bedrock::BedrockWorld,
    x: i32,
    sy: i32,
    z: i32,
    poi_type: &str,
) {
    match poi_type {
        // Coffee specifically — brewing stand
        "coffee_shop" => {
            world.set_block(x, sy + 1, z, Block::BrewingStand);
        }
        // Food & Drink — furnace (kitchen)
        "restaurant"
        | "cafe"
        | "fast_food"
        | "bar"
        | "pub"
        | "biergarten"
        | "food_court"
        | "mexican_restaurant"
        | "pizza_restaurant"
        | "fast_food_restaurant"
        | "breakfast_and_brunch_restaurant"
        | "barbecue_restaurant" => {
            world.set_block(x, sy + 1, z, Block::Furnace);
        }
        // Lodging — bed
        "hotel" | "motel" | "hostel" | "guest_house" => {
            world.set_block(x, sy + 1, z, Block::Bed);
        }
        // Education — bookshelf
        "school" | "university" | "college" | "kindergarten" | "library" | "elementary_school" => {
            world.set_block(x, sy + 1, z, Block::Bookshelf);
        }
        // Medical — white concrete + red concrete cross (2 blocks tall)
        "hospital" | "clinic" | "doctors" | "dentist" | "pharmacy" | "medical_center"
        | "doctor" | "optometrist" | "pediatric_dentist" => {
            world.set_block(x, sy + 1, z, Block::WhiteConcrete);
            world.set_block(x, sy + 2, z, Block::WhiteConcrete);
        }
        // Worship — bell
        "place_of_worship" | "church_cathedral" => {
            world.set_block(x, sy + 1, z, Block::OakFence);
            world.set_block(x, sy + 2, z, Block::Bell);
        }
        // Post / mail — dispenser on fence (mailbox)
        "post_office" => {
            world.set_block(x, sy + 1, z, Block::OakFence);
            world.set_block(x, sy + 2, z, Block::Dispenser);
        }
        // Fire station — campfire + lantern
        "fire_station" => {
            world.set_block(x, sy + 1, z, Block::Campfire);
            world.set_block(x, sy + 2, z, Block::Lantern);
        }
        // Farm — hay bale
        "farm" => {
            world.set_block(x, sy + 1, z, Block::HayBale);
        }
        // Gas station — dispenser (fuel pump)
        "gas_station" | "fuel" => {
            world.set_block(x, sy + 1, z, Block::Dispenser);
        }
        // Parking — iron bars
        "parking" => {
            world.set_block(x, sy + 1, z, Block::OakFence);
        }
        // Banks / ATM — barrel (vault)
        "bank" | "atm" | "atms" | "banks" | "bank_credit_union" | "financial_service" => {
            world.set_block(x, sy + 1, z, Block::Barrel);
        }
        // Shops / stores — barrel
        "supermarket" | "convenience" | "convenience_store" | "grocery_store"
        | "department_store" | "mall" => {
            world.set_block(x, sy + 1, z, Block::Barrel);
        }
        // Default: lantern on fence post (street furniture)
        _ => {
            world.set_block(x, sy + 1, z, Block::OakFence);
            world.set_block(x, sy + 2, z, Block::Lantern);
        }
    }
}

/// Place a tree at an exact position (from individual tree node data).
fn place_tree(world: &mut bedrock::BedrockWorld, x: i32, z: i32, sy: i32) {
    let species = coord_hash(x, z) % 5;
    let (log_block, leaf_block) = match species {
        0..=2 => (Block::OakLog, Block::OakLeaves),
        3 => (Block::BirchLog, Block::BirchLeaves),
        _ => (Block::OakLog, Block::OakLeaves),
    };
    let trunk_height: i32 = if species == 4 { 6 } else { 4 };
    let canopy_radius: i32 = if species == 4 { 3 } else { 2 };

    for dy in 1..=trunk_height {
        world.set_block(x, sy + dy, z, log_block);
    }
    for lx in -canopy_radius..=canopy_radius {
        for lz in -canopy_radius..=canopy_radius {
            if lx.abs() == canopy_radius && lz.abs() == canopy_radius {
                continue; // round the corners
            }
            world.set_block(x + lx, sy + trunk_height + 1, z + lz, leaf_block);
            if canopy_radius > 2 {
                world.set_block(x + lx, sy + trunk_height, z + lz, leaf_block);
            }
        }
    }
    // Top cap
    world.set_block(x, sy + trunk_height + 2, z, leaf_block);
}

/// Place a tree or undergrowth at `(x, sy, z)` if the block is OakLog (forest)
/// and the coordinate hash selects this position.
fn maybe_place_tree(world: &mut bedrock::BedrockWorld, x: i32, z: i32, sy: i32, block: Block) {
    if block == Block::OakLog && coord_hash(x, z).is_multiple_of(7) {
        let species = coord_hash(x, z) % 5;
        let (log_block, leaf_block) = match species {
            0..=2 => (Block::OakLog, Block::OakLeaves), // 60% oak
            3 => (Block::BirchLog, Block::BirchLeaves), // 20% birch
            _ => (Block::OakLog, Block::OakLeaves),     // 20% tall oak variant
        };
        let trunk_height: i32 = if species == 4 { 6 } else { 4 };
        let canopy_radius: i32 = if species == 4 { 1 } else { 2 };

        for dy in 1..=trunk_height {
            world.set_block(x, sy + dy, z, log_block);
        }
        for lx in -canopy_radius..=canopy_radius {
            for lz in -canopy_radius..=canopy_radius {
                world.set_block(x + lx, sy + trunk_height + 1, z + lz, leaf_block);
            }
        }
        if coord_hash(x + 2, z).is_multiple_of(15) {
            world.set_block(x, sy + trunk_height + 2, z, Block::Torch);
        }
    } else if block == Block::OakLog && coord_hash(x + 1, z + 1).is_multiple_of(3) {
        // Undergrowth between trees
        let plant_roll = coord_hash(x, z + 1) % 10;
        let plant = if plant_roll < 5 {
            Block::TallGrass
        } else if plant_roll < 8 {
            Block::Fern
        } else {
            Block::Poppy
        };
        world.set_block(x, sy + 1, z, plant);
    }
}

// ── Geometry helpers ───────────────────────────────────────────────────────────

/// Compute the axis-aligned bounding box of the four map-corner block coordinates.
///
/// Converts the four corners of a geographic bounding box to block coordinates
/// and returns `(min_x, max_x, min_z, max_z)`.  The array always has exactly
/// four elements so `min`/`max` are guaranteed to return `Some`.
fn bounding_box(corners: &[(i32, i32); 4]) -> (i32, i32, i32, i32) {
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
fn compute_terrain_bounds(data: &osm::OsmData, conv: &CoordConverter) -> (i32, i32, i32, i32) {
    let mut xs: Vec<i32> = Vec::new();
    let mut zs: Vec<i32> = Vec::new();
    for way in &data.ways {
        for id in &way.node_refs {
            if let Some(node) = data.nodes.get(id) {
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
fn resolve_relations<'a>(
    data: &'a osm::OsmData,
    conv: &CoordConverter,
) -> Vec<ResolvedRelation<'a>> {
    data.relations
        .iter()
        .filter_map(|rel| {
            let mut outers: Vec<Vec<(i32, i32)>> = Vec::new();
            let mut inners: Vec<Vec<(i32, i32)>> = Vec::new();
            for member in &rel.members {
                if let Some(&idx) = data.ways_by_id.get(&member.way_id) {
                    let way = &data.ways[idx];
                    let pts: Vec<(i32, i32)> = way
                        .node_refs
                        .iter()
                        .filter_map(|id| data.nodes.get(id))
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
fn resolve_ways<'a>(
    data: &'a osm::OsmData,
    conv: &CoordConverter,
) -> Vec<(&'a osm::OsmWay, Vec<(i32, i32)>)> {
    data.ways
        .iter()
        .map(|way| {
            let pts: Vec<(i32, i32)> = way
                .node_refs
                .iter()
                .filter_map(|id| data.nodes.get(id))
                .map(|n| conv.to_block_xz(n.lat, n.lon))
                .collect();
            (way, pts)
        })
        .collect()
}

/// Compute spawn point from params, CoordConverter, and HeightMap.
fn resolve_spawn(
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

// ── Public pipeline entry points ──────────────────────────────────────────────

/// Run the conversion pipeline and return the `BedrockWorld` without saving.
///
/// Used by the preview endpoint (server) to inspect the world in memory.
/// Uses the same [`render_osm_features`] function as the streaming pipeline,
/// so preview results include signs, POI markers, and barriers.
pub fn run_conversion_preview(
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(bedrock::BedrockWorld, i32, i32, i32)> {
    if params.scale <= 0.0 {
        bail!("scale must be positive");
    }
    progress_cb(0.0, "Parsing OSM data");
    let path = params.input.as_deref().ok_or_else(|| {
        anyhow::anyhow!("ConvertParams.input is required for file-based conversion")
    })?;
    log::info!("Reading {}", path.display());
    let data = crate::osm::parse_osm_file(path)?;
    if data.ways.is_empty() {
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
) -> Result<(bedrock::BedrockWorld, i32, i32, i32)> {
    if data.ways.is_empty() {
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
    if data.ways.is_empty() {
        bail!("No ways found in OSM data.");
    }

    let (origin_lat, origin_lon) = {
        let (min_lat, min_lon, max_lat, max_lon) = data
            .bounds
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
        let style = blocks::highway_to_style(hw_type);
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

/// Inner in-memory pipeline: `OsmData` → `BedrockWorld`.
///
/// Used only by [`run_conversion_preview`].  For large inputs this loads
/// all chunk data into memory; the streaming pipeline should be preferred for
/// production conversions.  Now calls [`render_osm_features`] so it has full
/// feature parity with the streaming pipeline.
fn run_pipeline(
    data: osm::OsmData,
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(bedrock::BedrockWorld, i32, i32, i32)> {
    let (origin_lat, origin_lon) = {
        let (min_lat, min_lon, max_lat, max_lon) = data
            .bounds
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
        type ChunkResult = ((i32, i32), bedrock::ChunkData, Vec<((i32, i32), i32)>);
        let filled: Vec<ChunkResult> = chunk_coords
            .par_iter()
            .map(|&(cx, cz)| {
                let mut chunk = bedrock::ChunkData::new();
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
                        let base_y = (sy - thickness).max(bedrock::MIN_Y);
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

        let mut world = bedrock::BedrockWorld::new(&params.output);
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
    log::info!("Processing {} ways...", data.ways.len());

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
    type ChunkResult = ((i32, i32), bedrock::ChunkData, Vec<((i32, i32), i32)>);
    let filled: Vec<ChunkResult> = chunk_coords
        .par_iter()
        .map(|&(cx, cz)| {
            let mut chunk = bedrock::ChunkData::new();
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
                    let base_y = (sy - thickness).max(bedrock::MIN_Y);
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

    let mut world = bedrock::BedrockWorld::new(&params.output);
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
    render_osm_features(&mut world, &ctx, &tile);

    // Compute spawn point
    let (spawn_x, spawn_y, spawn_z) =
        resolve_spawn(params, &conv, &height_map, min_cx, max_cx, min_cz, max_cz);
    log::info!("Spawn point: ({}, {}, {})", spawn_x, spawn_y, spawn_z);

    progress_cb(0.85, "Conversion complete");
    Ok((world, spawn_x, spawn_y, spawn_z))
}

/// Run the full OSM-to-Bedrock conversion pipeline.
///
/// Uses the streaming (tile-based) pipeline so that only one
/// `TILE_CHUNKS × TILE_CHUNKS` tile of chunk data lives in memory at a time.
pub fn run_conversion(params: &ConvertParams, progress_cb: &dyn Fn(f32, &str)) -> Result<()> {
    if params.scale <= 0.0 {
        bail!("scale must be positive");
    }
    let timer = crate::metadata::MetadataTimer::start();

    progress_cb(0.0, "Parsing OSM data");
    let path = params.input.as_deref().ok_or_else(|| {
        anyhow::anyhow!("ConvertParams.input is required for file-based conversion")
    })?;
    log::info!("Reading {}", path.display());
    let source_info = crate::metadata::source_info(path).ok();
    let data = crate::osm::parse_osm_file(path)?;
    if data.ways.is_empty() {
        bail!("No ways found in OSM file.");
    }

    let metadata = crate::metadata::build_metadata(params, &data, &timer, source_info);
    let (spawn_x, spawn_y, spawn_z) = run_pipeline_streaming(data, params, progress_cb)?;

    // Write metadata after successful conversion (re-compute timing)
    let metadata = crate::metadata::WorldMetadata {
        timing: timer.finish(),
        ..metadata
    };
    if let Err(e) = crate::metadata::write_metadata(&params.output, &metadata) {
        log::warn!("Failed to write world_info.json: {e}");
    }

    progress_cb(1.0, "Conversion complete");
    log::info!(
        "Done! Open the '{}' folder in Minecraft Bedrock.",
        params.output.display()
    );
    let _ = (spawn_x, spawn_y, spawn_z);
    Ok(())
}

/// Run the full conversion pipeline from pre-fetched `OsmData` and save to disk.
///
/// Used by Overpass-based flows where OSM data has already been fetched and
/// does not need to be read from a file.
pub fn run_conversion_from_data(
    data: osm::OsmData,
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<()> {
    if data.ways.is_empty() {
        bail!("No ways found in OSM data.");
    }
    let timer = crate::metadata::MetadataTimer::start();
    let metadata = crate::metadata::build_metadata(params, &data, &timer, None);

    run_pipeline_streaming(data, params, progress_cb)?;

    // Write metadata with final timing
    let metadata = crate::metadata::WorldMetadata {
        timing: timer.finish(),
        ..metadata
    };
    if let Err(e) = crate::metadata::write_metadata(&params.output, &metadata) {
        log::warn!("Failed to write world_info.json: {e}");
    }

    progress_cb(1.0, "Conversion complete");
    Ok(())
}

/// Default surface thickness when elevation is active and the user didn't
/// override.  Thick enough to avoid hollow hillsides when digging sideways.
const ELEVATION_SURFACE_THICKNESS: i32 = 32;

/// Compute effective surface thickness: if elevation data is present and the
/// configured thickness is the compiled default (4), auto-increase to 32 so
/// hillsides aren't hollow.
fn effective_thickness(configured: i32, has_elevation: bool) -> i32 {
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
fn load_elevation(
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

/// Tile-based streaming conversion pipeline.
///
/// Processes the world in `TILE_CHUNKS × TILE_CHUNKS` chunk tiles so that
/// only one tile's chunk data lives in memory at a time.  Each tile is
/// encoded and sent to a background [`bedrock::ChunkWriter`] thread before
/// the next tile begins, pipelining CPU encoding with LevelDB disk I/O.
fn run_pipeline_streaming(
    data: osm::OsmData,
    params: &ConvertParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(i32, i32, i32)> {
    // ── Determine origin ─────────────────────────────────────────────────────
    let (origin_lat, origin_lon) = {
        let (min_lat, min_lon, max_lat, max_lon) = data
            .bounds
            .ok_or_else(|| anyhow::anyhow!("OSM file has no nodes"))?;
        ((min_lat + max_lat) / 2.0, (min_lon + max_lon) / 2.0)
    };

    let conv = CoordConverter::new(origin_lat, origin_lon, params.scale);
    let elevation_data = load_elevation(params.elevation.as_deref(), params.vertical_scale);
    let surface_thickness = effective_thickness(params.surface_thickness, elevation_data.is_some());

    // Pass 1: compute terrain bounding box
    progress_cb(0.10, "Computing terrain bounds");
    let (min_x, max_x, min_z, max_z) = compute_terrain_bounds(&data, &conv);

    let min_cx = min_x.div_euclid(16);
    let max_cx = max_x.div_euclid(16);
    let min_cz = min_z.div_euclid(16);
    let max_cz = max_z.div_euclid(16);

    log::info!(
        "Terrain bounds: x=[{}..{}] z=[{}..{}] ({} x {} blocks, chunks cx=[{}..{}] cz=[{}..{}])",
        min_x,
        max_x,
        min_z,
        max_z,
        max_x - min_x,
        max_z - min_z,
        min_cx,
        max_cx,
        min_cz,
        max_cz
    );

    // Pass 2: pre-compute global HeightMap (parallel, no ChunkData)
    progress_cb(0.20, "Computing height map");
    let surface = params.sea_level;
    let mut height_map: HeightMap = {
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
        hm
    };
    log::info!("Height map complete (bounded Vec)");

    if params.elevation_smoothing > 0 && elevation_data.is_some() {
        log::info!(
            "Smoothing height map (radius={})",
            params.elevation_smoothing
        );
        height_map.smooth(params.elevation_smoothing);
    }

    // Build resolved ways + spatial index
    progress_cb(0.30, "Building spatial index");
    let resolved_ways = resolve_ways(&data, &conv);
    let resolved_relations = resolve_relations(&data, &conv);
    let spatial_index = SpatialIndex::build(&resolved_ways);
    log::info!(
        "Spatial index: {} highway, {} building, {} landuse, {} water, {} railway, {} barrier ways",
        spatial_index.highways.len(),
        spatial_index.buildings.len(),
        spatial_index.landuse.len(),
        spatial_index.waterways.len(),
        spatial_index.railways.len(),
        spatial_index.barriers.len(),
    );

    // Compute spawn point
    let (spawn_x, spawn_y, spawn_z) =
        resolve_spawn(params, &conv, &height_map, min_cx, max_cx, min_cz, max_cz);
    log::info!("Spawn point: ({}, {}, {})", spawn_x, spawn_y, spawn_z);

    // Open LevelDB writer
    std::fs::create_dir_all(&params.output)
        .with_context(|| format!("creating output dir {}", params.output.display()))?;
    let db_path = params.output.join("db");
    std::fs::create_dir_all(&db_path)?;
    let chunk_writer = bedrock::ChunkWriter::open(db_path)?;

    // Pass 3: tile-based terrain + feature rendering
    progress_cb(0.35, "Converting in tiles");

    let tile_cx_count = (max_cx - min_cx + TILE_CHUNKS) / TILE_CHUNKS;
    let tile_cz_count = (max_cz - min_cz + TILE_CHUNKS) / TILE_CHUNKS;
    let total_tiles = tile_cx_count * tile_cz_count;
    log::info!(
        "Processing {total_tiles} tiles ({tile_cx_count}×{tile_cz_count}, each up to {}×{} chunks)",
        TILE_CHUNKS,
        TILE_CHUNKS
    );

    let mut tile_num = 0i32;
    let mut last_logged_pct = 0;
    let mut tile_cx0 = min_cx;
    while tile_cx0 <= max_cx {
        let tile_cx1 = (tile_cx0 + TILE_CHUNKS - 1).min(max_cx);
        let mut tile_cz0 = min_cz;
        while tile_cz0 <= max_cz {
            let tile_cz1 = (tile_cz0 + TILE_CHUNKS - 1).min(max_cz);
            tile_num += 1;

            let tile_progress = 0.35 + 0.50 * (tile_num as f32 / total_tiles as f32);
            progress_cb(tile_progress, &format!("Tile {tile_num}/{total_tiles}"));

            // Log at every 10% increment
            let pct = tile_num * 100 / total_tiles.max(1);
            if pct / 10 > last_logged_pct / 10 {
                last_logged_pct = pct;
                log::info!("Tile progress: {pct}% ({tile_num}/{total_tiles})");
            }

            let tile_min_x = tile_cx0 * 16;
            let tile_max_x = (tile_cx1 + 1) * 16 - 1;
            let tile_min_z = tile_cz0 * 16;
            let tile_max_z = (tile_cz1 + 1) * 16 - 1;

            let mut tile_world = bedrock::BedrockWorld::new_bounded(
                &params.output,
                tile_cx0,
                tile_cx1,
                tile_cz0,
                tile_cz1,
            );

            // Terrain fill (parallel rayon)
            let tile_chunks: Vec<(i32, i32)> = (tile_cx0..=tile_cx1)
                .flat_map(|cx| (tile_cz0..=tile_cz1).map(move |cz| (cx, cz)))
                .collect();

            let filled: Vec<((i32, i32), bedrock::ChunkData)> = tile_chunks
                .par_iter()
                .map(|&(cx, cz)| {
                    let mut chunk = bedrock::ChunkData::new();
                    for lx in 0..16i32 {
                        for lz in 0..16i32 {
                            let bx = cx * 16 + lx;
                            let bz = cz * 16 + lz;
                            let sy = height_map.get(bx, bz);
                            let base_y = (sy - surface_thickness).max(bedrock::MIN_Y);
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
                tile_world.insert_chunk(cx, cz, chunk);
            }

            // Spatial filter: find way indices intersecting this tile
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
            // Using bbox overlap (rather than checking whether any vertex lies inside
            // the tile) ensures that a large relation whose outer ring spans multiple
            // tiles is included in every tile it visually covers, even when none of its
            // vertices happen to fall inside a particular tile.
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
                resolved_ways: &resolved_ways,
                resolved_relations: &resolved_relations,
                data: &data,
                params,
                height_map: &height_map,
                conv: &conv,
                spatial_index: &spatial_index,
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
            render_osm_features(&mut tile_world, &ctx, &tile_ways);

            // Drain tile chunks → async writer
            tile_world
                .drain_chunks_to_writer(&chunk_writer)
                .with_context(|| {
                    format!("writing tile ({tile_cx0}..{tile_cx1}, {tile_cz0}..{tile_cz1})")
                })?;

            tile_cz0 += TILE_CHUNKS;
        }
        tile_cx0 += TILE_CHUNKS;
    }

    // Close the channel; the writer thread drains its queue and joins.
    progress_cb(0.88, "Flushing LevelDB");
    chunk_writer.finish()?;

    // Write level.dat
    progress_cb(0.95, "Writing level.dat");
    let tmp_world = bedrock::BedrockWorld::new(&params.output);
    tmp_world.write_level_dat(spawn_x, spawn_y, spawn_z)?;

    progress_cb(0.99, "Streaming conversion complete");
    log::info!("Streamed {total_tiles} tiles → {}", params.output.display());

    Ok((spawn_x, spawn_y, spawn_z))
}

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
#[allow(dead_code)]
pub fn run_terrain_only(
    params: &TerrainParams,
    progress_cb: &dyn Fn(f32, &str),
) -> Result<(bedrock::BedrockWorld, i32, i32, i32)> {
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

    type ChunkResult = ((i32, i32), bedrock::ChunkData, Vec<((i32, i32), i32)>);
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

    let mut world = bedrock::BedrockWorld::new(&params.output);
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
    let db_path = params.output.join("db");
    std::fs::create_dir_all(&db_path)?;
    let chunk_writer = bedrock::ChunkWriter::open(db_path)?;

    let sea = params.sea_level;
    let snow_line = params.snow_line;
    let vertical_scale = params.vertical_scale;
    let surface_thickness = effective_thickness(params.surface_thickness, elevation_data.is_some());
    let empty_signs: HashMap<(i32, i32, i32), i32> = HashMap::new();
    let empty_dirs: HashMap<(i32, i32, i32), i32> = HashMap::new();

    let mut height_map = HeightMap::new(sea);

    let cx_tiles = ((max_cx - min_cx + TILE_CHUNKS) / TILE_CHUNKS) as u64;
    let cz_tiles = ((max_cz - min_cz + TILE_CHUNKS) / TILE_CHUNKS) as u64;
    let total_tiles = cx_tiles * cz_tiles;
    let mut tile_idx = 0u64;
    let mut last_logged_pct = 0u64;

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

            type ChunkResult = ((i32, i32), bedrock::ChunkData, Vec<((i32, i32), i32)>);
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

    progress_cb(1.0, "Terrain world complete");
    log::info!(
        "Done! Streamed {} chunks to '{}'.",
        total_chunks,
        params.output.display()
    );
    Ok(())
}

/// Fill a single terrain chunk with biome-appropriate blocks.
///
/// Shared by `run_terrain_only` (in-memory) and `run_terrain_only_to_disk` (streaming).
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
    let mut chunk = bedrock::ChunkData::new();
    let mut local_heights: Vec<((i32, i32), i32)> = Vec::with_capacity(256);
    for lx in 0..16i32 {
        for lz in 0..16i32 {
            let bx = cx * 16 + lx;
            let bz = cz * 16 + lz;
            let sy = compute_surface_y(bx, bz, elevation_data, conv, sea, vertical_scale);

            if sy <= sea {
                let base_y = (sy - surface_thickness).max(bedrock::MIN_Y);
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
                let base_y = (sy - surface_thickness).max(bedrock::MIN_Y);
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
                let base_y = (sy - surface_thickness).max(bedrock::MIN_Y);
                chunk.set(lx, base_y, lz, Block::Bedrock);
                for y in (base_y + 1)..sy {
                    chunk.set(lx, y, lz, Block::Stone);
                }
                chunk.set(lx, sy, lz, Block::Stone);
                let snow_y = (sy + 1).min(bedrock::MAX_Y);
                chunk.set(lx, snow_y, lz, Block::SnowLayer);
                local_heights.push(((bx, bz), snow_y));
            } else {
                let base_y = (sy - surface_thickness).max(bedrock::MIN_Y);
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

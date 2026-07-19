//! Per-layer OSM feature rendering.
//!
//! [`render_osm_features`] is the shared orchestrator called by both the
//! in-memory preview pipeline ([`super::preview::run_pipeline`]) and the
//! streaming tile loop ([`super::terrain::process_tile`]). Each layer is
//! guarded by the corresponding `ConvertParams.filter.*` flag and lives in
//! its own `render_*` helper below.

use crate::blocks::{self, Block};
use crate::convert::{
    self, CoordConverter, rasterize_line, rasterize_polygon, rasterize_polygon_with_holes,
};
use crate::geometry::{
    draw_bridge, draw_building, draw_road, draw_roof, draw_tunnel, draw_waterway,
};
use crate::nbt::encode_sign_block_entity;
use crate::osm;
use crate::params::ConvertParams;
use crate::sign::{format_poi_sign, format_sign_text, nearest_road_vector, vec_to_sign_dir};
use crate::spatial::{HeightMap, ResolvedRelation, SpatialIndex};
use crate::world::WorldWriter;

use super::decoration::{maybe_place_tree, place_poi_decoration, place_tree, resolve_poi_type};
use super::util::is_closed_way;

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
/// This function is the single shared orchestrator called by both the
/// in-memory preview pipeline and the tile-based streaming pipeline.
/// Each layer lives in its own `render_*` helper below and is guarded by
/// the corresponding `params.filter.*` flag (except barriers, which always
/// run). The orchestrator only sequences the layers; per-layer logic is
/// contained in `render_landuse`, `render_water`, `render_railways`,
/// `render_roads`, `render_barriers`, `render_buildings`,
/// `render_street_signs`, `render_address_signs`,
/// `render_poi_markers`, `render_tree_nodes`, and
/// `render_poi_decorations`.
#[allow(clippy::too_many_arguments)]
pub fn render_osm_features(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    if ctx.params.filter.landuse {
        render_landuse(world, ctx, tile);
    }
    if ctx.params.filter.water {
        render_water(world, ctx, tile);
    }
    if ctx.params.filter.railways {
        render_railways(world, ctx, tile);
    }
    if ctx.params.filter.roads {
        render_roads(world, ctx, tile);
    }
    // Barriers are not gated by a filter flag — they always render.
    render_barriers(world, ctx, tile);
    if ctx.params.filter.buildings {
        render_buildings(world, ctx, tile);
    }
    if ctx.params.signs {
        render_street_signs(world, ctx, tile);
    }
    if ctx.params.address_signs {
        render_address_signs(world, ctx, tile);
    }
    if ctx.params.poi_markers {
        render_poi_markers(world, ctx, tile);
    }
    if ctx.params.nature_decorations {
        render_tree_nodes(world, ctx, tile);
    }
    if ctx.params.poi_decorations {
        render_poi_decorations(world, ctx, tile);
    }
}

// ── Layer 1: Natural / landuse areas ────────────────────────────────────────

fn render_landuse(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;

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
}

// ── Layer 2: Water ──────────────────────────────────────────────────────────

fn render_water(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let params = ctx.params;
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;
    let surface = ctx.surface;

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
}

// ── Layer 2b: Railways ──────────────────────────────────────────────────────

fn render_railways(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;

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
}

// ── Layer 3: Roads ──────────────────────────────────────────────────────────

fn render_roads(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;

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
}

// ── Layer 3c: Barriers ──────────────────────────────────────────────────────

fn render_barriers(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;

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
}

// ── Layer 4: Buildings ──────────────────────────────────────────────────────

fn render_buildings(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let params = ctx.params;
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;
    let surface = ctx.surface;
    let spatial_index = ctx.spatial_index;

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
            let straight_pts = convert::straighten_polygon(pts, params.wall_straighten_threshold);
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
                    for (x, z) in rasterize_line(outer[i].0, outer[i].1, outer[j].0, outer[j].1) {
                        for dy in 1..params.building_height {
                            world.set_block(x, rel_surface + dy, z, wall);
                        }
                    }
                }
                for inner in &rel.inners {
                    let ni = inner.len();
                    for i in 0..ni {
                        let j = (i + 1) % ni;
                        for (x, z) in rasterize_line(inner[i].0, inner[i].1, inner[j].0, inner[j].1)
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
}

// ── Layer 5: Street name signs ──────────────────────────────────────────────

fn render_street_signs(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;

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

// ── Layer 5b: Address signs ─────────────────────────────────────────────────

fn render_address_signs(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;
    let data = ctx.data;
    let conv = ctx.conv;
    let spatial_index = ctx.spatial_index;

    let bounds = tile.tile_bounds;
    for addr in data.addr_nodes() {
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

// ── Layer 6: POI markers ────────────────────────────────────────────────────

fn render_poi_markers(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let params = ctx.params;
    let height_map = ctx.height_map;
    let resolved_ways = ctx.resolved_ways;
    let data = ctx.data;
    let conv = ctx.conv;

    let bounds = tile.tile_bounds;
    for poi in data.poi_nodes() {
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

fn render_tree_nodes(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let height_map = ctx.height_map;
    let data = ctx.data;
    let conv = ctx.conv;

    let bounds = tile.tile_bounds;
    for tree in data.tree_nodes() {
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

// ── Layer 8: POI decorations ────────────────────────────────────────────────

fn render_poi_decorations(world: &mut dyn WorldWriter, ctx: &RenderContext, tile: &TileWays) {
    let height_map = ctx.height_map;
    let data = ctx.data;
    let conv = ctx.conv;

    let bounds = tile.tile_bounds;
    for poi in data.poi_nodes() {
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

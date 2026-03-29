//! Geometry helpers for rendering OSM features as Minecraft blocks.
//!
//! All `draw_*` functions write blocks into a [`bedrock::BedrockWorld`] or
//! a tile-bounded variant.  They accept a `get_surface: F` closure so they
//! can query terrain height at arbitrary positions.

use crate::bedrock;
use crate::blocks::{self, Block};
use crate::convert::rasterize_line;
use std::collections::HashMap;

// ── Bridge / tunnel geometry constants ────────────────────────────────────────
pub const BRIDGE_HEIGHT: i32 = 4; // blocks above terrain for bridge deck
pub const TUNNEL_DEPTH: i32 = 3; // blocks below terrain for tunnel floor
pub const TUNNEL_HEADROOM: i32 = 4; // clear air blocks above tunnel road
pub const SLOPE_LEN: usize = 8; // ramp length in center-line points

/// Compute the unit perpendicular (±1 in each axis) for a road segment.
/// Returns `(perp_x, perp_z)` — the sidewalk offset direction.
/// Falls back to `(1, 0)` for zero-length segments.
pub fn road_perpendicular(x0: i32, z0: i32, x1: i32, z1: i32) -> (i32, i32) {
    let dx = (x1 - x0) as f64;
    let dz = (z1 - z0) as f64;
    let len = (dx * dx + dz * dz).sqrt();
    if len < 0.5 {
        return (1, 0);
    }
    let perp_x = (-dz / len).round() as i32;
    let perp_z = (dx / len).round() as i32;
    if perp_x == 0 && perp_z == 0 {
        (1, 0)
    } else {
        (perp_x, perp_z)
    }
}

/// Compute the effective ramp slope length for a way with `total` center-line
/// points and a configured `slope_len`. Mirrors the formula used in
/// `bridge_y_offsets` so that ramp-section detection is always consistent.
pub fn bridge_effective_slope(total: usize, slope_len: usize) -> usize {
    (if total <= 2 * slope_len {
        total / 2
    } else {
        slope_len
    })
    .max(1)
}

/// Compute per-center-point Y offsets for a bridge or tunnel way.
///
/// Returns a Vec of length `total` where each value is in `0..=height`.
/// The shape is: ramp 0→height over the first `slope_len` points, flat at
/// `height`, then ramp height→0 over the last `slope_len` points.
///
/// For short ways (total <= 2 * slope_len) the effective slope is `total / 2`
/// so both ends meet at the midpoint with no flat section.
pub fn bridge_y_offsets(total: usize, height: i32, slope_len: usize) -> Vec<i32> {
    if total == 0 {
        return vec![];
    }
    let effective_slope = bridge_effective_slope(total, slope_len);
    (0..total)
        .map(|i| {
            let from_end = total.saturating_sub(1 + i);
            let ramp_pos = i.min(from_end).min(effective_slope);
            height * ramp_pos as i32 / effective_slope as i32
        })
        .collect()
}

/// Draw a bridge: elevated road deck with railings, deck underside, support
/// pillars, and gravel-filled ramp approaches.
///
/// `get_surface` returns the terrain Y at a given (x, z). `bridge_y` at each
/// center-line point is `terrain_y + y_offset` where `y_offset` comes from
/// `bridge_y_offsets`.
pub fn draw_bridge<F>(
    world: &mut bedrock::BedrockWorld,
    pts: &[(i32, i32)],
    get_surface: F,
    style: &blocks::RoadStyle,
) where
    F: Fn(i32, i32) -> i32,
{
    if pts.len() < 2 {
        return;
    }
    let hw = style.half_width;

    // Build a flat list of (cx, cz, perp_x, perp_z) for all rasterized center-line points.
    // Drop the last point of each segment (except the final segment) to avoid duplicate
    // join points while keeping the correct perpendicular for every point.
    let all_center: Vec<(i32, i32, i32, i32)> = pts
        .windows(2)
        .enumerate()
        .flat_map(|(seg_idx, w)| {
            let (x0, z0) = w[0];
            let (x1, z1) = w[1];
            let is_last_seg = seg_idx == pts.len() - 2;
            let (px, pz) = road_perpendicular(x0, z0, x1, z1);
            let points = rasterize_line(x0, z0, x1, z1);
            let n = points.len();
            let take = if is_last_seg { n } else { n.saturating_sub(1) };
            points
                .into_iter()
                .take(take)
                .map(move |(cx, cz)| (cx, cz, px, pz))
        })
        .collect();

    let total = all_center.len();
    if total == 0 {
        return;
    }
    let y_offsets = bridge_y_offsets(total, BRIDGE_HEIGHT, SLOPE_LEN);
    let effective_slope = bridge_effective_slope(total, SLOPE_LEN);
    let in_ramp = |i: usize| i < effective_slope || i >= total.saturating_sub(effective_slope);

    for (i, &(cx, cz, perp_x, perp_z)) in all_center.iter().enumerate() {
        let sy = get_surface(cx, cz);
        let y_off = y_offsets[i];
        let bridge_y = sy + y_off;
        let ramp = in_ramp(i);

        // Road surface at bridge_y, square ±hw around center
        for dx in -hw..=hw {
            for dz in -hw..=hw {
                world.set_block(cx + dx, bridge_y, cz + dz, style.surface);
            }
        }

        // Center line skipped — no yellow slab exists in vanilla Bedrock

        // Deck underside: StoneSlab one block below, one block wider than road
        for dx in -(hw + 1)..=(hw + 1) {
            for dz in -(hw + 1)..=(hw + 1) {
                world.set_block(cx + dx, bridge_y - 1, cz + dz, Block::StoneSlab);
            }
        }

        // Railings: CobblestoneWall at bridge_y, perpendicular edges ±(hw+1)
        let rx = perp_x * (hw + 1);
        let rz = perp_z * (hw + 1);
        world.set_block(cx + rx, bridge_y, cz + rz, Block::CobblestoneWall);
        world.set_block(cx - rx, bridge_y, cz - rz, Block::CobblestoneWall);

        // Ramp approach: fill Gravel below road surface down to terrain
        if ramp && bridge_y > sy {
            for dx in -hw..=hw {
                for dz in -hw..=hw {
                    for fill_y in (sy + 1)..bridge_y {
                        world.set_block(cx + dx, fill_y, cz + dz, Block::Gravel);
                    }
                }
            }
        }

        // Support pillars: every SLOPE_LEN points, not in ramp sections
        if !ramp && i % SLOPE_LEN == 0 {
            // hw >= 2: dual pillars at ±hw perpendicular; otherwise single center pillar
            let pillar_offsets: Vec<(i32, i32)> = if hw >= 2 {
                vec![(perp_x * hw, perp_z * hw), (-perp_x * hw, -perp_z * hw)]
            } else {
                vec![(0, 0)]
            };
            for (ox, oz) in pillar_offsets {
                let px = cx + ox;
                let pz = cz + oz;
                let psy = get_surface(px, pz);
                // Only draw pillar if there's room between terrain and deck underside
                if bridge_y - 2 >= psy + 1 {
                    for py in (psy + 1)..=(bridge_y - 2) {
                        world.set_block(px, py, pz, Block::StoneBrick);
                    }
                }
            }
        }
    }
}

/// Draw a tunnel: excavated road channel with walls, ceiling, torch lighting,
/// and portal arches at the tunnel mouth. Open-cut ramp sections at each end.
///
/// `get_surface` returns the terrain Y at a given (x, z). The tunnel floor
/// (`tunnel_y`) at each center-line point is `terrain_y - y_offset` where
/// `y_offset` comes from `bridge_y_offsets` (same ramp shape, sign negated).
pub fn draw_tunnel<F>(
    world: &mut bedrock::BedrockWorld,
    pts: &[(i32, i32)],
    get_surface: F,
    style: &blocks::RoadStyle,
) where
    F: Fn(i32, i32) -> i32,
{
    if pts.len() < 2 {
        return;
    }
    let hw = style.half_width;

    // Same segment-join dedup pattern as draw_bridge: drop last point of non-final segments.
    let all_center: Vec<(i32, i32, i32, i32)> = pts
        .windows(2)
        .enumerate()
        .flat_map(|(seg_idx, w)| {
            let (x0, z0) = w[0];
            let (x1, z1) = w[1];
            let is_last_seg = seg_idx == pts.len() - 2;
            let (px, pz) = road_perpendicular(x0, z0, x1, z1);
            let points = rasterize_line(x0, z0, x1, z1);
            let n = points.len();
            let take = if is_last_seg { n } else { n.saturating_sub(1) };
            points
                .into_iter()
                .take(take)
                .map(move |(cx, cz)| (cx, cz, px, pz))
        })
        .collect();

    let total = all_center.len();
    if total == 0 {
        return;
    }
    // Reuse bridge_y_offsets: offsets are positive; negate to go downward.
    let y_offsets = bridge_y_offsets(total, TUNNEL_DEPTH, SLOPE_LEN);
    let effective_slope = bridge_effective_slope(total, SLOPE_LEN);
    let in_ramp = |i: usize| i < effective_slope || i >= total.saturating_sub(effective_slope);

    // Portal arch at first and last full-depth index.
    let portal_enter = effective_slope;
    let portal_exit = total.saturating_sub(effective_slope + 1);
    // Only render portal arch if indices are in the non-ramp section.
    let has_portal = portal_enter < total && !in_ramp(portal_enter);

    // ── Pass 1: Excavation ────────────────────────────────────────────────────
    // Excavate everything first so that structural blocks placed in Pass 2 are
    // not cleared by neighbouring points' excavation rectangles.
    for (i, &(cx, cz, _perp_x, _perp_z)) in all_center.iter().enumerate() {
        let sy = get_surface(cx, cz);
        let y_off = y_offsets[i];
        let tunnel_y = sy - y_off;
        let ramp = in_ramp(i);

        if ramp {
            // Open-cut ramp: excavate terrain down to ramp surface.
            for dx in -(hw + 2)..=(hw + 2) {
                for dz in -(hw + 2)..=(hw + 2) {
                    for cut_y in tunnel_y..=sy {
                        world.set_block(cx + dx, cut_y, cz + dz, Block::Air);
                    }
                }
            }
        } else {
            // Enclosed section: clear interior from tunnel_y to tunnel_y + TUNNEL_HEADROOM.
            for dx in -(hw + 2)..=(hw + 2) {
                for dz in -(hw + 2)..=(hw + 2) {
                    for cut_y in tunnel_y..=(tunnel_y + TUNNEL_HEADROOM) {
                        world.set_block(cx + dx, cut_y, cz + dz, Block::Air);
                    }
                }
            }
        }
    }

    // ── Pass 2: Structural blocks, road surface, lighting ─────────────────────
    for (i, &(cx, cz, perp_x, perp_z)) in all_center.iter().enumerate() {
        let sy = get_surface(cx, cz);
        let y_off = y_offsets[i];
        let tunnel_y = sy - y_off;
        let ramp = in_ramp(i);

        if ramp {
            // Road surface at ramp level.
            for dx in -hw..=hw {
                for dz in -hw..=hw {
                    world.set_block(cx + dx, tunnel_y, cz + dz, style.surface);
                }
            }
            continue;
        }

        // ── Enclosed tunnel section ──────────────────────────────────────────

        // Road surface at tunnel_y, width ±hw
        for dx in -hw..=hw {
            for dz in -hw..=hw {
                world.set_block(cx + dx, tunnel_y, cz + dz, style.surface);
            }
        }

        // Walls: StoneBrick from tunnel_y to tunnel_y + TUNNEL_HEADROOM - 1, at ±(hw+1) perp
        // Portal columns extend one block higher than normal walls (to tunnel_y+TUNNEL_HEADROOM)
        // so they frame the full height including the ceiling course.
        let wx = perp_x * (hw + 1);
        let wz = perp_z * (hw + 1);
        for wy in tunnel_y..=(tunnel_y + TUNNEL_HEADROOM - 1) {
            world.set_block(cx + wx, wy, cz + wz, Block::StoneBrick);
            world.set_block(cx - wx, wy, cz - wz, Block::StoneBrick);
        }

        // Ceiling: StoneBrick at tunnel_y + TUNNEL_HEADROOM, width ±(hw+1)
        for dx in -(hw + 1)..=(hw + 1) {
            for dz in -(hw + 1)..=(hw + 1) {
                world.set_block(
                    cx + dx,
                    tunnel_y + TUNNEL_HEADROOM,
                    cz + dz,
                    Block::StoneBrick,
                );
            }
        }

        // Lighting: Torch at wall at tunnel_y + 2, every 12 points, alternating sides
        if i % 12 == 0 {
            let side = if (i / 12) % 2 == 0 { 1i32 } else { -1i32 };
            let lx = cx + perp_x * (hw + 1) * side;
            let lz = cz + perp_z * (hw + 1) * side;
            world.set_block(lx, tunnel_y + 2, lz, Block::Torch);
        }

        // Portal arch: at the first and last full-depth points
        if has_portal && (i == portal_enter || i == portal_exit) {
            // Excavate the lintel row (one above the ceiling, not cleared by main excavation pass)
            for dx in -(hw + 1)..=(hw + 1) {
                for dz in -(hw + 1)..=(hw + 1) {
                    world.set_block(cx + dx, tunnel_y + TUNNEL_HEADROOM + 1, cz + dz, Block::Air);
                }
            }
            // Columns: StoneBrick from tunnel_y through tunnel_y + TUNNEL_HEADROOM on both sides
            for py in tunnel_y..=(tunnel_y + TUNNEL_HEADROOM) {
                world.set_block(cx + wx, py, cz + wz, Block::StoneBrick);
                world.set_block(cx - wx, py, cz - wz, Block::StoneBrick);
            }
            // Lintel: StoneBrick at tunnel_y + TUNNEL_HEADROOM + 1 across full width ±(hw+1)
            for dx in -(hw + 1)..=(hw + 1) {
                for dz in -(hw + 1)..=(hw + 1) {
                    world.set_block(
                        cx + dx,
                        tunnel_y + TUNNEL_HEADROOM + 1,
                        cz + dz,
                        Block::StoneBrick,
                    );
                }
            }
        }
    }
}

/// Draw a road with width, optional sidewalks, center line, edge lines, curbs,
/// benches, and street lighting.
///
/// `get_surface` returns the ground Y for a given (block_x, block_z), allowing
/// roads to follow real terrain when elevation data is available.
pub fn draw_road<F>(
    world: &mut bedrock::BedrockWorld,
    pts: &[(i32, i32)],
    get_surface: F,
    style: &blocks::RoadStyle,
) where
    F: Fn(i32, i32) -> i32,
{
    let hw = style.half_width;
    // Sidewalk is 1 block wide on each side
    let sidewalk_hw = hw + 1;
    // Furniture sits just outside the sidewalk, on the grass
    let furniture_offset = sidewalk_hw + 1;

    // Pre-compute all segments once (rasterize_line is called once per segment).
    // Each entry: (center_line_points, perp_x, perp_z)
    #[allow(clippy::type_complexity)]
    let segments: Vec<(Vec<(i32, i32)>, i32, i32)> = pts
        .windows(2)
        .map(|w| {
            let (x0, z0) = w[0];
            let (x1, z1) = w[1];
            let cl = rasterize_line(x0, z0, x1, z1);
            let (px, pz) = road_perpendicular(x0, z0, x1, z1);
            (cl, px, pz)
        })
        .collect();

    // Pass 1 — sidewalks (must come before road so road always wins on overlap)
    if style.sidewalk {
        for (center_line, _, _) in &segments {
            for (cx, cz) in center_line {
                for dx in -sidewalk_hw..=sidewalk_hw {
                    for dz in -sidewalk_hw..=sidewalk_hw {
                        // Fill the full 2-block band outside the road surface
                        if dx.abs() > hw || dz.abs() > hw {
                            let sy = get_surface(cx + dx, cz + dz);
                            world.set_block(cx + dx, sy, cz + dz, style.sidewalk_surface);
                        }
                    }
                }
            }
        }
    }

    // Pass 2 — road surface, markings, and center-point collection
    let mut all_center_points: Vec<(i32, i32, i32, i32)> = Vec::new();

    for (center_line, perp_x, perp_z) in &segments {
        for (cx, cz) in center_line {
            for dx in -hw..=hw {
                for dz in -hw..=hw {
                    let sy = get_surface(cx + dx, cz + dz);
                    world.set_block(cx + dx, sy, cz + dz, style.surface);
                }
            }
        }

        // Center line skipped — no yellow slab exists in vanilla Bedrock

        // Edge lines (white solid) — perpendicular to road direction, at each edge
        if style.edge_lines {
            for (cx, cz) in center_line {
                world.set_block(
                    cx + perp_x * hw,
                    get_surface(cx + perp_x * hw, cz + perp_z * hw),
                    cz + perp_z * hw,
                    Block::WhiteConcrete,
                );
                world.set_block(
                    cx - perp_x * hw,
                    get_surface(cx - perp_x * hw, cz - perp_z * hw),
                    cz - perp_z * hw,
                    Block::WhiteConcrete,
                );
            }
        }

        all_center_points.extend(
            center_line
                .iter()
                .map(|&(cx, cz)| (cx, cz, *perp_x, *perp_z)),
        );
    }

    // Sidewalk furniture: street lights
    if style.sidewalk {
        for (i, &(cx, cz, perp_x, perp_z)) in all_center_points.iter().enumerate() {
            // Street lighting every 30 blocks — alternate sides, on inner sidewalk tile
            if i % 30 == 0 {
                let side = if (i / 30) % 2 == 0 { 1i32 } else { -1i32 };
                let fx = cx + perp_x * furniture_offset * side;
                let fz = cz + perp_z * furniture_offset * side;
                let fsy = get_surface(fx, fz);
                world.set_block(fx, fsy + 1, fz, Block::OakFence);
                world.set_block(fx, fsy + 2, fz, Block::OakFence);
                world.set_block(fx, fsy + 3, fz, Block::Lantern);
            }
        }
    }
}

/// Draw building walls (perimeter + solid floor/ceiling) with material variety,
/// interior floors, windows, and door openings.
pub fn draw_building(
    world: &mut bedrock::BedrockWorld,
    pts: &[(i32, i32)],
    surface: i32,
    height: i32,
    tags: &HashMap<String, String>,
    road_dir: Option<(f64, f64)>,
) {
    let wall = blocks::building_block(tags);

    // Floor + ceiling via polygon fill
    let floor_pts = crate::convert::rasterize_polygon(pts);
    for &(x, z) in &floor_pts {
        world.set_block(x, surface, z, wall); // floor (replace grass)
        world.set_block(x, surface + height, z, wall); // ceiling
    }

    // Walls: perimeter line, repeated for each story
    let n = pts.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let (x0, z0) = pts[i];
        let (x1, z1) = pts[j];
        for (x, z) in rasterize_line(x0, z0, x1, z1) {
            for dy in 1..height {
                world.set_block(x, surface + dy, z, wall);
            }
        }
    }

    // Compute bounding box from pts
    if pts.is_empty() {
        return;
    }
    let bmin_x = pts.iter().map(|p| p.0).min().unwrap();
    let bmax_x = pts.iter().map(|p| p.0).max().unwrap();
    let bmin_z = pts.iter().map(|p| p.1).min().unwrap();
    let bmax_z = pts.iter().map(|p| p.1).max().unwrap();
    let width_x = bmax_x - bmin_x;
    let width_z = bmax_z - bmin_z;

    // Only add interiors/windows/doors for buildings >= 4x4
    if width_x < 4 || width_z < 4 {
        return;
    }

    // Interior floors
    let levels: i32 = tags
        .get("building:levels")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or_else(|| (height / 4).max(1));
    let floor_height = height / levels;
    if floor_height > 1 {
        for level in 1..levels {
            let floor_y = surface + level * floor_height;
            for &(x, z) in &floor_pts {
                // Use wall material for structural floor (full block, no gap)
                world.set_block(x, floor_y, z, wall);
            }
        }
    }

    // Windows on perimeter walls: every 3rd block, at floor_y + 2 for each level
    let perimeter_edges: Vec<Vec<(i32, i32)>> = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            rasterize_line(pts[i].0, pts[i].1, pts[j].0, pts[j].1)
        })
        .collect();

    for level in 0..levels {
        let base_y = surface + level * floor_height;
        let window_y = base_y + 2;
        if window_y >= surface + height {
            continue;
        }
        for edge in &perimeter_edges {
            if edge.len() < 3 {
                continue;
            }
            // Skip first and last block on each edge to preserve corners
            for (idx, &(x, z)) in edge.iter().enumerate() {
                if idx == 0 || idx == edge.len() - 1 {
                    continue;
                }
                if idx % 3 == 0 {
                    world.set_block(x, window_y, z, Block::GlassPane);
                }
            }
        }
    }

    // Door opening: pick the wall edge whose outward normal best faces the road.
    // Falls back to the longest edge when no road direction is available.
    let centroid_x = pts.iter().map(|p| p.0 as f64).sum::<f64>() / n as f64;
    let centroid_z = pts.iter().map(|p| p.1 as f64).sum::<f64>() / n as f64;

    let mut best_score = f64::NEG_INFINITY;
    let mut best_door_edge = 0usize;
    for (idx, edge) in perimeter_edges.iter().enumerate() {
        let elen = edge.len();
        if elen < 3 {
            continue;
        }
        let edge_dx = (pts[(idx + 1) % n].0 - pts[idx].0) as f64;
        let edge_dz = (pts[(idx + 1) % n].1 - pts[idx].1) as f64;
        let edge_len = (edge_dx * edge_dx + edge_dz * edge_dz).sqrt();
        if edge_len < 0.5 {
            continue;
        }
        // Midpoint of this edge
        let mid = elen / 2;
        let mx = edge[mid].0 as f64;
        let mz = edge[mid].1 as f64;
        // Two perpendicular candidates; pick the one pointing away from centroid
        let n1 = (edge_dz / edge_len, -edge_dx / edge_len);
        let n2 = (-edge_dz / edge_len, edge_dx / edge_len);
        let out_dx = mx - centroid_x;
        let out_dz = mz - centroid_z;
        let outward = if n1.0 * out_dx + n1.1 * out_dz >= 0.0 {
            n1
        } else {
            n2
        };

        let score = if let Some((rdx, rdz)) = road_dir {
            let road_len = (rdx * rdx + rdz * rdz).sqrt().max(1.0);
            outward.0 * rdx / road_len + outward.1 * rdz / road_len
        } else {
            elen as f64 // fallback: longest edge
        };

        if score > best_score {
            best_score = score;
            best_door_edge = idx;
        }
    }
    let door_edge = &perimeter_edges[best_door_edge];
    if door_edge.len() >= 3 {
        let mid = door_edge.len() / 2;
        let (dx, dz) = door_edge[mid];
        world.set_block(dx, surface + 1, dz, Block::Air);
        world.set_block(dx, surface + 2, dz, Block::Air);
    }
}

/// Draw a roof on top of a building.
pub fn draw_roof(
    world: &mut bedrock::BedrockWorld,
    pts: &[(i32, i32)],
    surface: i32,
    height: i32,
    tags: &HashMap<String, String>,
) {
    if pts.is_empty() {
        return;
    }

    let roof_shape = tags.get("roof:shape").map(|s| s.as_str()).unwrap_or("flat");
    if roof_shape == "flat" {
        return;
    }

    let wall = blocks::building_block(tags);
    let stair_block = if wall == Block::StoneBrick {
        Block::StoneBrickStairs
    } else {
        Block::OakStairs
    };

    let bmin_x = pts.iter().map(|p| p.0).min().unwrap();
    let bmax_x = pts.iter().map(|p| p.0).max().unwrap();
    let bmin_z = pts.iter().map(|p| p.1).min().unwrap();
    let bmax_z = pts.iter().map(|p| p.1).max().unwrap();
    let width_x = bmax_x - bmin_x;
    let width_z = bmax_z - bmin_z;
    let roof_base = surface + height;

    match roof_shape {
        "gabled" => {
            // Gable along the longest axis
            if width_x >= width_z {
                // Ridge runs along X axis, stairs from north and south edges
                let half_z = width_z / 2;
                for layer in 0..=half_z {
                    let y = roof_base + layer + 1;
                    let z_north = bmin_z + layer;
                    let z_south = bmax_z - layer;
                    for x in bmin_x..=bmax_x {
                        if z_north != z_south {
                            // North side stairs (direction 2 = south-facing)
                            world.set_block(x, y, z_north, stair_block);
                            world.set_block_direction(x, y, z_north, 2);
                            // South side stairs (direction 3 = north-facing)
                            world.set_block(x, y, z_south, stair_block);
                            world.set_block_direction(x, y, z_south, 3);
                        } else {
                            // Ridge line
                            world.set_block(x, y, z_north, wall);
                        }
                    }
                }
                // Fill gable triangles on east and west ends
                for layer in 0..=half_z {
                    let y = roof_base + layer + 1;
                    for z in (bmin_z + layer)..=(bmax_z - layer) {
                        world.set_block(bmin_x, y, z, wall);
                        world.set_block(bmax_x, y, z, wall);
                    }
                }
            } else {
                // Ridge runs along Z axis, stairs from east and west edges
                let half_x = width_x / 2;
                for layer in 0..=half_x {
                    let y = roof_base + layer + 1;
                    let x_west = bmin_x + layer;
                    let x_east = bmax_x - layer;
                    for z in bmin_z..=bmax_z {
                        if x_west != x_east {
                            // West side stairs (direction 0 = east-facing)
                            world.set_block(x_west, y, z, stair_block);
                            world.set_block_direction(x_west, y, z, 0);
                            // East side stairs (direction 1 = west-facing)
                            world.set_block(x_east, y, z, stair_block);
                            world.set_block_direction(x_east, y, z, 1);
                        } else {
                            // Ridge line
                            world.set_block(x_west, y, z, wall);
                        }
                    }
                }
                // Fill gable triangles on north and south ends
                for layer in 0..=half_x {
                    let y = roof_base + layer + 1;
                    for x in (bmin_x + layer)..=(bmax_x - layer) {
                        world.set_block(x, y, bmin_z, wall);
                        world.set_block(x, y, bmax_z, wall);
                    }
                }
            }
        }
        "pyramidal" | "hipped" => {
            // Stairs ascending inward from all 4 edges
            let max_layers = (width_x.min(width_z) / 2).max(1);
            for layer in 0..max_layers {
                let y = roof_base + layer + 1;
                let x0 = bmin_x + layer;
                let x1 = bmax_x - layer;
                let z0 = bmin_z + layer;
                let z1 = bmax_z - layer;
                if x0 > x1 || z0 > z1 {
                    break;
                }
                // North edge (direction 2 = south-facing)
                for x in x0..=x1 {
                    world.set_block(x, y, z0, stair_block);
                    world.set_block_direction(x, y, z0, 2);
                }
                // South edge (direction 3 = north-facing)
                for x in x0..=x1 {
                    world.set_block(x, y, z1, stair_block);
                    world.set_block_direction(x, y, z1, 3);
                }
                // West edge (direction 0 = east-facing)
                for z in (z0 + 1)..z1 {
                    world.set_block(x0, y, z, stair_block);
                    world.set_block_direction(x0, y, z, 0);
                }
                // East edge (direction 1 = west-facing)
                for z in (z0 + 1)..z1 {
                    world.set_block(x1, y, z, stair_block);
                    world.set_block_direction(x1, y, z, 1);
                }
            }
            // Cap the top with wall material
            let top_y = roof_base + max_layers + 1;
            let cx0 = bmin_x + max_layers;
            let cx1 = bmax_x - max_layers;
            let cz0 = bmin_z + max_layers;
            let cz1 = bmax_z - max_layers;
            if cx0 <= cx1 && cz0 <= cz1 {
                for x in cx0..=cx1 {
                    for z in cz0..=cz1 {
                        world.set_block(x, top_y, z, wall);
                    }
                }
            }
        }
        _ => {} // Unknown roof shape, skip
    }
}

/// Draw a waterway channel with variable width and depth.
///
/// For each rasterized center-line point, expands axis-aligned by `style.half_width`,
/// digs from the terrain surface down by `style.depth` with Water, places Sand at
/// the riverbed, and for wide channels (half_width >= 2) places Dirt banks one step
/// outside the water edge (only over terrain blocks).
///
/// `get_surface` returns the ground Y for a given (block_x, block_z), allowing
/// waterways to cut into real terrain when elevation data is available.
pub fn draw_waterway<F>(
    world: &mut bedrock::BedrockWorld,
    pts: &[(i32, i32)],
    get_surface: F,
    style: &blocks::WaterwayStyle,
) where
    F: Fn(i32, i32) -> i32,
{
    let hw = style.half_width;
    let depth = style.depth;

    for w in pts.windows(2) {
        let (x0, z0) = w[0];
        let (x1, z1) = w[1];

        for (cx, cz) in rasterize_line(x0, z0, x1, z1) {
            // Water channel + sand bed
            for dx in -hw..=hw {
                for dz in -hw..=hw {
                    let wx = cx + dx;
                    let wz = cz + dz;
                    let sy = get_surface(wx, wz);
                    // Sand riverbed
                    world.set_block(wx, sy - depth, wz, Block::Sand);
                    // Water fill (from surface down to surface - depth + 1)
                    for dy in (-(depth - 1))..=0 {
                        world.set_block(wx, sy + dy, wz, Block::Water);
                    }
                }
            }

            // Bank transition for river/canal (half_width >= 2)
            // Only place Dirt over terrain — do not overwrite roads, buildings, etc.
            if hw >= 2 {
                let bank_hw = hw + 1;
                for dx in -bank_hw..=bank_hw {
                    for dz in -bank_hw..=bank_hw {
                        // Only the outer ring is the bank
                        if dx.abs() == bank_hw || dz.abs() == bank_hw {
                            let bx = cx + dx;
                            let bz = cz + dz;
                            let bsy = get_surface(bx, bz);
                            let existing = world.get_block(bx, bsy - 1, bz);
                            if matches!(existing, Block::GrassBlock | Block::Dirt | Block::Air) {
                                world.set_block(bx, bsy - 1, bz, Block::Dirt);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::WaterwayStyle;
    use std::path::Path;

    #[test]
    fn perp_east_west_road() {
        // Road going east: (0,0)→(10,0), direction=(10,0)
        // Perpendicular = (-dz/len, dx/len) = (0/10, 10/10) rounded = (0, 1)
        assert_eq!(road_perpendicular(0, 0, 10, 0), (0, 1));
    }

    #[test]
    fn perp_north_south_road() {
        // Road going north (−Z): (0,0)→(0,-10), direction=(0,-10)
        // Perpendicular = (-(-10)/10, 0/10) = (1, 0)
        assert_eq!(road_perpendicular(0, 0, 0, -10), (1, 0));
    }

    #[test]
    fn perp_diagonal_road() {
        // Road going NE: (0,0)→(10,-10), direction=(10,-10), len≈14.14
        let (px, pz) = road_perpendicular(0, 0, 10, -10);
        assert!(px == 1 || px == 0, "px={px}");
        assert!(pz == 1 || pz == 0, "pz={pz}");
        assert!(px != 0 || pz != 0, "perpendicular must be non-zero");
    }

    #[test]
    fn perp_zero_length_segment_defaults() {
        // Degenerate segment: same point → return (1, 0)
        assert_eq!(road_perpendicular(5, 5, 5, 5), (1, 0));
    }

    #[test]
    fn draw_waterway_river_depth_and_width() {
        let mut world = bedrock::BedrockWorld::new(Path::new("/tmp/test_waterway_world"));
        let style = WaterwayStyle {
            half_width: 1,
            depth: 3,
        };
        let pts = vec![(0i32, 0i32), (4i32, 0i32)];
        let surface = 65;

        draw_waterway(&mut world, &pts, |_, _| surface, &style);

        // Channel center should have water at surface
        assert_eq!(world.get_block(2, surface, 0), Block::Water);
        // Water fills depth (surface-1, surface-2)
        assert_eq!(world.get_block(2, surface - 1, 0), Block::Water);
        assert_eq!(world.get_block(2, surface - 2, 0), Block::Water);
        // Sand riverbed one block below the water
        assert_eq!(world.get_block(2, surface - 3, 0), Block::Sand);
        // Width expansion: block at half_width distance should also be water
        assert_eq!(world.get_block(2, surface, 1), Block::Water);
        assert_eq!(world.get_block(2, surface, -1), Block::Water);
    }

    #[test]
    fn draw_waterway_ditch_single_block() {
        let mut world = bedrock::BedrockWorld::new(Path::new("/tmp/test_ditch_world"));
        let style = WaterwayStyle {
            half_width: 0,
            depth: 1,
        };
        let pts = vec![(0i32, 0i32), (3i32, 0i32)];
        let surface = 65;

        draw_waterway(&mut world, &pts, |_, _| surface, &style);

        // Single water block at surface
        assert_eq!(world.get_block(1, surface, 0), Block::Water);
        // Sand one below
        assert_eq!(world.get_block(1, surface - 1, 0), Block::Sand);
        // No width expansion — adjacent block should not be water
        assert_eq!(world.get_block(1, surface, 1), Block::Air);
    }

    #[test]
    fn bridge_y_offset_ramps_correctly() {
        // 20 points, BRIDGE_HEIGHT=4, slope_len=4 → [0,1,2,3,4,…(12 fours)…,3,2,1,0]
        let offsets = bridge_y_offsets(20, 4, 4);
        assert_eq!(offsets.len(), 20);
        // Ramp up
        assert_eq!(offsets[0], 0);
        assert_eq!(offsets[1], 1);
        assert_eq!(offsets[2], 2);
        assert_eq!(offsets[3], 3);
        // Flat middle
        assert_eq!(offsets[4], 4);
        assert_eq!(offsets[10], 4);
        assert_eq!(offsets[15], 4);
        // Ramp down
        assert_eq!(offsets[16], 3);
        assert_eq!(offsets[17], 2);
        assert_eq!(offsets[18], 1);
        assert_eq!(offsets[19], 0);
    }

    #[test]
    fn bridge_y_offset_empty_returns_empty() {
        assert_eq!(bridge_y_offsets(0, 4, 4), vec![] as Vec<i32>);
    }

    #[test]
    fn bridge_y_offset_exact_2x_slope_len() {
        // total == 2*slope_len → short-way formula, peak at midpoint
        let offsets = bridge_y_offsets(8, 4, 4);
        assert_eq!(offsets.len(), 8);
        assert_eq!(offsets[0], 0);
        assert_eq!(offsets[3], 3); // ramp up
        assert_eq!(offsets[4], 3); // symmetric peak (total/2=4, ramp_pos capped at 3)
        assert_eq!(offsets[7], 0); // ramp down
    }

    #[test]
    fn bridge_has_pillars_at_interval() {
        let mut world = bedrock::BedrockWorld::new(Path::new("/tmp/test_bridge_pillars"));
        // Straight horizontal way: 40 points along X axis
        let pts: Vec<(i32, i32)> = (0..=39).map(|x| (x, 0)).collect();
        let style = blocks::RoadStyle {
            surface: Block::PolishedBlackstoneSlab,
            sidewalk_surface: Block::SmoothStoneSlab,
            half_width: 2,
            sidewalk: false,
            center_line: false,
            edge_lines: false,
        };
        let sea_level = 65i32;
        draw_bridge(&mut world, &pts, |_, _| sea_level, &style);

        // SLOPE_LEN=8 → ramp is first/last 8 points. Flat section starts at index 8.
        // Pillars placed every 8 points in flat section → first pillar at index 8 (cx=8).
        // hw=2 ≥ 2 → two pillars at perp offsets ±hw.
        // For horizontal road along X, perpendicular = (0, 1).
        // Pillars at (cx, cz±2). bridge_y = 65 + 4 = 69.
        // Pillar spans bridge_y-2=67 down to sy+1=66.
        let bridge_y = sea_level + BRIDGE_HEIGHT; // 69
        // Check pillar at cx=16 (well inside flat section, index 16 % 8 == 0)
        assert_eq!(
            world.get_block(16, bridge_y - 2, 2),
            Block::StoneBrick,
            "pillar top at bridge_y-2 (+z side) should be StoneBrick"
        );
        assert_eq!(
            world.get_block(16, bridge_y - 3, 2),
            Block::StoneBrick,
            "pillar lower at bridge_y-3 (+z side) should be StoneBrick"
        );
        assert_eq!(
            world.get_block(16, bridge_y - 2, -2),
            Block::StoneBrick,
            "pillar top at bridge_y-2 (-z side) should be StoneBrick"
        );
        // Verify no pillar in ramp section (cx=4, index 4 < SLOPE_LEN=8)
        assert_ne!(
            world.get_block(4, bridge_y - 2, 2),
            Block::StoneBrick,
            "no pillar in ramp section"
        );
    }

    #[test]
    fn tunnel_clears_blocks_above_road() {
        let mut world = bedrock::BedrockWorld::new(Path::new("/tmp/test_tunnel_clear"));
        // Straight horizontal way: 30 points along X axis
        let pts: Vec<(i32, i32)> = (0..=29).map(|x| (x, 0)).collect();
        let style = blocks::RoadStyle {
            surface: Block::PolishedBlackstoneSlab,
            sidewalk_surface: Block::SmoothStoneSlab,
            half_width: 2,
            sidewalk: false,
            center_line: false,
            edge_lines: false,
        };
        let sea_level = 65i32;
        draw_tunnel(&mut world, &pts, |_, _| sea_level, &style);

        // SLOPE_LEN=8, TUNNEL_DEPTH=3, TUNNEL_HEADROOM=4
        // At center point cx=15 (index 15, well inside flat section):
        //   tunnel_y = 65 - 3 = 62
        let tunnel_y = sea_level - TUNNEL_DEPTH; // 62
        let hw = 2i32;

        // Interior air: tunnel_y+1 through tunnel_y+TUNNEL_HEADROOM-1 at center column
        assert_eq!(
            world.get_block(15, tunnel_y + 1, 0),
            Block::Air,
            "interior should be Air at tunnel_y+1"
        );
        assert_eq!(
            world.get_block(15, tunnel_y + 2, 0),
            Block::Air,
            "interior should be Air at tunnel_y+2"
        );
        assert_eq!(
            world.get_block(15, tunnel_y + 3, 0),
            Block::Air,
            "interior should be Air at tunnel_y+3"
        );

        // Ceiling at tunnel_y + TUNNEL_HEADROOM, at wall position (hw+1=3 perp units)
        // For horizontal road along X, perp = (0,1), so wall is at cz+3
        assert_eq!(
            world.get_block(15, tunnel_y + TUNNEL_HEADROOM, hw + 1),
            Block::StoneBrick,
            "ceiling at tunnel_y+TUNNEL_HEADROOM should be StoneBrick"
        );

        // Road surface at tunnel_y, center column
        assert_eq!(
            world.get_block(15, tunnel_y, 0),
            Block::PolishedBlackstoneSlab,
            "road surface at tunnel_y should be the road block"
        );
    }
}

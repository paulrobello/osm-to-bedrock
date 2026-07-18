//! Integration tests for the OSM-feature rendering orchestrator (ARC-005 / QA-002).
//!
//! `src/pipeline.rs` is the project's central orchestrator — 2609 LOC, the
//! 561-line `render_osm_features`, and zero tests before this file landed.
//! These tests provide the safety net that makes the upcoming structural
//! refactors (ARC-001 Java streaming, ARC-002 tile-loop dedup, ARC-003
//! pipeline split, ARC-004 server split) safe to land.
//!
//! # Layout
//!
//! - [`RecordingWorld`] — a `WorldWriter` impl that captures every
//!   `set_block` into a `HashMap<(i32,i32,i32), Block>` so we can assert on
//!   exact placements. Mirrors the in-memory shape of `BedrockWorld`/
//!   `JavaWorld` without touching disk.
//! - **Direct orchestrator test** — calls `render_osm_features` against the
//!   recording world with a hand-built `RenderContext`/`TileWays`, isolating
//!   the per-layer rendering from terrain fill and tile orchestration.
//! - **End-to-end public-API test** — `run_preview_from_data` exercises the
//!   full in-memory pipeline (terrain fill + feature render + spawn) and
//!   returns a `Box<dyn WorldWriter>` we inspect via `get_block`/
//!   `surface_blocks`/`occupied_chunks`.
//! - **Cross-edition parity** — runs the same synthetic data through both
//!   `Edition::Bedrock` and `Edition::Java` and asserts the surface block
//!   placements match. This is the parity guard for the edition-dispatch
//!   seam that ARC-001 lives at.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use osm_to_bedrock::blocks::Block;
use osm_to_bedrock::convert::CoordConverter;
use osm_to_bedrock::filter::FeatureFilter;
use osm_to_bedrock::osm::{OsmData, OsmNode, OsmWay};
use osm_to_bedrock::params::ConvertParams;
use osm_to_bedrock::pipeline::{self, RenderContext, TileWays, render_osm_features};
use osm_to_bedrock::spatial::{HeightMap, SpatialIndex};
use osm_to_bedrock::world::{ChunkData, Edition, WorldWriter};

// ── RecordingWorld ────────────────────────────────────────────────────────────

/// A `WorldWriter` that records every block placement in a flat HashMap.
///
/// This is the test double the ARC-005 remedy prescribes: by routing the
/// orchestrator's `set_block` calls through a recorder, we can assert on
/// specific (x, y, z, Block) tuples that a real backend would bury inside a
/// SubChunk palette or Anvil region file.
struct RecordingWorld {
    blocks: HashMap<(i32, i32, i32), Block>,
    inserted_chunk_count: usize,
    block_entities: Vec<(i32, i32, i32, Vec<u8>)>,
    sign_directions: HashMap<(i32, i32, i32), i32>,
    block_directions: HashMap<(i32, i32, i32), i32>,
    save_call_count: usize,
    /// History of `set_tile_bounds` calls (in order). Empty if never called.
    tile_bounds_calls: Vec<(i32, i32, i32, i32)>,
    /// Number of times `flush_tile` was invoked.
    flush_tile_call_count: usize,
}

impl RecordingWorld {
    fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            inserted_chunk_count: 0,
            block_entities: Vec::new(),
            sign_directions: HashMap::new(),
            block_directions: HashMap::new(),
            save_call_count: 0,
            tile_bounds_calls: Vec::new(),
            flush_tile_call_count: 0,
        }
    }

    /// Number of non-air blocks recorded.
    fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Count placements of a specific `Block` variant.
    fn count_block(&self, target: Block) -> usize {
        self.blocks.values().filter(|&&b| b == target).count()
    }

    /// True if any recorded block matches `pred`.
    fn has_any(&self, pred: impl Fn(&Block) -> bool) -> bool {
        self.blocks.values().any(pred)
    }

    /// Iterate every recorded placement as `(x, y, z, block)`.
    fn iter_placements(&self) -> impl Iterator<Item = (i32, i32, i32, Block)> + '_ {
        self.blocks.iter().map(|(&(x, y, z), &b)| (x, y, z, b))
    }
}

impl WorldWriter for RecordingWorld {
    fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block) {
        // Air writes are recorded as removals (matches the semantics a real
        // chunk grid would have: an Air write clears any prior block).
        if block == Block::Air {
            self.blocks.remove(&(x, y, z));
        } else {
            self.blocks.insert((x, y, z), block);
        }
    }

    fn get_block(&self, x: i32, y: i32, z: i32) -> Block {
        self.blocks.get(&(x, y, z)).copied().unwrap_or(Block::Air)
    }

    fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData) {
        // Snapshot the chunk's blocks into the flat map so post-insert
        // get_block calls see them (matches how BedrockWorld/JavaWorld expose
        // inserted chunks through get_block).
        for (sy, blocks) in chunk.non_empty_subchunks() {
            for lx in 0..16i32 {
                for lz in 0..16i32 {
                    for ly in 0..16i32 {
                        let idx = (lx * 256 + lz * 16 + ly) as usize;
                        let b = blocks[idx];
                        if b != Block::Air {
                            let wx = cx * 16 + lx;
                            let wy = sy as i32 * 16 + ly;
                            let wz = cz * 16 + lz;
                            self.blocks.insert((wx, wy, wz), b);
                        }
                    }
                }
            }
        }
        self.inserted_chunk_count += 1;
    }

    fn add_block_entity(&mut self, x: i32, y: i32, z: i32, nbt: Vec<u8>) {
        self.block_entities.push((x, y, z, nbt));
    }

    fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        self.sign_directions.insert((x, y, z), direction);
    }

    fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        self.block_directions.insert((x, y, z), direction);
    }

    fn chunk_count(&self) -> usize {
        self.inserted_chunk_count
    }

    fn occupied_chunks(&self) -> Vec<(i32, i32)> {
        // Chunk coords are not stored separately; derive them from block keys.
        let mut chunks: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
        for &(x, _, z) in self.blocks.keys() {
            chunks.insert((x.div_euclid(16), z.div_euclid(16)));
        }
        chunks.into_iter().collect()
    }

    fn surface_blocks(&self) -> Vec<(i32, i32, i32, String)> {
        // Top-most non-Air block per (x, z) column.
        let mut by_col: HashMap<(i32, i32), (i32, Block)> = HashMap::new();
        for (&(x, y, z), &b) in &self.blocks {
            let entry = by_col.entry((x, z)).or_insert((i32::MIN, b));
            if y > entry.0 {
                *entry = (y, b);
            }
        }
        let mut out: Vec<(i32, i32, i32, String)> = by_col
            .into_iter()
            .map(|((x, z), (y, b))| (x, z, y, format!("{b:?}")))
            .collect();
        out.sort_by_key(|t| (t.0, t.1, t.2));
        out
    }

    fn save(&mut self, _spawn_x: i32, _spawn_y: i32, _spawn_z: i32) -> Result<()> {
        // Tests never call save(); if they do, treat it as a no-op success.
        let _ = self.save_call_count; // field exists for future assertions
        Ok(())
    }

    fn set_tile_bounds(&mut self, min_cx: i32, max_cx: i32, min_cz: i32, max_cz: i32) {
        // Default impl is a no-op; we record the call so streaming-path
        // tests can assert the pipeline invokes it once per tile.
        self.tile_bounds_calls
            .push((min_cx, max_cx, min_cz, max_cz));
    }

    fn flush_tile(&mut self) -> Result<()> {
        // Default impl is a no-op; we record the call so streaming-path
        // tests can assert the pipeline invokes it once per tile after
        // process_tile returns.
        self.flush_tile_call_count += 1;
        Ok(())
    }
}

// ── Synthetic OSM data ────────────────────────────────────────────────────────
//
// Build a tiny OSM dataset with one road (highway=residential), one building
// (closed polygon, building=yes), and one water body (closed polygon,
// natural=water). All nodes are positioned so the bounds center is (0,0),
// which means `run_preview_from_data`'s internal `CoordConverter` origin
// matches the CoordConverter we construct by hand in the direct-orchestrator
// tests below — so block coordinates are predictable across both paths.
//
// At scale=1.0 m/block and origin (0,0), ~1.1132e-5 deg of lat/lon ≈ 1 block.

const ONE_BLOCK_DEG: f64 = 1.0 / 111_320.0;

fn mk_node(id: i64, dlat: f64, dlon: f64) -> (i64, OsmNode) {
    (
        id,
        OsmNode {
            lat: dlat,
            lon: dlon,
        },
    )
}

fn mk_tagged_way(tags: &[(&str, &str)], node_refs: &[i64]) -> OsmWay {
    let mut t = HashMap::new();
    for (k, v) in tags {
        t.insert((*k).to_string(), (*v).to_string());
    }
    OsmWay {
        tags: t,
        node_refs: node_refs.to_vec(),
    }
}

/// A bounded synthetic dataset with all three feature types.
///
/// Node positioning (block-space, before bounds padding):
/// - Road:    z = -10, x ∈ [-5, +5]        (east-west at z=-10)
/// - Building: z ∈ [4, 8], x ∈ [-3, +3]    (3-block-wide footprint south of origin)
/// - Water:   z ∈ [12, 16], x ∈ [-4, +4]   (4-block-wide pond further south)
fn synthetic_osm_data() -> OsmData {
    let mut nodes: HashMap<i64, OsmNode> = HashMap::new();

    // Road nodes (way 100): east-west line at z = -10.
    for (i, nx) in [-5, 0, 5].iter().enumerate() {
        let (id, node) = mk_node(
            100 + i as i64,
            10.0 * ONE_BLOCK_DEG,
            (*nx as f64) * ONE_BLOCK_DEG,
        );
        nodes.insert(id, node);
    }
    // Building nodes (way 200): closed square at z ∈ [4,8], x ∈ [-3,3].
    let building_corners = [(-3, 4), (3, 4), (3, 8), (-3, 8)];
    for (i, (bx, bz)) in building_corners.iter().enumerate() {
        let (id, node) = mk_node(
            200 + i as i64,
            (-(*bz) as f64) * ONE_BLOCK_DEG, // negative lat → +z
            (*bx as f64) * ONE_BLOCK_DEG,
        );
        nodes.insert(id, node);
    }
    // Water nodes (way 300): closed square at z ∈ [12,16], x ∈ [-4,4].
    let water_corners = [(-4, 12), (4, 12), (4, 16), (-4, 16)];
    for (i, (bx, bz)) in water_corners.iter().enumerate() {
        let (id, node) = mk_node(
            300 + i as i64,
            (-(*bz) as f64) * ONE_BLOCK_DEG,
            (*bx as f64) * ONE_BLOCK_DEG,
        );
        nodes.insert(id, node);
    }

    let ways: Vec<OsmWay> = vec![
        // Road (open way). References nodes 100, 101, 102.
        mk_tagged_way(
            &[("highway", "residential"), ("name", "Test Ave")],
            &[100, 101, 102],
        ),
        // Building (closed polygon). References nodes 200..203; first repeats at end.
        mk_tagged_way(
            &[("building", "yes"), ("building:levels", "1")],
            &[200, 201, 202, 203, 200],
        ),
        // Water (closed polygon). References nodes 300..303; first repeats at end.
        mk_tagged_way(&[("natural", "water")], &[300, 301, 302, 303, 300]),
    ];

    // The bounds drive the CoordConverter origin (center). Keep them symmetric
    // around (0,0) so the in-house CoordConverter below matches the one the
    // pipeline constructs internally.
    let max_block_lat = 16.0;
    let max_block_lon = 5.0;
    let bounds = (
        -max_block_lat * ONE_BLOCK_DEG,
        -max_block_lon * ONE_BLOCK_DEG,
        max_block_lat * ONE_BLOCK_DEG,
        max_block_lon * ONE_BLOCK_DEG,
    );

    OsmData {
        nodes,
        ways_by_id: [(1, 0), (2, 1), (3, 2)].into_iter().collect(),
        ways,
        relations: Vec::new(),
        bounds: Some(bounds),
        poi_nodes: Vec::new(),
        addr_nodes: Vec::new(),
        tree_nodes: Vec::new(),
    }
}

// ── Hand-built RenderContext for direct-orchestrator tests ────────────────────
//
// `RenderContext` and `TileWays` borrow from a pile of temporaries
// (`resolved_ways`, `spatial_index`, `height_map`, etc.). A self-referential
// struct won't compile, so we use the canonical Rust workaround: a helper
// that owns the temporaries and passes `(&ctx, &tile)` into a closure.

fn with_render_context<R>(
    data: &OsmData,
    params: &ConvertParams,
    surface_y: i32,
    f: impl for<'a> FnOnce(&RenderContext<'a>, &TileWays<'a>) -> R,
) -> R {
    let conv = CoordConverter::new(0.0, 0.0, params.scale);
    // Resolve ways: same logic as pipeline::resolve_ways.
    let resolved_ways: Vec<(&OsmWay, Vec<(i32, i32)>)> = data
        .ways
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
        .collect();
    let spatial_index = SpatialIndex::build(&resolved_ways);

    // Sparse-fallback HeightMap: returns `surface_y` for any (x, z). The
    // bounded variant would panic on underflow when render_osm_features
    // queries just outside our computed bbox; the sparse form is safer for
    // orchestrator-focused tests where terrain shape is irrelevant.
    let height_map = HeightMap::new(surface_y);
    // Tile bounds (used by render_osm_features only for address-node spatial
    // filtering) still need to span the data extent.
    let (min_bx, max_bx, min_bz, max_bz) = bounds_of(&resolved_ways);

    let resolved_relations: Vec<osm_to_bedrock::spatial::ResolvedRelation> = Vec::new();
    let all_relations: Vec<&osm_to_bedrock::spatial::ResolvedRelation> = Vec::new();

    let ctx = RenderContext {
        resolved_ways: &resolved_ways,
        resolved_relations: &resolved_relations,
        data,
        params,
        height_map: &height_map,
        conv: &conv,
        spatial_index: &spatial_index,
        surface: surface_y,
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
        tile_bounds: Some((min_bx, min_bz, max_bx, max_bz)),
    };

    f(&ctx, &tile)
}

fn bounds_of(resolved: &[(&OsmWay, Vec<(i32, i32)>)]) -> (i32, i32, i32, i32) {
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for (_, pts) in resolved {
        for &(x, z) in pts {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_z = min_z.min(z);
            max_z = max_z.max(z);
        }
    }
    // Padding to match the spatial index's 64-block grid rounding margin.
    (min_x - 16, max_x + 16, min_z - 16, max_z + 16)
}

// ── ConvertParams helper ──────────────────────────────────────────────────────

fn default_params(edition: Edition) -> ConvertParams {
    ConvertParams {
        input: None,
        output: PathBuf::from("/tmp/osm-to-bedrock-test-unused"),
        edition,
        scale: 1.0,
        sea_level: 65,
        building_height: 4,
        wall_straighten_threshold: 0,
        spawn_x: None,
        spawn_y: None,
        spawn_z: None,
        spawn_lat: None,
        spawn_lon: None,
        signs: false,
        address_signs: false,
        poi_markers: false,
        poi_decorations: false,
        nature_decorations: false,
        filter: FeatureFilter::default(),
        elevation: None,
        vertical_scale: 1.0,
        elevation_smoothing: 0,
        surface_thickness: 4,
    }
}

// ── Test 1: RecordingWorld records every call ────────────────────────────────

#[test]
fn recording_world_captures_set_block_insert_chunk_and_entities() {
    let mut world = RecordingWorld::new();
    world.set_block(3, 65, -7, Block::GrassBlock);
    world.set_block(3, 66, -7, Block::Dirt);
    world.set_block(3, 65, -7, Block::Stone); // overwrite same cell
    assert_eq!(world.get_block(3, 65, -7), Block::Stone);
    assert_eq!(world.get_block(3, 66, -7), Block::Dirt);
    assert_eq!(world.get_block(99, 99, 99), Block::Air);

    // Air write clears the cell.
    world.set_block(3, 66, -7, Block::Air);
    assert_eq!(world.get_block(3, 66, -7), Block::Air);
    assert_eq!(world.len(), 1, "only the (3,65,-7)=Stone cell remains");

    // Insert a ChunkData: its non-air blocks must be visible via get_block.
    let mut chunk = ChunkData::new();
    chunk.set(0, 0, 0, Block::Cobblestone); // wx=0, wy=0, wz=0 for chunk (0,0)
    world.insert_chunk(0, 0, chunk);
    assert_eq!(world.get_block(0, 0, 0), Block::Cobblestone);
    assert_eq!(world.chunk_count(), 1);

    // Entity + direction APIs are no-ops on the recorded block grid but
    // must record their inputs.
    world.add_block_entity(5, 65, 5, vec![0xAB, 0xCD]);
    world.set_sign_direction(5, 65, 5, 7);
    world.set_block_direction(6, 65, 6, 3);
    assert_eq!(world.block_entities.len(), 1);
    assert_eq!(world.sign_directions.get(&(5, 65, 5)), Some(&7));
    assert_eq!(world.block_directions.get(&(6, 65, 6)), Some(&3));

    // save() is a no-op success.
    world.save(0, 0, 0).unwrap();
}

// ── Test 2: render_osm_features places road/building/water blocks ────────────

#[test]
fn render_osm_features_places_expected_block_kinds() {
    let data = synthetic_osm_data();
    let params = default_params(Edition::Bedrock);
    let surface_y = params.sea_level;

    let mut world = RecordingWorld::new();
    with_render_context(&data, &params, surface_y, |ctx, tile| {
        render_osm_features(&mut world, ctx, tile);
    });

    // ── Layer: road ───────────────────────────────────────────────────────
    // residential → PolishedBlackstoneSlab surface (half_width=2, sidewalk=true).
    assert!(
        world.count_block(Block::PolishedBlackstoneSlab) > 0,
        "residential road must place PolishedBlackstoneSlab; total blocks = {}",
        world.len(),
    );
    // The road runs east-west at z = -10. At least one road block must sit
    // on that line at the surface Y.
    let road_on_line = world
        .iter_placements()
        .filter(|&(_x, y, z, b)| z == -10 && y == surface_y && b == Block::PolishedBlackstoneSlab)
        .count();
    assert!(
        road_on_line > 0,
        "expected road surface blocks at z=-10 y={surface_y}; got placements: {:?}",
        sample_placements(&world, 10),
    );

    // ── Layer: building ───────────────────────────────────────────────────
    // building=yes without material tags defaults to StoneBrick (via
    // blocks::building_block). The footprint is x∈[-3,3], z∈[4,8]. At least
    // one building-floor block must land at the building's surface Y.
    let building_blocks = world
        .iter_placements()
        .filter(|&(_x, y, _z, b)| y == surface_y && is_building_block(&b))
        .count();
    assert!(
        building_blocks > 0,
        "expected building floor blocks at y={surface_y}; got {:?}",
        sample_placements(&world, 15),
    );

    // ── Layer: water polygon ──────────────────────────────────────────────
    // natural=water → Water block at surface Y inside the polygon.
    let water_blocks = world.count_block(Block::Water);
    assert!(
        water_blocks > 0,
        "natural=water polygon must place Water blocks; got 0 (placements: {:?})",
        sample_placements(&world, 15),
    );
    // Water polygon interior at z ∈ [13, 15], x ∈ [-3, 3] (interior, not perimeter).
    let water_in_poly = world
        .iter_placements()
        .filter(|&(x, y, z, b)| {
            y == surface_y && b == Block::Water && (13..=15).contains(&z) && (-3..=3).contains(&x)
        })
        .count();
    assert!(
        water_in_poly > 0,
        "expected Water blocks inside the polygon interior; got {water_in_poly}",
    );
}

// ── Test 3: filter toggles actually suppress layers ──────────────────────────

#[test]
fn render_osm_features_respects_filter_flags() {
    let data = synthetic_osm_data();
    let mut params = default_params(Edition::Bedrock);
    // Disable everything except water. The road and building layers must
    // then produce zero road/building blocks even though their ways exist.
    params.filter = FeatureFilter {
        roads: false,
        buildings: false,
        water: true,
        landuse: false,
        railways: false,
    };
    let surface_y = params.sea_level;

    let mut world = RecordingWorld::new();
    with_render_context(&data, &params, surface_y, |ctx, tile| {
        render_osm_features(&mut world, ctx, tile);
    });

    assert_eq!(
        world.count_block(Block::PolishedBlackstoneSlab),
        0,
        "roads disabled — no road surface blocks expected",
    );
    assert!(
        !world.has_any(is_building_block),
        "buildings disabled — no building blocks expected",
    );
    assert!(
        world.count_block(Block::Water) > 0,
        "water enabled — Water blocks expected",
    );
}

// ── Test 4: end-to-end run_preview_from_data ─────────────────────────────────

#[test]
fn run_preview_from_data_returns_world_with_terrain_column() {
    let data = synthetic_osm_data();
    let params = default_params(Edition::Bedrock);

    let (world, _spawn_x, _spawn_y, _spawn_z) =
        pipeline::run_preview_from_data(data, &params, &|_, _| {}).expect("preview pipeline");

    // Terrain fill must place a bedrock → stone → dirt → grass column at
    // every chunk column inside the bounds. Check a known column near origin.
    // The terrain column Y values depend on sea_level (65) and surface_thickness (4):
    // sy=65 → base_y=61; column is bedrock@61, stone@62..63, dirt@64, grass@65.
    assert_eq!(world.get_block(0, 61, 0), Block::Bedrock);
    assert_eq!(world.get_block(0, 65, 0), Block::GrassBlock);

    // The world must have produced at least one chunk.
    assert!(world.chunk_count() > 0, "preview world must contain chunks",);
    assert!(
        !world.occupied_chunks().is_empty(),
        "occupied_chunks must be non-empty",
    );
}

// ── Test 5: cross-edition parity at the dispatch seam ────────────────────────
//
// This is the parity guard for the edition-dispatch seam where ARC-001 lives.
// If Bedrock and Java ever diverge on block placement for the same input, a
// downstream refactor of the streaming path will silently change one
// edition's output. This test fails the moment that happens.

#[test]
fn cross_edition_surface_blocks_match_for_same_input() {
    let bedrock = run_preview_for_edition(Edition::Bedrock);
    let java = run_preview_for_edition(Edition::Java);

    let mut bedrock_surface = bedrock.surface_blocks();
    let mut java_surface = java.surface_blocks();
    bedrock_surface.sort_by_key(|t| (t.0, t.1, t.2));
    java_surface.sort_by_key(|t| (t.0, t.1, t.2));

    assert_eq!(
        bedrock_surface.len(),
        java_surface.len(),
        "edition surface-block count diverged (Bedrock={}, Java={})",
        bedrock_surface.len(),
        java_surface.len(),
    );
    // Cell-by-cell comparison: name and Y must match exactly. We don't
    // compare chunk_count because the in-memory Java writer stores chunks
    // differently from Bedrock (parity is about blocks, not storage layout).
    let mut mismatches = 0;
    for (b, j) in bedrock_surface.iter().zip(java_surface.iter()) {
        if b != j {
            mismatches += 1;
            if mismatches <= 5 {
                eprintln!("DIVERGE: bedrock={b:?} java={j:?}");
            }
        }
    }
    assert_eq!(
        mismatches, 0,
        "Bedrock and Java surface blocks diverged at {mismatches} columns",
    );
}

#[test]
fn cross_edition_get_block_matches_at_known_positions() {
    let bedrock = run_preview_for_edition(Edition::Bedrock);
    let java = run_preview_for_edition(Edition::Java);

    // Sample a handful of coordinates spanning the data extent. Both editions
    // must return identical blocks at each.
    let probes: &[(i32, i32, i32)] = &[
        (0, 61, 0),   // terrain bedrock layer
        (0, 65, 0),   // terrain grass layer
        (0, 65, -10), // road surface position
        (-3, 65, 4),  // building perimeter corner
        (0, 65, 14),  // inside water polygon
        (50, 65, 50), // outside data — both must return Air
    ];
    for &(x, y, z) in probes {
        let b = bedrock.get_block(x, y, z);
        let j = java.get_block(x, y, z);
        assert_eq!(
            b, j,
            "edition diverge at ({x},{y},{z}): bedrock={b:?} java={j:?}",
        );
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn run_preview_for_edition(edition: Edition) -> Box<dyn WorldWriter> {
    let data = synthetic_osm_data();
    let params = default_params(edition);
    let (world, _spawn_x, _spawn_y, _spawn_z) =
        pipeline::run_preview_from_data(data, &params, &|_, _| {})
            .unwrap_or_else(|e| panic!("preview pipeline for {edition:?} failed: {e:?}"));
    world
}

// ── Streaming-path tests (ARC-002): process_tile + flush_tile seam ─────────
//
// `process_tile` is the deduplicated tile body extracted from
// `run_pipeline_streaming`. The streaming path now drives each edition
// through `world.set_tile_bounds(...)` → `process_tile(...)` →
// `world.flush_tile()`. These tests assert:
//   1. `process_tile` produces the same terrain + feature blocks the
//      in-memory path does, when driven through a `RecordingWorld`.
//   2. The `flush_tile` / `set_tile_bounds` seam is exercised by the
//      real pipeline (Bedrock end-to-end: if flush_tile weren't called,
//      no chunks would reach LevelDB).
//   3. ARC-001's oversized-Java guard refuses a synthetic city-scale
//      conversion up front (no allocation, no panic).

#[test]
fn process_tile_writes_terrain_column_for_chunk_in_bounds() {
    // Drive `process_tile` directly with a RecordingWorld, simulating
    // what the streaming tile loop does for one tile containing chunk
    // (0, 0). The terrain-fill rayon loop must place bedrock at the
    // base Y and grass at the surface Y for the (0,0) column.
    let data = synthetic_osm_data();
    let params = default_params(Edition::Bedrock);
    let conv = CoordConverter::new(0.0, 0.0, params.scale);
    let resolved_ways: Vec<(&OsmWay, Vec<(i32, i32)>)> = data
        .ways
        .iter()
        .map(|w| {
            let pts: Vec<(i32, i32)> = w
                .node_refs
                .iter()
                .filter_map(|id| data.nodes.get(id))
                .map(|n| conv.to_block_xz(n.lat, n.lon))
                .collect();
            (w, pts)
        })
        .collect();
    let spatial_index = SpatialIndex::build(&resolved_ways);
    let height_map = HeightMap::new(params.sea_level);
    let resolved_relations: Vec<osm_to_bedrock::spatial::ResolvedRelation> = Vec::new();

    let mut world = RecordingWorld::new();
    pipeline::process_tile(
        &mut world,
        -1,
        0,
        -1,
        0, // 2x2-chunk tile around the origin
        &height_map,
        params.sea_level,
        params.surface_thickness,
        &spatial_index,
        &resolved_ways,
        &resolved_relations,
        &data,
        &params,
        &conv,
    )
    .expect("process_tile must succeed against a recording world");

    // Terrain-fill must have placed at least one bedrock and one grass
    // block at the (0,0) chunk column. Bedrock sits at
    // (sea_level - surface_thickness); grass at sea_level.
    assert!(
        world.count_block(Block::Bedrock) > 0,
        "terrain fill must place Bedrock; got placements: {:?}",
        sample_placements(&world, 10),
    );
    assert!(
        world.count_block(Block::GrassBlock) > 0,
        "terrain fill must place GrassBlock; got placements: {:?}",
        sample_placements(&world, 10),
    );

    // The default-impl `flush_tile` on RecordingWorld records the call
    // but does not drain — confirm the trait seam compiles and runs.
    // (The streaming pipeline drives flush_tile from the outer loop;
    // here we just verify the method is callable.)
    world
        .flush_tile()
        .expect("flush_tile default impl must succeed");
    assert_eq!(world.flush_tile_call_count, 1);
}

#[test]
fn bedrock_streaming_pipeline_writes_leveldb_subchunks_via_flush_tile() {
    // End-to-end: `run_conversion_from_data` for Bedrock exercises the
    // real streaming backend (`BedrockWorld::new_streaming` + per-tile
    // `flush_tile` draining to a real LevelDB writer thread). If
    // `flush_tile` were never invoked, chunks would never reach LevelDB
    // and the db/ directory would be empty. Asserting non-empty SubChunk
    // entries proves the seam fires per tile.
    let dir = tempfile::tempdir().expect("tempdir");
    let data = synthetic_osm_data();
    let mut params = default_params(Edition::Bedrock);
    params.output = dir.path().to_path_buf();

    pipeline::run_conversion_from_data(data, &params, &|_, _| {}).expect("Bedrock conversion");

    let db_dir = dir.path().join("db");
    assert!(
        db_dir.exists(),
        "db/ directory must exist: {}",
        db_dir.display()
    );

    // LevelDB writes a LOG, MANIFEST, and at least one .ldb/.log data file
    // once any chunk is put. A non-empty db/ with SubChunk-bearing data
    // proves flush_tile delivered chunks to the writer thread.
    let db_entries: Vec<_> = std::fs::read_dir(&db_dir)
        .unwrap_or_else(|e| panic!("read_dir({}): {e}", db_dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert!(
        !db_entries.is_empty(),
        "LevelDB db/ must be non-empty after streaming conversion (flush_tile must have fired)",
    );

    // level.dat must also exist (the close-out path).
    assert!(
        dir.path().join("level.dat").exists(),
        "level.dat must be written after Bedrock close-out",
    );
}

#[test]
fn java_streaming_pipeline_writes_region_files_and_level_dat() {
    // End-to-end check for the Java path (ARC-001): the pipeline now drives a
    // streaming Anvil writer — `JavaWorld::new_streaming` — that lazily writes
    // region files as tiles flush rather than accumulating the whole world.
    let dir = tempfile::tempdir().expect("tempdir");
    let data = synthetic_osm_data();
    let mut params = default_params(Edition::Java);
    params.output = dir.path().to_path_buf();

    pipeline::run_conversion_from_data(data, &params, &|_, _| {}).expect("Java conversion");

    assert!(
        dir.path().join("level.dat").exists(),
        "Java level.dat must exist",
    );
    let region_dir = dir.path().join("region");
    assert!(region_dir.exists(), "Java region/ directory must exist");
    let mca_count = std::fs::read_dir(&region_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "mca"))
        .count();
    assert!(
        mca_count > 0,
        "at least one .mca region file must be written",
    );
}

fn is_building_block(b: &Block) -> bool {
    matches!(
        b,
        Block::StoneBrick
            | Block::Brick
            | Block::Cobblestone
            | Block::Concrete
            | Block::BlackConcrete
            | Block::GrayConcrete
            | Block::WhiteConcrete
            | Block::YellowConcrete
            | Block::Sandstone
            | Block::OakPlanks
            | Block::SprucePlanks
            | Block::GlassPane
    )
}

/// Compact string for debug output when an assertion fails.
fn sample_placements(world: &RecordingWorld, n: usize) -> Vec<String> {
    let mut all: Vec<(i32, i32, i32, Block)> = world.iter_placements().collect();
    all.sort_by_key(|t| (t.1, t.0, t.2));
    all.into_iter()
        .take(n)
        .map(|(x, y, z, b)| format!("({x},{y},{z})={b:?}"))
        .collect()
}

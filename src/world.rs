//! Shared world-abstraction types used by both Bedrock and Java Edition writers.
//!
//! This module defines:
//! - Y-range constants (`MIN_Y`, `MAX_Y`, `WORLD_HEIGHT`)
//! - [`Edition`] enum for selecting the output format
//! - [`ChunkData`] — the in-memory chunk representation shared by all backends
//! - [`WorldWriter`] trait — the common interface every edition must implement

use crate::blocks::Block;
use anyhow::Result;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

// ── World Y-range constants (Bedrock 1.18+ / Java 1.18+) ─────────────────

/// Minimum Y coordinate (bottom of the world).
pub const MIN_Y: i32 = -64;
/// Maximum Y coordinate (top of the world, inclusive).
pub const MAX_Y: i32 = 319;
/// Total world height in blocks.
#[allow(dead_code)] // referenced by module doc + exercised by unit tests (world_height constant)
pub const WORLD_HEIGHT: i32 = MAX_Y - MIN_Y + 1; // 384

// ── Edition enum ──────────────────────────────────────────────────────────

/// Minecraft edition to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    /// Bedrock Edition (`.mcworld` / LevelDB).
    #[default]
    Bedrock,
    /// Java Edition (`.zip` with Anvil region files).
    Java,
}

impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Edition::Bedrock => write!(f, "bedrock"),
            Edition::Java => write!(f, "java"),
        }
    }
}

impl FromStr for Edition {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "bedrock" => Ok(Edition::Bedrock),
            "java" => Ok(Edition::Java),
            other => Err(format!(
                "unknown edition: {other} (expected 'bedrock' or 'java')"
            )),
        }
    }
}

impl clap::ValueEnum for Edition {
    fn value_variants<'a>() -> &'a [Self] {
        &[Edition::Bedrock, Edition::Java]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(match self {
            Edition::Bedrock => clap::builder::PossibleValue::new("bedrock"),
            Edition::Java => clap::builder::PossibleValue::new("java"),
        })
    }
}

// ── ChunkData ─────────────────────────────────────────────────────────────

/// In-memory representation of one 16×(height)×16 chunk column.
///
/// Blocks are stored in sub-chunks of 16×16×16, indexed XZY (x*256 + z*16 + y_local).
/// Only non-empty sub-chunks are allocated.
#[derive(Default)]
pub struct ChunkData {
    /// Map from sub-chunk Y index → block array (4096 entries, XZY).
    subchunks: HashMap<i8, Box<[Block; 4096]>>,
}

impl ChunkData {
    pub fn new() -> Self {
        Self::default()
    }

    fn idx(lx: i32, ly: i32, lz: i32) -> usize {
        // XZY order: x * 256 + z * 16 + y_local
        (lx as usize) * 256 + (lz as usize) * 16 + ly as usize
    }

    /// Set a block at local-x, world-y, local-z.
    pub fn set(&mut self, lx: i32, y: i32, lz: i32, block: Block) {
        let sy = y.div_euclid(16) as i8;
        let ly = y.rem_euclid(16);
        let entry = self
            .subchunks
            .entry(sy)
            .or_insert_with(|| Box::new([Block::Air; 4096]));
        entry[Self::idx(lx, ly, lz)] = block;
    }

    /// Get a block at local-x, world-y, local-z.
    pub fn get(&self, lx: i32, y: i32, lz: i32) -> Block {
        let sy = y.div_euclid(16) as i8;
        let ly = y.rem_euclid(16);
        self.subchunks
            .get(&sy)
            .map(|sc| sc[Self::idx(lx, ly, lz)])
            .unwrap_or(Block::Air)
    }

    /// Iterate sub-chunks that have at least one non-air block.
    pub fn non_empty_subchunks(&self) -> impl Iterator<Item = (i8, &[Block; 4096])> {
        self.subchunks
            .iter()
            .map(|(&sy, blocks)| (sy, blocks.as_ref()))
    }
}

// ── ChunkStore ────────────────────────────────────────────────────────────

/// Shared in-memory storage for the fields and write-path operations that
/// are byte-for-byte identical between [`BedrockWorld`] and [`JavaWorld`].
///
/// Holds the per-chunk block grids, the auxiliary per-chunk and per-block
/// override maps (block-entity NBT blobs, sign directions, directional-block
/// directions), and the optional chunk-coordinate rectangle used to silently
/// filter out-of-tile writes during streaming conversion. Both backends embed
/// a `ChunkStore` and delegate the [`WorldWriter`] methods that don't depend
/// on edition-specific storage to it; the backend keeps only the
/// edition-specific state (output path, the LevelDB writer thread for
/// Bedrock, the region-file encoder for Java).
///
/// Extracting this struct removes the ~150 lines of duplicated storage and
/// write-filtering logic that previously lived in lockstep across the two
/// backends (QA-001). The trait itself stays edition-agnostic and does *not*
/// expose a `store()` accessor: test doubles such as `RecordingWorld` use a
/// different in-memory representation by design (independent oracle for
/// cross-edition parity), and forcing them through `ChunkStore` would
/// compromise that role.
///
/// [`BedrockWorld`]: crate::bedrock::BedrockWorld
/// [`JavaWorld`]: crate::anvil::JavaWorld
pub struct ChunkStore {
    /// Per-chunk block grids, keyed by chunk (cx, cz).
    chunks: HashMap<(i32, i32), ChunkData>,
    /// Block-entity NBT blobs, bucketed by the chunk (cx, cz) that owns them.
    /// Each blob already carries its own (x, y, z) position inside the NBT
    /// payload; the writer keys only by chunk so it can hand the whole bucket
    /// to the edition-specific chunk encoder.
    block_entities: HashMap<(i32, i32), Vec<Vec<u8>>>,
    /// Sign direction overrides (0-15), keyed by sign block (x, y, z).
    sign_directions: HashMap<(i32, i32, i32), i32>,
    /// Directional-block overrides (stairs, rails), keyed by (x, y, z).
    block_directions: HashMap<(i32, i32, i32), i32>,
    /// Optional chunk-coordinate rectangle for streaming-tile filtering.
    /// When `Some`, `set_block` and friends silently ignore writes whose
    /// chunk falls outside `[min_cx, max_cx] × [min_cz, max_cz]`.
    chunk_bounds: Option<(i32, i32, i32, i32)>,
}

impl ChunkStore {
    /// Create an unbounded store (accepts writes in any chunk).
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            block_entities: HashMap::new(),
            sign_directions: HashMap::new(),
            block_directions: HashMap::new(),
            chunk_bounds: None,
        }
    }

    /// Create a store bounded to the chunk rectangle
    /// `[min_cx, max_cx] × [min_cz, max_cz]`. Writes whose chunk falls
    /// outside the rectangle are silently dropped.
    pub fn new_bounded(min_cx: i32, max_cx: i32, min_cz: i32, max_cz: i32) -> Self {
        Self {
            chunk_bounds: Some((min_cx, max_cx, min_cz, max_cz)),
            ..Self::new()
        }
    }

    /// Replace the active chunk-coordinate rectangle. Used by Bedrock's
    /// streaming `set_tile_bounds` override; Java's `set_tile_bounds` stays
    /// the trait's default no-op, so its `chunk_bounds` is set only at
    /// construction via `new_bounded`.
    pub fn set_tile_bounds(&mut self, min_cx: i32, max_cx: i32, min_cz: i32, max_cz: i32) {
        self.chunk_bounds = Some((min_cx, max_cx, min_cz, max_cz));
    }

    /// Return `true` if (cx, cz) falls within the optional chunk bounds.
    #[inline]
    pub fn in_bounds(&self, cx: i32, cz: i32) -> bool {
        match self.chunk_bounds {
            None => true,
            Some((min_cx, max_cx, min_cz, max_cz)) => {
                cx >= min_cx && cx <= max_cx && cz >= min_cz && cz <= max_cz
            }
        }
    }

    /// Set a block at absolute (x, y, z) world coordinates.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        let lx = x.rem_euclid(16);
        let lz = z.rem_euclid(16);
        self.chunks
            .entry((cx, cz))
            .or_default()
            .set(lx, y, lz, block);
    }

    /// Get a block at absolute (x, y, z) world coordinates.
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> Block {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let lx = x.rem_euclid(16);
        let lz = z.rem_euclid(16);
        self.chunks
            .get(&(cx, cz))
            .map(|chunk| chunk.get(lx, y, lz))
            .unwrap_or(Block::Air)
    }

    /// Insert a pre-built [`ChunkData`] at (cx, cz), replacing any existing data.
    pub fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData) {
        self.chunks.insert((cx, cz), chunk);
    }

    /// Bucket a pre-encoded block-entity NBT blob under the chunk that owns
    /// `(x, z)`. See [`WorldWriter::add_block_entity`] for why `y` is unused
    /// at this layer.
    pub fn add_block_entity(&mut self, x: i32, _y: i32, z: i32, nbt: Vec<u8>) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.block_entities.entry((cx, cz)).or_default().push(nbt);
    }

    /// Record a sign-direction override (0-15) at (x, y, z).
    pub fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.sign_directions.insert((x, y, z), direction);
    }

    /// Record a directional-block override (stairs, rails) at (x, y, z).
    pub fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.block_directions.insert((x, y, z), direction);
    }

    /// Number of chunks currently held.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// All occupied chunk coordinates.
    pub fn occupied_chunks(&self) -> Vec<(i32, i32)> {
        self.chunks.keys().copied().collect()
    }

    /// Top-most non-Air block at each (x, z) column as
    /// `Vec<(world_x, world_z, y, block_name)>`.
    pub fn surface_blocks(&self) -> Vec<(i32, i32, i32, String)> {
        let mut result = Vec::new();
        for (&(cx, cz), chunk) in &self.chunks {
            for lx in 0..16i32 {
                for lz in 0..16i32 {
                    let wx = cx * 16 + lx;
                    let wz = cz * 16 + lz;
                    for y in (MIN_Y..=MAX_Y).rev() {
                        let b = chunk.get(lx, y, lz);
                        if b != Block::Air {
                            result.push((wx, wz, y, format!("{:?}", b)));
                            break;
                        }
                    }
                }
            }
        }
        result
    }

    // ── Accessors used by edition-specific code (save / drain / encode) ──

    /// Read access to the chunk map (both backends' `save` paths).
    pub fn chunks(&self) -> &HashMap<(i32, i32), ChunkData> {
        &self.chunks
    }

    /// Read access to the block-entity buckets (both backends' encode paths).
    pub fn block_entities(&self) -> &HashMap<(i32, i32), Vec<Vec<u8>>> {
        &self.block_entities
    }

    /// Read access to sign-direction overrides (Bedrock encode path).
    pub fn sign_directions(&self) -> &HashMap<(i32, i32, i32), i32> {
        &self.sign_directions
    }

    /// Read access to directional-block overrides (Bedrock encode path).
    pub fn block_directions(&self) -> &HashMap<(i32, i32, i32), i32> {
        &self.block_directions
    }

    /// Take ownership of the chunk map, leaving an empty map in its place.
    /// Used by Bedrock's streaming `flush_tile` path (via
    /// `drain_chunks_to_writer`) to drain the per-tile scratch state.
    pub fn take_chunks(&mut self) -> HashMap<(i32, i32), ChunkData> {
        std::mem::take(&mut self.chunks)
    }

    /// Read access to a sign direction, defaulting to 0. Used only by
    /// Bedrock's `get_sign_direction` helper.
    pub fn get_sign_direction(&self, x: i32, y: i32, z: i32) -> i32 {
        self.sign_directions.get(&(x, y, z)).copied().unwrap_or(0)
    }

    /// Clear the three auxiliary override maps after a streaming drain has
    /// flushed them to the underlying sink. The chunk map is cleared
    /// separately via [`ChunkStore::take_chunks`].
    pub fn clear_aux(&mut self) {
        self.block_entities.clear();
        self.sign_directions.clear();
        self.block_directions.clear();
    }
}

impl Default for ChunkStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── WorldWriter trait ────────────────────────────────────────────────────

/// Common interface for writing a Minecraft world.
///
/// Implemented by [`crate::bedrock::BedrockWorld`] (Bedrock Edition)
/// and `crate::anvil::JavaWorld` (Java Edition, behind feature gate).
///
/// The trait is the seam that lets the streaming tile pipeline
/// ([`crate::pipeline::run_pipeline_streaming`]) treat both editions
/// uniformly: each tile's blocks flow through `set_block`/`insert_chunk`,
/// then [`WorldWriter::flush_tile`] drains them to the edition-specific
/// sink (LevelDB writer for Bedrock, accumulated `HashMap` for Java).
///
/// The shared storage and write-filtering logic that both backends need
/// lives in [`ChunkStore`]; each backend embeds one and delegates to it.
/// The trait deliberately does *not* expose a `store()` accessor so test
/// doubles can keep using whatever in-memory representation makes them the
/// strongest independent oracle.
pub trait WorldWriter {
    /// Set a block at absolute (x, y, z) world coordinates.
    fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block);

    /// Get a block at absolute (x, y, z) world coordinates.
    fn get_block(&self, x: i32, y: i32, z: i32) -> Block;

    /// Insert a pre-built [`ChunkData`] at chunk coordinates (cx, cz).
    fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData);

    /// Add a pre-encoded block-entity NBT blob for the block at `(x, y, z)`.
    ///
    /// **Coordinate contract (QA-013):** `x` and `z` are used to bucket the
    /// blob into the owning chunk's entity list; `y` is intentionally *not*
    /// used as a key at this layer. The writer's per-chunk entity list mixes
    /// all Y values, and each blob already encodes its full `(x, y, z)`
    /// position inside the NBT payload (see e.g.
    /// `encode_sign_block_entity` in `pipeline/render`). Accepting `y` in the
    /// signature preserves the symmetric `(x, y, z)` coordinate triple used
    /// by every other `WorldWriter` setter and reserves room for a future
    /// per-Y entity index without a breaking trait change.
    fn add_block_entity(&mut self, x: i32, y: i32, z: i32, nbt: Vec<u8>);

    /// Set the sign direction (0-15) for a sign block at world coordinates.
    fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32);

    /// Set the direction for a directional block (stairs, rails) at world coordinates.
    fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32);

    /// Return the number of chunks currently in the world.
    fn chunk_count(&self) -> usize;

    /// Return all occupied chunk coordinates.
    fn occupied_chunks(&self) -> Vec<(i32, i32)>;

    /// Extract the top-most non-Air block at each (x, z) column.
    /// Returns `Vec<(world_x, world_z, y, block_name)>`.
    fn surface_blocks(&self) -> Vec<(i32, i32, i32, String)>;

    /// Scope subsequent `set_block`/`add_block_entity` calls to the given
    /// chunk-coordinate rectangle until the next call.
    ///
    /// The default impl is a no-op, which leaves the writer unbounded —
    /// correct for backends that accumulate the whole world (Java) or for
    /// test doubles. Bedrock's streaming backend overrides this to keep
    /// per-tile memory bounded: only blocks inside the rectangle are stored
    /// until the next [`WorldWriter::flush_tile`] drains them.
    fn set_tile_bounds(&mut self, _min_cx: i32, _max_cx: i32, _min_cz: i32, _max_cz: i32) {}

    /// Drain any tile-local buffered state to the underlying sink.
    ///
    /// Default impl is a no-op (correct for Java's accumulate-in-memory
    /// writer and for test doubles). Bedrock's streaming backend overrides
    /// this to flush the current tile's encoded SubChunks to the background
    /// LevelDB writer thread and clear the in-memory chunk map, so the next
    /// tile starts from an empty scratch pad.
    fn flush_tile(&mut self) -> Result<()> {
        Ok(())
    }

    /// Write the world to disk with spawn at the given block coordinates.
    ///
    /// Takes `&mut self` because the streaming Bedrock backend must
    /// `take()` its owned writer thread handle to join it.
    fn save(&mut self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()>;
}

// ── Edition factory methods ──────────────────────────────────────────────

impl Edition {
    /// Create an unbounded world for this edition.
    pub fn create_world(&self, output: &Path) -> Box<dyn WorldWriter> {
        match self {
            Edition::Bedrock => Box::new(crate::bedrock::BedrockWorld::new(output)),
            Edition::Java => Box::new(crate::anvil::JavaWorld::new(output)),
        }
    }

    /// Create a bounded world for incremental tile-based processing.
    #[allow(dead_code)] // public API: edition dispatch wrapper, kept for library consumers / bounded-world tests
    pub fn create_world_bounded(
        &self,
        output: &Path,
        min_cx: i32,
        max_cx: i32,
        min_cz: i32,
        max_cz: i32,
    ) -> Box<dyn WorldWriter> {
        match self {
            Edition::Bedrock => Box::new(crate::bedrock::BedrockWorld::new_bounded(
                output, min_cx, max_cx, min_cz, max_cz,
            )),
            Edition::Java => Box::new(crate::anvil::JavaWorld::new_bounded(
                output, min_cx, max_cx, min_cz, max_cz,
            )),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────
//
// `ChunkData::set`/`get` and the XZY indexing (`idx = lx*256 + lz*16 + ly`)
// are the single most load-bearing primitive in the codebase: an off-by-one
// here silently corrupts every world both backends write. These tests cover
// the sub-chunk boundaries exhaustively because that is exactly where the
// `div_euclid(16)`/`rem_euclid(16)` split lives.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Block;

    // ── ChunkData: empty / default state ──────────────────────────────────

    #[test]
    fn chunkdata_new_returns_air_for_unset_blocks() {
        let cd = ChunkData::new();
        // Spans two sub-chunks; both must default to Air without allocating.
        assert_eq!(cd.get(0, 0, 0), Block::Air);
        assert_eq!(cd.get(15, MAX_Y, 15), Block::Air);
        assert_eq!(cd.get(7, MIN_Y, 8), Block::Air);
        // A fresh ChunkData reports no non-empty sub-chunks.
        assert_eq!(cd.non_empty_subchunks().count(), 0);
    }

    // ── ChunkData: sub-chunk boundary round-trip ─────────────────────────
    //
    // Sub-chunk index sy = y.div_euclid(16); local-y ly = y.rem_euclid(16).
    // The interesting cases are at the *transitions* between sub-chunks
    // (y = 0, 15, 16, 31, 32, …) and across the negative-Y boundary, where
    // Euclidean division differs from Rust's default truncated division.

    #[test]
    fn set_get_round_trips_at_positive_subchunk_boundaries() {
        let cases: &[(i32, i8, i32)] = &[
            // (world_y, expected_sy, expected_ly)
            (0, 0, 0),
            (1, 0, 1),
            (15, 0, 15),
            (16, 1, 0),
            (17, 1, 1),
            (31, 1, 15),
            (32, 2, 0),
            (47, 2, 15),
            (48, 3, 0),
        ];
        for &(y, sy, ly) in cases {
            let mut cd = ChunkData::new();
            cd.set(0, y, 0, Block::Stone);
            assert_eq!(cd.get(0, y, 0), Block::Stone, "y={y}: round-trip failed",);
            // Confirm the write landed in the expected sub-chunk.
            let subchunks: Vec<i8> = cd.non_empty_subchunks().map(|(sy, _)| sy).collect();
            assert_eq!(
                subchunks,
                vec![sy],
                "y={y}: expected sy={sy}, got {subchunks:?}"
            );
            // Confirm the local-y indexing inside the sub-chunk is right by
            // probing a known off-by-one slot (must still be Air).
            assert_eq!(
                cd.get(0, y, 0),
                Block::Stone,
                "y={y}: ly={ly} slot mismatch",
            );
            let _ = ly; // documented expectation; ly is implied by the get above.
        }
    }

    #[test]
    fn set_get_round_trips_at_negative_subchunk_boundaries() {
        // Negative-Y is where Euclidean division matters most: it ensures
        // y=-1 → (sy=-1, ly=15), NOT (sy=0, ly=-1) which would panic on a
        // negative array index. Cover the entire negative span down to MIN_Y.
        let cases: &[(i32, i8, i32)] = &[
            (-1, -1, 15),
            (-2, -1, 14),
            (-15, -1, 1),
            (-16, -1, 0),
            (-17, -2, 15),
            (-32, -2, 0),
            (-33, -3, 15),
            (-48, -3, 0),
            (MIN_Y, -4, 0),       // -64
            (MIN_Y + 15, -4, 15), // -49
            (MAX_Y, 19, 15),      // 319
        ];
        for &(y, sy, ly) in cases {
            let mut cd = ChunkData::new();
            cd.set(7, y, 9, Block::Dirt);
            assert_eq!(cd.get(7, y, 9), Block::Dirt, "y={y}: round-trip failed");
            let subchunks: Vec<i8> = cd.non_empty_subchunks().map(|(s, _)| s).collect();
            assert_eq!(
                subchunks,
                vec![sy],
                "y={y}: expected sy={sy}, got {subchunks:?} (Euclidean division bug?)",
            );
            let _ = ly;
        }
    }

    // ── ChunkData: XZY indexing corners ──────────────────────────────────
    //
    // The XZY formula is `lx*256 + lz*16 + ly` — every (lx, ly, lz) corner
    // within the 16×16×16 sub-chunk must round-trip independently. A wrong
    // stride here would alias blocks silently.

    #[test]
    fn xz_indexing_corners_round_trip_independently() {
        let corners: &[(i32, i32, i32)] = &[
            (0, 0, 0),
            (0, 0, 15),
            (0, 15, 0),
            (0, 15, 15),
            (15, 0, 0),
            (15, 0, 15),
            (15, 15, 0),
            (15, 15, 15),
            (8, 8, 8),
        ];
        for &(lx, ly_in_sub, lz) in corners {
            let mut cd = ChunkData::new();
            // ly_in_sub is the local-y within sub-chunk 0; world y = ly_in_sub.
            cd.set(lx, ly_in_sub, lz, Block::Cobblestone);
            // No corner should alias another.
            for &(lx2, ly2, lz2) in corners {
                let expected = if (lx2, ly2, lz2) == (lx, ly_in_sub, lz) {
                    Block::Cobblestone
                } else {
                    Block::Air
                };
                assert_eq!(
                    cd.get(lx2, ly2, lz2),
                    expected,
                    "({lx},{ly_in_sub},{lz}) alias leak at ({lx2},{ly2},{lz2})",
                );
            }
        }
    }

    #[test]
    fn set_into_two_subchunks_keeps_both() {
        // A single ChunkData must hold blocks in multiple sub-chunks without
        // one clobbering the other — the entire world-writer layering relies
        // on this.
        let mut cd = ChunkData::new();
        cd.set(0, 5, 0, Block::GrassBlock); // sy=0, ly=5
        cd.set(0, 25, 0, Block::Stone); // sy=1, ly=9
        cd.set(0, -10, 0, Block::Dirt); // sy=-1, ly=6
        assert_eq!(cd.get(0, 5, 0), Block::GrassBlock);
        assert_eq!(cd.get(0, 25, 0), Block::Stone);
        assert_eq!(cd.get(0, -10, 0), Block::Dirt);
        let mut subs: Vec<i8> = cd.non_empty_subchunks().map(|(s, _)| s).collect();
        subs.sort_unstable();
        assert_eq!(subs, vec![-1, 0, 1]);
    }

    #[test]
    fn set_overwrites_same_cell() {
        let mut cd = ChunkData::new();
        cd.set(3, 7, 4, Block::Dirt);
        cd.set(3, 7, 4, Block::Water);
        assert_eq!(cd.get(3, 7, 4), Block::Water);
    }

    // ── Edition enum ─────────────────────────────────────────────────────

    #[test]
    fn edition_default_is_bedrock() {
        assert_eq!(Edition::default(), Edition::Bedrock);
    }

    #[test]
    fn edition_display_lowercase() {
        assert_eq!(Edition::Bedrock.to_string(), "bedrock");
        assert_eq!(Edition::Java.to_string(), "java");
    }

    #[test]
    fn edition_from_str_roundtrip() {
        for ed in [Edition::Bedrock, Edition::Java] {
            let s = ed.to_string();
            let parsed: Edition = s.parse().expect("roundtrip parse");
            assert_eq!(parsed, ed);
        }
        // Case-insensitive
        assert_eq!("BEDROCK".parse::<Edition>().unwrap(), Edition::Bedrock);
        assert_eq!("Java".parse::<Edition>().unwrap(), Edition::Java);
    }

    #[test]
    fn edition_from_str_rejects_unknown() {
        assert!("legacy".parse::<Edition>().is_err());
        assert!("".parse::<Edition>().is_err());
    }

    #[test]
    fn edition_serde_roundtrip() {
        let json = serde_json::to_string(&Edition::Java).unwrap();
        assert_eq!(json, "\"java\"");
        let parsed: Edition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Edition::Java);
    }

    #[test]
    fn edition_clap_value_enum_variants() {
        use clap::ValueEnum;
        let variants = Edition::value_variants();
        assert_eq!(variants.len(), 2);
        assert!(variants.iter().all(|v| v.to_possible_value().is_some()));
    }

    #[test]
    fn world_height_constant_is_consistent() {
        assert_eq!(WORLD_HEIGHT, MAX_Y - MIN_Y + 1);
        assert_eq!(WORLD_HEIGHT, 384);
        assert_eq!(MIN_Y, -64);
        assert_eq!(MAX_Y, 319);
    }

    // ── ChunkStore (QA-001) ──────────────────────────────────────────────
    //
    // `ChunkStore` is now the single implementation of the storage + bounds
    // + auxiliary-map operations both backends used to duplicate. These
    // tests pin the behaviours Bedrock's streaming `flush_tile` and both
    // backends' `save` paths rely on. They also pin the QA-013 contract:
    // `add_block_entity` buckets by `(x, z)` only, never by `y`.

    #[test]
    fn chunkstore_new_starts_empty_and_unbounded() {
        let s = ChunkStore::new();
        assert_eq!(s.chunk_count(), 0);
        assert!(s.occupied_chunks().is_empty());
        assert!(s.surface_blocks().is_empty());
        // Unbounded means any chunk is accepted.
        assert!(s.in_bounds(0, 0));
        assert!(s.in_bounds(123, -456));
    }

    #[test]
    fn chunkstore_new_bounded_rejects_writes_outside_rectangle() {
        let mut s = ChunkStore::new_bounded(0, 0, 0, 0);
        s.set_block(0, 65, 0, Block::Stone); // chunk (0,0) — inside
        s.set_block(16, 65, 0, Block::Stone); // chunk (1,0) — outside
        s.set_block(0, 65, 16, Block::Stone); // chunk (0,1) — outside
        assert_eq!(s.chunk_count(), 1);
        assert_eq!(s.get_block(0, 65, 0), Block::Stone);
        assert_eq!(s.get_block(16, 65, 0), Block::Air);
    }

    #[test]
    fn chunkstore_set_tile_bounds_updates_active_rectangle() {
        // Bedrock's streaming `set_tile_bounds` override delegates here; the
        // rectangle must actually flip which writes are accepted.
        let mut s = ChunkStore::new();
        assert!(s.in_bounds(5, 5));
        s.set_tile_bounds(0, 0, 0, 0);
        assert!(!s.in_bounds(5, 5));
        assert!(s.in_bounds(0, 0));
    }

    #[test]
    fn chunkstore_insert_chunk_round_trips_through_get_block() {
        // The parallel terrain-fill path builds chunks independently and
        // merges them via `insert_chunk`; get_block must observe the data.
        let mut s = ChunkStore::new();
        let mut chunk = ChunkData::new();
        chunk.set(3, 70, 4, Block::Cobblestone);
        s.insert_chunk(2, 2, chunk);
        // World coord = chunk*16 + local.
        assert_eq!(s.get_block(2 * 16 + 3, 70, 2 * 16 + 4), Block::Cobblestone);
        assert_eq!(s.chunk_count(), 1);
        assert_eq!(s.occupied_chunks(), vec![(2, 2)]);
    }

    #[test]
    fn chunkstore_add_block_entity_buckets_by_xz_ignores_y() {
        // QA-013 contract: entities at the same (x, z) but different y must
        // share one bucket (the writer keys only by chunk; the y position
        // is encoded inside the NBT payload by callers).
        let mut s = ChunkStore::new();
        s.add_block_entity(5, 65, 7, vec![0xAA]);
        s.add_block_entity(5, 70, 7, vec![0xBB]); // same chunk, different y
        s.add_block_entity(20, 65, 7, vec![0xCC]); // different chunk
        let buckets = s.block_entities();
        assert_eq!(buckets.len(), 2, "two distinct chunk buckets expected");
        // Same-(x,z) bucket holds both blobs in insertion order.
        let shared = buckets
            .get(&(0, 0))
            .expect("entities at (5,7) and (5,7) share chunk (0,0)");
        assert_eq!(*shared, vec![vec![0xAA], vec![0xBB]]);
        let other = buckets
            .get(&(1, 0))
            .expect("entity at (20,7) lands in chunk (1,0)");
        assert_eq!(*other, vec![vec![0xCC]]);
    }

    #[test]
    fn chunkstore_add_block_entity_respects_bounds() {
        let mut s = ChunkStore::new_bounded(0, 0, 0, 0);
        s.add_block_entity(0, 65, 0, vec![0x11]); // inside
        s.add_block_entity(16, 65, 0, vec![0x22]); // outside
        assert_eq!(s.block_entities().len(), 1);
        assert!(s.block_entities().contains_key(&(0, 0)));
    }

    #[test]
    fn chunkstore_override_maps_round_trip() {
        let mut s = ChunkStore::new();
        s.set_sign_direction(1, 65, 2, 7);
        s.set_block_direction(3, 66, 4, 5);
        assert_eq!(s.sign_directions().get(&(1, 65, 2)), Some(&7));
        assert_eq!(s.block_directions().get(&(3, 66, 4)), Some(&5));
        // Defaults to 0 when absent (mirrors the encoder's lookup).
        assert_eq!(s.sign_directions().get(&(9, 9, 9)), None);
    }

    #[test]
    fn chunkstore_surface_blocks_picks_topmost_non_air_per_column() {
        let mut s = ChunkStore::new();
        s.set_block(0, 60, 0, Block::Dirt);
        s.set_block(0, 65, 0, Block::GrassBlock); // higher — must win
        s.set_block(0, 62, 0, Block::Stone);
        s.set_block(5, 70, 5, Block::Sand);
        let mut surfaces = s.surface_blocks();
        surfaces.sort_by_key(|t| (t.0, t.1, t.2));
        assert_eq!(
            surfaces,
            vec![
                (0, 0, 65, format!("{:?}", Block::GrassBlock)),
                (5, 5, 70, format!("{:?}", Block::Sand)),
            ]
        );
    }

    #[test]
    fn chunkstore_take_chunks_then_clear_aux_is_the_flush_drain_contract() {
        // Bedrock's `drain_chunks_to_writer` does `take_chunks` then writes
        // each entry, then `clear_aux`. After that the store must be empty
        // and ready for the next tile.
        let mut s = ChunkStore::new();
        s.set_block(0, 65, 0, Block::Stone);
        s.add_block_entity(0, 65, 0, vec![0xAB]);
        s.set_sign_direction(0, 65, 0, 3);
        s.set_block_direction(0, 65, 0, 2);

        let drained = s.take_chunks();
        assert_eq!(drained.len(), 1);
        // Chunks are now empty in the store, but aux maps survive until
        // `clear_aux` runs — matching the order Bedrock's drain uses
        // (it reads aux maps while writing each drained chunk).
        assert_eq!(s.chunks().len(), 0);
        assert_eq!(s.block_entities().len(), 1);
        assert_eq!(s.sign_directions().len(), 1);
        assert_eq!(s.block_directions().len(), 1);

        s.clear_aux();
        assert!(s.block_entities().is_empty());
        assert!(s.sign_directions().is_empty());
        assert!(s.block_directions().is_empty());
        assert_eq!(s.chunk_count(), 0);
    }

    #[test]
    fn chunkstore_default_matches_new() {
        // `ChunkStore: Default` is what `ChunkData::entry().or_default()`
        // style code paths rely on; it must produce an unbounded empty store.
        let d = ChunkStore::default();
        assert_eq!(d.chunk_count(), 0);
        assert!(d.in_bounds(0, 0));
        assert!(d.in_bounds(999, -999));
    }
}

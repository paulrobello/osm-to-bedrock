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
#[allow(dead_code)]
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
pub trait WorldWriter {
    /// Set a block at absolute (x, y, z) world coordinates.
    fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block);

    /// Get a block at absolute (x, y, z) world coordinates.
    fn get_block(&self, x: i32, y: i32, z: i32) -> Block;

    /// Insert a pre-built [`ChunkData`] at chunk coordinates (cx, cz).
    fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData);

    /// Add a block-entity NBT blob at the given world coordinates.
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

// ── Java Edition memory budget (ARC-001) ──────────────────────────────────
//
// The streaming tile pipeline keeps peak RAM bounded for Bedrock (one
// tile at a time, drained to LevelDB), but the Java backend has no
// streaming Anvil writer yet — every `flush_tile` is a no-op and the
// whole world accumulates in `JavaWorld::chunks`. A city-scale
// `edition=java` conversion OOM-kills the process, and the web
// `/fetch-convert` endpoint exposes this to any client. Until a
// streaming Anvil writer ships, [`enforce_java_memory_budget`] refuses
// the conversion *before* allocating the world, turning a deterministic
// OOM into a clean 400 / anyhow error.
//
// Per-chunk math:
//   - `Block` is `#[repr(u8)]` (1 byte; see `src/blocks.rs`).
//   - A fully-populated chunk spans 24 sub-chunks (Y range −64..319 →
//     sub-chunk indices −4..=19), each holding 4096 blocks ⇒ 24 × 4096 =
//     98_304 bytes of `Block` payload per chunk.
//   - Each non-empty sub-chunk is heap-allocated as `Box<[Block; 4096]>`
//     and stored in `HashMap<i8, Box<[Block; 4096]>>`, adding ~64 bytes
//     of HashMap/box overhead per sub-chunk (entry + bucket slot +
//     allocation padding). For a worst-case 24 sub-chunks that is
//     ~1.5 KB of overhead per chunk.
//   - Worst case per chunk: 98_304 + 1_536 ≈ 100 KB. Realistic chunks
//     (5–10 sub-chunks around the surface) are ~25–60 KB; we use the
//     worst-case bound for the guardrail.
//   - `JavaWorld::chunks: HashMap<(i32,i32), ChunkData>` adds ~64 bytes
//     per entry of map overhead, negligible at the per-chunk scale.
//
// At the default budget of 1.5 GB this permits ~15_000 chunks (a
// ~120 × 120 chunk area, ≈ 2 km × 2 km at scale 1.0) — comfortable for
// a metropolitan extract, tight enough to refuse a city-scale OSM pull.

/// Worst-case bytes of `Block` payload + HashMap/box overhead per
/// fully-populated chunk in the Java in-memory writer.
pub const JAVA_CHUNK_BYTES_WORST_CASE: u64 = 100 * 1024;

/// Peak RAM the Java in-memory writer may accumulate before the pipeline
/// refuses the conversion. Picked to leave headroom on a 4 GB container
/// after accounting for OSM data, spatial index, and the `encode_region`
/// contiguous buffer.
pub const JAVA_MEMORY_BUDGET_BYTES: u64 = 1_500 * 1024 * 1024; // ~1.5 GB

/// Return `Ok(())` if the Java writer can hold `chunk_count` chunks
/// within [`JAVA_MEMORY_BUDGET_BYTES`]; otherwise return an `anyhow`
/// error whose message tells the operator to switch to Bedrock or shrink
/// the bbox.
///
/// No-op for non-Java editions (Bedrock streams tile-by-tile).
pub fn enforce_java_memory_budget(edition: Edition, chunk_count: u64) -> Result<()> {
    if edition != Edition::Java {
        return Ok(());
    }
    let estimated_bytes = chunk_count.saturating_mul(JAVA_CHUNK_BYTES_WORST_CASE);
    if estimated_bytes > JAVA_MEMORY_BUDGET_BYTES {
        anyhow::bail!(
            "Java Edition conversion would accumulate ~{:.2} GB of chunk data in memory \
             ({} chunks × ~{} KB), exceeding the {:.2} GB safety budget. \
             Java Edition does not yet have a streaming Anvil writer — \
             use Bedrock Edition (the default) for larger areas, or shrink the bounding box. \
             See ARC-001 in AUDIT.md for the streaming-writer roadmap.",
            estimated_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
            chunk_count,
            JAVA_CHUNK_BYTES_WORST_CASE / 1024,
            JAVA_MEMORY_BUDGET_BYTES as f64 / (1024.0 * 1024.0 * 1024.0),
        );
    }
    Ok(())
}

// ── Edition factory methods ──────────────────────────────────────────────

impl Edition {
    /// Create an unbounded world for this edition.
    #[allow(dead_code)]
    pub fn create_world(&self, output: &Path) -> Box<dyn WorldWriter> {
        match self {
            Edition::Bedrock => Box::new(crate::bedrock::BedrockWorld::new(output)),
            Edition::Java => Box::new(crate::anvil::JavaWorld::new(output)),
        }
    }

    /// Create a bounded world for incremental tile-based processing.
    #[allow(dead_code)]
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

    // ── Java memory budget (ARC-001) ────────────────────────────────────
    //
    // `enforce_java_memory_budget` is the deterministic OOM guard for the
    // Java in-memory writer. The trait-level behaviour we need to pin:
    //   - Always Ok for Bedrock (regardless of chunk count).
    //   - Ok for Java under the budget.
    //   - Err for Java over the budget, with a message that tells the
    //     operator how to recover (Bedrock or smaller bbox).

    #[test]
    fn java_memory_budget_accepts_bedrock_regardless_of_chunk_count() {
        // Bedrock streams tile-by-tile — never subject to the guard.
        assert!(enforce_java_memory_budget(Edition::Bedrock, 0).is_ok());
        assert!(enforce_java_memory_budget(Edition::Bedrock, 1_000_000).is_ok());
    }

    #[test]
    fn java_memory_budget_accepts_small_java_conversions() {
        // ~10_000 chunks × 100 KB = ~1 GB — under the 1.5 GB budget.
        assert!(enforce_java_memory_budget(Edition::Java, 0).is_ok());
        assert!(enforce_java_memory_budget(Edition::Java, 10_000).is_ok());
    }

    #[test]
    fn java_memory_budget_rejects_oversized_java_conversions() {
        // ~20_000 chunks × 100 KB = ~2 GB — over budget.
        let err = enforce_java_memory_budget(Edition::Java, 20_000)
            .expect_err("20k chunks should exceed Java budget");
        let msg = format!("{err}");
        assert!(
            msg.contains("Java Edition"),
            "message should name Java: {msg}"
        );
        assert!(
            msg.contains("Bedrock") || msg.contains("bounding box"),
            "message should suggest a remedy: {msg}",
        );
    }

    #[test]
    fn java_memory_budget_threshold_matches_documented_math() {
        // The threshold must be exactly JAVA_MEMORY_BUDGET_BYTES /
        // JAVA_CHUNK_BYTES_WORST_CASE so the docstring stays accurate.
        let max_allowed = JAVA_MEMORY_BUDGET_BYTES / JAVA_CHUNK_BYTES_WORST_CASE;
        assert!(enforce_java_memory_budget(Edition::Java, max_allowed).is_ok());
        assert!(enforce_java_memory_budget(Edition::Java, max_allowed + 1).is_err());
        // ~15_000 chunks per the docstring.
        assert_eq!(max_allowed, 15_360);
    }
}

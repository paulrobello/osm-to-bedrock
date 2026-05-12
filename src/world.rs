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

    /// Write the world to disk with spawn at the given block coordinates.
    fn save(&self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()>;
}

// ── Edition factory methods ──────────────────────────────────────────────
// TODO: Uncomment once BedrockWorld implements WorldWriter (Task 2).
//
// impl Edition {
//     /// Create an unbounded world for this edition.
//     #[allow(dead_code)]
//     pub fn create_world(&self, output: &Path) -> Box<dyn WorldWriter> {
//         match self {
//             Edition::Bedrock => Box::new(crate::bedrock::BedrockWorld::new(output)),
//             Edition::Java => {
//                 #[cfg(feature = "java")]
//                 {
//                     Box::new(crate::anvil::JavaWorld::new(output))
//                 }
//                 #[cfg(not(feature = "java"))]
//                 {
//                     panic!("Java Edition support requires the 'java' feature")
//                 }
//             }
//         }
//     }
//
//     /// Create a bounded world for incremental tile-based processing.
//     #[allow(dead_code)]
//     pub fn create_world_bounded(
//         &self,
//         output: &Path,
//         min_cx: i32,
//         max_cx: i32,
//         min_cz: i32,
//         max_cz: i32,
//     ) -> Box<dyn WorldWriter> {
//         match self {
//             Edition::Bedrock => Box::new(crate::bedrock::BedrockWorld::new_bounded(
//                 output, min_cx, max_cx, min_cz, max_cz,
//             )),
//             Edition::Java => {
//                 #[cfg(feature = "java")]
//                 {
//                     Box::new(crate::anvil::JavaWorld::new_bounded(
//                         output, min_cx, max_cx, min_cz, max_cz,
//                     ))
//                 }
//                 #[cfg(not(feature = "java"))]
//                 {
//                     panic!("Java Edition support requires the 'java' feature")
//                 }
//             }
//         }
//     }
// }

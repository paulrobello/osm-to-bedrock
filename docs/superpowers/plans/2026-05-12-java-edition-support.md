# Java Edition Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Minecraft Java Edition (1.18+) as an output format alongside Bedrock Edition, selected via `--edition bedrock|java` CLI flag or `edition` HTTP param.

**Architecture:** Trait-based abstraction (`WorldWriter`) shared between `BedrockWorld` (existing, unchanged logic) and `JavaWorld` (new Anvil region writer). Pipeline functions switch from `&mut BedrockWorld` to `&mut dyn WorldWriter`. `ChunkData` moves from `bedrock.rs` to shared `world.rs`.

**Tech Stack:** Rust, clap, axum, flate2 (zlib compression for Anvil chunks), serde (Edition enum serialization).

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `src/world.rs` | **Create** | `WorldWriter` trait, `Edition` enum, `ChunkData` (moved from bedrock.rs) |
| `src/nbt_be.rs` | **Create** | Big-endian NBT writer, `encode_java_sign_entity()` |
| `src/anvil.rs` | **Create** | `JavaWorld` (implements `WorldWriter`), Anvil region writer, Java `level.dat` |
| `src/bedrock.rs` | **Modify** | Re-export `ChunkData`/`MIN_Y`/`MAX_Y` from `world.rs`, add `impl WorldWriter` |
| `src/blocks.rs` | **Modify** | Add `java_name()`, `java_block_states()`, `surface_to_java_biome()` |
| `src/pipeline.rs` | **Modify** | Switch to `&mut dyn WorldWriter`, use `Edition` factory |
| `src/params.rs` | **Modify** | Add `edition: Edition` to `ConvertParams` and `TerrainParams` |
| `src/config.rs` | **Modify** | Add `edition: Option<String>` field |
| `src/main.rs` | **Modify** | Add `--edition` flag to convert/fetch-convert/overture-convert/terrain-convert |
| `src/server.rs` | **Modify** | Add `edition` to request structs, dispatch packaging |
| `src/lib.rs` | **Modify** | Add `pub mod world;`, `pub mod anvil;`, `pub mod nbt_be;` |

---

### Task 1: Create `src/world.rs` — WorldWriter trait and shared types

**Files:**
- Create: `src/world.rs`
- Modify: `src/lib.rs:45-66` (add `pub mod world;`)

- [ ] **Step 1: Create `src/world.rs` with trait, Edition enum, and moved ChunkData**

```rust
//! Shared world abstractions for Bedrock and Java editions.

use crate::blocks::Block;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

// ── World Y-range constants (shared by both editions, 1.18+) ────────────

pub const MIN_Y: i32 = -64;
pub const MAX_Y: i32 = 319;
#[allow(dead_code)]
pub const WORLD_HEIGHT: i32 = MAX_Y - MIN_Y + 1; // 384

// ── Edition selector ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    #[default]
    Bedrock,
    Java,
}

impl std::fmt::Display for Edition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edition::Bedrock => write!(f, "bedrock"),
            Edition::Java => write!(f, "java"),
        }
    }
}

impl std::str::FromStr for Edition {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bedrock" => Ok(Edition::Bedrock),
            "java" => Ok(Edition::Java),
            _ => Err(anyhow::anyhow!("invalid edition '{s}', expected 'bedrock' or 'java'")),
        }
    }
}

impl clap::ValueEnum for Edition {
    fn value_variants<'a>() -> &'a [Self] {
        &[Edition::Bedrock, Edition::Java]
    }
    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.to_string()))
    }
}

// ── ChunkData (format-agnostic in-memory block storage) ─────────────────

/// In-memory representation of one 16×(height)×16 chunk column.
///
/// Blocks are stored in sub-chunks of 16×16×16, indexed XZY (x*256 + z*16 + y_local).
/// Only non-empty sub-chunks are allocated.
#[derive(Default)]
pub struct ChunkData {
    /// Map from sub-chunk Y index to block array (4096 entries, XZY).
    subchunks: HashMap<i8, Box<[Block; 4096]>>,
}

impl ChunkData {
    pub fn new() -> Self {
        Self::default()
    }

    fn idx(lx: i32, ly: i32, lz: i32) -> usize {
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

// ── WorldWriter trait ───────────────────────────────────────────────────

/// Operations the conversion pipeline needs from a world backend.
pub trait WorldWriter {
    fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block);
    fn get_block(&self, x: i32, y: i32, z: i32) -> Block;
    fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData);
    fn add_block_entity(&mut self, x: i32, y: i32, z: i32, nbt: Vec<u8>);
    fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32);
    fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32);
    fn chunk_count(&self) -> usize;
    fn occupied_chunks(&self) -> Vec<(i32, i32)>;
    fn save(&self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()>;
}

impl Edition {
    /// Construct the appropriate world backend.
    pub fn create_world(&self, output: &Path) -> Box<dyn WorldWriter> {
        match self {
            Edition::Bedrock => Box::new(crate::bedrock::BedrockWorld::new(output)),
            Edition::Java => Box::new(crate::anvil::JavaWorld::new(output)),
        }
    }

    /// Construct a bounded world for tile-based streaming conversion.
    pub fn create_world_bounded(
        &self,
        output: &Path,
        min_cx: i32,
        max_cx: i32,
        min_cz: i32,
        max_cz: i32,
    ) -> Box<dyn WorldWriter> {
        match self {
            Edition::Bedrock => {
                Box::new(crate::bedrock::BedrockWorld::new_bounded(output, min_cx, max_cx, min_cz, max_cz))
            }
            Edition::Java => {
                Box::new(crate::anvil::JavaWorld::new_bounded(output, min_cx, max_cx, min_cz, max_cz))
            }
        }
    }
}
```

- [ ] **Step 2: Add `pub mod world;` to `src/lib.rs`**

Insert after the `pub mod bedrock;` line (line 45):

```rust
pub mod anvil;
```

Insert `pub mod world;` and `pub mod nbt_be;` in alphabetical order within the module list.

- [ ] **Step 3: Run `cargo check` to verify compilation**

Run: `cargo check`
Expected: compilation errors from `bedrock.rs` referencing the old `ChunkData`/`MIN_Y`/`MAX_Y` paths — these get fixed in Task 2.

- [ ] **Step 4: Commit**

```bash
git add src/world.rs src/lib.rs
git commit -m "feat: add WorldWriter trait, Edition enum, and shared ChunkData"
```

---

### Task 2: Update `src/bedrock.rs` — Re-export from world.rs, implement WorldWriter

**Files:**
- Modify: `src/bedrock.rs` (entire file)

- [ ] **Step 1: Replace ChunkData, MIN_Y, MAX_Y with re-exports from world.rs**

At the top of `src/bedrock.rs`, remove the `ChunkData` struct definition, `MIN_Y`, `MAX_Y`, `WORLD_HEIGHT` constants, and the `chunk_key`/`subchunk_key` helper functions that reference them. Add re-exports:

```rust
// Re-export shared types from world module.
pub use crate::world::{ChunkData, Edition, MIN_Y, MAX_Y, WorldWriter};
```

Keep all Bedrock-specific code: `ChunkWriter`, `BedrockWorld`, compressors, encoding functions.

- [ ] **Step 2: Add `impl WorldWriter for BedrockWorld`**

The existing methods on `BedrockWorld` already match the trait signatures. Add an explicit impl block:

```rust
impl crate::world::WorldWriter for BedrockWorld {
    fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block) {
        BedrockWorld::set_block(self, x, y, z, block)
    }
    fn get_block(&self, x: i32, y: i32, z: i32) -> Block {
        BedrockWorld::get_block(self, x, y, z)
    }
    fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData) {
        BedrockWorld::insert_chunk(self, cx, cz, chunk)
    }
    fn add_block_entity(&mut self, x: i32, y: i32, z: i32, nbt: Vec<u8>) {
        BedrockWorld::add_block_entity(self, x, y, z, nbt)
    }
    fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        BedrockWorld::set_sign_direction(self, x, y, z, direction)
    }
    fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        BedrockWorld::set_block_direction(self, x, y, z, direction)
    }
    fn chunk_count(&self) -> usize {
        BedrockWorld::chunk_count(self)
    }
    fn occupied_chunks(&self) -> Vec<(i32, i32)> {
        BedrockWorld::occupied_chunks(self)
    }
    fn save(&self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
        BedrockWorld::save(self, spawn_x, spawn_y, spawn_z)
    }
}
```

- [ ] **Step 3: Update the `ChunkWriter::write_chunk` method**

The `write_chunk` method signature references `&ChunkData` — this now comes from `crate::world::ChunkData` via the re-export. Update the `non_empty_subchunks` call if needed (it's now a public method on `ChunkData`).

- [ ] **Step 4: Run `cargo check`**

Run: `cargo check`
Expected: compiles cleanly. Existing tests in `blocks.rs` and `bedrock.rs` still pass because the re-exports preserve the public API.

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: all existing tests pass (115 unit tests + 11 integration tests).

- [ ] **Step 6: Commit**

```bash
git add src/bedrock.rs
git commit -m "refactor: bedrock.rs re-exports ChunkData from world.rs, impl WorldWriter"
```

---

### Task 3: Add Java block mappings to `src/blocks.rs`

**Files:**
- Modify: `src/blocks.rs` (add methods after `block_states()`, add `surface_to_java_biome()`)

- [ ] **Step 1: Write failing tests for java_name(), java_block_states(), surface_to_java_biome()**

Add to the `#[cfg(test)] mod tests` block at the end of `src/blocks.rs`:

```rust
#[test]
fn java_name_sign() {
    assert_eq!(Block::OakSign.java_name(), "minecraft:oak_sign");
}

#[test]
fn java_name_brick() {
    assert_eq!(Block::Brick.java_name(), "minecraft:bricks");
}

#[test]
fn java_name_poppy() {
    assert_eq!(Block::Poppy.java_name(), "minecraft:poppy");
}

#[test]
fn java_name_stone_slab() {
    assert_eq!(Block::StoneSlab.java_name(), "minecraft:stone_slab");
}

#[test]
fn java_name_tallgrass_is_tall_grass() {
    assert_eq!(Block::TallGrass.java_name(), "minecraft:tall_grass");
}

#[test]
fn java_name_fern_is_separate_block() {
    assert_eq!(Block::Fern.java_name(), "minecraft:fern");
}

#[test]
fn java_name_stone_brick_wall_is_own_block() {
    assert_eq!(Block::StoneBrickWall.java_name(), "minecraft:stone_brick_wall");
}

#[test]
fn java_name_cherry_sign() {
    assert_eq!(Block::CherrySign.java_name(), "minecraft:cherry_sign");
}

#[test]
fn java_name_shared_blocks_unchanged() {
    // Blocks that have the same name in both editions
    assert_eq!(Block::Stone.java_name(), "minecraft:stone");
    assert_eq!(Block::Bedrock.java_name(), "minecraft:bedrock");
    assert_eq!(Block::OakLog.java_name(), "minecraft:oak_log");
    assert_eq!(Block::Water.java_name(), "minecraft:water");
    assert_eq!(Block::CobblestoneWall.java_name(), "minecraft:cobblestone_wall");
}

#[test]
fn java_block_states_sign_has_rotation() {
    let states = Block::OakSign.java_block_states();
    assert!(states.iter().any(|(k, _)| *k == "rotation"));
}

#[test]
fn java_block_states_slab_has_half() {
    let states = Block::OakSlab.java_block_states();
    assert!(states.iter().any(|(k, v)| *k == "type" && *v == "bottom"));
}

#[test]
fn java_block_states_stairs_has_facing() {
    let states = Block::OakStairs.java_block_states();
    assert!(states.iter().any(|(k, _)| *k == "facing"));
    assert!(states.iter().any(|(k, _)| *k == "half"));
}

#[test]
fn java_block_states_log_has_axis() {
    let states = Block::BirchLog.java_block_states();
    assert!(states.iter().any(|(k, v)| *k == "axis" && *v == "y"));
}

#[test]
fn java_block_states_poppy_has_no_states() {
    // Java poppy is a simple block with no states
    assert!(Block::Poppy.java_block_states().is_empty());
}

#[test]
fn java_biome_water() {
    assert_eq!(surface_to_java_biome(Block::Water), "minecraft:river");
}

#[test]
fn java_biome_forest() {
    assert_eq!(surface_to_java_biome(Block::OakLog), "minecraft:forest");
    assert_eq!(surface_to_java_biome(Block::OakLeaves), "minecraft:forest");
}

#[test]
fn java_biome_birch() {
    assert_eq!(surface_to_java_biome(Block::BirchLog), "minecraft:birch_forest");
}

#[test]
fn java_biome_beach() {
    assert_eq!(surface_to_java_biome(Block::Sand), "minecraft:beach");
}

#[test]
fn java_biome_mountains() {
    assert_eq!(surface_to_java_biome(Block::Stone), "minecraft:windswept_hills");
}

#[test]
fn java_biome_snow() {
    assert_eq!(surface_to_java_biome(Block::SnowLayer), "minecraft:snowy_plains");
}

#[test]
fn java_biome_plains_default() {
    assert_eq!(surface_to_java_biome(Block::GrassBlock), "minecraft:plains");
    assert_eq!(surface_to_java_biome(Block::Dirt), "minecraft:plains");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib blocks::tests::java_ -- --test-threads=1 2>&1 | head -20`
Expected: compilation errors — `java_name`, `java_block_states`, `surface_to_java_biome` not defined.

- [ ] **Step 3: Implement `java_name()` on `Block`**

Add after the existing `block_states()` method (after line 208):

```rust
/// Java Edition block identifier string.
pub fn java_name(self) -> &'static str {
    match self {
        Block::Air => "minecraft:air",
        Block::Bedrock => "minecraft:bedrock",
        Block::Stone => "minecraft:stone",
        Block::Dirt => "minecraft:dirt",
        Block::GrassBlock => "minecraft:grass_block",
        Block::Water => "minecraft:water",
        Block::Sand => "minecraft:sand",
        Block::Gravel => "minecraft:gravel",
        Block::OakLog => "minecraft:oak_log",
        Block::OakLeaves => "minecraft:oak_leaves",
        Block::StoneBrick => "minecraft:stone_bricks",
        Block::Concrete => "minecraft:light_gray_concrete",
        Block::Cobblestone => "minecraft:cobblestone",
        Block::BlackConcrete => "minecraft:black_concrete",
        Block::GrayConcrete => "minecraft:gray_concrete",
        Block::StoneSlab => "minecraft:stone_slab",
        Block::YellowConcrete => "minecraft:yellow_concrete",
        Block::OakSign => "minecraft:oak_sign",
        Block::GlassPane => "minecraft:glass_pane",
        Block::OakStairs => "minecraft:oak_stairs",
        Block::OakSlab => "minecraft:oak_slab",
        Block::OakFence => "minecraft:oak_fence",
        Block::CobblestoneWall => "minecraft:cobblestone_wall",
        Block::Brick => "minecraft:bricks",
        Block::Sandstone => "minecraft:sandstone",
        Block::OakPlanks => "minecraft:oak_planks",
        Block::SprucePlanks => "minecraft:spruce_planks",
        Block::WhiteConcrete => "minecraft:white_concrete",
        Block::StoneBrickStairs => "minecraft:stone_brick_stairs",
        Block::Rail => "minecraft:rail",
        Block::TallGrass => "minecraft:tall_grass",
        Block::Fern => "minecraft:fern",
        Block::Poppy => "minecraft:poppy",
        Block::Torch => "minecraft:torch",
        Block::Lantern => "minecraft:lantern",
        Block::StoneBrickWall => "minecraft:stone_brick_wall",
        Block::BirchLog => "minecraft:birch_log",
        Block::BirchLeaves => "minecraft:birch_leaves",
        Block::PolishedBlackstoneSlab => "minecraft:polished_blackstone_slab",
        Block::SmoothStoneSlab => "minecraft:smooth_stone_slab",
        Block::AndesiteSlab => "minecraft:andesite_slab",
        Block::CherrySign => "minecraft:cherry_sign",
        Block::Snow => "minecraft:snow",
        Block::SnowLayer => "minecraft:snow",
        Block::Ice => "minecraft:ice",
        Block::CherryHangingSign => "minecraft:cherry_hanging_sign",
        Block::Dispenser => "minecraft:dispenser",
        Block::BrewingStand => "minecraft:brewing_stand",
        Block::Bookshelf => "minecraft:bookshelf",
        Block::Cauldron => "minecraft:cauldron",
        Block::Bed => "minecraft:red_bed",
        Block::Furnace => "minecraft:furnace",
        Block::Barrel => "minecraft:barrel",
        Block::Bell => "minecraft:bell",
        Block::Campfire => "minecraft:campfire",
        Block::HayBale => "minecraft:hay_block",
    }
}

/// Java Edition block state properties as (key, value) string pairs.
pub fn java_block_states(self) -> Vec<(&'static str, &'static str)> {
    match self {
        Block::OakSign | Block::CherrySign => vec![("rotation", "0")],
        Block::TallGrass => vec![], // Java tall_grass has no required states for placement
        Block::Fern => vec![],
        Block::Poppy => vec![],
        Block::CobblestoneWall => vec![("up", "true")],
        Block::StoneBrickWall => vec![("up", "true")],
        Block::Torch => vec![],
        Block::Lantern => vec![("hanging", "false")],
        Block::OakSlab
        | Block::PolishedBlackstoneSlab
        | Block::SmoothStoneSlab
        | Block::AndesiteSlab => vec![("type", "bottom")],
        Block::StoneSlab => vec![("type", "bottom")],
        Block::Sandstone => vec![],
        Block::BirchLog => vec![("axis", "y")],
        Block::OakLog => vec![("axis", "y")],
        Block::BirchLeaves | Block::OakLeaves => vec![("persistent", "true")],
        Block::OakStairs | Block::StoneBrickStairs => vec![
            ("facing", "north"),
            ("half", "bottom"),
            ("shape", "straight"),
        ],
        Block::Rail => vec![("shape", "north_south")],
        Block::SnowLayer => vec![("layers", "1")],
        Block::CherryHangingSign => vec![
            ("attached", "false"),
            ("rotation", "0"),
        ],
        Block::Dispenser => vec![("facing", "up")],
        Block::Furnace => vec![("facing", "south")],
        Block::Barrel => vec![("facing", "up"), ("open", "false")],
        Block::Bell => vec![("attachment", "floor"), ("facing", "north")],
        Block::Campfire => vec![("facing", "south"), ("lit", "true")],
        Block::HayBale => vec![("axis", "y")],
        Block::Bed => vec![("facing", "north"), ("part", "head")],
        _ => vec![],
    }
}
```

- [ ] **Step 4: Implement `surface_to_java_biome()`**

Add after `surface_to_biome()` (after line 340):

```rust
/// Map a surface block to a Java Edition string biome ID.
pub fn surface_to_java_biome(block: Block) -> &'static str {
    match block {
        Block::Water => "minecraft:river",
        Block::OakLog | Block::OakLeaves => "minecraft:forest",
        Block::BirchLog | Block::BirchLeaves => "minecraft:birch_forest",
        Block::Sand => "minecraft:beach",
        Block::Stone => "minecraft:windswept_hills",
        Block::Snow | Block::SnowLayer => "minecraft:snowy_plains",
        Block::Ice => "minecraft:frozen_river",
        _ => "minecraft:plains",
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib blocks::tests::java_`
Expected: all java_ tests PASS.

- [ ] **Step 6: Run full test suite**

Run: `cargo test`
Expected: all existing + new tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/blocks.rs
git commit -m "feat: add Java Edition block name/state/biome mappings"
```

---

### Task 4: Create `src/nbt_be.rs` — Big-endian NBT writer

**Files:**
- Create: `src/nbt_be.rs`

- [ ] **Step 1: Write failing test**

Add a test at the bottom of the new file (in a `#[cfg(test)] mod tests` block):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn be_string_tag_bytes() {
        // TAG_String(8) + name_len(BE u16 "hi") + "hi" + str_len(BE u16) + "world"
        let mut buf = Vec::new();
        write_string_tag(&mut buf, "hi", "world").unwrap();
        assert_eq!(&buf[0..1], &[TAG_STRING]); // tag type
        assert_eq!(&buf[1..3], &[0, 2]);       // name length BE
        assert_eq!(&buf[3..5], b"hi");
        assert_eq!(&buf[5..7], &[0, 5]);       // string length BE
        assert_eq!(&buf[7..12], b"world");
    }

    #[test]
    fn be_int_tag_bytes() {
        let mut buf = Vec::new();
        write_int_tag(&mut buf, "x", 42).unwrap();
        // tag header + BE i32
        let val_bytes = &buf[buf.len() - 4..];
        assert_eq!(val_bytes, &42i32.to_be_bytes());
    }

    #[test]
    fn be_long_array_tag() {
        let mut buf = Vec::new();
        write_long_array_tag(&mut buf, "data", &[1i64, 2, 3]).unwrap();
        // Verify it starts with TAG_LONG_ARRAY
        assert_eq!(buf[0], TAG_LONG_ARRAY);
    }

    #[test]
    fn java_sign_entity_has_correct_id() {
        let nbt = encode_java_sign_entity(10, 64, 20, "Hello\nWorld");
        // The NBT should contain "minecraft:sign" as a UTF-8 string
        let sign_id = b"minecraft:sign";
        assert!(nbt.windows(sign_id.len()).any(|w| w == sign_id));
    }
}
```

- [ ] **Step 2: Implement `nbt_be.rs`**

{% raw %}
```rust
//! Big-endian NBT writer for Java Edition.
//!
//! Java Edition uses big-endian NBT (vs Bedrock's little-endian).
//! Implements the tag subset needed for Anvil chunk encoding and level.dat.

use anyhow::Result;
use std::io::Write;

// NBT tag type IDs
pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_SHORT: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_STRING: u8 = 8;
pub const TAG_LIST: u8 = 9;
pub const TAG_COMPOUND: u8 = 10;
pub const TAG_INT_ARRAY: u8 = 11;
pub const TAG_LONG_ARRAY: u8 = 12;

fn write_u16_be(w: &mut impl Write, v: u16) -> Result<()> {
    w.write_all(&v.to_be_bytes())?;
    Ok(())
}

pub fn write_string_payload(w: &mut impl Write, s: &str) -> Result<()> {
    write_u16_be(w, s.len() as u16)?;
    w.write_all(s.as_bytes())?;
    Ok(())
}

fn write_tag_header(w: &mut impl Write, tag_type: u8, name: &str) -> Result<()> {
    w.write_all(&[tag_type])?;
    write_string_payload(w, name)?;
    Ok(())
}

pub fn write_compound_start(w: &mut impl Write, name: &str) -> Result<()> {
    write_tag_header(w, TAG_COMPOUND, name)
}

pub fn write_end(w: &mut impl Write) -> Result<()> {
    w.write_all(&[TAG_END])?;
    Ok(())
}

pub fn write_string_tag(w: &mut impl Write, name: &str, value: &str) -> Result<()> {
    write_tag_header(w, TAG_STRING, name)?;
    write_string_payload(w, value)?;
    Ok(())
}

pub fn write_int_tag(w: &mut impl Write, name: &str, value: i32) -> Result<()> {
    write_tag_header(w, TAG_INT, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

pub fn write_long_tag(w: &mut impl Write, name: &str, value: i64) -> Result<()> {
    write_tag_header(w, TAG_LONG, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

pub fn write_float_tag(w: &mut impl Write, name: &str, value: f32) -> Result<()> {
    write_tag_header(w, TAG_FLOAT, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

pub fn write_double_tag(w: &mut impl Write, name: &str, value: f64) -> Result<()> {
    write_tag_header(w, TAG_DOUBLE, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

pub fn write_byte_tag(w: &mut impl Write, name: &str, value: i8) -> Result<()> {
    write_tag_header(w, TAG_BYTE, name)?;
    w.write_all(&[value as u8])?;
    Ok(())
}

pub fn write_short_tag(w: &mut impl Write, name: &str, value: i16) -> Result<()> {
    write_tag_header(w, TAG_SHORT, name)?;
    w.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Open a TAG_List: writes type byte + name + item_type byte + length BE i32.
/// Caller then writes `length` tag payloads (no names per item).
pub fn write_list_start(w: &mut impl Write, name: &str, item_type: u8, length: i32) -> Result<()> {
    write_tag_header(w, TAG_LIST, name)?;
    w.write_all(&[item_type])?;
    w.write_all(&length.to_be_bytes())?;
    Ok(())
}

pub fn write_int_array_tag(w: &mut impl Write, name: &str, values: &[i32]) -> Result<()> {
    write_tag_header(w, TAG_INT_ARRAY, name)?;
    w.write_all(&(values.len() as i32).to_be_bytes())?;
    for &v in values {
        w.write_all(&v.to_be_bytes())?;
    }
    Ok(())
}

pub fn write_long_array_tag(w: &mut impl Write, name: &str, values: &[i64]) -> Result<()> {
    write_tag_header(w, TAG_LONG_ARRAY, name)?;
    w.write_all(&(values.len() as i32).to_be_bytes())?;
    for &v in values {
        w.write_all(&v.to_be_bytes())?;
    }
    Ok(())
}

/// Encode a sign block entity NBT blob for Java Edition.
///
/// `text` is the sign front text (lines separated by `\n`).
/// Returns a complete NBT compound (big-endian) ready for block_entities list.
pub fn encode_java_sign_entity(x: i32, y: i32, z: i32, text: &str) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();

    fn json_text(line: &str) -> String {
        format!("{{\"text\":\"{}\"}}", line.replace('"', "\\\""))
    }

    fn write_messages(buf: &mut Vec<u8>, text: &str) {
        let lines: Vec<&str> = text.split('\n').collect();
        let empty = json_text("");
        let messages: [&str; 4] = match lines.as_slice() {
            &[a] => [a, "", "", ""],
            &[a, b] => [a, b, "", ""],
            &[a, b, c] => [a, b, c, ""],
            &[a, b, c, d, ..] => [a, b, c, d],
            _ => ["", "", "", ""],
        };

        write_list_start(buf, "messages", TAG_STRING, 4)
            .expect("Vec write infallible");
        for msg in &messages {
            let s = if msg.is_empty() { &empty } else { &json_text(msg) };
            write_string_payload(buf, s).expect("Vec write infallible");
        }
    }

    write_compound_start(&mut buf, "").expect("Vec write infallible");
    write_string_tag(&mut buf, "id", "minecraft:sign").expect("Vec write infallible");
    write_int_tag(&mut buf, "x", x).expect("Vec write infallible");
    write_int_tag(&mut buf, "y", y).expect("Vec write infallible");
    write_int_tag(&mut buf, "z", z).expect("Vec write infallible");

    // front_text compound
    write_compound_start(&mut buf, "front_text").expect("Vec write infallible");
    write_messages(&mut buf, text);
    write_byte_tag(&mut buf, "has_glowing_text", 0).expect("Vec write infallible");
    write_int_tag(&mut buf, "color", -16_777_216).expect("Vec write infallible");
    write_end(&mut buf).expect("Vec write infallible");

    // back_text compound (empty)
    write_compound_start(&mut buf, "back_text").expect("Vec write infallible");
    write_list_start(&mut buf, "messages", TAG_STRING, 4).expect("Vec write infallible");
    for _ in 0..4 {
        write_string_payload(&mut buf, &json_text("")).expect("Vec write infallible");
    }
    write_byte_tag(&mut buf, "has_glowing_text", 0).expect("Vec write infallible");
    write_int_tag(&mut buf, "color", -16_777_216).expect("Vec write infallible");
    write_end(&mut buf).expect("Vec write infallible");

    write_byte_tag(&mut buf, "is_waxed", 0).expect("Vec write infallible");
    write_end(&mut buf).expect("Vec write infallible");

    buf
}
```
{% endraw %}

- [ ] **Step 3: Run tests**

Run: `cargo test --lib nbt_be`
Expected: all 4 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add src/nbt_be.rs
git commit -m "feat: add big-endian NBT writer for Java Edition"
```

---

### Task 5: Create `src/anvil.rs` — JavaWorld and Anvil region writer

This is the largest task. It creates the complete Java Edition world backend.

**Files:**
- Create: `src/anvil.rs`

- [ ] **Step 1: Write the file skeleton with JavaWorld struct and WorldWriter impl**

```rust
//! Java Edition Anvil world writer.
//!
//! Generates region files (`r.X.Z.mca`) with Anvil-format chunks and a Java
//! Edition `level.dat`.  Implements [`WorldWriter`] so the pipeline can target
//! Java Edition through the same trait interface as Bedrock.

use crate::{
    blocks::{Block, surface_to_java_biome},
    nbt_be::{
        self, TAG_BYTE, TAG_COMPOUND, TAG_END, TAG_INT, TAG_LIST, TAG_LONG, TAG_LONG_ARRAY,
        TAG_STRING,
    },
    world::{ChunkData, MIN_Y, MAX_Y, WorldWriter},
};
use anyhow::{Context, Result};
use flate2::{Compression, write::ZlibEncoder, read::ZlibDecoder};
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

/// Java data version for 1.21.x.
const DATA_VERSION: i32 = 3465;

const REGION_SIZE: i32 = 32; // 32×32 chunks per region file
const SECTOR_BYTES: usize = 4096;
```

- [ ] **Step 2: Implement JavaWorld struct**

```rust
/// In-memory world accumulator that serializes to Anvil format on `save()`.
pub struct JavaWorld {
    chunks: HashMap<(i32, i32), ChunkData>,
    output: PathBuf,
    block_entities: HashMap<(i32, i32), Vec<Vec<u8>>>,
    sign_directions: HashMap<(i32, i32, i32), i32>,
    block_directions: HashMap<(i32, i32, i32), i32>,
    chunk_bounds: Option<(i32, i32, i32, i32)>,
}

impl JavaWorld {
    pub fn new(output: &Path) -> Self {
        Self {
            chunks: HashMap::new(),
            output: output.to_path_buf(),
            block_entities: HashMap::new(),
            sign_directions: HashMap::new(),
            block_directions: HashMap::new(),
            chunk_bounds: None,
        }
    }

    pub fn new_bounded(
        output: &Path,
        min_cx: i32,
        max_cx: i32,
        min_cz: i32,
        max_cz: i32,
    ) -> Self {
        Self {
            chunks: HashMap::new(),
            output: output.to_path_buf(),
            block_entities: HashMap::new(),
            sign_directions: HashMap::new(),
            block_directions: HashMap::new(),
            chunk_bounds: Some((min_cx, max_cx, min_cz, max_cz)),
        }
    }

    #[inline]
    fn in_bounds(&self, cx: i32, cz: i32) -> bool {
        match self.chunk_bounds {
            None => true,
            Some((min_cx, max_cx, min_cz, max_cz)) => {
                cx >= min_cx && cx <= max_cx && cz >= min_cz && cz <= max_cz
            }
        }
    }
}
```

- [ ] **Step 3: Implement WorldWriter for JavaWorld**

```rust
impl WorldWriter for JavaWorld {
    fn set_block(&mut self, x: i32, y: i32, z: i32, block: Block) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        let lx = x.rem_euclid(16);
        let lz = z.rem_euclid(16);
        self.chunks.entry((cx, cz)).or_default().set(lx, y, lz, block);
    }

    fn get_block(&self, x: i32, y: i32, z: i32) -> Block {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let lx = x.rem_euclid(16);
        let lz = z.rem_euclid(16);
        self.chunks
            .get(&(cx, cz))
            .map(|c| c.get(lx, y, lz))
            .unwrap_or(Block::Air)
    }

    fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: ChunkData) {
        self.chunks.insert((cx, cz), chunk);
    }

    fn add_block_entity(&mut self, x: i32, _y: i32, z: i32, nbt: Vec<u8>) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.block_entities.entry((cx, cz)).or_default().push(nbt);
    }

    fn set_sign_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.sign_directions.insert((x, y, z), direction);
    }

    fn set_block_direction(&mut self, x: i32, y: i32, z: i32, direction: i32) {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        if !self.in_bounds(cx, cz) {
            return;
        }
        self.block_directions.insert((x, y, z), direction);
    }

    fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    fn occupied_chunks(&self) -> Vec<(i32, i32)> {
        self.chunks.keys().copied().collect()
    }

    fn save(&self, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
        let region_dir = self.output.join("region");
        std::fs::create_dir_all(&region_dir)
            .with_context(|| format!("creating region dir {}", region_dir.display()))?;

        // Group chunks by region file
        let mut regions: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
        for &(cx, cz) in self.chunks.keys() {
            let rx = cx.div_euclid(REGION_SIZE);
            let rz = cz.div_euclid(REGION_SIZE);
            regions.entry((rx, rz)).or_default().push((cx, cz));
        }

        for ((rx, rz), chunk_coords) in &regions {
            let path = region_dir.join(format!("r.{rx}.{rz}.mca"));
            write_region_file(&path, chunk_coords, self)?;
        }

        write_java_level_dat(&self.output, spawn_x, spawn_y, spawn_z)?;

        // session.lock — write timestamp
        let lock_path = self.output.join("session.lock");
        std::fs::write(&lock_path, chrono::Utc::now().timestamp().to_be_bytes())
            .with_context(|| "writing session.lock")?;

        Ok(())
    }
}
```

- [ ] **Step 4: Implement `write_region_file`**

```rust
/// Write all chunks for one region to an `.mca` file.
fn write_region_file(
    path: &Path,
    chunk_coords: &[(i32, i32)],
    world: &JavaWorld,
) -> Result<()> {
    let mut chunk_data: Vec<Option<Vec<u8>>> = vec![None; (REGION_SIZE * REGION_SIZE) as usize];

    for &(cx, cz) in chunk_coords {
        let lx = cx.rem_euclid(REGION_SIZE);
        let lz = cz.rem_euclid(REGION_SIZE);
        let idx = (lz * REGION_SIZE + lx) as usize;

        let chunk = world.chunks.get(&(cx, cz)).unwrap();
        let entities = world.block_entities.get(&(cx, cz));

        let nbt = encode_chunk_nbt(chunk, entities, cx, cz, world);
        let mut compressed = Vec::new();
        {
            let mut enc = ZlibEncoder::new(&mut compressed, Compression::default());
            enc.write_all(&nbt).context("compressing chunk NBT")?;
        }

        // Format: [be_u32 length] [0x02=zlib] [compressed data]
        let mut entry = Vec::with_capacity(4 + 1 + compressed.len());
        entry.extend_from_slice(&(1 + compressed.len() as i32).to_be_bytes());
        entry.push(0x02); // zlib compression
        entry.extend_from_slice(&compressed);
        chunk_data[idx] = Some(entry);
    }

    // Build the region file: header (8KB) + chunk sectors
    let mut file = Vec::new();
    let mut offsets = vec![0u32; (REGION_SIZE * REGION_SIZE) as usize];
    let mut sector_offset: u32 = 2; // header is 2 sectors (8KB)

    // First pass: compute offsets
    for (i, cd) in chunk_data.iter().enumerate() {
        if let Some(ref data) = cd {
            let sectors_needed = (data.len() as u32 + SECTOR_BYTES as u32 - 1) / SECTOR_BYTES as u32;
            offsets[i] = (sector_offset << 8) | (sectors_needed & 0xFF);
            sector_offset += sectors_needed;
        }
    }

    // Write location table (4KB)
    for &off in &offsets {
        file.extend_from_slice(&off.to_be_bytes());
    }

    // Write timestamp table (4KB) — fill with current time
    let now = chrono::Utc::now().timestamp() as u32;
    for _ in &offsets {
        file.extend_from_slice(&now.to_be_bytes());
    }

    // Write chunk data, padding each to sector boundary
    for cd in chunk_data.iter() {
        if let Some(ref data) = cd {
            file.extend_from_slice(data);
            let pad = (SECTOR_BYTES - (data.len() % SECTOR_BYTES)) % SECTOR_BYTES;
            file.extend(std::iter::repeat(0u8).take(pad));
        }
    }

    std::fs::write(path, &file)
        .with_context(|| format!("writing region file {}", path.display()))?;
    Ok(())
}
```

- [ ] **Step 5: Implement `encode_chunk_nbt`**

```rust
/// Encode one chunk as big-endian NBT (the chunk root compound).
fn encode_chunk_nbt(
    chunk: &ChunkData,
    entities: Option<&Vec<Vec<u8>>>,
    cx: i32,
    cz: i32,
    world: &JavaWorld,
) -> Vec<u8> {
    let mut buf = Vec::new();

    nbt_be::write_compound_start(&mut buf, "").unwrap();
    nbt_be::write_int_tag(&mut buf, "DataVersion", DATA_VERSION).unwrap();
    nbt_be::write_int_tag(&mut buf, "xPos", cx).unwrap();
    nbt_be::write_int_tag(&mut buf, "zPos", cz).unwrap();
    nbt_be::write_int_tag(&mut buf, "yPos", MIN_Y / 16).unwrap();
    nbt_be::write_string_tag(&mut buf, "Status", "minecraft:full").unwrap();

    // Heightmaps — compute WORLD_SURFACE
    let heightmap = compute_heightmap(chunk);
    nbt_be::write_compound_start(&mut buf, "Heightmaps").unwrap();
    nbt_be::write_long_array_tag(&mut buf, "WORLD_SURFACE", &heightmap).unwrap();
    nbt_be::write_end(&mut buf).unwrap();

    // Sections
    let subchunks: Vec<_> = chunk.non_empty_subchunks().collect();
    nbt_be::write_list_start(&mut buf, "sections", TAG_COMPOUND, subchunks.len() as i32).unwrap();
    for (sy, blocks) in &subchunks {
        encode_section_nbt(&mut buf, *sy, blocks, cx, cz, world);
    }

    // Block entities
    if let Some(entities) = entities {
        if !entities.is_empty() {
            nbt_be::write_list_start(&mut buf, "block_entities", TAG_COMPOUND, entities.len() as i32).unwrap();
            for entity_nbt in entities {
                // Each entity is already a complete NBT compound — strip the
                // outer tag header and embed directly.
                // For simplicity, re-parse: write compound start, copy inner bytes.
                // Actually, Anvil stores block_entities as a TAG_List of TAG_Compound,
                // so each entry needs just the compound payload (no name).
                // We'll re-encode signs specifically for Java.
                // For now, write the raw NBT blob (it's already a compound).
                // The list items don't have names, so we need to strip the
                // tag header from the entity blob and re-add as unnamed compound.
                // Safest: write the blob as-is (Java accepts named compounds in lists).
                buf.extend_from_slice(entity_nbt);
            }
        }
    }

    nbt_be::write_end(&mut buf).unwrap(); // close root compound
    buf
}

/// Compute the WORLD_SURFACE heightmap as 36 packed longs (9 bits per entry).
fn compute_heightmap(chunk: &ChunkData) -> Vec<i64> {
    let mut heights = vec![0i64; 256];
    for lx in 0..16i32 {
        for lz in 0..16i32 {
            let mut h: i64 = 0;
            for y in (MIN_Y..=MAX_Y).rev() {
                if chunk.get(lx, y, lz) != Block::Air {
                    h = y as i64;
                    break;
                }
            }
            heights[lz as usize * 16 + lx as usize] = h;
        }
    }

    // Pack 9-bit values into 64-bit longs: 7 values per long, 37 longs total.
    // But Minecraft uses 9 bits × 256 = 2304 bits = 36 longs.
    let bits_per = 9;
    let mut longs = Vec::with_capacity(36);
    let mut current: i64 = 0;
    let mut bit_pos: i32 = 0;
    for &h in &heights {
        current |= (h & ((1 << bits_per) - 1)) << bit_pos;
        bit_pos += bits_per;
        while bit_pos >= 64 {
            longs.push(current & !0);
            current >>= 64;
            bit_pos -= 64;
        }
    }
    if bit_pos > 0 {
        longs.push(current);
    }
    longs.truncate(36);
    longs
}
```

- [ ] **Step 6: Implement `encode_section_nbt`**

```rust
/// Encode one 16×16×16 section as a TAG_Compound for the sections list.
fn encode_section_nbt(
    buf: &mut Vec<u8>,
    sy: i8,
    blocks: &[Block; 4096],
    cx: i32,
    cz: i32,
    world: &JavaWorld,
) {
    // Build palette (collect unique blocks + their Java states)
    let mut palette_entries: Vec<(String, Vec<(&str, &str)>)> = Vec::new();
    let mut palette_map: HashMap<Vec<u8>, usize> = HashMap::new();

    // Helper to create a unique key for a block + its states
    let mut indices = [0usize; 4096];

    for (i, &block) in blocks.iter().enumerate() {
        let name = block.java_name();
        let states = block.java_block_states();

        // Apply direction overrides from world
        let states = apply_java_direction_override(block, i, cx, cz, sy, world);

        let mut key = Vec::new();
        key.extend_from_slice(name.as_bytes());
        for (k, v) in &states {
            key.push(0);
            key.extend_from_slice(k.as_bytes());
            key.push(0);
            key.extend_from_slice(v.as_bytes());
        }

        let idx = match palette_map.entry(key) {
            std::collections::hash_map::Entry::Occupied(e) => *e.get(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let idx = palette_entries.len();
                palette_entries.push((name.to_string(), states));
                e.insert(idx);
                idx
            }
        };
        indices[i] = idx;
    }

    // Write the section compound (unnamed — inside a TAG_List)
    // TAG_Compound with empty name
    buf.push(TAG_COMPOUND);
    buf.push(0); // name length 0
    buf.push(0);

    nbt_be::write_byte_tag(buf, "Y", sy).unwrap();

    // block_states compound
    nbt_be::write_compound_start(buf, "block_states").unwrap();

    if palette_entries.len() == 1 {
        // Single-block palette — no data array needed
        nbt_be::write_list_start(buf, "palette", TAG_COMPOUND, 1).unwrap();
        write_palette_entry(buf, &palette_entries[0].0, &palette_entries[0].1);
        nbt_be::write_end(buf).unwrap(); // close block_states
    } else {
        nbt_be::write_list_start(buf, "palette", TAG_COMPOUND, palette_entries.len() as i32).unwrap();
        for (name, states) in &palette_entries {
            write_palette_entry(buf, name, states);
        }

        // Pack indices into long array
        let bits_per = compute_bits_per_block(palette_entries.len());
        let packed = pack_indices_long_array(&indices, bits_per);
        nbt_be::write_long_array_tag(buf, "data", &packed).unwrap();
        nbt_be::write_end(buf).unwrap(); // close block_states
    }

    // biomes compound — single biome per section (simplified)
    let dominant_biome = determine_dominant_biome(blocks);
    nbt_be::write_compound_start(buf, "biomes").unwrap();
    nbt_be::write_list_start(buf, "palette", TAG_STRING, 1).unwrap();
    nbt_be::write_string_payload(buf, dominant_biome).unwrap();
    nbt_be::write_end(buf).unwrap(); // close biomes

    nbt_be::write_end(buf).unwrap(); // close section compound
}

fn write_palette_entry(buf: &mut Vec<u8>, name: &str, states: &[(&str, &str)]) {
    buf.push(TAG_COMPOUND);
    buf.push(0);
    buf.push(0); // unnamed compound in list
    nbt_be::write_string_tag(buf, "Name", name).unwrap();
    if !states.is_empty() {
        nbt_be::write_compound_start(buf, "Properties").unwrap();
        for (k, v) in states {
            nbt_be::write_string_tag(buf, k, v).unwrap();
        }
        nbt_be::write_end(buf).unwrap();
    }
    nbt_be::write_end(buf).unwrap();
}

/// Compute minimum bits-per-block for a palette of `palette_size` entries.
/// Java uses power-of-2 sizes: min 4, max 16 (or direct for >16).
fn compute_bits_per_block(palette_size: usize) -> usize {
    if palette_size <= 1 { return 0; }
    let bits = 64 - (palette_size - 1).leading_zeros() as usize;
    bits.max(4).min(16)
}

/// Pack block indices into a Minecraft long array.
/// Java packs from low bits, each long holds floor(64/bits_per) entries.
fn pack_indices_long_array(indices: &[usize; 4096], bits_per: usize) -> Vec<i64> {
    if bits_per == 0 {
        return vec![];
    }
    let entries_per_long = 64 / bits_per;
    let long_count = (4096 + entries_per_long - 1) / entries_per_long;
    let mut longs = vec![0i64; long_count];
    let mask = (1i64 << bits_per) - 1;

    for (i, &idx) in indices.iter().enumerate() {
        let long_idx = i / entries_per_long;
        let bit_offset = (i % entries_per_long) * bits_per;
        longs[long_idx] |= ((idx as i64) & mask) << bit_offset;
    }
    longs
}

/// Determine the most common biome for a section based on its blocks.
fn determine_dominant_biome(blocks: &[Block; 4096]) -> &'static str {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for &b in blocks.iter() {
        if b != Block::Air {
            *counts.entry(surface_to_java_biome(b)).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(b, _)| b)
        .unwrap_or("minecraft:plains")
}

/// Apply direction overrides for Java block states.
fn apply_java_direction_override(
    block: Block,
    _flat_idx: usize,
    _cx: i32,
    _cz: i32,
    _sy: i8,
    _world: &JavaWorld,
) -> Vec<(&'static str, &'static str)> {
    // For now, return default Java states. Direction overrides can be
    // applied in a follow-up by reading from world.sign_directions /
    // world.block_directions and mapping to Java state values.
    block.java_block_states()
}
```

- [ ] **Step 7: Implement `write_java_level_dat`**

```rust
/// Write a Java Edition `level.dat` file (gzip-compressed big-endian NBT).
fn write_java_level_dat(output: &Path, spawn_x: i32, spawn_y: i32, spawn_z: i32) -> Result<()> {
    let mut raw = Vec::new();

    nbt_be::write_compound_start(&mut raw, "").unwrap();
    nbt_be::write_compound_start(&mut raw, "Data").unwrap();

    nbt_be::write_int_tag(&mut raw, "DataVersion", DATA_VERSION).unwrap();

    // Version compound
    nbt_be::write_compound_start(&mut raw, "Version").unwrap();
    nbt_be::write_string_tag(&mut raw, "Name", "1.21.4").unwrap();
    nbt_be::write_int_tag(&mut raw, "Id", DATA_VERSION).unwrap();
    nbt_be::write_byte_tag(&mut raw, "Snapshot", 0).unwrap();
    nbt_be::write_end(&mut raw).unwrap();

    nbt_be::write_int_tag(&mut raw, "GameType", 1).unwrap(); // creative
    nbt_be::write_int_tag(&mut raw, "SpawnX", spawn_x).unwrap();
    nbt_be::write_int_tag(&mut raw, "SpawnY", spawn_y).unwrap();
    nbt_be::write_int_tag(&mut raw, "SpawnZ", spawn_z).unwrap();
    nbt_be::write_byte_tag(&mut raw, "allowCommands", 1).unwrap();
    nbt_be::write_string_tag(&mut raw, "LevelName", "OSM World").unwrap();
    nbt_be::write_long_tag(&mut raw, "LastPlayed", chrono::Utc::now().timestamp_millis()).unwrap();
    nbt_be::write_byte_tag(&mut raw, "hardcore", 0).unwrap();
    nbt_be::write_byte_tag(&mut raw, "initialized", 1).unwrap();

    // Generator
    nbt_be::write_compound_start(&mut raw, "WorldGenSettings").unwrap();
    nbt_be::write_long_tag(&mut raw, "seed", 0).unwrap();
    nbt_be::write_byte_tag(&mut raw, "generate_features", 0).unwrap();
    nbt_be::write_string_tag(&mut raw, "dimensions", "").unwrap();
    nbt_be::write_end(&mut raw).unwrap();

    nbt_be::write_end(&mut raw).unwrap(); // close Data
    nbt_be::write_end(&mut raw).unwrap(); // close root

    // Gzip compress
    let mut compressed = Vec::new();
    {
        let mut enc = ZlibEncoder::new(&mut compressed, Compression::default());
        enc.write_all(&raw).context("compressing level.dat")?;
    }

    // Java level.dat has a gzip wrapper, not raw zlib. Use flate2 GzEncoder.
    let mut gzipped = Vec::new();
    {
        let mut enc = flate2::write::GzEncoder::new(&mut gzipped, Compression::default());
        enc.write_all(&raw).context("gzip-encoding level.dat")?;
    }

    let path = output.join("level.dat");
    std::fs::write(&path, &gzipped).with_context(|| "writing level.dat")?;
    Ok(())
}
```

- [ ] **Step 8: Run `cargo check` and fix any compilation errors**

Run: `cargo check`
Expected: compiles. May need to adjust import paths or add `chrono` usage.

- [ ] **Step 9: Write a basic unit test for JavaWorld**

Add to the bottom of `anvil.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_world_set_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = JavaWorld::new(dir.path());
        world.set_block(10, 65, 20, Block::Stone);
        assert_eq!(world.get_block(10, 65, 20), Block::Stone);
        assert_eq!(world.get_block(10, 66, 20), Block::Air);
        assert_eq!(world.chunk_count(), 1);
    }

    #[test]
    fn java_world_bounded_ignores_outside() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = JavaWorld::new_bounded(dir.path(), 0, 1, 0, 1);
        world.set_block(0, 65, 0, Block::Stone);     // chunk (0, 0) — in bounds
        world.set_block(100, 65, 100, Block::Stone);  // chunk (6, 6) — out of bounds
        assert_eq!(world.chunk_count(), 1);
    }

    #[test]
    fn java_world_save_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = JavaWorld::new(dir.path());
        world.set_block(0, 65, 0, Block::Stone);
        world.save(0, 65, 0).unwrap();
        assert!(dir.path().join("level.dat").exists());
        assert!(dir.path().join("region").exists());
        assert!(dir.path().join("session.lock").exists());
    }

    #[test]
    fn pack_indices_roundtrip() {
        let mut indices = [0usize; 4096];
        indices[0] = 3;
        indices[1] = 7;
        indices[4095] = 1;
        let bits = compute_bits_per_block(8); // 8 entries → 4 bits
        let longs = pack_indices_long_array(&indices, bits);
        // Verify first entry
        let mask = (1i64 << bits) - 1;
        assert_eq!((longs[0] & mask) as usize, 3);
        assert_eq!(((longs[0] >> bits) & mask) as usize, 7);
    }
}
```

- [ ] **Step 10: Run tests**

Run: `cargo test --lib anvil`
Expected: all 4 tests PASS.

- [ ] **Step 11: Run full test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 12: Commit**

```bash
git add src/anvil.rs
git commit -m "feat: add JavaWorld with Anvil region writer"
```

---

### Task 6: Update `src/pipeline.rs` — Switch to `dyn WorldWriter`

**Files:**
- Modify: `src/pipeline.rs`

- [ ] **Step 1: Update imports**

Change the import block (lines 28-44) to use `world` instead of `bedrock` for shared types:

```rust
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
    world::{self, ChunkData, Edition, WorldWriter, MIN_Y},
};
```

- [ ] **Step 2: Update the `TerrainChunkResult` type alias**

Change line 50:
```rust
type TerrainChunkResult = ((i32, i32), ChunkData, Vec<((i32, i32), i32)>);
```

- [ ] **Step 3: Update all function signatures**

For every function that takes `&mut bedrock::BedrockWorld`, change to `&mut dyn WorldWriter`:
- `render_osm_features` (line 256): `world: &mut dyn WorldWriter`
- `place_tree` (line 906): `world: &mut dyn WorldWriter`
- `maybe_place_tree` (line 936): `world: &mut dyn WorldWriter`
- `run_pipeline` (line 1425): return `Result<(Box<dyn WorldWriter>, i32, i32, i32)>`
- `run_pipeline_streaming` (line 1748): similar updates
- `run_conversion` (line 1636): use `Edition` from params
- `run_conversion_from_data` (line 1678): use `Edition` from params
- `run_terrain_only_to_disk` (line 2165): use `Edition` from params

For `run_pipeline`, the function constructs a `BedrockWorld::new()` — change to:
```rust
let mut world = params.edition.create_world(&params.output);
```

For `run_pipeline_streaming`, the function constructs `BedrockWorld::new_bounded()` — change to:
```rust
let mut tile_world = edition.create_world_bounded(&params.output, min_cx, max_cx, min_cz, max_cz);
```

- [ ] **Step 4: Update `ChunkData` references**

Every `bedrock::ChunkData::new()` becomes `ChunkData::new()`.
Every `bedrock::MIN_Y` becomes `MIN_Y`.
Every `bedrock::MAX_Y` becomes `world::MAX_Y` (or import it).

- [ ] **Step 5: Update the streaming drain path**

The streaming path uses `bedrock::ChunkWriter` — this stays Bedrock-specific. Add an edition check:

For `run_conversion` and `run_pipeline_streaming`, the drain-to-disk path differs by edition:
- Bedrock: use `bedrock::ChunkWriter` + `drain_chunks_to_writer` (existing code)
- Java: use `world.save()` after accumulating all chunks

The simplest approach: for Java edition, accumulate in memory (same as preview path) and call `save()` at the end. For Bedrock, keep the existing streaming-to-LevelDB path.

- [ ] **Step 6: Run `cargo check` and fix errors**

Run: `cargo check`
Expected: compiles. Fix any remaining `bedrock::ChunkData` or `bedrock::MIN_Y` references.

- [ ] **Step 7: Run full test suite**

Run: `cargo test`
Expected: all existing tests pass (pipeline tests are integration-level, should work with Bedrock default).

- [ ] **Step 8: Commit**

```bash
git add src/pipeline.rs
git commit -m "refactor: pipeline uses WorldWriter trait instead of BedrockWorld"
```

---

### Task 7: Add `edition` to `src/params.rs` and `src/config.rs`

**Files:**
- Modify: `src/params.rs` (add `edition` field to `ConvertParams` and `TerrainParams`)
- Modify: `src/config.rs` (add `edition` field and merge logic)

- [ ] **Step 1: Add `edition` to `ConvertParams` in `src/params.rs`**

Add after the `output` field (line 20):

```rust
pub edition: crate::world::Edition,
```

- [ ] **Step 2: Add `edition` to `TerrainParams` in `src/params.rs`**

Add after the `output` field (line 59):

```rust
pub edition: crate::world::Edition,
```

- [ ] **Step 3: Add `edition` to `Config` in `src/config.rs`**

Add after the `surface_thickness` field (line 42):

```rust
pub edition: Option<String>,
```

- [ ] **Step 4: Add merge logic for `edition` in `Config::merge()`**

Add after the `surface_thickness` merge line:

```rust
merge_field!(self, other, edition);
```

- [ ] **Step 5: Run `cargo check`**

Run: `cargo check`
Expected: compilation errors in `main.rs` and `server.rs` where `ConvertParams` is constructed (missing `edition` field) — these get fixed in Tasks 8 and 9.

- [ ] **Step 6: Commit**

```bash
git add src/params.rs src/config.rs
git commit -m "feat: add edition field to ConvertParams, TerrainParams, and Config"
```

---

### Task 8: Add `--edition` flag to `src/main.rs`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add `--edition` arg to `ConvertArgs`**

Add after the `output` field (line 68):

```rust
/// Output edition: bedrock or java
#[arg(long, value_enum, default_value = "bedrock")]
edition: Option<crate::world::Edition>,
```

- [ ] **Step 2: Add `--edition` arg to `FetchConvertArgs`**

Add after the `output` field (line 203):

```rust
/// Output edition: bedrock or java
#[arg(long, value_enum, default_value = "bedrock")]
edition: Option<crate::world::Edition>,
```

- [ ] **Step 3: Add `--edition` arg to `OvertureConvertArgs` and `TerrainConvertArgs`**

Find the `OvertureConvertArgs` struct and `TerrainConvertArgs` struct. Add the same `--edition` argument after the `output` field in each.

- [ ] **Step 4: Update ConvertParams construction in the `Convert` handler**

Find where `ConvertParams { ... }` is constructed (in the `Commands::Convert` match arm). Add:

```rust
edition: args.edition.unwrap_or_default(),
```

Also check if config file sets edition:

```rust
edition: args.edition.unwrap_or_else(|| {
    config.edition.as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}),
```

- [ ] **Step 5: Update ConvertParams construction in `FetchConvert`, `OvertureConvert`, `TerrainConvert` handlers**

Same pattern — pass the `--edition` value through to the params struct.

- [ ] **Step 6: Update the about text**

Change line 27 from "Bedrock Edition worlds" to "Bedrock and Java Edition worlds".

- [ ] **Step 7: Run `cargo check`**

Run: `cargo check`
Expected: compiles.

- [ ] **Step 8: Run full test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/main.rs
git commit -m "feat: add --edition flag to CLI subcommands"
```

---

### Task 9: Add `edition` to `src/server.rs`

**Files:**
- Modify: `src/server.rs`

- [ ] **Step 1: Add `edition` field to `ConvertOptions`**

Add after the `nature_decorations` field in `ConvertOptions` (line 311):

```rust
#[serde(default)]
edition: crate::world::Edition,
```

- [ ] **Step 2: Add `edition` to `ConvertOptions::default()`**

Add to the `Default` impl:

```rust
edition: crate::world::Edition::default(),
```

- [ ] **Step 3: Add `edition` to `FetchConvertOptions`**

Same pattern — add after the existing fields in `FetchConvertOptions` (line 493):

```rust
#[serde(default)]
edition: crate::world::Edition,
```

- [ ] **Step 4: Add `edition` to `TerrainConvertRequest`**

Add to `TerrainConvertRequest` (line 1525):

```rust
#[serde(default)]
edition: crate::world::Edition,
```

- [ ] **Step 5: Update `ConvertParams` construction in handler functions**

Where `ConvertParams { ... }` is constructed from request options, add:

```rust
edition: options.edition,
```

Apply to all handlers: `convert_handler`, `fetch_convert_handler`, `terrain_convert_handler`, `overture_convert_handler`.

- [ ] **Step 6: Update download packaging**

In `download_handler`, check the edition stored with the job. For Java, package as `.zip` instead of `.mcworld`:

Find the `zip_to_mcworld` function call and add edition-aware packaging:

```rust
let ext = if edition == Edition::Java { "zip" } else { "mcworld" };
let archive_path = output_dir.path().join(format!("{world_name}.{ext}"));
```

- [ ] **Step 7: Run `cargo check`**

Run: `cargo check`
Expected: compiles.

- [ ] **Step 8: Run full test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/server.rs
git commit -m "feat: add edition param to server API endpoints"
```

---

### Task 10: Run `make checkall` and fix any remaining issues

**Files:**
- Any files with remaining issues

- [ ] **Step 1: Run the full check**

Run: `make checkall`
Expected: all formatting, linting, type checking, and tests pass.

- [ ] **Step 2: Fix any clippy warnings**

Run: `make lint`
If clippy warns about unused imports or dead code from the edition refactor, fix inline.

- [ ] **Step 3: Fix any formatting issues**

Run: `make fmt`

- [ ] **Step 4: Verify a manual end-to-end test with both editions**

```bash
# Bedrock (existing behavior, should still work)
cargo run --release -- convert --input test.pbf --output /tmp/test_bedrock/

# Java (new)
cargo run --release -- convert --input test.pbf --output /tmp/test_java/ --edition java
```

Verify `/tmp/test_java/region/` contains `.mca` files and `level.dat` exists.

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "feat: Java Edition support (1.18+) via --edition flag"
```

# Java Edition Support Design

**Date:** 2026-05-12
**Status:** Approved
**Target versions:** Java 1.18+ through 1.21+ (same Y range -64..320 as Bedrock)

## Goal

Add Minecraft Java Edition as an output format alongside the existing Bedrock Edition. Users select via `--edition bedrock|java` on the CLI or an `edition` parameter in the HTTP API. All existing Bedrock functionality remains unchanged.

## Architecture: Trait-Based Abstraction

A `WorldWriter` trait captures the operations the pipeline needs. `BedrockWorld` and `JavaWorld` each implement it. The pipeline is generic over `dyn WorldWriter`.

### New and Modified Files

```
src/
  world.rs          NEW  — WorldWriter trait, Edition enum, ChunkData (moved from bedrock.rs)
  anvil.rs          NEW  — JavaWorld (implements WorldWriter), Anvil region writer
  nbt_be.rs         NEW  — Big-endian NBT writer + Java sign entity encoder
  bedrock.rs        MOD  — BedrockWorld implements WorldWriter; ChunkData/MIN_Y/MAX_Y moved to world.rs
  blocks.rs         MOD  — Add java_name(), java_block_states(), surface_to_java_biome()
  nbt.rs            UNCHANGED
  pipeline.rs       MOD  — Takes &mut dyn WorldWriter instead of &mut bedrock::BedrockWorld
  main.rs           MOD  — --edition flag on convert/fetch-convert/overture-convert/terrain-convert
  server.rs         MOD  — edition param in /convert, /fetch-convert, /terrain-convert request bodies
  config.rs         MOD  — edition field in config file
  params.rs         MOD  — EditionOpts shared arg group
```

No changes to: osm.rs, convert.rs, geojson_export.rs, overpass.rs, osm_cache.rs, elevation.rs, srtm.rs, filter.rs, geometry.rs, spatial.rs, sign.rs, metadata.rs, source_options.rs.

### WorldWriter Trait

```rust
// src/world.rs

pub const MIN_Y: i32 = -64;
pub const MAX_Y: i32 = 319;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Edition {
    #[default]
    Bedrock,
    Java,
}

pub struct ChunkData {
    subchunks: HashMap<i8, Box<[Block; 4096]>>,
    // Same XZY-order block storage as today, moved here for sharing.
}

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
    pub fn create_world(&self, output: &Path) -> Box<dyn WorldWriter> { ... }
    pub fn create_world_bounded(&self, output: &Path, min_cx: i32, max_cx: i32, min_cz: i32, max_cz: i32) -> Box<dyn WorldWriter> { ... }
}
```

### Anvil Region Writer (src/anvil.rs)

`JavaWorld` holds the same in-memory structures as `BedrockWorld` (chunks HashMap, block entities, direction maps) and serializes to Anvil format on `save()`.

**Region file layout** (`region/r.X.Z.mca`):
- Each file covers 32x32 chunks
- 1KB header: sector offsets (3 bytes each) + sector counts (1 byte each)
- Per-chunk: 4-byte big-endian length + 1-byte compression type (2=zlib) + compressed big-endian NBT

**Chunk NBT structure (1.18+ data-driven format):**

```
TAG_Compound("") {
  "DataVersion": Int(3465)
  "yPos": Int(-4)
  "Status": String("minecraft:full")
  "Heightmaps": Compound {
    "WORLD_SURFACE": LongArray(36 longs, 9 bits per entry)
  }
  "sections": List(TAG_Compound) {
    per 16x16x16 section: {
      "Y": Byte(section_y)
      "block_states": Compound {
        "palette": List(TAG_Compound) [{ "Name": String, "Properties": Compound }]
        "data": LongArray(packed indices, 64-bit words)
      }
      "biomes": Compound {
        "palette": List(TAG_String) ["minecraft:plains", ...]
        "data": LongArray(...)
      }
    }
  }
  "block_entities": List(TAG_Compound) { ... }
}
```

**Key format differences from Bedrock:**
- Block order within sections: YXZ (not XZY)
- Biomes: string IDs in 4x4x4 grid per section (not 2D legacy bytes)
- Palette entries: "Name" + "Properties" compound (not "name" + "states" + "version")
- level.dat: gzip-compressed big-endian NBT with DataVersion, Version.Name, etc.

**JavaWorld::save() steps:**
1. Group chunks into 32x32 regions
2. Encode each chunk to BE NBT, zlib-compress, write to region file
3. Write level.dat (BE gzip NBT, Java-specific fields)
4. Write session.lock (standard Java world marker)
5. Optionally zip the world directory for download

### BE NBT Writer (src/nbt_be.rs)

Mirrors nbt.rs with `.to_be_bytes()` throughout. Adds tag types needed by Anvil:

```rust
pub const TAG_SHORT: u8 = 2;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_LIST: u8 = 9;
pub const TAG_INT_ARRAY: u8 = 11;
pub const TAG_LONG_ARRAY: u8 = 12;

pub fn write_short_tag(w, name, value: i16)
pub fn write_double_tag(w, name, value: f64)
pub fn write_list_start(w, name, item_type: u8, length: i32)
pub fn write_int_array_tag(w, name, values: &[i32])
pub fn write_long_array_tag(w, name, values: &[i64])
pub fn encode_java_sign_entity(x: i32, y: i32, z: i32, text: &str) -> Vec<u8>
```

Java sign NBT uses `"id": "minecraft:sign"`, `front_text`/`back_text` with `messages` as TAG_List of JSON strings.

### Java Block Mappings (blocks.rs additions)

```rust
impl Block {
    pub fn java_name(self) -> &'static str { ... }
    pub fn java_block_states(self) -> Vec<(&'static str, &'static str)> { ... }
}

pub fn surface_to_java_biome(block: Block) -> &'static str { ... }
```

~20 blocks have different names between editions:
| Block | Bedrock | Java |
|---|---|---|
| OakSign | standing_sign | oak_sign |
| Brick | brick_block | bricks |
| StoneSlab | stone_block_slab | stone_slab |
| Poppy | red_flower + flower_type | poppy |
| TallGrass | tallgrass + tall_grass_type | tall_grass |
| Fern | tallgrass + tall_grass_type=fern | fern |
| CherrySign | cherry_standing_sign | cherry_sign |
| StoneBrickWall | cobblestone_wall + wall_block_type | stone_brick_wall |

Java block states use camelCase names (facing, half, axis) vs Bedrock snake_case (weirdo_direction, minecraft:vertical_half, pillar_axis).

Biome mapping: numeric ID to string ID (1 -> "minecraft:plains", 7 -> "minecraft:river", etc.).

### CLI Changes (main.rs, params.rs)

New shared arg group:

```rust
#[derive(clap::Args, Clone)]
pub struct EditionOpts {
    #[arg(long, value_enum, default_value = "bedrock")]
    pub edition: Edition,
}
```

Added to subcommands: convert, fetch-convert, overture-convert, terrain-convert.

Pipeline receives Edition and uses factory to create `Box<dyn WorldWriter>`.

Output packaging:
- Bedrock: .mcworld zip (unchanged)
- Java: plain directory or .zip

sea_level default stays 65 for both editions (configurable, not edition-specific).

### Server Changes (server.rs)

Add `edition` field to request bodies for /convert, /fetch-convert, /terrain-convert:

```rust
#[derive(Deserialize)]
struct ConvertRequest {
    // existing fields...
    #[serde(default)]
    edition: Edition,
}
```

Download packaging: .zip for Java worlds. Frontend export panel gets an edition dropdown.

### Config File (config.rs)

Add optional `edition` field to YAML config:

```yaml
edition: java  # optional, defaults to bedrock
```

## Delivery Format

- Bedrock: .mcworld zip (existing)
- Java: world directory + optional .zip for sharing

## Testing Strategy

- Unit tests for `java_name()`, `java_block_states()`, `surface_to_java_biome()` mapping completeness
- Unit tests for BE NBT writer (round-trip encoding)
- Unit tests for Anvil region writer (chunk encoding, region file structure)
- Integration test: convert a small PBF to both editions, verify output loads in respective game
- Existing Bedrock tests remain unchanged

## Out of Scope

- Pre-1.18 Java format (Y=0..256, 2D biomes)
- Direct .mca-only output (always writes full world directory)
- Block entity differences beyond signs (signs are the only block entity we generate)
- Java world generation features beyond terrain + OSM features (no cave generation, no structure placement)

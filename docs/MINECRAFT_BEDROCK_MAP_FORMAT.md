# Minecraft Bedrock Edition Map Format

Comprehensive reference for the Minecraft Bedrock Edition on-disk world format, covering
LevelDB structure, chunk encoding, block states, biomes, entities, and level.dat fields.

## Table of Contents

- [Overview](#overview)
- [LevelDB Database](#leveldb-database)
  - [Mojang's Modified LevelDB](#mojangs-modified-leveldb)
    - [Compression IDs](#compression-ids)
  - [World Directory Structure](#world-directory-structure)
- [Chunk Key Format](#chunk-key-format)
  - [Chunk Tag Byte Reference](#chunk-tag-byte-reference)
  - [Special Non-Chunk Keys](#special-non-chunk-keys)
- [Modern Actor Storage](#modern-actor-storage)
- [SubChunk Format](#subchunk-format)
  - [SubChunk Version Bytes](#subchunk-version-bytes)
  - [Block Storage Layer Format](#block-storage-layer-format)
  - [Block Index Ordering](#block-index-ordering)
  - [SubChunk Y Position](#subchunk-y-position)
- [Block State Encoding](#block-state-encoding)
  - [PersistentID Format](#persistentid-format)
  - [Block Namespace Differences from Java](#block-namespace-differences-from-java)
- [Biome Data](#biome-data)
- [Entities and Block Entities](#entities-and-block-entities)
- [level.dat Structure](#leveldat-structure)
  - [Key Fields](#key-fields)
  - [Flat World Generation JSON](#flat-world-generation-json)
- [World Generation Differences from Java](#world-generation-differences-from-java)
- [NBT Format Differences](#nbt-format-differences)
- [osm-to-bedrock Implementation Details](#osm-to-bedrock-implementation-details)
  - [What the Writer Produces](#what-the-writer-produces)
  - [NBT Subset](#nbt-subset)
- [Further Reading](#further-reading)

## Overview

Minecraft Bedrock Edition stores world data in a modified version of Google's LevelDB, using
Zlib or raw Deflate compression (see [Compression IDs](#compression-ids)). This differs
fundamentally from Java Edition, which uses region files (`.mca`) containing NBT data in
big-endian format.

This document covers the on-disk format introduced in Bedrock Edition 1.2.13 (the paletted
subchunk era). For pre-1.2.13 legacy formats, see [Bedrock Edition level format/History](https://minecraft.wiki/w/Bedrock_Edition_level_format/History).

Key differences from Java Edition:

| Aspect | Bedrock | Java |
|--------|---------|------|
| Storage engine | LevelDB (Mojang fork, Zlib/Deflate) | Region files (.mca) |
| NBT byte order | Little-endian | Big-endian |
| Block ordering | XZY (X fastest) | YZX (Y fastest) |
| Block state format | NBT compound with `name`+`states` | Compound with `Name`+`Properties` |
| Biome format (modern) | Palettized per-subchunk | Palettized per-section |
| Entity storage (modern) | Individual per-actor LevelDB keys | Per-chunk NBT list |

## LevelDB Database

### Mojang's Modified LevelDB

Bedrock uses a fork of Google LevelDB with two key modifications:

- **Custom compression** added for chunk data (replacing Snappy)
- **Windows support** added

Source: https://github.com/Mojang/leveldb-mcpe

The database lives in the `db/` subdirectory of a world folder. The path passed to the LevelDB
API is the directory path, not any file within it.

#### Compression IDs

Bedrock's LevelDB fork prefixes each data block with a 1-byte compressor ID. Three compressors
are registered:

| ID | Algorithm | Notes |
|----|-----------|-------|
| 0 | None (raw) | Uncompressed data |
| 2 | Zlib | Used by older worlds |
| 4 | Raw Deflate | Default for modern worlds; used by this project for all writes |

When reading, the library selects the decompressor based on the stored ID byte. When writing,
this project uses compressor ID 4 (raw deflate) for all new data.

### World Directory Structure

```
<world-folder>/
├── db/                          # LevelDB database
│   ├── 000005.ldb               # SST table (block data, compacted)
│   ├── 000006.log               # Write-ahead log
│   ├── CURRENT                  # Points to active MANIFEST
│   ├── LOCK                     # Exclusive lock file (must be closed before editing)
│   └── MANIFEST-000004          # LevelDB manifest
├── level.dat                    # World metadata (little-endian NBT, see below)
├── level.dat_old                # Backup of previous level.dat
├── levelname.txt                # World display name (plain text)
├── world_icon.jpeg              # World thumbnail (800×450 JPEG)
├── behavior_packs/              # World-scoped behavior packs
├── resource_packs/              # World-scoped resource packs
├── world_behavior_packs.json    # List of active behavior packs
└── world_resource_packs.json    # List of active resource packs
```

## Chunk Key Format

All chunk data is stored as binary key-value pairs in LevelDB. Chunk keys are constructed from:

1. **X coordinate** — signed 32-bit little-endian integer (chunk X = block X ÷ 16)
2. **Z coordinate** — signed 32-bit little-endian integer (chunk Z = block Z ÷ 16)
3. **Dimension** _(optional)_ — 32-bit little-endian integer: `1` = Nether, `2` = End; omitted for Overworld
4. **Tag byte** — one byte identifying the data type (see table below)
5. **SubChunk index** _(SubChunkPrefix only)_ — one byte, 0–15 (or signed for extended height)

Resulting key lengths: 9, 10, 13, or 14 bytes.

### Chunk Tag Byte Reference

| Dec | Hex | Name | Structure | Description |
|-----|-----|------|-----------|-------------|
| 43 | 2B | Data3D | 256×2 heightmap + biome palettes | Modern biome data (v1.18+). 25 palettes; biome IDs as int32 |
| 44 | 2C | Version | 1 byte | Chunk storage format version |
| 45 | 2D | Data2D | 256×2 heightmap + 256×1 biomes | 8-bit biome IDs. Not written since v1.18.0 |
| 46 | 2E | Data2DLegacy | 256×2 heightmap + 256×4 biomes | Biome ID + RGB colour. Not written since v1.0.0 |
| 47 | 2F | SubChunkPrefix | version byte + layer data | Terrain for a 16×16×16 subchunk |
| 48 | 30 | LegacyTerrain | IDs + meta + lighting | XZY order. Not written since v1.0.0 |
| 49 | 31 | BlockEntity | Concatenated NBT roots | Block entity (tile entity) data — little-endian NBT |
| 50 | 32 | Entity | Concatenated NBT roots | Entity data — little-endian NBT (pre-1.18.30 only) |
| 51 | 33 | PendingTicks | NBT compound | Pending block tick data |
| 52 | 34 | LegacyBlockExtraData | Count + entries | Deprecated stacked block data |
| 53 | 35 | BiomeState | — | Biome state (additional biome data) |
| 54 | 36 | FinalizedState | 4 bytes | 32-bit little-endian integer |
| 56 | 38 | BorderBlocks | — | Education Edition feature |
| 57 | 39 | HardcodedSpawners | binary | Bounding boxes for structure spawns |
| 58 | 3A | RandomTicks | NBT compound | Random tick data |
| 59 | 3B | Checksums | — | xxHash checksums. Not written since v1.18.0 |
| 64 | 40 | BlendingData | — | Caves & Cliffs terrain blending data |
| 118 | 76 | LegacyVersion | 1 byte | Moved to tag 44 in v1.16.100 |

In C++ enum form (from Microsoft documentation):

```cpp
enum class LevelChunkTag : char {
    Data3D              = 43,
    Version             = 44,
    Data2D              = 45,
    Data2DLegacy        = 46,
    SubChunkPrefix      = 47,
    LegacyTerrain       = 48,
    BlockEntity         = 49,
    Entity              = 50,
    PendingTicks        = 51,
    LegacyBlockExtraData = 52,
    BiomeState          = 53,
    FinalizedState      = 54,
    BorderBlocks        = 56,
    HardcodedSpawners   = 57,
    RandomTicks         = 58,
    CheckSums           = 59,
};
```

### Special Non-Chunk Keys

| Key pattern | Description |
|-------------|-------------|
| `~local_player` | Local player entity data (single NBT root compound) |
| `player_<clientid>` | Remote player, e.g. `player_-12345678` |
| `game_flatworldlayers` | Flat world config as ASCII JSON, e.g. `[7,3,3,2]` |
| `actorprefix<uuid>` | Individual actor data (modern storage, post-1.18.30) |
| `digp<chunk_key>` | Chunk-to-actor digest mapping (modern storage) |
| `map_<id>` | In-game map data |
| `portals` | Nether portal linkage table |
| `structuretemplate` | Saved structure template data |
| `tickingarea` | Always-loaded area definitions |
| `DynamicProperties` | Script API dynamic property storage |
| `scoreboard` | Scoreboard objective and player data |
| `VILLAGE_<DIM>_<uuid>_DWELLERS` | Village mob references |
| `VILLAGE_<DIM>_<uuid>_INFO` | Village bounding box |
| `VILLAGE_<DIM>_<uuid>_POI` | Villager↔job/bed/bell mapping |

## Modern Actor Storage

Before version 1.18.30, all entities in a chunk were stored as a single blob under the
`Entity` (tag 50) key. Writing one changed entity required re-serialising all entities in
that chunk.

Since 1.18.30, each actor (entity) has its own unique LevelDB key using `actorprefix` as the
key space prefix. Chunks maintain a *digest* under `digp` prefix keys that maps chunk positions
to the set of actor keys they contain.

```
actorprefix<unique_actor_id>  →  NBT compound for that actor
digp<chunk_x><chunk_z><dim>   →  list of actor_id bytes in that chunk
```

This enables single-actor save operations and makes entity transfer between chunks efficient.

## SubChunk Format

The SubChunkPrefix (tag 47) stores terrain data for a 16×16×16 block volume.

### SubChunk Version Bytes

The first byte of each SubChunkPrefix value is a version indicating internal layout:

| Version byte | Format |
|-------------|--------|
| 0,2,3,4,5,6,7 | **Legacy**: `[version][4096 block IDs][2048 data nibbles]` |
| 1 | **Palettized (beta)**: single block storage layer |
| 8 | **Current format**: `[version][num_storages][storage_1]...[storage_N]` |
| 9 | **v9**: same as 8 with updated biome handling |

Version 8/9 is the standard current format. Multiple storage layers allow waterlogging:
layer 0 = solid blocks, layer 1 = fluid (water or air).

### Block Storage Layer Format

Each storage layer within a version 8/9 subchunk:

```
[header_byte][block_index_words...][palette_size_int32][palette_entry_0]...[palette_entry_N]
```

**Header byte** (bits interpreted as):
- Bit 0 (LSB): `1` = network/runtime IDs, `0` = persistence/disk PersistentIDs
- Bits 1–7: bits-per-block value selecting the palette density

**Bits-per-block density table:**

| Header value (÷2) | Blocks per word | Padding per word |
|-------------------|-----------------|-----------------|
| 1 | 32 | none |
| 2 | 16 | none |
| 3 | 10 | 2 bits |
| 4 | 8 | none |
| 5 | 6 | 2 bits |
| 6 | 5 | 2 bits |
| 8 | 4 | none |
| 16 | 2 | none |

A "word" is a 4-byte (32-bit) little-endian unsigned integer. Total words = `ceil(4096 / blocksPerWord)`. For densities where 32 is not evenly divisible by the bits-per-block value (3, 5, 6), the last word in each group contains unused padding bits that are zeroed.

**Palette entries (disk format)** — one NBT compound per entry:

```nbt
{
  "name": TAG_String "minecraft:stone",
  "states": TAG_Compound {
    "stone_type": TAG_String "andesite"
  },
  "version": TAG_Int 18105860
}
```

The `version` field encodes `(major << 24) | (minor << 16) | (patch << 8) | revision`.

**Palette entries (network format)** — varint RuntimeIDs. Never write RuntimeIDs to disk; they
are session-ephemeral and may change between game versions.

### Block Index Ordering

Blocks within a subchunk are indexed in **XZY order** (X changes fastest, Y slowest):

```python
# Block index within the 16×16×16 subchunk at local position (lx, lz, ly)
index = (lx << 8) | (lz << 4) | ly

# Reverse: decode index to (lx, lz, ly)
lx = (index >> 8) & 0xF
lz = (index >> 4) & 0xF
ly = index & 0xF
```

This XZY ordering is opposite to Java Edition's YZX ordering.

### SubChunk Y Position

The subchunk index byte in the LevelDB key maps to world Y range:

| Index (signed byte) | Y range |
|--------------------|---------|
| -4 | Y -64 – -49 |
| -3 | Y -48 – -33 |
| -2 | Y -32 – -17 |
| -1 | Y -16 – -1 |
| 0 | Y 0 – 15 |
| 1 | Y 16 – 31 |
| ... | ... |
| 15 | Y 240 – 255 |
| 19 | Y 304 – 319 |

The 1.18 Caves & Cliffs update extended the range to Y -64..319 (384 blocks total), requiring
subchunk indices -4..19 for a full Overworld column.

## Block State Encoding

### PersistentID Format

All block states written to disk use the **PersistentID** format — an NBT compound that uniquely
identifies a block type and all its properties:

```nbt
{
  "name": "minecraft:oak_log",
  "states": {
    "pillar_axis": "y"
  },
  "version": 18105860
}
```

This differs from Java's flat properties string (`minecraft:oak_log[axis=y]`). Bedrock
explicitly separates the block name from its state properties in separate NBT fields.

### Block Namespace Differences from Java

| Java block state | Bedrock block state |
|-----------------|---------------------|
| `minecraft:grass_block` | `minecraft:grass_block` (matches post-parity) |
| `minecraft:oak_log[axis=y]` | name:`minecraft:oak_log` states:`{"pillar_axis":"y"}` |
| `minecraft:smooth_stone` | `minecraft:smooth_stone` (matches) |
| `minecraft:stone_bricks` | `minecraft:stonebrick` (legacy Bedrock name) |
| `minecraft:mossy_cobblestone` | `minecraft:mossy_cobblestone` (matches) |
| `minecraft:stone[type=granite]` | name:`minecraft:stone` states:`{"stone_type":"granite"}` |

Many names were unified in the 1.13 Java / ~1.16 Bedrock parity push, but older worlds or
less common blocks may still differ. Use PyMCTranslate for reliable cross-edition translation.

## Biome Data

### Modern Format (v1.18+, Data3D key tag 43)

The Data3D record contains:

1. **256×2 heightmap** — one 16-bit little-endian integer per 1×1 column (256 values)
2. **25 biome palettes** — one per subchunk column (one extra for full height coverage)
   - Each palette uses the same header+words+entries structure as block palettes
   - Biome IDs stored as 32-bit integers
   - 4×4×4 biome resolution within each subchunk (64 values per palette)

### Legacy Format (Data2D, tag 45, pre-1.18)

256×2 heightmap (one 16-bit LE integer per column, 512 bytes total) followed by one biome ID
byte per 1×1 column (256 bytes, for 768 bytes total). Not written by vanilla Bedrock since
v1.18.0, but this project uses the Data2D format for compatibility (see
[Implementation Details](#osm-to-bedrock-implementation-details)).

## Entities and Block Entities

All entity and block entity data uses **little-endian NBT**, which is the reverse of Java
Edition's big-endian NBT. Any NBT parsing library must be configured for little-endian mode
when reading Bedrock data.

### Block Entity (Tile Entity) Structure

Stored under tag 49 (`BlockEntity`) as concatenated NBT root compounds:

```nbt
{
  "id": "Chest",       // block entity type identifier
  "x": 10,            // absolute world X (int32)
  "y": 64,            // absolute world Y (int32)
  "z": -5,            // absolute world Z (int32)
  "isMovable": 1b,    // whether the block entity can be moved by pistons
  "Items": [...]      // type-specific fields; chest contains an Items list
}
```

Common block entity IDs include: `Chest`, `Furnace`, `Sign`, `Mob`, `FlowerPot`,
`EnchantTable`, `Beacon`, `Skull`, `CommandBlock`, `Spawner`, `Banner`.

**Sign block entity (modern format, post-1.20):** Sign data uses `FrontText` and `BackText`
sub-compounds rather than flat text fields:

```nbt
{
  "id": "Sign",
  "x": 10, "y": 64, "z": -5,
  "isMovable": 1b,
  "FrontText": {
    "Text": "Line1\nLine2",
    "SignTextColor": -16777216,
    "IgnoreLighting": 0b,
    "HideGlowOutline": 0b,
    "PersistFormatting": 1b,
    "TextOwner": ""
  },
  "BackText": {
    "Text": "",
    "SignTextColor": -16777216,
    "IgnoreLighting": 0b,
    "HideGlowOutline": 0b,
    "PersistFormatting": 1b,
    "TextOwner": ""
  },
  "IsWaxed": 0b
}
```

> **Note:** `SignTextColor` value `-16777216` corresponds to `0xFF000000` (opaque black).
> The `IsWaxed` field controls whether the sign text can be edited after placement.

### Entity Structure

- **Pre-1.18.30**: Stored as tag 50 blob per chunk, concatenated NBT root compounds
- **Post-1.18.30**: Individual keys with `actorprefix` prefix in the key space

Common entity fields:

```nbt
{
  "id": "minecraft:creeper",
  "Pos": [0.5f, 64.0f, 0.5f],    // 3-float list
  "Rotation": [0.0f, 0.0f],      // [yaw, pitch] floats
  "Motion": [0.0f, 0.0f, 0.0f],
  "Health": 20f,
  "Attributes": [...]
}
```

## level.dat Structure

The `level.dat` file stores world-level metadata as **uncompressed little-endian NBT**. The
file begins with an 8-byte header:

```
Bytes 0–3:  StorageVersion as little-endian int32 (currently 10)
Bytes 4–7:  Length of remaining NBT data as little-endian int32
Bytes 8+:   NBT compound (the actual world data)
```

### Key Fields

| Field | Type | Description |
|-------|------|-------------|
| `LevelName` | String | World display name |
| `RandomSeed` | Long | World generation seed (Bedrock-specific algorithm) |
| `LastPlayed` | Long | Unix timestamp of last play session |
| `Time` | Long | Current world tick (20 ticks/sec, 24000 ticks/game-day) |
| `StorageVersion` | Int | Bedrock storage version, currently `10` |
| `NetworkVersion` | Int | Protocol version of the last client that opened the world |
| `Generator` | Int | `0`=Old (finite), `1`=Infinite, `2`=Flat, `5`=Void |
| `GameType` | Int | `0`=Survival, `1`=Creative, `2`=Adventure, `6`=Spectator |
| `Difficulty` | Int | `0`=Peaceful, `1`=Easy, `2`=Normal, `3`=Hard |
| `SpawnX` / `SpawnY` / `SpawnZ` | Int | World spawn coordinates (defaults: 0, 64, 0) |
| `FlatWorldLayers` | String | JSON controlling flat world generation (see below) |
| `lastOpenedWithVersion` | List[Int] | 5 integers: last version that opened this world |
| `MinimumCompatibleClientVersion` | List[Int] | 5 integers: minimum compatible client version |
| `educationFeaturesEnabled` | Byte | Education Edition chemistry features enabled |
| `eduOffer` | Int | `1` = Education Edition world (will NOT open in regular Bedrock) |
| `experiments` | Compound | Active experimental gameplay flags |
| `abilities` | Compound | Default player permission flags |
| `cheatsEnabled` | Byte | Whether cheats are enabled |
| `hasBeenLoadedInCreative` | Byte | Achievement lock: `1` if achievements permanently disabled |
| `IsHardcore` | Byte | Hardcore mode enabled |
| `Dimension` | Int | Current dimension player is in: `0`=Overworld, `1`=Nether, `2`=End |
| `currentTick` | Long | Total game ticks elapsed |
| `daylightCycle` | Int | Day/night cycle state |
| `rainLevel` / `lightningLevel` | Float | Current storm intensity |

### Flat World Generation JSON

The `FlatWorldLayers` string contains JSON controlling flat world generation:

```json
{
  "biome_id": 1,
  "block_layers": [
    {"block_name": "minecraft:bedrock", "count": 1},
    {"block_name": "minecraft:dirt",    "count": 2},
    {"block_name": "minecraft:grass_block", "count": 1}
  ],
  "encoding_version": 6,
  "preset_id": "ClassicFlat",
  "structure_options": null,
  "world_version": "version.post_1_18"
}
```

`block_layers` must contain at least 2 valid block entries; fewer results in a void world.
Layers are placed bottom-to-top.

## World Generation Differences from Java

### Seed Compatibility

Bedrock and Java use entirely different terrain generation algorithms. The same numeric seed
produces a completely different world in each edition. There is no cross-edition seed parity.

### Build Height

| Bedrock Version | Y range | Total height |
|----------------|---------|--------------|
| pre-1.18 | 0–255 | 256 blocks |
| 1.18+ | -64–319 | 384 blocks |

### World Type Generator Values

| `Generator` value | World type | Notes |
|------------------|------------|-------|
| 0 | Old (finite) | Fixed 256×256 map, no borders beyond |
| 1 | Infinite | Standard infinite world |
| 2 | Flat (Superflat) | Uses `FlatWorldLayers` JSON |
| 5 | Void | Empty void world |

### Structure Generation Differences

Bedrock structures use the same seed-based system as Java but with different placement logic:

- Strongholds: different spiral distance from spawn
- Nether fortresses: different orientation and frequency
- Village layout: slightly different building compositions
- Woodland mansions: different generation algorithm

### Caves & Cliffs World Blending (v1.18)

Bedrock v1.18 uses chunk blending data (`BlendingData` tag 64) to smoothly transition
old-height chunks (0–255) to new-height chunks (-64–319) when opening pre-1.18 worlds.
The `Data3D` (tag 43) replaced `Data2D` (tag 45) for biome storage in this update.

## NBT Format Differences

| Aspect | Bedrock | Java |
|--------|---------|------|
| Byte order | Little-endian | Big-endian |
| Tag type codes | Same | Same |
| Root compound | Unnamed (empty string name) | Unnamed (empty string name) |
| Level.dat header | 8-byte custom header + NBT | GZip compressed NBT |

When using any NBT library with Bedrock data, always enable little-endian mode. Amulet-NBT,
`amulet-core`, and `bedrock-parser` handle this automatically.

## osm-to-bedrock Implementation Details

This section documents how this project's Bedrock world writer (`bedrock.rs`, `nbt.rs`) maps
onto the format described above. The implementation targets a minimal but functional subset of
the full Bedrock format.

### What the Writer Produces

**Per-chunk LevelDB entries:**

| Tag | Hex | Description |
|-----|-----|-------------|
| Version | 0x2C | Single byte: `40` (chunk format version) |
| FinalizedState | 0x36 | 4-byte LE int32: `2` (terrain fully generated) |
| SubChunkPrefix | 0x2F | Version-8 subchunk with single storage layer |
| Data2D | 0x2D | Legacy heightmap (256 x i16 LE) + biome array (256 bytes) |
| BlockEntity | 0x31 | Concatenated sign NBT blobs (when signs are present) |

> **Note:** The writer uses the legacy Data2D format (tag 0x2D) for biome data rather than
> the modern palettized Data3D (tag 0x2B). This is intentional for maximum compatibility with
> older Bedrock clients. The biome byte for each column is derived from the top surface block
> (e.g., water columns produce river biome ID 7, grass produces plains ID 1).

**SubChunk encoding choices:**

- Version byte: `8` (current paletted format, single storage layer)
- Storage count: always `1` (no waterlogging layer)
- Header bit 0: `0` (persistence/disk format with NBT palettes, not runtime IDs)
- Bits-per-block: smallest valid value from `[1, 2, 3, 4, 5, 6, 8, 16]` that fits the palette
- Palette version field: `18105860` (encodes version 1.20.64.4)

**level.dat fields written:**

| Field | Value | Notes |
|-------|-------|-------|
| `StorageVersion` | `10` | Current Bedrock storage version |
| `NetworkVersion` | `594` | Protocol version |
| `Generator` | `2` | Flat world |
| `GameType` | `1` | Creative mode |
| `Difficulty` | `0` | Peaceful (no hostile mobs) |
| `SpawnX/Y/Z` | configurable | Spawn coordinates passed at save time |
| `Time` | `6000` | Morning (avoids midnight spawn) |
| `commandsEnabled` | `1` | Commands enabled |
| `hasBeenLoadedInCreative` | `1` | Marks world as creative-loaded |
| `showcoordinates` | `1` | Shows coordinate HUD |
| `dodaylightcycle` | `0` | Freezes time of day |
| `doweathercycle` | `0` | No weather changes |
| `domobspawning` | `0` | No mob spawning |
| `rainLevel` / `lightningLevel` | `0.0` | Clear weather |

The level.dat header uses the standard 8-byte prefix: `[StorageVersion: u32 LE][NBT length: u32 LE]`.

> **Note:** The writer does not emit `FlatWorldLayers` JSON, entity data (tag 50), modern
> actor storage (`actorprefix`/`digp`), Data3D biome palettes, or the `lastOpenedWithVersion`
> version list. These are not needed for the generated flat OSM worlds to load correctly.

### NBT Subset

The NBT writer (`nbt.rs`) implements only the tag types needed for SubChunk palettes,
level.dat, and sign block entities:

| Tag ID | Type | Used for |
|--------|------|----------|
| 0 | TAG_End | Closing compounds |
| 1 | TAG_Byte | Block states, game rules, boolean flags |
| 3 | TAG_Int | Coordinates, versions, directions |
| 4 | TAG_Long | Time, timestamps |
| 5 | TAG_Float | Rain/lightning levels |
| 8 | TAG_String | Block names, level name, sign text |
| 10 | TAG_Compound | Root compounds, block state groups |

TAG_Short (2), TAG_Double (6), TAG_Byte_Array (7), TAG_List (9), TAG_Int_Array (11), and
TAG_Long_Array (12) are not implemented since they are not needed for the current feature set.

## Further Reading

- [Bedrock Edition level format — Minecraft Wiki](https://minecraft.wiki/w/Bedrock_Edition_level_format)
- [SubChunk & Block State format — Tomcc/Mojang gist](https://gist.github.com/Tomcc/a96af509e275b1af483b25c543cfbf37)
- [Actor Storage in Bedrock — Microsoft Learn](https://learn.microsoft.com/en-us/minecraft/creator/documents/actorstorage?view=minecraft-bedrock-stable)
- [com.mojang directory structure — Minecraft Wiki](https://minecraft.wiki/w/Com.mojang)
- [Bedrock Edition level format/Block entity format](https://minecraft.wiki/w/Bedrock_Edition_level_format/Block_entity_format)
- [Bedrock Edition level format/Entity format](https://minecraft.wiki/w/Bedrock_Edition_level_format/Entity_format)
- [Mojang LevelDB fork source](https://github.com/Mojang/leveldb-mcpe)

## Related Documentation

- [Minecraft Bedrock Tools and Import](./MINECRAFT_BEDROCK_TOOLS_AND_IMPORT.md)

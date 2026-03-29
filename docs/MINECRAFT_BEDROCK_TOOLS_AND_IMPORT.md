# Minecraft Bedrock Edition — Tools, Import, and Python Examples

Practical guide for working with Bedrock Edition worlds: the `.mcworld` file format, world
import procedures on each platform, the Python tool ecosystem, and complete Amulet-Core
code examples for world generation and modification.

## Table of Contents

- [Overview](#overview)
- [The .mcworld File Format](#the-mcworld-file-format)
  - [Related Bedrock File Extensions](#related-bedrock-file-extensions)
  - [mcworld vs mctemplate](#mcworld-vs-mctemplate)
- [World Save Folder Locations](#world-save-folder-locations)
  - [Windows](#windows)
  - [Android](#android)
  - [iOS / iPadOS](#ios--ipados)
  - [Xbox](#xbox)
  - [Minecraft Education Edition](#minecraft-education-edition)
- [Importing .mcworld Files](#importing-mcworld-files)
  - [Windows 10 / 11](#windows-10--11)
  - [Android](#android-1)
  - [iOS / iPadOS](#ios--ipados-1)
  - [Xbox](#xbox-1)
  - [Converting ZIP to mcworld](#converting-zip-to-mcworld)
- [Python Libraries](#python-libraries)
  - [Amulet-Core](#amulet-core)
  - [Amulet-NBT](#amulet-nbt)
  - [PyMCTranslate](#pymctranslate)
  - [Amulet Map Editor](#amulet-map-editor)
  - [GUI Tools](#gui-tools)
- [Python Code Examples](#python-code-examples)
  - [Setup](#setup)
  - [Open an Existing Bedrock World](#open-an-existing-bedrock-world)
  - [Read a Block at a Position](#read-a-block-at-a-position)
  - [Iterate Chunks in a Region](#iterate-chunks-in-a-region)
  - [Modify Blocks](#modify-blocks)
  - [Create a Flat World from Scratch](#create-a-flat-world-from-scratch)
  - [Add Structures Programmatically](#add-structures-programmatically)
  - [Place a Block Entity (Chest)](#place-a-block-entity-chest)
  - [Export to .mcworld](#export-to-mcworld)
- [Version Compatibility](#version-compatibility)
- [Limitations and Gotchas](#limitations-and-gotchas)
- [Further Reading](#further-reading)

## Overview

Bedrock Edition worlds are stored in a LevelDB-based format. Three distinct operations cover
most use cases:

1. **Import** — bring a `.mcworld` file into the Minecraft client on any platform
2. **Edit** — programmatically read and modify world data with Amulet-Core (Python)
3. **Export** — package a world back into a `.mcworld` file for distribution or import

## The .mcworld File Format

A `.mcworld` file is a **ZIP archive** with the `.mcworld` extension. It contains a complete
world directory:

```text
my-world.mcworld  (ZIP archive — rename to .zip to extract)
├── level.dat                    # World metadata (little-endian NBT)
├── levelname.txt                # World display name (optional)
├── world_icon.jpeg              # Thumbnail (800×450 JPEG, optional)
├── db/                          # LevelDB database
│   ├── 000005.ldb
│   ├── 000006.log
│   ├── CURRENT
│   ├── LOCK
│   └── MANIFEST-000004
├── behavior_packs/              # Optional
├── resource_packs/              # Optional
├── world_behavior_packs.json    # Optional
└── world_resource_packs.json    # Optional
```

> **📝 Note:** Worlds produced by `osm-to-bedrock` contain `level.dat`, the `db/` directory,
> and a `world_info.json` file (conversion metadata). Optional fields such as `levelname.txt`,
> `world_icon.jpeg`, and pack manifests are not generated but may be added manually.

You can verify this by renaming any `.mcworld` file to `.zip` and extracting with any archive
tool (7-Zip, WinRAR, macOS Archive Utility, etc.).

### Related Bedrock File Extensions

| Extension | Type | Description |
|-----------|------|-------------|
| `.mcworld` | ZIP | Complete playable Bedrock/Education world |
| `.mctemplate` | ZIP | World template (same layout + `manifest.json`) |
| `.mcaddon` | ZIP | Bundle of `.mcpack` + optional `.mcworld` for distribution |
| `.mcpack` | ZIP | Resource pack or behavior pack |
| `.mcproject` | ZIP | Bedrock Editor project (cannot be imported as a normal world) |
| `.mcstructure` | NBT | Structure export from the Structure Block tool |

### mcworld vs mctemplate

- `.mcworld` imports directly as a ready-to-play world
- `.mctemplate` imports as a template from which new worlds are created; found in
  `com.mojang/world_templates`; supports `"allow_random_seed": true` in `manifest.json`
  to regenerate on each world creation

## World Save Folder Locations

### Windows

**Minecraft for Windows (modern launcher)** — Each signed-in user has their own directory:

```text
%appdata%\Minecraft Bedrock\Users\<userID>\games\com.mojang\minecraftWorlds\
```

For Minecraft Preview:
```text
%appdata%\Minecraft Bedrock Preview\Users\<userID>\games\com.mojang\minecraftWorlds\
```

Shared (guest / signed-out players):
```text
%appdata%\Minecraft Bedrock\Users\Shared\games\com.mojang\minecraftWorlds\
```

**Microsoft Store (UWP) installation** — older store installs use:
```text
%LocalAppData%\Packages\Microsoft.MinecraftUWP_8wekyb3d8bbwe\LocalState\games\com.mojang\minecraftWorlds\
```

### Android

```text
# External storage (accessible without root, default for "External" storage option):
/storage/emulated/0/Android/data/com.mojang.minecraftpe/files/games/com.mojang/minecraftWorlds/

# Internal storage (requires root):
/data/user/0/com.mojang.minecraftpe/games/com.mojang/minecraftWorlds/
```

On Android 11+, external access to app-private directories requires ADB or a file manager
with special permissions.

### iOS / iPadOS

```text
On My iPhone → Minecraft → games → com.mojang → minecraftWorlds
```

Accessible via the **Files** app under "On My iPhone" > "Minecraft".

### Xbox

Worlds are stored in Xbox Live cloud storage; direct file access is not possible. Export
`.mcworld` via the in-game **Export** button and transfer via OneDrive or USB.

### Minecraft Education Edition

```text
%appdata%\Minecraft Education Edition\games\com.mojang\minecraftWorlds\
```

**Critical:** Education Edition worlds have `eduOffer = 1` in `level.dat`. Worlds with this
flag will not open in regular Bedrock Edition. Set `eduOffer = 0` (or delete the field) with
an NBT editor to convert them. Education-specific blocks (chemistry elements, etc.) may
misbehave in regular Bedrock.

## Importing .mcworld Files

### Windows 10 / 11

**Method 1 — Double-click (recommended):**
Double-click the `.mcworld` file in File Explorer. Minecraft launches and imports automatically.

**Method 2 — Direct placement:**

1. Rename `.mcworld` to `.zip` and extract
2. Copy the extracted folder into the appropriate path (see [World Save Folder Locations](#world-save-folder-locations))
3. Each world must be in its own subfolder with a UUID-style name

### Android

**Method 1 — Share to Minecraft:**
Use any file manager to locate the `.mcworld` file > tap "Open with" > select Minecraft.

**Method 2 — Direct placement:**
Copy the extracted world folder to:
```text
/storage/emulated/0/Android/data/com.mojang.minecraftpe/files/games/com.mojang/minecraftWorlds/
```

### iOS / iPadOS

**Method 1 — Files app:**
In the Files app, tap the `.mcworld` file > tap "Share" > "Open in Minecraft".

**Method 2 — iTunes File Sharing (older iOS/macOS):**
Connect device > Finder/iTunes > select device > Files > Minecraft > drag `.mcworld` in.

### Xbox

Export the world from another platform as `.mcworld`, upload to OneDrive, then on Xbox:
use the "Import" button within the game's world management screen.

### Converting ZIP to mcworld

If a world is distributed as a plain `.zip`:

```bash
# Simply rename — it's the same format
mv world.zip world.mcworld
```

Then double-click to import on Windows, or transfer to mobile as above.

## Python Libraries

### Amulet-Core

The primary Python library for reading and writing Minecraft worlds. Supports all major
Bedrock versions since 1.7 and Java since 1.12.

| Property | Value |
|----------|-------|
| PyPI package | `amulet-core` |
| GitHub | https://github.com/Amulet-Team/Amulet-Core |
| Documentation | https://amulet-core.readthedocs.io |
| Python requirement | >= 3.11 |
| Dependencies | `amulet-nbt`, `PyMCTranslate`, `numpy` |

Key capabilities:
- Unified API for both Bedrock LevelDB and Java Region formats
- Automatic block state translation between game versions via PyMCTranslate
- Block palette reading/writing
- Entity and block entity access
- `.mcworld` and `.mcstructure` import/export

### Amulet-NBT

The NBT serialization library used by Amulet-Core. Handles both big-endian (Java) and
little-endian (Bedrock) NBT. Can be used standalone for low-level NBT parsing.

| Property | Value |
|----------|-------|
| PyPI package | `amulet-nbt` |
| GitHub | https://github.com/Amulet-Team/Amulet-NBT |

### PyMCTranslate

Block state translation layer between all Minecraft editions and versions. Used internally
by Amulet-Core; can be used standalone for block ID/state conversion.

| Property | Value |
|----------|-------|
| PyPI package | `PyMCTranslate` |
| GitHub | https://github.com/gentlegiantJGC/PyMCTranslate |

### Amulet Map Editor

The GUI application built on Amulet-Core. Also useful as a reference for Amulet API patterns.

| Property | Value |
|----------|-------|
| PyPI package | `amulet-map-editor` |
| GitHub | https://github.com/Amulet-Team/Amulet-Map-Editor |
| Python requirement | >= 3.11 |
| Supported Bedrock | 1.7+ |
| Supported Java | 1.12+ |

### GUI Tools

| Tool | Platform | Notes |
|------|----------|-------|
| **Amulet Map Editor** | Win/Mac/Linux | Python-based, open source |
| **MCCToolChest Bedrock** | Windows | https://mcctoolchest.com/ |
| **Universal Minecraft Editor** | Windows | https://www.universalminecrafteditor.com/ |

## Python Code Examples

### Setup

```bash
pip install amulet-core numpy
```

All examples require Python 3.11+.

### Open an Existing Bedrock World

```python
import amulet

# Amulet auto-detects Bedrock vs Java from the world directory
level = amulet.load_level("/path/to/world/folder")

print(level.level_wrapper.version_string)  # e.g. "1.21.0"
print(level.bounds("minecraft:overworld")) # SelectionBox with world bounds
```

### Read a Block at a Position

```python
import amulet

level = amulet.load_level("/path/to/world/folder")

# get_version_block returns a (Block, BlockEntity | None) tuple
block, block_entity = level.get_version_block(
    x=0, y=64, z=0,
    dimension="minecraft:overworld",
    version=("bedrock", (1, 21, 0))
)

print(block.blockstate_string)  # "minecraft:grass_block"
print(block.properties)         # {"snowy": "false"}

level.close()
```

### Iterate Chunks in a Region

```python
import amulet

level = amulet.load_level("/path/to/world/folder")

for cx, cz in level.all_chunk_coords("minecraft:overworld"):
    chunk = level.get_chunk(cx, cz, "minecraft:overworld")
    # chunk.blocks is a 16×384×16 numpy int32 array of palette indices
    # chunk.block_palette maps index → BlockStack
    block_count = (chunk.blocks != 0).sum()
    print(f"Chunk ({cx:4}, {cz:4}): {block_count} non-air blocks")

level.close()
```

### Modify Blocks

```python
import amulet
from amulet.api.block import Block

level = amulet.load_level("/path/to/world/folder")
version = ("bedrock", (1, 21, 0))
dimension = "minecraft:overworld"

# Single block placement
stone = Block("minecraft", "stone")
level.set_version_block(
    x=10, y=64, z=5,
    dimension=dimension,
    version=version,
    block=stone,
    block_entity=None
)

# Fill a 10×1×10 glass platform
glass = Block("minecraft", "glass")
for x in range(0, 10):
    for z in range(0, 10):
        level.set_version_block(
            x=x, y=60, z=z,
            dimension=dimension,
            version=version,
            block=glass,
            block_entity=None
        )

level.save()
level.close()
```

### Create a Flat World from Scratch

Amulet-Core requires an existing world to open. For a custom flat world, the recommended
workflow is:

1. Create a Flat world in-game (or use the raw approach below)
2. Open with Amulet and modify chunks programmatically
3. Save and re-import via `.mcworld`

**Raw level.dat creation** (lowest-level approach):

```python
"""
Create a minimal Bedrock world skeleton on disk.
Requires: pip install amulet-nbt
"""
import os
import struct
import amulet_nbt as nbt

world_dir = "my_custom_world"
os.makedirs(f"{world_dir}/db", exist_ok=True)

# Build level.dat NBT payload
level_data = nbt.TAG_Compound({
    "LevelName":      nbt.TAG_String("My Custom World"),
    "RandomSeed":     nbt.TAG_Long(12345),
    "StorageVersion": nbt.TAG_Int(10),
    "GameType":       nbt.TAG_Int(0),   # 0=Survival, 1=Creative
    "Difficulty":     nbt.TAG_Int(2),   # 0=Peaceful, 1=Easy, 2=Normal, 3=Hard
    "Generator":      nbt.TAG_Int(2),   # 2=Flat
    "SpawnX":         nbt.TAG_Int(0),
    "SpawnY":         nbt.TAG_Int(64),
    "SpawnZ":         nbt.TAG_Int(0),
    "lastOpenedWithVersion": nbt.TAG_List([
        nbt.TAG_Int(1), nbt.TAG_Int(21), nbt.TAG_Int(0),
        nbt.TAG_Int(0), nbt.TAG_Int(0)
    ]),
    "FlatWorldLayers": nbt.TAG_String(
        '{"biome_id":1,'
        '"block_layers":['
        '{"block_name":"minecraft:bedrock","count":1},'
        '{"block_name":"minecraft:dirt","count":2},'
        '{"block_name":"minecraft:grass_block","count":1}'
        '],'
        '"encoding_version":6,'
        '"preset_id":"ClassicFlat",'
        '"structure_options":null,'
        '"world_version":"version.post_1_18"}'
    ),
})

# Write level.dat: 8-byte header + NBT
# Bedrock NBT is little-endian, no compression, no wrapping GZip
nbt_bytes = level_data.to_nbt(
    little_endian=True,
    compressed=False,
    string_encoder=lambda x, _: x  # UTF-8 passthrough
)
header = struct.pack("<II", 10, len(nbt_bytes))  # version=10, length
with open(f"{world_dir}/level.dat", "wb") as f:
    f.write(header + nbt_bytes)

with open(f"{world_dir}/levelname.txt", "w", encoding="utf-8") as f:
    f.write("My Custom World")

# Initialize empty LevelDB (Amulet will handle this when you open the world)
print(f"World skeleton written to {world_dir}/")
print("Open with Amulet to populate chunks, then export as .mcworld")
```

### Add Structures Programmatically

```python
import amulet
from amulet.api.block import Block

def place_hollow_cube(
    level: amulet.level.BaseLevel,
    origin: tuple[int, int, int],
    size: int,
    material: Block,
    dimension: str,
    version: tuple,
) -> None:
    """Place a hollow cube of the given material and size."""
    ox, oy, oz = origin
    air = Block("minecraft", "air")

    for dx in range(size):
        for dy in range(size):
            for dz in range(size):
                on_face = (
                    dx in (0, size - 1)
                    or dy in (0, size - 1)
                    or dz in (0, size - 1)
                )
                block = material if on_face else air
                level.set_version_block(
                    x=ox + dx, y=oy + dy, z=oz + dz,
                    dimension=dimension,
                    version=version,
                    block=block,
                    block_entity=None,
                )


level = amulet.load_level("/path/to/world/folder")
version = ("bedrock", (1, 21, 0))
dimension = "minecraft:overworld"

stone_bricks = Block("minecraft", "stonebrick")  # Bedrock name
place_hollow_cube(
    level=level,
    origin=(0, 64, 0),
    size=7,
    material=stone_bricks,
    dimension=dimension,
    version=version,
)

level.save()
level.close()
```

### Place a Block Entity (Chest)

```python
import amulet
import amulet_nbt as nbt
from amulet.api.block import Block
from amulet.api.block_entity import BlockEntity

level = amulet.load_level("/path/to/world/folder")
version = ("bedrock", (1, 21, 0))
dimension = "minecraft:overworld"

# The block itself
chest_block = Block("minecraft", "chest", {"facing_direction": "2"})

# The block entity NBT (little-endian — Amulet handles byte order)
chest_nbt = nbt.TAG_Compound({
    "id":        nbt.TAG_String("Chest"),
    "x":         nbt.TAG_Int(5),
    "y":         nbt.TAG_Int(64),
    "z":         nbt.TAG_Int(0),
    "isMovable": nbt.TAG_Byte(1),
    "Items": nbt.TAG_List([
        nbt.TAG_Compound({
            "Count":        nbt.TAG_Byte(64),
            "Damage":       nbt.TAG_Short(0),
            "Name":         nbt.TAG_String("minecraft:diamond"),
            "Slot":         nbt.TAG_Byte(0),
            "WasPickedUp":  nbt.TAG_Byte(0),
        })
    ]),
})

block_entity = BlockEntity("minecraft", "chest", 5, 64, 0, chest_nbt)

level.set_version_block(
    x=5, y=64, z=0,
    dimension=dimension,
    version=version,
    block=chest_block,
    block_entity=block_entity,
)

level.save()
level.close()
```

### Export to .mcworld

```python
import amulet

level = amulet.load_level("/path/to/world/folder")

# Save any pending changes first
level.save()

# Export as .mcworld (Amulet creates a ZIP archive with the .mcworld extension)
level.export("/output/path/my_world.mcworld")
level.close()
```

The resulting file can be double-clicked on Windows or shared to mobile devices directly.

## Version Compatibility

### Bedrock Version Support in Amulet

Amulet supports reading and writing Bedrock worlds from version 1.7 onwards. PyMCTranslate
handles block state translation transparently between versions.

Key format milestones:

| Bedrock Version | Format Change |
|----------------|---------------|
| 0.9.0 | Switched from binary format to LevelDB |
| 1.0.0 | SubChunks split into separate LevelDB keys |
| 1.2.13 | Palettized subchunks with PersistentID block states |
| 1.18.0 | Extended height (-64 to 319), Data3D biome palettes |
| 1.18.30 | Modern per-actor LevelDB keys (actorprefix) |

### Python Version Requirements

Both `amulet-core` and `amulet-map-editor` require **Python 3.11 or later**.

## Limitations and Gotchas

### LevelDB File Locking

Bedrock's LevelDB uses a `LOCK` file. If Minecraft has the world open, any external tool
(including Amulet) will fail to open it. Always close Minecraft (and ensure the process has
fully terminated) before editing worlds programmatically.

### RuntimeID vs PersistentID

RuntimeIDs are assigned at game startup and are session-ephemeral. They may change between
game versions. **Never write RuntimeIDs to disk.** On-disk format always uses PersistentID
(NBT compound with `name` + `states`).

### Education Edition Worlds

`level.dat` fields `eduOffer = 1` and `educationFeaturesEnabled = 1` mark a world as
Education Edition. Such worlds will not open in regular Bedrock Edition. Use an NBT editor
to set `eduOffer = 0` before attempting to open them in regular Bedrock.

### Bedrock vs Java Block Names

Block names in Bedrock and Java are not always identical. While many converged after the
1.13 parity updates, legacy differences remain:
- `minecraft:stone_bricks` (Java) vs `minecraft:stonebrick` (Bedrock legacy)
- `minecraft:grass_block` (Java) vs `minecraft:grass` (very old Bedrock)

Use `PyMCTranslate` or Amulet's built-in translation when converting between editions.

### NBT Byte Order

Bedrock uses **little-endian NBT** throughout. Java uses **big-endian NBT**. Mixing them
will produce corrupted data silently. Always use an appropriate reader.

### Large World Performance

For worlds with many thousands of chunks, consider:

- Prefer NumPy array operations on `chunk.blocks` rather than per-block `set_version_block`
- Call `level.save()` periodically to flush the LevelDB write cache
- Use `level.all_chunk_coords()` for efficient iteration rather than coordinate range scanning
- Amulet loads chunk data lazily; only accessed chunks are held in memory

### Subchunk Extended Height

Pre-1.18 worlds have SubChunk indices 0–15 (Y 0–255). Post-1.18 worlds use signed indices
-4 to 19 (Y -64 to 319). Code that hardcodes subchunk index ranges will break on v1.18+
worlds. Use Amulet's API or check `level.bounds()` for the actual range.

## Further Reading

- [Amulet-Core GitHub](https://github.com/Amulet-Team/Amulet-Core)
- [Amulet-Core Documentation](https://amulet-core.readthedocs.io)
- [Amulet-NBT GitHub](https://github.com/Amulet-Team/Amulet-NBT)
- [PyMCTranslate GitHub](https://github.com/gentlegiantJGC/PyMCTranslate)
- [Amulet Map Editor GitHub](https://github.com/Amulet-Team/Amulet-Map-Editor)
- [MCCToolChest Bedrock](https://mcctoolchest.com/)
- [Universal Minecraft Editor](https://www.universalminecrafteditor.com/)
- [Minecraft File Extensions — Microsoft Learn](https://learn.microsoft.com/en-us/minecraft/creator/documents/minecraftfileextensions)
- [com.mojang directory — Minecraft Wiki](https://minecraft.wiki/w/Com.mojang)

## Related Documentation

- [Minecraft Bedrock Map Format](./MINECRAFT_BEDROCK_MAP_FORMAT.md) — LevelDB key schema, SubChunk encoding, block state NBT, and level.dat field reference
- [Developer Info](./DEVELOPER_INFO.md) — osm-to-bedrock architecture, module descriptions, and usage examples

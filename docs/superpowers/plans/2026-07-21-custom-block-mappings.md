# Custom Block Mappings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users override the built-in OSM tag → Minecraft block mappings via a `--block-mapping <path>` YAML file covering building material, road surface, landuse, and natural ground cover.

**Architecture:** A `BlockOverrides` struct (four `HashMap<String, Block>`) loads from YAML, rides on `ConvertParams` → `RenderContext`, and each of the four mapping functions consults it (taking `Option<&BlockOverrides>`) before falling back to the built-in default. `draw_building`/`draw_roof` receive the already-resolved wall block. v1 is CLI-only; the server passes `None`.

**Tech Stack:** Rust (edition 2024, `let`-chains in use), `serde_yaml_ng` (already a dependency), `anyhow`, `clap`. No new dependencies.

## Global Constraints

- Targets are limited to the existing closed set of 56 `Block` enum variants — no new blocks.
- Block names in the YAML are exact PascalCase enum variant names (e.g. `OakLog`, `PolishedBlackstoneSlab`).
- Override semantics: merge-over-defaults. Listed tag values override; unlisted keep defaults; new value→block entries may be added.
- `highway` overrides replace only the road **surface** block (width, sidewalk, flags stay at the built-in default for that class).
- v1 is CLI-only (`--block-mapping` on `convert`, `fetch-convert`, `overture-convert`). Server passes `None`. No change to `Config`.
- Every task ends with `make checkall` green (fmt + clippy `-D warnings` + check + test + web-check).
- Two refinements vs. the spec, both behavior-preserving: (1) mapping fns take `Option<&BlockOverrides>` instead of `&BlockOverrides` (avoids needing a non-`const` default constant on `RenderContext`); (2) `BlockOverrides` is defined in `blocks.rs` and only the loader lives in `block_mapping.rs` (keeps the dependency direction one-way: `block_mapping.rs` → `blocks.rs`).

## File Structure

| File | Responsibility | Task |
|------|----------------|------|
| `src/blocks.rs` | Add `Block::from_name`; define `BlockOverrides`; add `Option<&BlockOverrides>` to the 4 mapping fns + `default_*` helpers; tests. | 1, 3 |
| `src/block_mapping.rs` (new) | `load_block_overrides(path)` + `load_block_overrides_arg(...)` CLI helper + YAML `Raw` intermediate + resolve/validation. | 2 |
| `src/lib.rs` | Declare `pub mod block_mapping;`; update doctest `ConvertParams` with `block_overrides: None`. | 2, 3 |
| `src/params.rs` | Add `block_overrides: Option<BlockOverrides>` to `ConvertParams`; update shape tests + helper. | 3 |
| `src/pipeline/render.rs` | Add `RenderContext.block_overrides`; pass it at mapping call sites; resolve `wall` once for `draw_building`/`draw_roof`; multipolygon `building_block` call. | 3 |
| `src/pipeline/terrain.rs` | Add `block_overrides` to the `RenderContext { … }` at ~line 346. | 3 |
| `src/pipeline/preview.rs` | `run_surface_preview` passes `None`; `run_pipeline` adds `block_overrides` to its `RenderContext`. | 3 |
| `src/geometry.rs` | `draw_building`/`draw_roof` take a `wall_block: Block` param instead of computing it. | 3 |
| `src/cli/args.rs` | Add `--block-mapping <PATH>` to `BuildingArgs`. | 4 |
| `src/cli/convert.rs` | Load + thread overrides into the 3 convert-family `ConvertParams` constructions. | 4 |
| Construction sites: `src/metadata.rs`, `src/server/handlers.rs` (×5), `src/cli/mod.rs`, `src/pipeline/mod.rs` doctest | Add `block_overrides: None`. | 3 |
| `CLAUDE.md`, `README.md`, `ENHANCEMENTS.md` | Document the feature; mark enhancement done. | 5 |

---

## Task 1: `Block::from_name` lookup

**Files:**
- Modify: `src/blocks.rs` (add method to the existing `impl Block` block at ~line 136)

**Interfaces:**
- Produces: `Block::from_name(name: &str) -> Option<Block>` — exact PascalCase match over all 56 variants. Used by Task 2's loader.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/blocks.rs` (after the existing `java_biome_plains_default` test):

```rust
    // ── Block::from_name tests ──────────────────────────────────────────

    /// The authoritative list of (variant, name) pairs. Adding a new Block
    /// variant requires adding it here AND to `Block::from_name`. This test
    /// enforces that every listed variant round-trips through its name.
    const ALL_BLOCK_VARIANTS: &[(Block, &str)] = &[
        (Block::Air, "Air"),
        (Block::Bedrock, "Bedrock"),
        (Block::Stone, "Stone"),
        (Block::Dirt, "Dirt"),
        (Block::GrassBlock, "GrassBlock"),
        (Block::Water, "Water"),
        (Block::Sand, "Sand"),
        (Block::Gravel, "Gravel"),
        (Block::OakLog, "OakLog"),
        (Block::OakLeaves, "OakLeaves"),
        (Block::StoneBrick, "StoneBrick"),
        (Block::Concrete, "Concrete"),
        (Block::Cobblestone, "Cobblestone"),
        (Block::BlackConcrete, "BlackConcrete"),
        (Block::GrayConcrete, "GrayConcrete"),
        (Block::StoneSlab, "StoneSlab"),
        (Block::YellowConcrete, "YellowConcrete"),
        (Block::OakSign, "OakSign"),
        (Block::GlassPane, "GlassPane"),
        (Block::OakStairs, "OakStairs"),
        (Block::OakSlab, "OakSlab"),
        (Block::OakFence, "OakFence"),
        (Block::CobblestoneWall, "CobblestoneWall"),
        (Block::Brick, "Brick"),
        (Block::Sandstone, "Sandstone"),
        (Block::OakPlanks, "OakPlanks"),
        (Block::SprucePlanks, "SprucePlanks"),
        (Block::WhiteConcrete, "WhiteConcrete"),
        (Block::StoneBrickStairs, "StoneBrickStairs"),
        (Block::Rail, "Rail"),
        (Block::TallGrass, "TallGrass"),
        (Block::Fern, "Fern"),
        (Block::Poppy, "Poppy"),
        (Block::Torch, "Torch"),
        (Block::Lantern, "Lantern"),
        (Block::StoneBrickWall, "StoneBrickWall"),
        (Block::BirchLog, "BirchLog"),
        (Block::BirchLeaves, "BirchLeaves"),
        (Block::PolishedBlackstoneSlab, "PolishedBlackstoneSlab"),
        (Block::SmoothStoneSlab, "SmoothStoneSlab"),
        (Block::AndesiteSlab, "AndesiteSlab"),
        (Block::CherrySign, "CherrySign"),
        (Block::Snow, "Snow"),
        (Block::SnowLayer, "SnowLayer"),
        (Block::Ice, "Ice"),
        (Block::CherryHangingSign, "CherryHangingSign"),
        (Block::Dispenser, "Dispenser"),
        (Block::BrewingStand, "BrewingStand"),
        (Block::Bookshelf, "Bookshelf"),
        (Block::Cauldron, "Cauldron"),
        (Block::Bed, "Bed"),
        (Block::Furnace, "Furnace"),
        (Block::Barrel, "Barrel"),
        (Block::Bell, "Bell"),
        (Block::Campfire, "Campfire"),
        (Block::HayBale, "HayBale"),
    ];

    #[test]
    fn from_name_round_trips_all_variants() {
        assert_eq!(ALL_BLOCK_VARIANTS.len(), 56, "expected 56 Block variants");
        for &(block, name) in ALL_BLOCK_VARIANTS {
            assert_eq!(
                Block::from_name(name),
                Some(block),
                "from_name({name:?}) should return {block:?}"
            );
        }
    }

    #[test]
    fn from_name_rejects_unknown() {
        assert_eq!(Block::from_name("NotABlock"), None);
        assert_eq!(Block::from_name("oak_log"), None); // minecraft-id form is NOT accepted
        assert_eq!(Block::from_name("oaklog"), None); // case must match exactly
        assert_eq!(Block::from_name(""), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib blocks::tests::from_name`
Expected: compile error — `from_name` is not defined on `Block`.

- [ ] **Step 3: Write minimal implementation**

Add this method inside the existing `impl Block { … }` block in `src/blocks.rs` (e.g. immediately after the closing `}` of `block_states`, before the impl's final `}`):

```rust
    /// Parse a block by its enum variant name (exact PascalCase), e.g. `"OakLog"`.
    ///
    /// Used by the custom block-mapping loader to resolve user-supplied names.
    /// The authoritative variant/name list lives in `ALL_BLOCK_VARIANTS` in the
    /// tests below — keep this match in sync with it.
    pub fn from_name(name: &str) -> Option<Block> {
        Some(match name {
            "Air" => Block::Air,
            "Bedrock" => Block::Bedrock,
            "Stone" => Block::Stone,
            "Dirt" => Block::Dirt,
            "GrassBlock" => Block::GrassBlock,
            "Water" => Block::Water,
            "Sand" => Block::Sand,
            "Gravel" => Block::Gravel,
            "OakLog" => Block::OakLog,
            "OakLeaves" => Block::OakLeaves,
            "StoneBrick" => Block::StoneBrick,
            "Concrete" => Block::Concrete,
            "Cobblestone" => Block::Cobblestone,
            "BlackConcrete" => Block::BlackConcrete,
            "GrayConcrete" => Block::GrayConcrete,
            "StoneSlab" => Block::StoneSlab,
            "YellowConcrete" => Block::YellowConcrete,
            "OakSign" => Block::OakSign,
            "GlassPane" => Block::GlassPane,
            "OakStairs" => Block::OakStairs,
            "OakSlab" => Block::OakSlab,
            "OakFence" => Block::OakFence,
            "CobblestoneWall" => Block::CobblestoneWall,
            "Brick" => Block::Brick,
            "Sandstone" => Block::Sandstone,
            "OakPlanks" => Block::OakPlanks,
            "SprucePlanks" => Block::SprucePlanks,
            "WhiteConcrete" => Block::WhiteConcrete,
            "StoneBrickStairs" => Block::StoneBrickStairs,
            "Rail" => Block::Rail,
            "TallGrass" => Block::TallGrass,
            "Fern" => Block::Fern,
            "Poppy" => Block::Poppy,
            "Torch" => Block::Torch,
            "Lantern" => Block::Lantern,
            "StoneBrickWall" => Block::StoneBrickWall,
            "BirchLog" => Block::BirchLog,
            "BirchLeaves" => Block::BirchLeaves,
            "PolishedBlackstoneSlab" => Block::PolishedBlackstoneSlab,
            "SmoothStoneSlab" => Block::SmoothStoneSlab,
            "AndesiteSlab" => Block::AndesiteSlab,
            "CherrySign" => Block::CherrySign,
            "Snow" => Block::Snow,
            "SnowLayer" => Block::SnowLayer,
            "Ice" => Block::Ice,
            "CherryHangingSign" => Block::CherryHangingSign,
            "Dispenser" => Block::Dispenser,
            "BrewingStand" => Block::BrewingStand,
            "Bookshelf" => Block::Bookshelf,
            "Cauldron" => Block::Cauldron,
            "Bed" => Block::Bed,
            "Furnace" => Block::Furnace,
            "Barrel" => Block::Barrel,
            "Bell" => Block::Bell,
            "Campfire" => Block::Campfire,
            "HayBale" => Block::HayBale,
            _ => return None,
        })
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib blocks::tests::from_name`
Expected: PASS (2 tests).

- [ ] **Step 5: Run the full gate and commit**

Run: `make checkall` → expected green.
```bash
git add src/blocks.rs
git commit -m "feat(blocks): add Block::from_name variant-name lookup"
```

---

## Task 2: `BlockOverrides` type + YAML loader (`block_mapping.rs`)

**Files:**
- Modify: `src/blocks.rs` (add the `BlockOverrides` struct + `HashMap` import)
- Create: `src/block_mapping.rs`
- Modify: `src/lib.rs` (declare module)

**Interfaces:**
- Consumes: `Block::from_name` (Task 1).
- Produces: `crate::blocks::BlockOverrides` (struct); `crate::block_mapping::load_block_overrides(path: &Path) -> Result<Option<BlockOverrides>>`; `crate::block_mapping::load_block_overrides_arg(path: &Option<PathBuf>) -> Result<Option<BlockOverrides>>`. Used by Task 3 (pipeline) and Task 4 (CLI).

- [ ] **Step 1: Add the `BlockOverrides` struct**

In `src/blocks.rs`, add `use std::collections::HashMap;` next to the existing `use crate::osm::TagMap;` at the top. Then add this struct just above the `// ── OSM tag → Block mappings ──` comment (before `pub struct RoadStyle`):

```rust
/// User-supplied overrides for the OSM tag → Block mappings.
///
/// Each map is keyed by the OSM tag *value*:
/// - `building`: the `building:material` value
/// - `highway`: the `highway` value (overrides the road **surface** block only)
/// - `landuse`: the `landuse` value
/// - `natural`: the `natural` value
///
/// An empty map (the `Default`) means "no overrides for this category" and the
/// built-in mapping is used. Loaded from a YAML file via
/// [`crate::block_mapping::load_block_overrides`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockOverrides {
    pub building: HashMap<String, Block>,
    pub highway: HashMap<String, Block>,
    pub landuse: HashMap<String, Block>,
    pub natural: HashMap<String, Block>,
}
```

- [ ] **Step 2: Create `src/block_mapping.rs`**

```rust
//! User-supplied overrides for the OSM tag → Block mappings.
//!
//! Loaded from a YAML file referenced by the `--block-mapping` CLI flag. See
//! `docs/superpowers/specs/2026-07-21-custom-block-mappings-design.md` for the
//! format and semantics.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::blocks::{Block, BlockOverrides};

/// Load [`BlockOverrides`] from a YAML file.
///
/// Returns `Ok(None)` when the file does not exist, and `Err` on any parse,
/// I/O, or validation error (unknown block name, unknown top-level key).
pub fn load_block_overrides(path: &Path) -> Result<Option<BlockOverrides>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading block mapping file {}", path.display()))?;
    let raw: RawBlockOverrides = serde_yaml_ng::from_str(&text)
        .with_context(|| format!("parsing block mapping file {}", path.display()))?;
    Ok(Some(resolve(raw, path)?))
}

/// Load overrides for the `--block-mapping` CLI flag.
///
/// `Ok(None)` when the flag was not set (`path` is `None`). If the flag was set
/// but the file is missing or invalid, this is a hard error.
pub fn load_block_overrides_arg(path: &Option<PathBuf>) -> Result<Option<BlockOverrides>> {
    match path {
        Some(p) => match load_block_overrides(p)? {
            Some(o) => Ok(Some(o)),
            None => bail!("block mapping file not found: {}", p.display()),
        },
        None => Ok(None),
    }
}

/// Intermediate deserialiser: tag-value → block-name strings, before the names
/// are resolved to `Block` variants. `deny_unknown_fields` catches top-level
/// typos like `buliding:`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBlockOverrides {
    #[serde(default)]
    building: HashMap<String, String>,
    #[serde(default)]
    highway: HashMap<String, String>,
    #[serde(default)]
    landuse: HashMap<String, String>,
    #[serde(default)]
    natural: HashMap<String, String>,
}

/// Resolve every block name in `raw` to a `Block`, producing a `BlockOverrides`.
fn resolve(raw: RawBlockOverrides, path: &Path) -> Result<BlockOverrides> {
    Ok(BlockOverrides {
        building: resolve_map(raw.building, "building", path)?,
        highway: resolve_map(raw.highway, "highway", path)?,
        landuse: resolve_map(raw.landuse, "landuse", path)?,
        natural: resolve_map(raw.natural, "natural", path)?,
    })
}

fn resolve_map(
    raw: HashMap<String, String>,
    category: &str,
    path: &Path,
) -> Result<HashMap<String, Block>> {
    let mut out = HashMap::with_capacity(raw.len());
    for (tag_value, block_name) in raw {
        let block = Block::from_name(&block_name).ok_or_else(|| {
            anyhow::anyhow!(
                "{file}: unknown block name \"{name}\" under \"{cat}\" (key \"{key}\"). \
                 Use a Block variant name, e.g. OakLog, PolishedBlackstoneSlab, Water.",
                file = path.display(),
                name = block_name,
                cat = category,
                key = tag_value,
            )
        })?;
        out.insert(tag_value, block);
    }
    Ok(out)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Block;
    use std::io::Write as _;

    #[test]
    fn missing_file_returns_none() {
        let result =
            load_block_overrides(Path::new("/tmp/__nonexistent_block_mapping__.yaml")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn empty_file_yields_default() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp).unwrap();
        let ov = load_block_overrides(tmp.path()).unwrap().unwrap();
        assert_eq!(ov, BlockOverrides::default());
    }

    #[test]
    fn parses_all_four_sections() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            "building:\n  brick: OakPlanks\nhighway:\n  motorway: SmoothStoneSlab\n\
             landuse:\n  farmland: Dirt\nnatural:\n  wood: OakLog\n"
        )
        .unwrap();
        let ov = load_block_overrides(tmp.path()).unwrap().unwrap();
        assert_eq!(ov.building.get("brick"), Some(&Block::OakPlanks));
        assert_eq!(ov.highway.get("motorway"), Some(&Block::SmoothStoneSlab));
        assert_eq!(ov.landuse.get("farmland"), Some(&Block::Dirt));
        assert_eq!(ov.natural.get("wood"), Some(&Block::OakLog));
    }

    #[test]
    fn partial_file_parses() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "building:\n  glass: GlassPane\n").unwrap();
        let ov = load_block_overrides(tmp.path()).unwrap().unwrap();
        assert_eq!(ov.building.get("glass"), Some(&Block::GlassPane));
        assert!(ov.highway.is_empty());
        assert!(ov.landuse.is_empty());
        assert!(ov.natural.is_empty());
    }

    #[test]
    fn unknown_block_name_errors() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "building:\n  brick: NotABlock\n").unwrap();
        let err = load_block_overrides(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown block name"), "{msg}");
        assert!(msg.contains("NotABlock"), "{msg}");
        assert!(msg.contains("brick"), "{msg}");
    }

    #[test]
    fn unknown_top_level_key_errors() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "buliding:\n  brick: Brick\n").unwrap(); // typo
        assert!(load_block_overrides(tmp.path()).is_err());
    }

    #[test]
    fn load_arg_none_when_flag_unset() {
        assert_eq!(load_block_overrides_arg(&None).unwrap(), None);
    }

    #[test]
    fn load_arg_errors_when_flag_set_but_file_missing() {
        let path = PathBuf::from("/tmp/__definitely_missing_block_mapping__.yaml");
        let err = load_block_overrides_arg(&Some(path)).unwrap_err();
        assert!(format!("{err:#}").contains("block mapping file not found"));
    }

    #[test]
    fn load_arg_loads_when_file_present() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "building:\n  wood: SprucePlanks\n").unwrap();
        let ov = load_block_overrides_arg(&Some(tmp.path().to_path_buf()))
            .unwrap()
            .unwrap();
        assert_eq!(ov.building.get("wood"), Some(&Block::SprucePlanks));
    }
}
```

- [ ] **Step 3: Declare the module in `src/lib.rs`**

Add `pub mod block_mapping;` alongside the other `pub mod …;` declarations in `src/lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib block_mapping`
Expected: PASS (8 tests).

- [ ] **Step 5: Run the full gate and commit**

Run: `make checkall` → expected green.
```bash
git add src/blocks.rs src/block_mapping.rs src/lib.rs
git commit -m "feat(block_mapping): add BlockOverrides type + YAML loader"
```

---

## Task 3: Wire overrides through the pipeline

This is the end-to-end integration task. It touches many files but is one atomic compile unit: the four mapping functions change signature, so every caller and the `RenderContext`/`ConvertParams` plumbing must move together to keep the build green.

**Files:**
- Modify: `src/blocks.rs` (4 mapping fns + `default_*` helpers + existing building_block tests)
- Modify: `src/params.rs` (field + shape tests + helper)
- Modify: `src/pipeline/render.rs` (RenderContext field + call sites + wall resolution)
- Modify: `src/pipeline/terrain.rs` (RenderContext construction)
- Modify: `src/pipeline/preview.rs` (RenderContext construction + `run_surface_preview` call)
- Modify: `src/geometry.rs` (`draw_building`/`draw_roof` take `wall_block`)
- Modify (add `block_overrides: None` to construction): `src/metadata.rs`, `src/server/handlers.rs`, `src/cli/mod.rs`, `src/pipeline/mod.rs`, `src/lib.rs`

**Interfaces:**
- Consumes: `BlockOverrides`, `Block::from_name` (Tasks 1–2).
- Produces: `ConvertParams.block_overrides: Option<BlockOverrides>`; `RenderContext.block_overrides: Option<&'a BlockOverrides>`; the four mapping fns now take `Option<&BlockOverrides>`; `draw_building`/`draw_roof` now take `wall_block: Block`.

- [ ] **Step 1: Change the four mapping functions in `src/blocks.rs`**

Replace the four mapping function bodies so each takes `Option<&BlockOverrides>` and consults it before the default. The existing `match` bodies move verbatim into private `default_*` helpers.

Replace `pub fn highway_to_style(highway_type: &str) -> RoadStyle {` and its body (the whole `match highway_type { … }`) with:

```rust
/// Map `highway=*` value to a road style (block, width, sidewalks).
///
/// When `ov` contains an entry for `highway_type`, its block replaces the road
/// **surface** only; width, sidewalk, and flags come from the built-in default.
pub fn highway_to_style(highway_type: &str, ov: Option<&BlockOverrides>) -> RoadStyle {
    let mut style = default_highway_to_style(highway_type);
    if let Some(o) = ov
        && let Some(&surface) = o.highway.get(highway_type)
    {
        style.surface = surface;
    }
    style
}

fn default_highway_to_style(highway_type: &str) -> RoadStyle {
```
(leave the existing `match highway_type { … }` body and closing braces exactly as they are — they now form the body of `default_highway_to_style`).

Replace `pub fn landuse_to_block(landuse: &str) -> Block {` + body with:

```rust
/// Map `landuse=*` value to a surface block, honouring user overrides.
pub fn landuse_to_block(landuse: &str, ov: Option<&BlockOverrides>) -> Block {
    if let Some(o) = ov
        && let Some(&b) = o.landuse.get(landuse)
    {
        return b;
    }
    default_landuse_to_block(landuse)
}

fn default_landuse_to_block(landuse: &str) -> Block {
```
(leave the existing `match landuse { … }` body as the helper body).

Replace `pub fn natural_to_block(natural: &str) -> Block {` + body with:

```rust
/// Block for `natural=*` features, honouring user overrides.
pub fn natural_to_block(natural: &str, ov: Option<&BlockOverrides>) -> Block {
    if let Some(o) = ov
        && let Some(&b) = o.natural.get(natural)
    {
        return b;
    }
    default_natural_to_block(natural)
}

fn default_natural_to_block(natural: &str) -> Block {
```
(leave the existing `match natural { … }` body as the helper body).

Replace `pub fn building_block(tags: &TagMap) -> Block {` + body with:

```rust
/// Choose a building wall block based on `building:material`, honouring user
/// overrides keyed by the material value.
pub fn building_block(tags: &TagMap, ov: Option<&BlockOverrides>) -> Block {
    if let Some(material) = tags.get("building:material")
        && let Some(o) = ov
        && let Some(&b) = o.building.get(material)
    {
        return b;
    }
    default_building_block(tags)
}

fn default_building_block(tags: &TagMap) -> Block {
```
(leave the existing `match tags.get("building:material")…` body as the helper body).

- [ ] **Step 2: Update the existing `building_block` tests in `src/blocks.rs`**

The three tests `building_block_brick`, `building_block_default`, `building_block_wood` call `building_block(&tags)` — change each call to `building_block(&tags, None)`. Example for `building_block_brick`:

```rust
        assert_eq!(building_block(&tags, None), Block::Brick);
```
Apply the same `, None` to the other two. Then add override-behavior tests at the end of the `mod tests` block:

```rust
    // ── override behavior tests ─────────────────────────────────────────

    #[test]
    fn building_block_override_wins() {
        let mut ov = BlockOverrides::default();
        ov.building.insert("brick".to_string(), Block::OakPlanks);
        let mut tags = TagMap::new();
        tags.insert("building:material".into(), "brick".to_string());
        assert_eq!(building_block(&tags, Some(&ov)), Block::OakPlanks);
    }

    #[test]
    fn building_block_override_for_unknown_material_adds_mapping() {
        let mut ov = BlockOverrides::default();
        ov.building.insert("glass".to_string(), Block::GlassPane);
        let mut tags = TagMap::new();
        tags.insert("building:material".into(), "glass".to_string());
        // "glass" has no built-in mapping; without the override it would fall
        // back to StoneBrick. With it, it returns the override.
        assert_eq!(building_block(&tags, None), Block::StoneBrick);
        assert_eq!(building_block(&tags, Some(&ov)), Block::GlassPane);
    }

    #[test]
    fn landuse_override_and_default() {
        let mut ov = BlockOverrides::default();
        ov.landuse.insert("farmland".to_string(), Block::Sand);
        assert_eq!(landuse_to_block("farmland", None), Block::Dirt); // default
        assert_eq!(landuse_to_block("farmland", Some(&ov)), Block::Sand); // override
        assert_eq!(landuse_to_block("forest", Some(&ov)), Block::OakLog); // untouched default
    }

    #[test]
    fn natural_override_and_default() {
        let mut ov = BlockOverrides::default();
        ov.natural.insert("wood".to_string(), Block::BirchLog);
        assert_eq!(natural_to_block("wood", None), Block::OakLog); // default
        assert_eq!(natural_to_block("wood", Some(&ov)), Block::BirchLog); // override
    }

    #[test]
    fn highway_override_changes_surface_only() {
        let mut ov = BlockOverrides::default();
        ov.highway.insert("motorway".to_string(), Block::SmoothStoneSlab);
        let default_style = highway_to_style("motorway", None);
        let overridden = highway_to_style("motorway", Some(&ov));
        assert_eq!(overridden.surface, Block::SmoothStoneSlab); // surface replaced
        assert_eq!(overridden.half_width, default_style.half_width); // width preserved
        assert_eq!(overridden.sidewalk, default_style.sidewalk); // sidewalk preserved
    }
```

- [ ] **Step 3: Add the field to `ConvertParams` (`src/params.rs`)**

Add this field to the `ConvertParams` struct (after `surface_thickness`):

```rust
    /// User-supplied OSM tag → Block overrides (loaded from `--block-mapping`).
    /// `None` (no overrides) on the server and in `terrain-convert`.
    pub block_overrides: Option<crate::blocks::BlockOverrides>,
```

Update every `ConvertParams { … }` construction in `src/params.rs` (the two tests at ~line 109 and ~line 145, and the `minimal_convert_params` helper at ~line 210) by adding `block_overrides: None,` to each (place it after `surface_thickness: 4,`).

- [ ] **Step 4: Add `block_overrides: None` to the remaining construction sites**

For each of these files, find every `ConvertParams { … }` and add `block_overrides: None,` after the `surface_thickness` line. The compiler will error on any you miss, naming the file — fix each:

- `src/metadata.rs` (2 constructions: ~line 219, ~line 280)
- `src/server/handlers.rs` (5 constructions: ~lines 455, 556, 680, 877, 1125)
- `src/cli/mod.rs` (1 construction in a test: ~line 200)
- `src/pipeline/mod.rs` doctest (~line 142)
- `src/lib.rs` doctest (~line 16) — add `block_overrides: None,` after `surface_thickness: 4,`

- [ ] **Step 5: Add `block_overrides` to `RenderContext` and its constructions**

In `src/pipeline/render.rs`, add a field to `RenderContext<'a>` (after `pub surface: i32,`):

```rust
    pub block_overrides: Option<&'a crate::blocks::BlockOverrides>,
```

In `src/pipeline/terrain.rs` (~line 346) and `src/pipeline/preview.rs` (~line 513), add to each `RenderContext { … }` construction (after `surface,`):

```rust
        block_overrides: params.block_overrides.as_ref(),
```

- [ ] **Step 6: Update the mapping call sites in `src/pipeline/render.rs`**

Pass `ctx.block_overrides` at each call (each is currently a one-arg / tag-only call):

- `natural_to_block(natural)` → `natural_to_block(natural, ctx.block_overrides)` (way path, ~line 125)
- `landuse_to_block(lu)` → `landuse_to_block(lu, ctx.block_overrides)` (way path, ~line 130)
- `natural_to_block(natural)` → `natural_to_block(natural, ctx.block_overrides)` (relation path, ~line 154)
- `landuse_to_block(lu)` → `landuse_to_block(lu, ctx.block_overrides)` (relation path, ~line 159)
- `highway_to_style(hw)` → `highway_to_style(hw, ctx.block_overrides)` (~line 293)
- `blocks::building_block(rel.tags)` → `blocks::building_block(rel.tags, ctx.block_overrides)` (~line 398)

`waterway_to_style` (~line 190) is unchanged — it takes no override.

- [ ] **Step 7: Resolve the wall block once for `draw_building`/`draw_roof` in `render.rs`**

At the way-buildings call site (~lines 377–391), insert a wall resolution before the two calls and pass it to both:

```rust
            let straight_pts = convert::straighten_polygon(pts, params.wall_straighten_threshold);
            let pts = &straight_pts;
            let wall = blocks::building_block(&way.tags, ctx.block_overrides);
            draw_building(
                world,
                pts,
                building_surface,
                params.building_height,
                &way.tags,
                wall,
                building_road_dir,
            );
            draw_roof(world, pts, building_surface, params.building_height, &way.tags, wall);
```

At the **multipolygon** `draw_roof` call (~line 435), pass the same `wall` already computed at ~line 398 (which Step 6 updated to honour overrides):

```rust
                draw_roof(world, outer, rel_surface, params.building_height, rel.tags, wall);
```

- [ ] **Step 8: Change `draw_building`/`draw_roof` signatures in `src/geometry.rs`**

`draw_building` (~line 457): add a `wall_block: Block,` parameter **after** `tags: &TagMap,` (and before `road_dir`), and replace `let wall = blocks::building_block(tags);` with `let wall = wall_block;`. New signature:

```rust
pub fn draw_building(
    world: &mut dyn WorldWriter,
    pts: &[(i32, i32)],
    surface: i32,
    height: i32,
    tags: &TagMap,
    wall_block: Block,
    road_dir: Option<(f64, f64)>,
) {
    let wall = wall_block;
```
(`tags` is retained because the body uses it for windows/doors.)

`draw_roof` (~line 605): add `wall_block: Block,` after `height: i32,`, and replace `let wall = blocks::building_block(tags);` (~line 620) with `let wall = wall_block;`:

```rust
pub fn draw_roof(
    world: &mut dyn WorldWriter,
    pts: &[(i32, i32)],
    surface: i32,
    height: i32,
    tags: &TagMap,
    wall_block: Block,
) {
    if pts.is_empty() {
        return;
    }

    let roof_shape = tags.get("roof:shape").map(|s| s.as_str()).unwrap_or("flat");
    if roof_shape == "flat" {
        return;
    }

    let wall = wall_block;
```

- [ ] **Step 9: Update the `run_surface_preview` call in `src/pipeline/preview.rs`**

At ~line 213, `let style = blocks::highway_to_style(hw_type);` → `let style = blocks::highway_to_style(hw_type, None);` (this rough 2D classification only reads `.half_width`; the surface block is unused here).

- [ ] **Step 10: Run the full gate and commit**

Run: `make checkall` → expected green. If clippy or the compiler flags a missed `ConvertParams` construction, add `block_overrides: None,` there and re-run.
```bash
git add -A
git commit -m "feat(pipeline): thread BlockOverrides through render pipeline"
```

---

## Task 4: `--block-mapping` CLI flag

**Files:**
- Modify: `src/cli/args.rs` (add flag to `BuildingArgs`)
- Modify: `src/cli/convert.rs` (load + thread in 3 convert-family functions)

**Interfaces:**
- Consumes: `load_block_overrides_arg` (Task 2), `ConvertParams.block_overrides` (Task 3).
- Produces: `--block-mapping <PATH>` available on `convert`, `fetch-convert`, `overture-convert`.

- [ ] **Step 1: Add the flag to `BuildingArgs` in `src/cli/args.rs`**

Add this field to the `BuildingArgs` struct (after `poi_markers`):

```rust
    /// Path to a YAML file overriding default OSM tag → block mappings
    /// (keys: building, highway, landuse, natural; values: Block variant names).
    #[arg(long, value_name = "PATH")]
    pub block_mapping: Option<PathBuf>,
```

- [ ] **Step 2: Load and thread overrides in the three convert-family functions**

In `src/cli/convert.rs`, in each of `run_convert` (~line 24), `run_fetch_convert` (~line 207), and `run_overture_convert` (~line 276), add this line just before the `let convert_params = ConvertParams {` construction:

```rust
    let block_overrides =
        crate::block_mapping::load_block_overrides_arg(&args.building.block_mapping)?;
```

Then add this field to each of those three `ConvertParams { … }` constructions (after `surface_thickness`):

```rust
        block_overrides,
```

(`run_terrain_convert` uses `TerrainParams`, not `ConvertParams`, so it is untouched.)

- [ ] **Step 3: Test that `--block-mapping` parses**

Add a test to `src/cli/mod.rs`'s test module asserting clap accepts the new flag. (There is no existing parse test, so add the `clap::Parser` import if needed.)

```rust
    #[test]
    fn block_mapping_flag_parses() {
        use crate::cli::args::{Cli, Commands};
        use clap::Parser;

        let cli = Cli::try_parse_from([
            "osm-to-bedrock",
            "convert",
            "-i",
            "map.osm.pbf",
            "-o",
            "World",
            "--block-mapping",
            "blocks.yaml",
        ])
        .expect("flag should parse");
        match cli.command {
            Commands::Convert(args) => {
                assert_eq!(
                    args.building.block_mapping,
                    Some(std::path::PathBuf::from("blocks.yaml"))
                );
            }
            _ => panic!("expected Convert subcommand"),
        }
    }
```

The override-resolution behavior itself is covered by the Task 2 loader tests and the Task 3 mapping tests; this test pins the CLI surface.

- [ ] **Step 4: Run the full gate and commit**

Run: `make checkall` → expected green. Manually verify the flag appears: `cargo run --release -- convert --help | grep block-mapping`.
```bash
git add -A
git commit -m "feat(cli): add --block-mapping flag for custom block mappings"
```

---

## Task 5: Documentation

**Files:**
- Modify: `CLAUDE.md` (module table)
- Modify: `README.md` (new section + sample)
- Modify: `ENHANCEMENTS.md` (mark done)
- Create: `examples/block-mapping.example.yaml`

- [ ] **Step 1: Add `block_mapping.rs` to the module table in `CLAUDE.md`**

In the "Module layout" table, add a row after the `blocks.rs` row:

```markdown
| `src/block_mapping.rs` | Loads user-supplied OSM tag → Block overrides from a YAML file (`--block-mapping`); `BlockOverrides` itself is defined in `blocks.rs`. |
```

Also append a line to the "Key design decisions" section:

```markdown
- Custom block mappings: a `--block-mapping <path>` YAML file overrides the default OSM tag → Block mappings (building material, road surface, landuse, natural). Targets are the 56 built-in `Block` variants (by exact PascalCase name); overrides merge over defaults.
```

- [ ] **Step 2: Create `examples/block-mapping.example.yaml`**

```yaml
# Custom block mappings for osm-to-bedrock.
# Usage: osm-to-bedrock convert -i map.osm.pbf -o World/ --block-mapping block-mapping.yaml
#
# Each top-level key is a mapping surface. Each entry maps an OSM tag VALUE to
# a target block, named by its exact PascalCase Block variant (e.g. OakLog,
# PolishedBlackstoneSlab). Entries override the built-in default for that value;
# you may also add values that currently fall through to the default.

# building:material value -> wall block
building:
  brick: OakPlanks
  glass: GlassPane
  concrete: WhiteConcrete

# highway value -> road SURFACE block (width/sidewalk/flags stay at the default for that class)
highway:
  motorway: SmoothStoneSlab
  residential: Cobblestone

# landuse value -> ground surface block
landuse:
  farmland: Dirt
  forest: BirchLog

# natural value -> ground surface block
natural:
  wood: OakLog
  bare_rock: Stone
```

- [ ] **Step 3: Add a "Custom block mappings" section to `README.md`**

Read `README.md`, find the section describing `convert` usage / flags, and add a subsection after it:

```markdown
### Custom block mappings

Override the default OSM tag → block mappings with a YAML file to customize how
your world looks. Pass it with `--block-mapping`:

```bash
osm-to-bedrock convert -i map.osm.pbf -o World/ --block-mapping block-mapping.yaml
```

The file maps an OSM tag **value** to a target block, named by its exact PascalCase
`Block` variant (e.g. `OakLog`, `PolishedBlackstoneSlab`). See
[`examples/block-mapping.example.yaml`](examples/block-mapping.example.yaml) for a
full sample. Entries override the built-in default for that tag value; you may
also add mappings for values that currently fall through to the default.

```yaml
building:        # building:material value
  brick: OakPlanks
highway:         # highway value -> road surface block
  residential: Cobblestone
landuse:         # landuse value
  farmland: Dirt
natural:         # natural value
  wood: OakLog
```

Only the four block-returning surfaces are overridable: `building`, `highway`
(surface block only), `landuse`, and `natural`. Targets must be one of the
built-in `Block` variants; an unknown name errors at startup with the valid names.
```

- [ ] **Step 4: Mark the enhancement done in `ENHANCEMENTS.md`**

Replace the "Custom Block Mappings" entry under "## Data Processing":

```markdown
### Custom Block Mappings
Support a user-provided JSON/YAML config file that overrides the default OSM tag → Minecraft block mappings. Lets users customize the look of their worlds.

**Done:** `--block-mapping <path>` loads a YAML file that overrides the four block-returning mappings (building material, road surface, landuse, natural) over the built-in defaults. Targets are the 56 `Block` variants by exact PascalCase name. See `examples/block-mapping.example.yaml`.
```

- [ ] **Step 5: Run the full gate and commit**

Run: `make checkall` → expected green.
```bash
git add -A
git commit -m "docs: custom block mappings — CLAUDE.md, README, example, mark ENHANCEMENTS done"
```

---

## Self-Review Notes

- **Spec coverage:** Override the four block-returning mappings → Task 3 (signatures) + Task 5 (docs). Merge-over-defaults + add-new-values → Task 3 tests. Dedicated file via flag → Task 4. Block variant-name validation → Task 1 + Task 2 error test. `draw_building` receives resolved wall → Task 3 Step 7–8. Server passes `None` → Task 3 Step 4 (server constructions get `None`). Shape tests updated → Task 3 Step 3. ENHANCEMENTS marked done → Task 5. CLI-only (no `Config` change) → confirmed (Task 4 touches only `args.rs`/`convert.rs`).
- **Type consistency:** `Option<&BlockOverrides>` used uniformly in mapping fns and `RenderContext`; `Option<BlockOverrides>` (owned) on `ConvertParams`; `load_block_overrides_arg(&Option<PathBuf>) -> Result<Option<BlockOverrides>>` matches `args.building.block_mapping: Option<PathBuf>`.
- **Compile-green boundaries:** Task 3 is atomic because the signature change breaks all callers; all callers and plumbing update within the task. Tasks 1, 2, 4, 5 each stand alone.
```

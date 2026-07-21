# Custom Block Mappings — Design Spec

**Date:** 2026-07-21
**Enhancement:** ENHANCEMENTS.md → Data Processing → "Custom Block Mappings"
**Status:** Approved (design); pending implementation plan

## Summary

Let users override the built-in OpenStreetMap tag → Minecraft block mappings with a user-provided YAML file, so they can customize the look of generated worlds. The file is supplied via a new `--block-mapping <path>` flag and applies to all four block-returning mapping surfaces: building material, road surface, landuse, and natural ground cover.

## Scope

**In scope (v1):**

- Override the four mapping functions that resolve an OSM tag value to a `Block`:
  - `building_block` — keyed by `building:material` value
  - `highway_to_style` — keyed by `highway` value; override replaces the road **surface** block only (width, sidewalk, and flags stay at built-in defaults)
  - `landuse_to_block` — keyed by `landuse` value
  - `natural_to_block` — keyed by `natural` value
- Merge-over-defaults semantics: a listed tag value overrides; an unlisted tag value keeps its built-in default. Users may also **add** mappings for tag values that currently fall through to the default (for example `landuse=orchard`).
- Targets are limited to the existing closed set of 56 `Block` enum variants. A mapping reassigns a tag to a different one of these 56 blocks.
- New `--block-mapping <path>` CLI flag on the three OSM-rendering subcommands (`convert`, `fetch-convert`, `overture-convert`).
- YAML file format loaded and validated at startup.

**Out of scope (v1):**

- Introducing **brand-new** blocks not in the `Block` enum (requires palette encoding, biome mapping, NBT states — a separate, larger feature).
- Overriding the `waterway` mapping. Waterways always render as `Water`; only their width/depth are configurable, and those already read OSM `width`/`depth` tags.
- Overriding road geometry fields (width, sidewalk presence, edge lines). v1 overrides only the surface block.
- Exposing block mappings through the HTTP server or the web ExportPanel. The server passes `None` for overrides; CLI-only in v1.
- Loading the mapping table from the main `.osm-to-bedrock.yaml` config. v1 uses a dedicated file via the flag.

## File Format

A YAML document with four optional top-level keys, one per mapping surface. Each key maps an OSM tag **value** (a string) to a target block **variant name** (one of the 56 `Block` enum variants, exact PascalCase).

```yaml
# building:material value -> block
building:
  brick: OakPlanks
  glass: GlassPane
  concrete: WhiteConcrete

# highway value -> road SURFACE block
# (width, sidewalk, and flags stay at built-in defaults for that highway class)
highway:
  motorway: SmoothStoneSlab
  residential: Cobblestone

# landuse value -> surface block
landuse:
  farmland: Dirt
  forest: BirchLog

# natural value -> surface block
natural:
  wood: OakLog
  bare_rock: Stone
```

Parsing rules:

- Each top-level key is optional (`#[serde(default)]` per field); a file with only a `building:` section is valid.
- Unknown **top-level keys** are rejected (`#[serde(deny_unknown_fields)]`) so a typo like `buliding:` fails loudly.
- Any OSM value string is accepted as a map key (these cannot be validated against a fixed set).
- Every target **block name** is resolved via `Block::from_name`. An unknown name is a hard error that lists the valid names.

## Block Naming

Users reference blocks by their Rust enum variant name in exact PascalCase — for example `OakLog`, `PolishedBlackstoneSlab`, `BlackConcrete`, `Sand`, `Water`. This is chosen over the Minecraft identifier form (`minecraft:oak_log`) because:

- It is unambiguous: the 56 variants are a closed, enumerable set, whereas Minecraft identifiers have collisions in this codebase (`TallGrass` and `Fern` both map to `minecraft:tallgrass`).
- The names are already human-readable.

`Block::from_name(&str) -> Option<Block>` is added to `blocks.rs` as an exact-match lookup over the 56 variants.

## Architecture

### New module: `src/block_mapping.rs`

Holds the override container and its loader.

```rust
/// User-supplied overrides for the OSM tag → Block mappings.
/// Empty maps (the `Default`) mean "use built-in defaults everywhere".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockOverrides {
    #[serde(default)]
    pub building: HashMap<String, Block>,
    #[serde(default)]
    pub highway: HashMap<String, Block>,
    #[serde(default)]
    pub landuse: HashMap<String, Block>,
    #[serde(default)]
    pub natural: HashMap<String, Block>,
}

impl BlockOverrides {
    /// Load overrides from a YAML file. `Ok(None)` if the path does not exist.
    pub fn from_file(path: &Path) -> Result<Option<Self>> { ... }
}
```

`from_file` deserializes a temporary intermediate struct whose map values are strings, then resolves each string via `Block::from_name`. Any unresolved name produces an error naming the offending key and listing the valid block names. This two-step keeps `Block`'s `Deserialize` impl out of the picture (it has none today) and gives precise error locations.

### Changes to `src/blocks.rs`

- Add `Block::from_name(&str) -> Option<Block>` — exact PascalCase match over all 56 variants.
- Each of the four mapping functions gains an `&BlockOverrides` parameter and consults the relevant map before falling back to the built-in default:
  - `building_block(tags: &TagMap, ov: &BlockOverrides) -> Block`
  - `landuse_to_block(landuse: &str, ov: &BlockOverrides) -> Block`
  - `natural_to_block(natural: &str, ov: &BlockOverrides) -> Block`
  - `highway_to_style(highway_type: &str, ov: &BlockOverrides) -> RoadStyle` — builds the default `RoadStyle`, then if `ov.highway` has an entry for `highway_type`, replaces `.surface` and returns.
- The current built-in `match` bodies move unchanged into private `default_building_block`, `default_landuse_to_block`, `default_natural_to_block`, and `default_highway_to_style` helpers. Existing unit tests call the public functions with `&BlockOverrides::default()` and continue to assert the same defaults.

### Changes to `src/params.rs`

- Add `pub block_overrides: Option<BlockOverrides>` to `ConvertParams`. The shape-pinning test and `lib.rs` doctest are updated with `block_overrides: None`; those tests exist specifically to surface this contract change.

### Changes to `src/pipeline/render.rs`

- `RenderContext` gains `pub block_overrides: &'a BlockOverrides`. The pipeline constructs it from `params.block_overrides.as_ref().unwrap_or(&DEFAULT)` (a `const BlockOverrides::default()`), so call sites always have a valid reference.
- All mapping calls pass `ctx.block_overrides`:
  - `landuse_to_block`, `natural_to_block` (×2 each, way + relation paths)
  - `waterway_to_style` — unchanged signature (no block override), but the call still threads params.
  - `highway_to_style` (road layer)
  - `building_block` for the multipolygon-relation building wall (resolved at the call site, passed into `draw_building`).

### Changes to `src/geometry.rs`

- `draw_building` and the bridge drawing path no longer call `building_block(tags)` internally. Instead they receive the resolved wall block:
  - `draw_building(..., wall_block: Block, road_dir: Option<(f64, f64)>)`
  - The bridge drawing helper takes `wall_block` similarly if it currently derives the wall internally.
- The render.rs call sites resolve `let wall = building_block(tags, &ctx.block_overrides);` before calling, keeping override logic out of `geometry.rs` internals.

### Changes to `src/pipeline/preview.rs`

`preview.rs` has two distinct paths that touch the mapping functions:

- **`run_pipeline` (full in-memory preview, ~line 513):** builds a real `RenderContext` from `ConvertParams` and calls `render_osm_features`. It threads `params.block_overrides` through the new `RenderContext` field like every other path — no special handling.
- **`run_surface_preview` (rough 2D top-down classification, ~line 213):** calls `highway_to_style` directly but only reads `.half_width` for geometry; it never resolves the surface `Block`. This call passes `&BlockOverrides::default()` (the surface block it would otherwise override is unused here).

### Changes to `src/cli/args.rs` and `src/cli/convert.rs`

- Add `--block-mapping <path>` to `BuildingArgs` (the shared flag group embedded in `convert`, `fetch-convert`, and `overture-convert` — the three subcommands that render OSM features). `terrain-convert` does not embed `BuildingArgs` and does not render OSM blocks.
- `cli/convert.rs` loads the file via `BlockOverrides::from_file` when the flag is set, errors clearly on a missing/invalid file, and threads the result into `ConvertParams.block_overrides`.
- v1 is flag-only. The `Config` struct (`src/config.rs`) is **not** changed; supplying the path from the main `.osm-to-bedrock.yaml` is a trivial follow-up if wanted later.

## Error Handling

- Missing file when `--block-mapping` is given explicitly → hard error: `"block mapping file not found: <path>"`.
- YAML parse error → hard error with the file path and serde message.
- Unknown top-level key → hard error (`deny_unknown_fields`).
- Unknown block name → hard error naming the offending key and listing the valid block names.
- An empty file or a file with only comments → `Ok(Some(BlockOverrides::default()))` (no overrides; equivalent to no flag).

## Testing

- **`Block::from_name`**: round-trips all 56 variants; returns `None` for an unknown name.
- **`BlockOverrides::from_file`**: parses a sample YAML covering all four sections; returns `Ok(None)` for a missing path; parses an empty file to defaults; rejects an unknown block name with a helpful message; rejects an unknown top-level key.
- **Override behavior** (per mapping): an override entry wins when present; the built-in default is used when the tag value is absent from overrides; an override for a previously-unmapped value adds it; `highway_to_style` override changes only `.surface` and preserves `half_width`, `sidewalk`, and flags.
- **Regression**: every existing `blocks.rs` mapping test passes unchanged once updated to pass `&BlockOverrides::default()`.
- **Shape**: `params.rs` and `lib.rs` doctest updated with `block_overrides: None`.

## Documentation

- `CLAUDE.md` module table gains `src/block_mapping.rs`.
- `README.md` gains a short "Custom block mappings" section with the sample YAML and the list of valid block names (or a pointer to `--help`).
- `ENHANCEMENTS.md` "Custom Block Mappings" entry marked **done**.

## Implementation Phasing

Each phase ends with `make checkall` (fmt + clippy + check + test + web-check).

1. **`Block::from_name` + `src/block_mapping.rs` + unit tests.** No pipeline touch. Verify: `cargo test` green, new tests pass.
2. **Thread overrides through the pipeline.** `ConvertParams` field, `RenderContext` field, mapping-fn signatures + `default_*` helpers, `render.rs`/`preview.rs`/`geometry.rs` call sites. Verify: `make checkall`, existing mapping tests pass with `default()`.
3. **CLI flag + load/wire.** `BuildingArgs` flag, `cli/convert.rs` loading + threading into `ConvertParams`. Verify: `make checkall`; a manual conversion with a sample mapping file produces the overridden blocks.
4. **Docs + mark ENHANCEMENTS done.** `CLAUDE.md`, `README.md`, `ENHANCEMENTS.md`. Verify: `make checkall`.

## Open Questions

None — all decisions resolved during brainstorming (scope: all four surfaces; home: dedicated file via flag; naming: enum variant names; semantics: merge-over-defaults).

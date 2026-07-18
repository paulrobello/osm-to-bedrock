# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_Audit remediation — see `AUDIT-REMEDIATION.md` for the full breakdown. 54 of 58 audit issues resolved (ARC-001 streaming Java writer + ARC-010 DashMap landed after the initial run)._

### Security
- Opt-in shared-secret auth on the HTTP API via `--api-key` / `OSM_TO_BEDROCK_API_KEY` (constant-time compare), checked on all routes except `/health`. The server now refuses to bind a non-loopback address (e.g. `--host 0.0.0.0`) without a key unless `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1` is set.
- `/status` and `/download` no longer leak verbatim error chains or OS strings to clients; full detail is logged server-side only.
- `Content-Security-Policy` header on the Next.js frontend (`connect-src 'self'`; all external calls are server-side proxied).
- `validate_bbox` (±90/±180 + max block-extent span) enforced on fetch/terrain/preview endpoints before the concurrency semaphore.
- Explicit 1 MiB body limit on JSON routes; client errors now return 400 instead of 500.
- Mutex-poisoning recovery — a panicked background job no longer crashes the API server on the next request.

### Added
- `make docs` Makefile target (rustdoc built with warnings-as-errors), wired into `make checkall` so broken intra-doc-links fail the gate; clears all 29 pre-existing `cargo doc` warnings left by the module splits (private intra-doc-links, stale-path links, a bare URL).
- `docs/DEPLOYMENT.md` — Docker, reverse-proxy, and self-hosting guide (ports, environment variables, and the auth requirement that applies to non-loopback binds).
- Test net for the previously-untested orchestrator: `ChunkData` round-trip tests, a `RecordingWorld` `render_osm_features` integration test, cross-edition Bedrock↔Java parity tests, and `params`/`nbt` coverage (Rust tests 163 → 254).
- `vitest` web test suite (api-config + `useConversion` polling state machine) with a CI step and a `web-test` Make target (web tests 0 → 21).
- `proxyToRust` helper centralising the Next.js→Rust proxy routes (routes dir −34% LOC).
- `AbortController` cleanup in `useConversion` so polling stops on unmount.

### Changed
- **Streaming Java Edition writer (ARC-001)** — `edition=java` conversions now stream tile-by-tile like Bedrock instead of accumulating the whole world in RAM. `JavaWorld::new_streaming` scopes its scratch store to the current tile and lazily writes 32×32 Anvil region files as tiles flush, so peak memory ≈ one tile + a small frontier of region buffers. The previous `enforce_java_memory_budget` OOM guard is retired (both editions now match Bedrock's per-tile profile, which never needed a guard).
- **DashMap job state (ARC-010)** — the API server's job map is now `Arc<DashMap<String, JobState>>` instead of `Arc<Mutex<HashMap<…>>>`, so the read-heavy `/status` + `/download` polling path no longer contends with worker progress writes. The `lock_jobs` mutex-poisoning helper is removed (DashMap shard locks never poison — SEC-006 recovery is now structural).
- Deduped the Bedrock/Java tile loop via a `process_tile` helper + `WorldWriter::flush_tile()`; the outer loop is now edition-agnostic.
- Split `src/pipeline.rs` (2591 LOC) → `src/pipeline/{mod,render,terrain,preview,decoration,util}.rs`.
- Split `src/server.rs` (3127 LOC) → `src/server/{mod,state,error,auth,options,handlers}.rs`; extracted a shared `spawn_conversion_job` helper.
- Split `src/main.rs` (1254 LOC) → `src/cli/` module; the binary is now a 16-line shim.
- Extracted a shared `ChunkStore` struct used by both `BedrockWorld` and `JavaWorld` (−196 duplicated lines).
- Extracted 11 per-layer `render_*` helpers from `render_osm_features` (558 → 34 LOC orchestrator).
- `NEXT_PUBLIC_API_URL` renamed to server-only `RUST_API_URL` (browser code never read it; the value is no longer baked into the client bundle).
- **`par-osm-rust` bumped `=0.1.1` → `=0.2.1` (ARC-011 complete).** `source_options` (the seven POI/Overture CLI string parsers + their tests) is donated upstream and consumed here via a `pub use par_osm_rust::source_options::*;` re-export shim. 0.2.x encapsulated `OsmData`, so the terrain/geometry/metadata/GeoJSON paths now read `nodes()` / `ways()` / `ways_by_id()` instead of the private fields, `OvertureParams` literals set `cache_ttl_secs: None`, and test fixtures route through `OsmData::new`.
- Documentation synced to the post-refactor reality (`CLAUDE.md`, `README.md`, `docs/ARCHITECTURE.md`, `docs/DEVELOPER_INFO.md`, `docs/CLI.md`, `docs/WEB_UI.md`, `CONTRIBUTING.md`).

### Removed
- Stale graphify git hooks (`.githooks/post-commit`, `.githooks/post-checkout`) — graphify integration was removed from settings/gitignore/CLAUDE.md in 0.8.0 but the local hook files were left behind, silently re-enabling the integration when `make install-hooks` was run. The `pre-commit` hook (fmt + clippy + test) is unaffected and remains the only hook `install-hooks` configures.
- Truly-dead code flagged by the `#[allow(dead_code)]` audit (3 stale markers + 1 unused function); remaining markers carry intent comments.

---

## [0.8.0] — 2026-05-12

### Added
- **Java Edition support** — generate Minecraft Java Edition (1.18+) worlds alongside Bedrock via `--edition java` CLI flag or `edition` HTTP API parameter
- `--edition <bedrock|java>` flag on `convert`, `fetch-convert`, `overture-convert`, and `terrain-convert` subcommands
- `edition` field in YAML config file (e.g. `edition: java`)
- `edition` parameter in server `/convert`, `/fetch-convert`, `/terrain-convert` request bodies
- `src/world.rs` — `WorldWriter` trait, `Edition` enum, `ChunkData` (shared between editions)
- `src/anvil.rs` — `JavaWorld` implementing `WorldWriter` with Anvil region file writer (`.mca`)
- `src/nbt_be.rs` — Big-endian NBT writer for Java Edition, including `TAG_LIST`, `TAG_LONG_ARRAY`, `TAG_INT_ARRAY`, and Java sign entity encoding
- `Block::java_name()`, `Block::java_block_states()`, `surface_to_java_biome()` for Java Edition block/biome mappings
- Java Edition download packaging as `.zip` (Bedrock continues to use `.mcworld`)
- 36 new tests covering Java block mappings, BE NBT writer, JavaWorld, and Anvil region writer

### Changed
- Pipeline refactored to use `dyn WorldWriter` trait instead of `BedrockWorld` directly
- `ChunkData`, `MIN_Y`, `MAX_Y` moved from `bedrock.rs` to shared `world.rs` module
- Streaming pipeline branches on edition: Bedrock uses LevelDB `ChunkWriter`, Java accumulates in memory and calls `save()`
- `geometry.rs` draw functions accept `&mut dyn WorldWriter`
- `edition` field added to `ConvertParams` and `TerrainParams`
- Server produces `.zip` for Java editions, `.mcworld` for Bedrock

---

## [0.7.0] — 2026-05-07

### Added
- Web Explorer bounding-box draw tool and Overpass URL input with `localStorage` persistence
- `~1/3` compression factor applied to `.mcworld` file size estimate in the export panel
- GitHub Actions CI workflow (Rust fmt/clippy/test + web lint/build)
- SSRF allowlist for user-controlled Overpass URL in both Rust and Next.js layers
- Per-route HTTP upload body limits (100 MB parse, 500 MB convert, 50 MB preview)
- Numeric bounds validation for `scale`, `sea_level`, and `building_height` parameters
- Background job TTL eviction task (15-minute sweep, 2-hour TTL)
- Concurrency cap of 4 simultaneous conversions via `tokio::sync::Semaphore`
- `CORS_ALLOWED_ORIGIN` env var to configure allowed CORS origin (default `http://localhost:8031`)
- Node-typed POI queries (amenity/shop/tourism/leisure/historic) always included in Overpass QL
- `web/src/lib/api-config.ts` — centralises `RUST_API_URL` and timeout constants
- `ConversionParametersForm`, `ConversionControls`, `DownloadProgress` extracted from `ExportPanel`
- Atomic write-then-rename for SRTM HGT files (eliminates mmap TOCTOU race)
- Next.js security headers: `X-Frame-Options`, `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`
- `CONTRIBUTING.md`, `LICENSE` (MIT), `docs/README.md` index, `web/.env.local.example`
- Graphify git hooks for keeping the local code graph current after commits and branch switches

### Changed
- Default map center changed from Sacramento to London (denser OSM coverage, globally recognisable)
- `HeightMap` uses a flat `Vec<i32>` for the streaming path (was `HashMap`); preview path retains `HashMap` fallback
- `ways_by_id` changed from `HashMap<i64, OsmWay>` (clone) to `HashMap<i64, usize>` (index)
- Error messages returned to HTTP clients are now generic; full errors logged server-side only
- ESLint 10 compatibility: `settings.react.version` pinned to avoid removed `getFilename()` API
- `par-osm-rust` now resolves from the published crates.io package instead of a sibling path dependency
- Documentation style guide refreshed for public documentation consistency

### Fixed
- Roads: skip centre line rendering (no yellow slab equivalent in vanilla Bedrock)
- Path traversal via `world_name` parameter — dots, slashes, and control chars stripped at all path construction sites
- `Content-Disposition` header injection in download handler
- Relation tile filter now uses AABB overlap (was point-containment — missed large relations)
- UTF-8 byte-slice panic in `format_sign_text` — uses `chars().take(n)` instead of byte-index slice
- `unwrap()` on infallible `Vec<u8>` writes in `nbt.rs` replaced with `expect()`
- `z-index` applied via `requestAnimationFrame` instead of duplicate `setTimeout(..., 0)` hacks
- Stale `#[allow(dead_code)]` annotations removed across multiple modules

---

## [0.6.0] — 2026-03

### Added
- `terrain-convert` subcommand: generate SRTM-only worlds with no OSM features
- Door orientation support — doors face the correct direction based on wall geometry
- Bounding-box reset button in the Web Explorer
- OSM cache containment lookup — a cached larger area satisfies a smaller request without re-fetching
- `OVERPASS_URL` environment variable override for Overpass mirrors

### Changed
- `main.rs` decomposed into five focused modules: `params.rs`, `sign.rs`, `spatial.rs`, `geometry.rs`, `pipeline.rs`

### Fixed
- `.mcworld` ZIP streaming — no more full in-memory accumulation for large worlds

---

## [0.5.0] — 2026-03

### Added
- Overpass API integration: `fetch-convert` subcommand fetches OSM data by bounding box
- Disk-backed Overpass response cache (SHA-256 keyed, `~/.cache/osm-to-bedrock/overpass/`)
- Feature filter flags: `--no-roads`, `--no-buildings`, `--no-water`, `--no-landuse`, `--no-railways`
- OSM cache (`osm_cache.rs`) with containment lookup — reuses a cached larger area
- Bridge and tunnel rendering (raised/lowered roadbed, barrier walls)
- Building wall straightening (`--wall-straighten-threshold`)

---

## [0.4.0] — 2026-03

### Added
- SRTM elevation support (`--elevation`, `--vertical-scale`): terrain follows real-world height data
- `elevation.rs` and `srtm.rs` modules with bilinear interpolation and auto-download of SRTM tiles
- POI markers (`--poi-markers`): signs placed at amenities, shops, and tourism nodes
- Address signs (`--address-signs`): house number signs on building facades
- Spawn point flags: `--spawn-lat/lon`, `--spawn-x/y/z`
- Rayon parallel chunk processing for faster conversion on multi-core systems

---

## [0.3.0] — 2026-03

### Added
- Web Explorer: Next.js frontend with OpenLayers map, layer toggles, feature inspector, export panel
- HTTP API server (`serve` subcommand) powered by Axum
- API endpoints: `/parse`, `/convert`, `/preview`, `/fetch-convert`, `/status/{id}`, `/download/{id}`
- GeoJSON export (`geojson_export.rs`) for the web frontend
- Street name signs (`--signs`) along named roads using Bedrock sign block entities

### Changed
- CLI restructured to use subcommands: `convert`, `serve`, `fetch-convert`

---

## [0.2.0] — 2026-03

### Added
- Waterway depth and width by type (river, stream, canal, drain)
- Biome assignment in `Data2D` chunks (auto-selected by land use)
- Block variety: more road surface types (concrete, cobblestone, gravel by highway class)
- Landuse polygon fill: parks, farmland, industrial, retail, residential areas

---

## [0.1.0] — 2026-03

### Added
- Initial working converter: `.osm.pbf` → Minecraft Bedrock LevelDB world
- `osm.rs`: PBF parser for nodes and ways
- `convert.rs`: equirectangular lat/lon → block coordinate projection with Bresenham rasterization
- `blocks.rs`: `Block` enum with 44 variants and OSM tag → block mapping
- `bedrock.rs`: `BedrockWorld`, `ChunkData`, SubChunk v8 encoding, LevelDB writer
- `nbt.rs`: minimal little-endian NBT writer (Bedrock uses LE, not BE like Java)
- Three-pass pipeline: collect chunks → fill terrain (bedrock/stone/dirt/grass) → overlay OSM features
- Roads, buildings, water bodies, waterways, forests, land use areas
- `level.dat` with creative mode, commands enabled, correct spawn point

[Unreleased]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/paulrobello/osm-to-bedrock/releases/tag/v0.1.0

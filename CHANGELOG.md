# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

### Changed
- Default map center changed from Sacramento to London (denser OSM coverage, globally recognisable)
- `HeightMap` uses a flat `Vec<i32>` for the streaming path (was `HashMap`); preview path retains `HashMap` fallback
- `ways_by_id` changed from `HashMap<i64, OsmWay>` (clone) to `HashMap<i64, usize>` (index)
- Error messages returned to HTTP clients are now generic; full errors logged server-side only
- ESLint 10 compatibility: `settings.react.version` pinned to avoid removed `getFilename()` API

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

[Unreleased]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/paulrobello/osm-to-bedrock/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/paulrobello/osm-to-bedrock/releases/tag/v0.1.0

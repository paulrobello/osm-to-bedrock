# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
make build        # Release build → target/release/osm-to-bedrock
make test         # cargo test
make lint         # cargo clippy --all-targets -- -D warnings
make fmt          # cargo fmt
make typecheck    # cargo check
make checkall     # fmt + lint + typecheck + test (run before committing)
make install      # cargo install --path .
make serve        # Start Rust API server on port 3002
make dev          # Start both Rust API + Next.js dev servers
make web-dev      # Start Next.js dev server only (port 8031)
make web-build    # Build Next.js frontend
make web-install  # Install web dependencies
make kill         # Kill both dev servers (ports 3002 + 8031)
make web-kill     # Kill Next.js dev server on port 8031

# Run a single test
cargo test test_name

# Run with logging
RUST_LOG=debug cargo run --release -- convert --input map.osm.pbf --output MyWorld/

# Convert via make (pass INPUT and OUTPUT)
make convert INPUT=city.osm.pbf OUTPUT=~/games/minecraft/worlds/MyCity

# Start the API server
RUST_LOG=info cargo run --release -- serve --port 3002
```

## Web Explorer

The `web/` directory contains a Next.js frontend for browsing OSM data on an interactive map and exporting to .mcworld files.

```bash
cd web && bun install    # First time setup
make dev                 # Start both servers (API on 3002, web on 8031)
# Open http://localhost:8031
```

Features: Location search, Overpass API queries, PBF file upload, layer toggles (roads/buildings/water/landuse/signs), feature inspector, bounding box drawing, spawn point placement, conversion with progress tracking.

### Web ↔ Rust API Architecture

The Next.js frontend proxies all backend calls through its own API routes (`web/src/app/api/`) to the Rust server. The Rust API base URL is configured via `NEXT_PUBLIC_API_URL` (default `http://localhost:3002`).

**Rust API endpoints** (`server.rs`):
- `GET  /health` — liveness check
- `POST /parse` — multipart upload `.osm.pbf`, returns GeoJSON + bounds + stats
- `POST /convert` — multipart upload `.osm.pbf` + options JSON, returns job ID
- `POST /fetch-convert` — fetch OSM from Overpass + convert in one step; accepts `overpass_url` override
- `POST /terrain-convert` — SRTM-only world (no OSM features)
- `POST /preview` — generate 3D block preview from PBF
- `GET  /status/{id}` — poll conversion progress
- `GET  /download/{id}` — download `.mcworld` file
- `GET  /cache/areas` — list cached Overpass bbox entries

**Next.js proxy routes** (`web/src/app/api/`): `upload/`, `convert/`, `fetch-convert/`, `status/[id]/`, `download/`, `geocode/`, `overpass/`, `cache/`

Key web components: `MapView` (OpenLayers map), `ExportPanel` (conversion controls), `DataSourcePanel` (PBF upload + Overpass), `LayerPanel` (feature toggles), `FeatureInspector` (click-to-inspect). Map state lives in `useMap` hook; conversion polling in `useConversion` hook. MapView footer shows live Minecraft `/tp` coordinates (click to copy) when a bbox is drawn, using the export panel's `scale` and `seaLevel` params.

## Architecture

This is a Rust CLI that converts OpenStreetMap `.osm.pbf` files into playable Minecraft Bedrock Edition worlds. The pipeline is a single-pass-with-context design:

1. **Parse** (`osm.rs`) — `parse_pbf()` reads all nodes and ways into `OsmData` (HashMap of nodes + Vec of ways). Relations are skipped.
2. **Convert** (`convert.rs`) — `CoordConverter` maps lat/lon → block (x, z) using equirectangular approximation. Bresenham line rasterization and scanline polygon fill handle geometry.
3. **Map blocks** (`blocks.rs`) — OSM tags (`highway=*`, `building`, `landuse=*`, `natural=*`, `waterway=*`) are mapped to a `Block` enum (44 variants). Roads use `RoadStyle` structs with variable width, sidewalks, and centre-line; waterways use `waterway_to_style()` with per-type depth/width defaults.
4. **Build world** (`main.rs`) — Three-pass loop: (1) collect affected chunks, (2) fill terrain layers (bedrock→stone→dirt→grass), (3) overlay OSM features. Buildings get floor + ceiling + perimeter walls. Optional `--signs` flag places street name signs along named roads.
5. **Write** (`bedrock.rs`) — `BedrockWorld` accumulates `ChunkData` in memory, then writes a LevelDB database with SubChunk v8 format (packed block indices + NBT palette) plus `level.dat`.
6. **NBT** (`nbt.rs`) — Minimal little-endian NBT writer (Bedrock uses LE, not BE like Java). Includes `encode_sign_block_entity()` using modern `FrontText`/`BackText` sub-compounds (Bedrock 1.20+).
7. **GeoJSON export** (`geojson_export.rs`) — Converts `OsmData` to GeoJSON `FeatureCollection` for the web frontend. Classifies ways as road/building/water/landuse/railway/other.
8. **HTTP server** (`server.rs`) — Axum-based API with multipart upload, background conversion jobs (tracked via `Arc<Mutex<HashMap>>`), and `.mcworld` ZIP download.
9. **Overpass** (`overpass.rs`) — Builds QL queries, fetches from Overpass API, writes results to disk cache. `default_overpass_url()` resolves `OVERPASS_URL` env var → hardcoded default.
10. **Cache** (`osm_cache.rs`) — Disk cache at `~/.cache/osm-to-bedrock/overpass/` (or `$OVERPASS_CACHE_DIR`). Key = SHA-256 of snapped bbox + filter. `find_containing()` returns a hit when the requested bbox is fully inside a cached larger area.
11. **Elevation** (`elevation.rs`, `srtm.rs`) — Downloads SRTM HGT tiles, builds a height grid, applies to terrain. Activated via `--elevation` flag; `--vertical-scale` controls exaggeration. `--elevation-smoothing` controls median-filter radius (default 1) to eliminate 1-block jitter on roads and buildings.
12. **Filter** (`filter.rs`) — `FeatureFilter` struct (roads/buildings/water/landuse/railways booleans) controls which OSM types are queried and converted.

### Coordinate conventions
- East → +X, North → −Z (Minecraft's north is −Z)
- Blocks are stored XZY order in SubChunks (x*256 + z*16 + y)
- Chunk keys: `[cx: i32 LE][cz: i32 LE][tag: u8]` (9 bytes) or 10 bytes for SubChunks with `[0x2f][sy: u8]`

### Key design decisions
- World is flat at configurable `--sea-level` (default 65); real elevation available via `--elevation` (SRTM)
- SubChunk encoding uses the smallest valid bits-per-block from `[1,2,3,4,5,6,8,16]`
- LevelDB via `rusty-leveldb` with Mojang-compatible zlib/deflate compressors (IDs 0, 2, 4)
- Forests place oak log+leaves trees every 4 blocks within forested polygons
- `run_conversion()` accepts a `progress_cb` callback for progress reporting (used by both CLI and server)
- Overpass cache key is SHA-256 of bbox (snapped to 4 dp) + filter; containment lookup reuses a larger cached area rather than re-fetching
- `OVERPASS_URL` env var overrides the default Overpass endpoint (useful for mirrors when `overpass-api.de` is busy)

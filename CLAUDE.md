# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
make build        # Release build → target/release/osm-to-bedrock
make test         # cargo test
make lint         # cargo clippy --all-targets -- -D warnings
make fmt          # cargo fmt
make typecheck    # cargo check
make web-test     # Run web unit tests (vitest)
make web-check    # Lint + unit-test + build-check the Next.js frontend
make checkall     # fmt + lint + typecheck + test + web-check (run before committing)
make install      # cargo install --path .
make serve        # Start Rust API server on port 3002
make serve-stop   # Gracefully stop the Rust API server on port 3002
make dev          # Start both Rust API + Next.js dev servers
make stop         # Gracefully stop both dev servers (ports 3002 + 8031)
make kill         # Force-kill both dev servers (ports 3002 + 8031)
make web-dev      # Start Next.js dev server only (port 8031)
make web-build    # Build Next.js frontend
make web-install  # Install web dependencies
make web-stop     # Gracefully stop Next.js dev server on port 8031
make web-kill     # Force-kill Next.js dev server on port 8031
make docker-build # Build the Docker image
make docker-run   # Run the Docker container (API on 3002, web on 8031)
make docker-stop  # Stop the running Docker container
make install-hooks # Install pre-commit framework git hook (secret scan + fmt/lint/test)
make pre-commit   # Run all pre-commit hooks repo-wide (secret scan + hygiene + fmt/lint/test)
make pre-commit-update # Bump pinned hook revs in .pre-commit-config.yaml

# Run a single test
cargo test test_name

# Run with logging
RUST_LOG=debug cargo run --release -- convert --input map.osm.pbf --output MyWorld/

# Convert for Java Edition
RUST_LOG=debug cargo run --release -- convert --input map.osm.pbf --output MyWorld/ --edition java

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

The Next.js frontend proxies all backend calls through its own API routes (`web/src/app/api/`) to the Rust server. The Rust API base URL is configured via the server-side `RUST_API_URL` env var (default `http://localhost:3002`); it is not `NEXT_PUBLIC_`-prefixed, so it is never inlined into the browser bundle and can be overridden at runtime without a rebuild.

**Rust API endpoints** (defined in `src/server/mod.rs::build_router_with_state`; 12 routes total):
- `GET  /health` — liveness check (public even when an API key is configured)
- `POST /parse` — multipart upload `.osm.pbf`, returns GeoJSON + bounds + stats
- `POST /convert` — multipart upload `.osm.pbf` + options JSON, returns job ID
- `POST /preview` — generate 3D block preview from PBF
- `POST /fetch-preview` — fetch from Overpass and return preview GeoJSON
- `POST /fetch-block-preview` — fetch from Overpass and return block-level preview
- `POST /fetch-convert` — fetch OSM from Overpass + convert in one step; accepts `overpass_url` override
- `POST /terrain-convert` — SRTM-only world (no OSM features)
- `POST /overture-convert` — build a world from Overture Maps data only
- `GET  /status/{id}` — poll conversion progress
- `GET  /download/{id}` — download `.mcworld` (Bedrock) or `.zip` (Java) file
- `GET  /cache/areas` — list cached Overpass bbox entries

All routes except `/health` require a key in the `Authorization` (or `X-API-Key`) header when `--api-key` / `OSM_TO_BEDROCK_API_KEY` is set. Binding a non-loopback host without a key is refused unless `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1` is set.

**Next.js proxy routes** (`web/src/app/api/`): `health/`, `upload/` → `/parse`, `convert/`, `fetch-convert/`, `terrain-convert/`, `overture-convert/`, `preview/`, `fetch-preview/`, `fetch-block-preview/`, `status/[id]/`, `download/`, `cache/`, `geocode/`, `overpass/`. All proxied through a shared `proxyToRust` helper in `web/src/lib/api-config.ts`.

Key web components: `MapView` (OpenLayers map), `ExportPanel` (conversion controls), `DataSourcePanel` (PBF upload + Overpass), `LayerPanel` (feature toggles), `FeatureInspector` (click-to-inspect). Map state lives in `useMap` hook; conversion polling in `useConversion` hook (single shared `runConversionJob` helper drives all four conversion flows; AbortController aborts in-flight polls on unmount). MapView footer shows live Minecraft `/tp` coordinates (click to copy) when a bbox is drawn, using the export panel's `scale` and `seaLevel` params.

## Architecture

This is a Rust CLI that converts OpenStreetMap `.osm.pbf` files into playable Minecraft Bedrock or Java Edition worlds. The headline design is the **streaming tile pipeline** in `src/pipeline/`: the world is processed in fixed-size tiles (64×64 chunks each) so peak memory stays bounded to one tile regardless of total map area. Both editions target the same `WorldWriter` trait and share `render_osm_features`, so the tile loop is edition-agnostic at the outer layer.

`src/main.rs` is now a 16-LOC shim — all CLI parsing and dispatch lives in `src/cli/` (ARC-008), and the conversion pipeline lives in `src/pipeline/` (ARC-003). The HTTP server lives in `src/server/` (ARC-004). See `docs/ARCHITECTURE.md` for the authoritative write-up; the map below is the short version.

### Module layout

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | 16-LOC binary entry — delegates to `cli::main()` so CLI types can be unit-tested. |
| `src/lib.rs` | Crate root — declares all public modules. |
| `src/cli/{mod,args,convert,cache}.rs` | clap CLI: `Cli`, `Commands` enum, shared flag groups `ConvertCommonArgs` + `BuildingArgs`. Six subcommands: `convert`, `serve`, `fetch-convert`, `terrain-convert`, `overture-convert`, `cache`. |
| `src/pipeline/{mod,render,terrain,preview,decoration,util}.rs` | Conversion pipeline. Entry points: `run_conversion`, `run_conversion_from_data`, `run_pipeline_streaming`, `run_conversion_preview`, `run_preview_from_data`, `run_surface_preview`, `run_terrain_only_to_disk`, `process_tile`. Per-layer `render_*` helpers (QA-006) live in `render.rs`; terrain fill + tile body in `terrain.rs`; in-memory preview entry points in `preview.rs`; POI/tree decoration in `decoration.rs`; `zip_directory`, `format_bytes`, `is_closed_way`, `coord_hash` in `util.rs`. |
| `src/params.rs` | `ConvertParams` + `TerrainParams` structs shared by CLI and server. |
| `src/source_options.rs` | POI source + Overture-failure policy enums shared across convert-family subcommands. |
| `src/convert.rs` | `CoordConverter` (lat/lon → block), Bresenham line rasterization, scanline polygon fill. |
| `src/geometry.rs` | High-level drawing: `draw_road`, `draw_building`, `draw_bridge`, `draw_tunnel`, `draw_waterway`, `draw_roof`. |
| `src/spatial.rs` | `SpatialIndex` (type-bucketed + grid-indexed way lookup), `HeightMap`, `TILE_CHUNKS` constant. |
| `src/sign.rs` | Street-name sign formatting, nearest-road-vector calculation, sign direction. |
| `src/blocks.rs` | `Block` enum (56 variants, grouped by section comment), OSM tag-to-block mapping, `RoadStyle`, `WaterwayStyle`. |
| `src/world.rs` | `WorldWriter` trait (`flush_tile`/`set_tile_bounds`/`save`), `Edition` enum + `create_world`/`create_world_bounded` factory methods, `ChunkData`, shared `ChunkStore` (QA-001) backing both backends. |
| `src/bedrock.rs` | `BedrockWorld` (LevelDB + SubChunk v8) with background `ChunkWriter` thread; `new_streaming` streams tile-by-tile. |
| `src/anvil.rs` | `JavaWorld` (Anvil `.mca` region files + gzip `level.dat` + `session.lock`). Two constructors: in-memory `new`/`new_bounded` (public library API) and `new_streaming`, which the tile pipeline uses — `flush_tile` drains the tile's chunks into 32×32 region buffers and lazily writes each `.mca` once the tile containing its max in-bounds chunk has flushed, bounding peak memory to ~one tile (matching Bedrock). |
| `src/nbt.rs` | Little-endian NBT writer (Bedrock). |
| `src/nbt_be.rs` | Big-endian NBT writer (Java) with `TAG_LIST` / `TAG_LONG_ARRAY` / `TAG_INT_ARRAY`. |
| `src/server/{mod,state,error,auth,options,handlers}.rs` | Axum HTTP API. `build_router_with_state` wires 12 routes; `run` is the public entry. Opt-in `require_api_key` middleware (SEC-001), `enforce_safe_bind` startup guard, `validate_bbox` (SEC-004), explicit body limits (SEC-005), `lock_jobs` mutex-poison recovery (QA-003/SEC-006), generic non-leaking error messages (SEC-002/SEC-008), `spawn_conversion_job` helper (QA-004). |
| `src/geojson_export.rs` | Converts `OsmData` → GeoJSON `FeatureCollection` for the web frontend; classifies ways as road/building/water/landuse/railway/other. |
| `src/metadata.rs` | `WorldMetadata` — writes `world_info.json` after conversion (parameters, timing, source info). |
| `src/config.rs` | YAML config file (`Config` struct) — load/merge/dump with `--config` / `.osm-to-bedrock.yaml` / `~/.config/osm-to-bedrock/config.yaml` search chain. |
| `src/{osm,overpass,osm_cache,filter,elevation,srtm,overture}.rs` | **Thin re-export shims** from the pinned `par-osm-rust = "=0.3.0"` crate (ARC-011). The real parser / Overpass / cache / filter / elevation / SRTM / Overture logic lives there; edits to these 7 stub files are no-ops. Extension work belongs in `par-osm-rust`, not here. |

### Pipeline shape (streaming, tile-based)

1. **Parse** (`osm` shim → `par-osm-rust::parse_osm_file`) — reads all nodes and ways into `OsmData` (HashMap of nodes + Vec of ways + relations + POI/address/tree nodes).
2. **Terrain bounds + height map** (`pipeline::terrain`) — pass 1 computes the block-coordinate bbox; pass 2 pre-computes the surface-Y for every block column in parallel (Rayon), with optional median-filter smoothing.
3. **Tile iteration** — the chunk bbox is divided into `TILE_CHUNKS × TILE_CHUNKS` tiles; each tile runs `world.set_tile_bounds(...)` → `process_tile(...)` (terrain fill + spatially-filtered feature render) → `world.flush_tile()`. Bedrock's `flush_tile` ships encoded SubChunks to a background `ChunkWriter` thread and clears the in-memory chunk map; Java's `flush_tile` drains the tile's chunks into 32×32 region buffers and writes any region whose last contributing tile has flushed. Both editions bound peak memory to ~one tile's worth of `ChunkData`.
4. **Close-out** — `world.save(spawn_x, spawn_y, spawn_z)` writes `level.dat` (+ `session.lock` for Java); `metadata::write_metadata` writes `world_info.json`. The CLI leaves the world as a directory; the server's `zip_and_persist` helper packages it as `.mcworld` (Bedrock) or `.zip` (Java).

### Coordinate conventions
- East → +X, North → −Z (Minecraft's north is −Z)
- Blocks are stored XZY order in SubChunks (x*256 + z*16 + y)
- Chunk keys: `[cx: i32 LE][cz: i32 LE][tag: u8]` (9 bytes) or 10 bytes for SubChunks with `[0x2f][sy: u8]`

### Key design decisions
- World is flat at configurable `--sea-level` (default 65); real elevation available via `--elevation` (SRTM)
- Streaming tile architecture bounds peak memory to one tile's worth of `ChunkData` for both editions: Bedrock flushes encoded SubChunks to a background `ChunkWriter` thread, Java drains each tile into lazily-written 32×32 region buffers (streaming Anvil, ARC-001 — the old `enforce_java_memory_budget` up-front guard was removed once streaming landed)
- SubChunk encoding uses the smallest valid bits-per-block from `[1,2,3,4,5,6,8,16]`
- LevelDB via `rusty-leveldb` with Mojang-compatible zlib/deflate compressors (IDs 0, 2, 4)
- `run_conversion` / `run_conversion_from_data` accept a `progress_cb: &dyn Fn(f32, &str)` callback for progress reporting (used by both CLI and server)
- Overpass cache key is SHA-256 of bbox (snapped to 4 dp) + filter; containment lookup reuses a larger cached area rather than re-fetching
- `OVERPASS_URL` env var overrides the default Overpass endpoint (useful for mirrors when `overpass-api.de` is busy)

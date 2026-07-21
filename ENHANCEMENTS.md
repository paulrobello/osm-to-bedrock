# Enhancement Ideas

Potential features and improvements for osm-to-bedrock, organized by category.

Items marked **Partial** have groundwork already landed; the note describes what remains.

---

## Data Processing

### Custom Block Mappings
Support a user-provided JSON/YAML config file that overrides the default OSM tag → Minecraft block mappings. Lets users customize the look of their worlds.

**Done:** `--block-mapping <path>` loads a YAML file that overrides the four block-returning mappings (building material, road surface, landuse, natural) over the built-in defaults. Targets are the 56 `Block` variants by exact PascalCase name. See `examples/block-mapping.example.yaml`.

### Multi-World Tiling
Split very large regions into multiple adjacent worlds with matching edges, so users can explore a large area across several Minecraft worlds.

---

## Performance

### Progress Estimation
Provide ETA estimates based on chunks completed vs total, processing rate, and historical data. Display in both CLI and web UI.

**Done:** ETA and a smoothed tiles/sec rate are derived from the current run's observed tile throughput inside the pipeline (`ProgressTracker` in `src/pipeline/progress.rs`) and surfaced through the progress callback as `ProgressReport { progress, message, elapsed, eta, rate }`. The CLI prints `~ETA … · X.Y tiles/s · … elapsed` during the tile phase; the server serializes `eta_seconds`/`rate` through `/status`; the web UI shows `~… left · X.Y tiles/s` next to the percentage. `eta`/`rate` are `None` (hidden) outside the tile phase. Current-run rate only — no cross-run persistence.

---

## Testing & Quality

### NBT Round-Trip Tests
Serialize NBT data and verify it deserializes correctly. Test sign block entities, level.dat fields, and palette entries.

**Partial:** `src/nbt.rs` pins SubChunk palette entries, `level.dat` fields, and sign-block-entity text via byte/substring checks, but this crate has no little-endian NBT reader, so writers are not yet verified by a full deserialize round-trip.

### Server API Tests
Test all HTTP endpoints with `axum::test` — upload, convert, poll status, and download. Verify error handling for malformed uploads, missing jobs, and timeouts.

**Done:** `tests/server_api.rs` (23 tests) drives real requests through the Axum router via `tower::ServiceExt::oneshot` against `build_router` / `build_router_with_key`: `/health`, `/cache/areas`, `/parse` (happy path verifying GeoJSON + bounds + stats, plus 400/500 for missing/empty/garbage uploads), the full `/convert` → `/status` → `/download` happy path (verifying the produced `.mcworld` ZIP archive + `Content-Disposition`/`Content-Length` headers), `/convert` error paths, garbage-upload → `JobState::Error` → 422 on `/download`, `/status` + `/download` 404 for unknown jobs, 409-while-running, `/fetch-convert` 400 validation (inverted/continent bbox, malformed JSON body), and SEC-001 auth wired through the router (401 missing/wrong key, 200 Bearer / `X-API-Key`, public `/health`). Fixtures: committed `tests/fixtures/sample.{osm,osm.pbf}` (1 road + 1 building + 1 water; 11 nodes, 3 ways). The pre-existing `src/server/{auth,error,state,options}.rs` unit tests remain as the per-module layer. Open gap: `/convert` parses its multipart `options` field and validates numeric ranges through `anyhow` → HTTP 500, whereas `/fetch-convert` returns 400 for the same shapes — pinned by tests (`*_as_internal_error`) and flagged for a future cleanup; the semaphore-exhaustion "server busy" path also remains uncovered (needs permit injection).

### Snapshot Tests
Generate worlds from known PBF inputs and compare against golden snapshots. Catches regressions in rendering logic.

### Fuzz Testing
Fuzz the PBF parser and NBT writer with random/malformed input to find panics and edge cases.

### Web E2E Tests
Add Playwright tests for the web UI: upload flow, layer toggles, search, bbox drawing, conversion progress, and download.

---

## DevOps & CI

### GitHub Actions CI
Add workflows for `make checkall` on every PR, with matrix testing across platforms (Linux, macOS, Windows).

**Partial:** `.github/workflows/ci.yml` runs Rust (fmt/clippy/test) and web (install/test/build/lint) on every PR and push to `main`, but only on `ubuntu-latest` — no Linux/macOS/Windows matrix.

### Automated Releases
Build release binaries for all platforms on git tag push. Publish `.mcworld` sample outputs as release artifacts.

**Partial:** `.github/workflows/release.yml` builds stripped binaries for Linux (x86_64/ARM64), macOS (x86_64/ARM64), and Windows and creates a GitHub release, but it is triggered manually (`workflow_dispatch`) rather than on tag push, and no `.mcworld` sample worlds are published as artifacts.

### Dependency Updates
Set up Dependabot or Renovate for automated Cargo and npm dependency PRs.

**Done:** `.github/dependabot.yml` (v2 config) opens weekly update PRs across three ecosystems: `cargo` (root `/`), `npm` (`/web`), and `github-actions` (keeps the `uses:` refs in `.github/workflows/*.yml` current). Cargo and npm minor+patch updates are grouped into one PR per ecosystem so majors stay isolated for individual review; action updates arrive one-per-PR. Conventional commit prefixes (`chore(deps)`, `chore(deps-web)`, `chore(ci)`) and `dependencies` + ecosystem labels (`rust`/`npm`/`ci`) are applied — the labels must exist on the repo for Dependabot to attach them.

### Binary Size Optimization
Profile and reduce release binary size with `strip`, LTO, and `opt-level=z`. The converter should be easy to distribute.

**Partial:** Release binaries are `strip`-ped in the release workflow, but `Cargo.toml` has no `[profile.release]` tuning (LTO, `opt-level=z`, `codegen-units=1`) yet.

---

## Developer Experience

### Plugin/Extension System
Allow users to write custom block mapping functions or post-processing hooks in Lua or WASM for advanced customization.

# Progress Estimation (ETA) — Design Spec

**Date:** 2026-07-21
**Enhancement:** ENHANCEMENTS.md → Performance → "Progress Estimation"
**Status:** Approved (design); pending implementation plan

## Summary

Add ETA and processing-rate estimation to the existing progress reporting so users
can see, in both the CLI and the web UI, roughly how long a conversion has left.
ETA is derived from the **current run's** observed tile rate — no cross-run
persistence, no predictive model. The pipeline owns the rate/ETA computation
because it is the only component that knows the phase structure (in particular the
tile count), and emits a richer progress report through the existing callback seam;
the CLI and the HTTP server consume those fields directly.

## Background

Progress reporting today is `(progress: f32, message: &str)`:

- **Pipeline** (`src/pipeline/mod.rs`) calls `progress_cb(f32, &str)` at fixed phase
  fractions: `0.10` (terrain bounds), `0.20` (height map), `0.30` (spatial index),
  `0.35` (start of tile loop), then `0.35 + 0.50 * (tile_num / total_tiles)` per
  tile, then `0.88` (finalize), `0.95` (level.dat), `0.99` (streaming complete),
  `1.0` (done).
- **CLI** (`src/cli/convert.rs`) prints `print_progress` → `[NN%] msg`, or logs the
  message via `log_progress`. No timing.
- **Server** (`src/server/state.rs`, `handlers.rs`) stores `JobState::Running {
  progress, message }` and `/status` serializes exactly `{ state, progress,
  message }`. No job start time is tracked on the `Running` variant.
- **Web** (`web/src/hooks/useConversion.ts`, `web/src/components/DownloadProgress.tsx`)
  polls `/api/status/{id}` every 2 s, consumes `{ state, progress, message }`, and
  renders the percentage + message in `DownloadProgress`. The Next.js status proxy
  passes JSON through unchanged.

The overall `progress` value is **non-linear in wall-clock time**: parsing may move
0 → 0.10 in seconds, the height-map phase 0.10 → 0.20 over a minute, tiles span
0.35 → 0.85 over minutes, and finalize 0.85 → 1.0 in seconds. A naïve
`eta = elapsed / progress * (1 - progress)` is therefore wildly wrong in early
phases and jumps at every phase boundary. The only phase with a clean denominator
and a roughly steady rate is the tile loop (`tile_num / total_tiles`). A good ETA
needs phase awareness, and phase awareness lives in the pipeline.

## Approach

**Pipeline-owned ETA via a richer progress callback (chosen).** The callback grows
from `Fn(f32, &str)` to `Fn(&ProgressReport)`. A `ProgressTracker` inside the
pipeline records the job start and per-tile samples, computes a smoothed
tiles/sec and `eta = remaining_tiles / rate` during the tile phase, and emits
`eta: None` elsewhere. CLI and server both just read the fields.

Rejected alternatives:

- **Consumer-owned ETA, no callback change.** Smaller diff, but rate logic is
  duplicated in CLI and server, and avoiding the non-linear jitter would require
  consumers to hardcode the pipeline's phase fractions (e.g. "0.35 = tiles
  start"), coupling them to pipeline internals.
- **Hybrid (pipeline emits phase/count, consumers compute rate).** Still changes
  the callback signature *and* splits the math across consumers — worst of both.

## Scope

**In scope (v1):**

- New `ProgressReport` struct: `{ progress: f32, message: String, elapsed:
  Duration, eta: Option<Duration>, rate: Option<f32> }`.
- New `ProgressTracker` owning job-start timing and a smoothed tile rate, with an
  injectable clock so it is unit-testable without `Instant::now()`.
- Change the public progress callback type from `&dyn Fn(f32, &str)` to
  `&dyn Fn(&ProgressReport)` across the pipeline entry points, the preview paths,
  the CLI, and the server worker closures.
- CLI renders `~ETA 2m 13s · 4.3 tiles/s` (fields omitted when `None`).
- Server carries `eta_seconds: Option<f64>` and `rate: Option<f32>` on
  `JobState::Running` and serializes them through `/status`.
- Web surfaces ETA + rate in `DownloadProgress.tsx`, hidden while absent.

**Out of scope (v1):**

- Cross-run historical persistence and predictive modeling ("historical data" in
  the ENHANCEMENTS note is satisfied by current-run rate for v1).
- Per-phase ETA breakdowns.
- ETA for the download/zip phase (separate progress surface).
- Locale-specific time formatting. Human-friendly compact formatting only
  (`1m 02s`, `42s`, `12m`, `1h 05m`).

## Components

### 1. `ProgressReport` (new; `src/pipeline/progress.rs`)

```rust
pub struct ProgressReport {
    pub progress: f32,          // 0.0..=1.0, same scale as today
    pub message: String,        // human stage label, same as today
    pub elapsed: Duration,      // wall-clock since job start
    pub eta: Option<Duration>,  // None until the tile phase has rate signal
    pub rate: Option<f32>,      // tiles/sec (EWMA-smoothed); None outside tiles
}
```

A new `src/pipeline/progress.rs` module holds `ProgressReport`, `ProgressTracker`,
the clock abstraction, the formatting helpers, and their unit tests. Declared from
`src/pipeline/mod.rs` (and `src/lib.rs` if needed for re-export).

### 2. `ProgressTracker` (new; `src/pipeline/progress.rs`)

Owns the job-start offset and a ring of recent `(elapsed, tile_num)` samples.

```rust
pub struct ProgressTracker<'a> {
    cb: &'a dyn Fn(&ProgressReport),
    now: Box<dyn Fn() -> Duration + Send + Sync>,  // injectable clock
    start: Duration,                                 // captured at construction
    samples: VecDeque<(Duration, u64)>,              // (elapsed, tile_num), capped
    in_tile_phase: bool,                             // set on first tile(...), cleared on next phase(...)
    rate_ewma: Option<f32>,                          // smoothed tiles/sec, None until 2+ samples
}
```

Methods (each computes fields then invokes `cb`):

- `phase(&mut self, progress: f32, message: &str)` — called at the existing
  milestone sites. Clears tile-specific signal: while not yet in the tile phase,
  `eta`/`rate` are `None`. (Entering the tile phase is the first `tile(...)` call.)
- `tile(&mut self, tile_num: u64, total_tiles: u64)` — called once per tile in the
  loop. Pushes `(elapsed, tile_num)` into the ring (cap ~10 samples), recomputes
  `progress = 0.35 + 0.50 * (tile_num / total_tiles)` to preserve today's mapping,
  recomputes `rate` as an EWMA over the ring, and sets
  `eta = remaining_tiles / rate` (where `remaining_tiles = total_tiles - tile_num`).
- `finish_progress()` / final calls at `0.88`/`0.95`/`0.99`/`1.0` go through
  `phase(...)`; once past the tile phase the tracker stops updating tile rate, so
  `eta`/`rate` return to `None` (the message — "Writing level.dat" etc. — already
  conveys the final stage).

**Rate smoothing.** The instantaneous tiles/sec between two adjacent samples is
`Δtile_num / Δelapsed`. The tracker keeps an EWMA of these per-sample rates
(smoothing factor ~0.3) so a single slow tile (dense city center) does not spike
the ETA. The displayed `rate` is this EWMA; `eta = remaining_tiles / rate`.

**Clock injection.** Construction takes a `now: impl Fn() -> Duration + Send +
Sync`. Production callers pass a closure returning `Instant::now() - start_instant`;
tests pass synthetic durations for deterministic assertions.

### 3. Pipeline wiring (`src/pipeline/mod.rs`, `src/pipeline/terrain.rs`)

- Public entry points `run_conversion`, `run_conversion_from_data`, and
  `run_terrain_only_to_disk` change their `progress_cb` parameter from
  `&dyn Fn(f32, &str)` to `&dyn Fn(&ProgressReport)`.
- Each constructs a `ProgressTracker` wrapping that callback (capturing a real
  wall-clock start) and passes `&mut tracker` down to `run_pipeline_streaming`
  (and the terrain-only path).
- In `run_pipeline_streaming`, the existing `progress_cb(0.10, "...")` …
  `progress_cb(0.30, "...")` milestone calls become `tracker.phase(...)`. The
  `0.35` "Converting in tiles" call becomes `tracker.phase(0.35, "Converting in
  tiles")`. Inside the tile loop, the existing
  `progress_cb(tile_progress, &format!("Tile {tile_num}/{total_tiles}"))` becomes
  `tracker.tile(tile_num as u64, total_tiles as u64)` (the tracker derives
  `progress` and the `Tile n/N` message). The trailing `0.88`/`0.95`/`0.99`/`1.0`
  calls become `tracker.phase(...)`.
- The per-10%-increment `log::info!` is preserved unchanged.

### 4. Preview paths (`src/pipeline/preview.rs`)

`run_conversion_preview`, `run_preview_from_data`, `run_surface_preview`, and the
shared `run_pipeline` adopt the new callback signature. They read only
`report.progress` and `report.message` and ignore `eta`/`rate`/`elapsed` — no
tracker is needed there (previews are fast and don't benefit from ETA). Their
existing milestone calls become `cb(&ProgressReport::simple(progress, message))`,
where `ProgressReport::simple` is a constructor that fills `elapsed: Duration::ZERO`
and `eta`/`rate: None`.

### 5. CLI (`src/cli/convert.rs`)

`print_progress` and `log_progress` change to `fn(report: &ProgressReport)`:

- `print_progress` renders
  `[NN%] msg · ~ETA 2m 13s · 4.3 tiles/s · 1m 02s elapsed`, omitting the `~ETA …`
  and `· X.Y tiles/s` segments when those fields are `None` (so pre-tile phases
  still print just `[NN%] msg`).
- `log_progress` logs `[progress] msg (eta=…, rate=…)` with the same omission rule.

New formatting helpers live in `src/pipeline/progress.rs`:
`format_duration(d: Duration) -> String` and `format_rate(r: f32) -> String`. The
CLI uses these directly. (The web side is TypeScript and re-implements the same
compact strings locally — the two sides need not share code, only agree on
output.)

### 6. Server (`src/server/state.rs`, `src/server/handlers.rs`)

- `JobState::Running` gains `eta_seconds: Option<f64>` and `rate: Option<f32>`:
  ```rust
  Running { progress: f32, message: String, eta_seconds: Option<f64>, rate: Option<f32> }
  ```
- Every site that constructs `JobState::Running` (the initial insert in
  `spawn_conversion_job`, and the per-worker progress closures for `convert`,
  `fetch-convert`, `terrain-convert`, `overture-convert`, plus the elevation
  progress closures) populates the new fields. The main conversion closure
  forwards `report.eta`/`report.rate`; the non-pipeline closures (elevation
  download, initial "Queued") pass `None`.
- `status_handler` serializes them:
  ```json
  { "state": "running", "progress": 0.47, "message": "Tile 120/256",
    "eta_seconds": 73.0, "rate": 4.3 }
  ```
  The `eta_seconds`/`rate` keys are **omitted** from the JSON when `None` (not
  serialized as `null`), so clients can simply check for key presence. Since
  `status_handler` builds responses with the `json!` macro today, the implementer
  should either build a `serde_json::Map` conditionally or introduce a small
  `#[derive(Serialize)]` response struct with `#[serde(skip_serializing_if =
  "Option::is_none")]` on the two new fields.

### 7. Web (`web/src/lib/api.ts`, `web/src/hooks/useConversion.ts`, `web/src/components/DownloadProgress.tsx`)

- Extend `JobStatus` with `eta_seconds?: number` and `rate?: number`.
- Update the inline cast in `useConversion.ts` (`pollStatus`) and add `etaSeconds`
  / `rate` to the hook's state and `UseConversionReturn`.
- In `DownloadProgress.tsx`, render an ETA line alongside the existing `NN%`:
  `~2m 13s left · 4.3 tiles/s`, shown only when `etaSeconds != null`. While
  converting without ETA signal, show the existing message + percentage unchanged.
- A `formatEta(seconds)` / `formatRate(r)` helper mirrors the Rust formatting
  (kept local to the web code; the two sides need not share code, only produce the
  same compact strings).

The Next.js status proxy route (`web/src/app/api/status/[id]/route.ts`) needs no
changes — it passes JSON through untouched.

## Data Flow

```
ProgressTracker (pipeline)
   │ constructs ProgressReport { progress, message, elapsed, eta, rate }
   ▼
Fn(&ProgressReport)
   ├─ CLI: print_progress / log_progress  →  terminal
   └─ Server worker closure → JobState::Running { progress, message, eta_seconds, rate }
                                  ▼
                          GET /status/{id}  →  { state, progress, message, eta_seconds, rate }
                                  ▼ (Next.js proxy, untouched)
                          useConversion poll (every 2 s)
                                  ▼
                          DownloadProgress.tsx  →  browser
```

## Edge Cases / Behavior

- **Pre-tile phases** (parse, terrain bounds, height map, spatial index): `eta`
  and `rate` are `None`. CLI prints just `[NN%] msg`; web hides the ETA line. No
  flickering placeholder.
- **Zero or one tile** (tiny/empty area, or `total_tiles == 0`): the tracker never
  accumulates enough samples for a stable rate; `eta`/`rate` stay `None`. Correct
  — there is nothing meaningful to estimate.
- **First tile**: a single sample yields rate from tile-phase-elapsed so far; the
  EWMA stabilizes within a few tiles. Early ETAs are intentionally coarse.
- **Finalize / level.dat** (`0.88`–`1.0`): the tracker stops updating tile rate, so
  `eta`/`rate` return to `None`. The existing stage message ("Writing level.dat",
  "Streaming conversion complete") conveys the near-done state.
- **Terrain-only (`terrain-convert`) and Overture paths**: they run the same
  pipeline seams, so they get ETA wherever tiles are processed. A terrain-only
  SRTM world with no tiles simply never populates `eta`, which is correct.
- **Rate variance across tiles** (dense city vs. ocean): EWMA smoothing plus
  rounded display (`4.3 tiles/s`, ETA to the nearest second) keeps the displayed
  numbers from thrashing.

## Testing

**Rust unit tests (`src/pipeline/progress.rs`):**

- `format_duration` / `format_rate` produce the compact strings for a matrix of
  inputs.
- `ProgressTracker` driven by an injected synthetic clock:
  - Constant tile rate → reported ETA is within a small tolerance of the true
    remaining time.
  - A single abnormally slow tile → EWMA dampens the ETA spike (assert it stays
    within a bounded factor of the steady-state ETA).
  - Pre-tile `phase(...)` calls → `eta` and `rate` are `None`.
  - After the final `phase(...)` past the tile loop → `eta`/`rate` return to
    `None`.
  - `total_tiles == 0` and `total_tiles == 1` → `eta`/`rate` stay `None`.
  - `progress` reported by `tile(...)` matches the existing
    `0.35 + 0.50 * (tile_num / total_tiles)` mapping (regression guard).

**Existing-test compatibility:**

- `src/server/options.rs` `phase_progress` / `fetch_convert_phase_progress` tests
  are unaffected (they test pure mapping math, not `JobState`).
- Any test that constructs `JobState::Running` directly (`src/server/state.rs`
  panic-recovery test, etc.) is updated to include the new fields (defaults
  `None`).

**Web (vitest):**

- `formatEta` / `formatRate` helpers.
- `DownloadProgress` renders the ETA line when `etaSeconds` is present and omits it
  when absent; the percentage + message still render in both cases.

## Verification

- `make checkall` (fmt + clippy + check + test + web-check) must pass before commit.
- Manual smoke: run a real conversion with `RUST_LOG=debug cargo run --release --
  convert ...` and confirm ETA/rate appear during the tile phase and disappear
  outside it; drive the web UI through a conversion and confirm the ETA line
  appears next to the percentage during tiles.

## Future Extensions (not v1)

- **Light historical persistence**: record per-phase durations keyed by area size
  to seed an ETA *before* the tile phase. The tracker's injected clock and EWMA
  are already structured so this can be added without rework.
- **Per-phase ETA breakdown** (parsing vs. height map vs. tiles vs. finalize).
- **Download/zip-phase ETA** on the separate zip progress surface.

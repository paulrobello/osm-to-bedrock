# Progress Estimation (ETA) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show ETA and tiles/sec rate during conversions in both the CLI and the web UI, derived from the current run's observed tile throughput.

**Architecture:** The pipeline owns a `ProgressTracker` that wraps the caller's progress callback. The callback widens from `Fn(f32, &str)` to `Fn(&ProgressReport)`, where `ProgressReport` adds `elapsed`, `eta: Option<Duration>`, and `rate: Option<f32>`. The tracker computes a smoothed (EWMA) tiles/sec from per-tile samples and `eta = remaining_tiles / rate` during the tile phase only; outside it, `eta`/`rate` are `None`. CLI reads the fields directly; the HTTP server forwards them into `JobState::Running` and serializes them through `/status`; the web UI renders an ETA line next to the existing percentage.

**Tech Stack:** Rust (pipeline + Axum server), TypeScript / Next.js 15 / React / vitest (web).

## Global Constraints

- **Project gate:** every task ends with `make checkall` (fmt + clippy + check + test + web-check) green before commit.
- **Callback signature:** the public pipeline progress callback type is `&dyn Fn(&ProgressReport)` everywhere it appears (pipeline, preview, terrain, CLI, server). External-library callbacks (`par_osm_rust::sources::fetch_map_data`, `crate::srtm::download_tiles_for_bbox`) keep their native `(f32, &str)` signatures and are not changed.
- **ETA semantics:** `eta` and `rate` are `Option`. They are `None` before the tile phase has ≥2 samples and after the tile phase ends. Consumers must hide their ETA UI when `None` (no "ETA: —" placeholder flicker).
- **JSON:** `/status` omits `eta_seconds`/`rate` keys entirely when `None` (never `null`), so web clients check key presence.
- **No persistence:** current-run rate only. No files written for history.
- **Style:** match surrounding code; remove any imports your changes orphan; conventional-commit each task.

---

## File Structure

| File | Responsibility | Task |
|------|----------------|------|
| `src/pipeline/progress.rs` (NEW) | `ProgressReport`, `ProgressReport::simple`, `format_duration`, `format_rate`, `ProgressTracker` + unit tests | 1, 2 |
| `src/pipeline/mod.rs` | Declare `progress` module; widen 3 entry-point signatures; wire `ProgressTracker` into `run_pipeline_streaming` | 3 |
| `src/pipeline/terrain.rs` | Widen 2 signatures; wire `ProgressTracker` into `run_terrain_only_to_disk` tile loop; `simple()` for in-memory `run_terrain_only` | 3 |
| `src/pipeline/preview.rs` | Widen 4 signatures; `simple()` at all call sites | 3 |
| `src/cli/convert.rs` | Rewrite `print_progress`/`log_progress` to take `&ProgressReport` (call sites unchanged) | 3 |
| `src/server/state.rs` | Add `eta_seconds`/`rate` to `JobState::Running`; update tests | 3 |
| `src/server/handlers.rs` | Forward report fields in all closures; serialize in `status_handler`; add `None` fields to external-callback closures | 3 |
| `web/src/lib/api.ts` | Extend `JobStatus` | 4 |
| `web/src/hooks/useConversion.ts` | Thread `etaSeconds`/`rate` through state + return + poll cast | 4 |
| `web/src/components/DownloadProgress.tsx` | Render ETA line; `formatEta`/`formatRate` + vitest | 4 |
| `web/src/components/ExportPanel.tsx` | Destructure `etaSeconds`/`rate` from hook; pass to `DownloadProgress` | 4 |
| `ENHANCEMENTS.md` | Mark "Progress Estimation" **Done** | 5 |

**Why a single wiring task (Task 3) and not several:** the callback *type* is shared across the public seam (one CLI `print_progress`/`log_progress` and several server closures feed every pipeline entry point), so changing it is atomic — there is no intermediate state that compiles. The new module (Tasks 1–2) is built and tested first, in isolation; the atomic migration lands as one coherent, well-stepped task ending green.

---

## Task 1: `ProgressReport` + formatting helpers (new module, TDD)

**Files:**
- Create: `src/pipeline/progress.rs`
- Modify: `src/pipeline/mod.rs` (add `mod progress;` + `pub use`)

**Interfaces:**
- Produces: `pub struct ProgressReport { progress: f32, message: String, elapsed: Duration, eta: Option<Duration>, rate: Option<f32> }`, `ProgressReport::simple(progress: f32, message: &str) -> ProgressReport`, `pub fn format_duration(d: Duration) -> String`, `pub fn format_rate(rate: f32) -> String`.

- [ ] **Step 1: Create the module with types + helpers**

Create `src/pipeline/progress.rs`:

```rust
//! Progress reporting for the conversion pipeline.
//!
//! The pipeline reports progress through a `&dyn Fn(&ProgressReport)` callback.
//! [`ProgressReport`] carries the completion fraction, a human stage label, the
//! elapsed wall-clock since the job started, and — during the tile phase — an
//! ETA and smoothed tiles/sec rate. Outside the tile phase `eta`/`rate` are
//! `None`; consumers should hide their ETA display then.

use std::time::Duration;

/// A single progress update emitted by the pipeline to its callback.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressReport {
    /// Overall completion fraction in `0.0..=1.0`.
    pub progress: f32,
    /// Human-readable stage label (e.g. `"Computing height map"`, `"Tile 12/64"`).
    pub message: String,
    /// Wall-clock time elapsed since the job started.
    pub elapsed: Duration,
    /// Estimated time remaining. `None` until the tile phase has rate signal.
    pub eta: Option<Duration>,
    /// Smoothed processing rate in tiles/sec. `None` outside the tile phase.
    pub rate: Option<f32>,
}

impl ProgressReport {
    /// Minimal report with no timing signal — used by preview paths and any
    /// caller that does not track a rate.
    pub fn simple(progress: f32, message: &str) -> Self {
        Self {
            progress,
            message: message.to_string(),
            elapsed: Duration::ZERO,
            eta: None,
            rate: None,
        }
    }
}

/// Format a `Duration` as a compact human string.
///
/// `0s`, `42s`, `1m 02s`, `2m`, `1h 05m`, `2h`.
pub fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs < 60 {
        format!("{total_secs}s")
    } else if total_secs < 3600 {
        let m = total_secs / 60;
        let s = total_secs % 60;
        if s == 0 {
            format!("{m}m")
        } else {
            format!("{m}m {s:02}s")
        }
    } else {
        let h = total_secs / 3600;
        let m = (total_secs % 3600) / 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m:02}m")
        }
    }
}

/// Format a tiles/sec rate to one decimal place: `4.3 tiles/s`.
pub fn format_rate(rate: f32) -> String {
    format!("{rate:.1} tiles/s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_compact() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(42)), "42s");
        assert_eq!(format_duration(Duration::from_secs(62)), "1m 02s");
        assert_eq!(format_duration(Duration::from_secs(120)), "2m");
        assert_eq!(format_duration(Duration::from_secs(3900)), "1h 05m");
        assert_eq!(format_duration(Duration::from_secs(7200)), "2h");
    }

    #[test]
    fn format_rate_one_decimal() {
        assert_eq!(format_rate(4.3), "4.3 tiles/s");
        assert_eq!(format_rate(2.0), "2.0 tiles/s");
    }

    #[test]
    fn simple_report_has_no_timing() {
        let r = ProgressReport::simple(0.5, "hi");
        assert_eq!(r.progress, 0.5);
        assert_eq!(r.message, "hi");
        assert_eq!(r.elapsed, Duration::ZERO);
        assert!(r.eta.is_none());
        assert!(r.rate.is_none());
    }
}
```

- [ ] **Step 2: Declare the module**

In `src/pipeline/mod.rs`, add with the other `mod`/`pub use` declarations near the top (e.g. next to the terrain/preview declarations):

```rust
pub mod progress;
pub use progress::{format_duration, format_rate, ProgressReport, ProgressTracker};
```

(`ProgressTracker` is forward-declared here; it is added in Task 2. If the compiler warns about an unresolved import, leave only `ProgressReport, format_duration, format_rate` for now and add `ProgressTracker` in Task 2.)

- [ ] **Step 3: Run the tests**

Run: `cargo test pipeline::progress --lib`
Expected: 3 tests PASS.

- [ ] **Step 4: Format + lint + commit**

```bash
make fmt
make lint
git add src/pipeline/progress.rs src/pipeline/mod.rs
git commit -m "feat(pipeline): add ProgressReport + formatting helpers"
```

---

## Task 2: `ProgressTracker` (TDD, injectable clock)

**Files:**
- Modify: `src/pipeline/progress.rs` (append `ProgressTracker` + tests)

**Interfaces:**
- Consumes: `ProgressReport` from Task 1.
- Produces:
  - `pub struct ProgressTracker<'a>` constructed via `ProgressTracker::new(cb: &'a dyn Fn(&ProgressReport), now: impl Fn() -> Duration + Send + Sync + 'static) -> Self`
  - `pub fn phase(&mut self, progress: f32, message: &str)` — emits a non-tile milestone; clears rate/eta to `None`.
  - `pub fn tile(&mut self, tile_num: u64, total_tiles: u64, progress: f32, message: &str)` — records a sample, updates EWMA rate, sets `eta = remaining / rate`; needs ≥2 samples before `rate`/`eta` are `Some`.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `src/pipeline/progress.rs`. The callback closure is built inline in each test (a `ProgressTracker` borrows the callback, so the callback must live in the same scope as the tracker — a helper returning a tracker would not compile):

```rust
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Shared elapsed-millis clock for deterministic timing. Returns the
    /// controllable handle and the `now` closure the tracker consumes.
    fn clock_pair() -> (Arc<AtomicU64>, impl Fn() -> Duration + Send + Sync + 'static) {
        let clock = Arc::new(AtomicU64::new(0));
        (clock.clone(), move || Duration::from_millis(clock.load(Ordering::Relaxed)))
    }

    #[test]
    fn phase_emits_no_eta_and_tracks_elapsed() {
        let reports = Rc::new(RefCell::new(Vec::<ProgressReport>::new()));
        let (clock, now) = clock_pair();
        let cb = |r: &ProgressReport| reports.borrow_mut().push(r.clone());
        let mut t = ProgressTracker::new(&cb, now);
        clock.store(1_000, Ordering::Relaxed);
        t.phase(0.10, "Computing terrain bounds");
        let r = reports.borrow()[0].clone();
        assert_eq!(r.progress, 0.10);
        assert_eq!(r.message, "Computing terrain bounds");
        assert_eq!(r.elapsed, Duration::from_secs(1));
        assert!(r.eta.is_none());
        assert!(r.rate.is_none());
    }

    #[test]
    fn tile_rate_and_eta_after_two_samples() {
        // 10 tiles, 1 tile/sec.
        let reports = Rc::new(RefCell::new(Vec::<ProgressReport>::new()));
        let (clock, now) = clock_pair();
        let cb = |r: &ProgressReport| reports.borrow_mut().push(r.clone());
        let mut t = ProgressTracker::new(&cb, now);
        clock.store(1_000, Ordering::Relaxed);
        t.tile(1, 10, 0.36, "Tile 1/10");
        clock.store(2_000, Ordering::Relaxed);
        t.tile(2, 10, 0.37, "Tile 2/10");
        let v: Vec<ProgressReport> = reports.borrow().clone();
        assert!(v[0].rate.is_none(), "first tile has no prior sample");
        assert!(v[0].eta.is_none());
        let rate = v[1].rate.expect("rate after 2 samples");
        assert!((rate - 1.0).abs() < 1e-2, "rate ~1.0 tiles/s, got {rate}");
        let eta = v[1].eta.expect("eta after 2 samples");
        assert_eq!(eta, Duration::from_secs(8), "8 remaining at 1/s");
    }

    #[test]
    fn ewma_dampens_single_slow_tile() {
        // steady 1 tile/s for tiles 1..5, then tile 6 takes 10s.
        let reports = Rc::new(RefCell::new(Vec::<ProgressReport>::new()));
        let (clock, now) = clock_pair();
        let cb = |r: &ProgressReport| reports.borrow_mut().push(r.clone());
        let mut t = ProgressTracker::new(&cb, now);
        for i in 1..=5 {
            clock.store(i * 1_000, Ordering::Relaxed);
            t.tile(i as u64, 50, 0.36 + i as f32 * 0.01, "x");
        }
        let steady_eta = reports.borrow().last().unwrap().eta.unwrap();
        clock.store(5_000 + 10_000, Ordering::Relaxed);
        t.tile(6, 50, 0.42, "x");
        let spike_eta = reports.borrow().last().unwrap().eta.unwrap();
        // EWMA must not let one slow tile blow the eta up by ~10x.
        assert!(
            spike_eta < steady_eta * 5,
            "spike eta {spike_eta:?} should stay within 5x of steady {steady_eta:?}"
        );
    }

    #[test]
    fn total_zero_or_one_tile_has_no_eta() {
        let reports = Rc::new(RefCell::new(Vec::<ProgressReport>::new()));
        let (clock, now) = clock_pair();
        let cb = |r: &ProgressReport| reports.borrow_mut().push(r.clone());
        let mut t = ProgressTracker::new(&cb, now);
        clock.store(1_000, Ordering::Relaxed);
        t.tile(0, 0, 0.35, "Tile 1/0");
        assert!(reports.borrow()[0].eta.is_none());

        let reports2 = Rc::new(RefCell::new(Vec::<ProgressReport>::new()));
        let (clock2, now2) = clock_pair();
        let cb2 = |r: &ProgressReport| reports2.borrow_mut().push(r.clone());
        let mut t2 = ProgressTracker::new(&cb2, now2);
        clock2.store(1_000, Ordering::Relaxed);
        t2.tile(1, 1, 0.85, "Tile 1/1"); // last tile → remaining 0
        assert!(reports2.borrow()[0].eta.is_none());
    }

    #[test]
    fn phase_after_tiles_clears_eta() {
        let reports = Rc::new(RefCell::new(Vec::<ProgressReport>::new()));
        let (clock, now) = clock_pair();
        let cb = |r: &ProgressReport| reports.borrow_mut().push(r.clone());
        let mut t = ProgressTracker::new(&cb, now);
        clock.store(1_000, Ordering::Relaxed);
        t.tile(1, 10, 0.36, "Tile 1/10");
        clock.store(2_000, Ordering::Relaxed);
        t.tile(2, 10, 0.37, "Tile 2/10");
        clock.store(2_500, Ordering::Relaxed);
        t.phase(0.88, "Finalizing");
        let last = reports.borrow().last().unwrap().clone();
        assert!(last.eta.is_none());
        assert!(last.rate.is_none());
        assert_eq!(last.message, "Finalizing");
    }
```

- [ ] **Step 2: Run tests to verify they fail (compile error)**

Run: `cargo test pipeline::progress --lib`
Expected: FAIL — `ProgressTracker` not defined.

- [ ] **Step 3: Implement `ProgressTracker`**

Append above the `#[cfg(test)] mod tests` block in `src/pipeline/progress.rs`:

```rust
/// EWMA smoothing factor for the per-sample tile rate (higher = more reactive).
const RATE_EWMA_ALPHA: f32 = 0.3;

/// Tracks job timing + tile throughput and emits [`ProgressReport`]s through a
/// caller-supplied callback. The clock is injected so the rate/ETA math is
/// deterministic in tests.
pub struct ProgressTracker<'a> {
    cb: &'a dyn Fn(&ProgressReport),
    now: Box<dyn Fn() -> Duration + Send + Sync>,
    start: Duration,
    last_sample: Option<(Duration, u64)>,
    rate_ewma: Option<f32>,
}

impl<'a> ProgressTracker<'a> {
    /// Construct a tracker whose elapsed time is measured from the moment of
    /// construction using the injected `now` clock.
    pub fn new(
        cb: &'a dyn Fn(&ProgressReport),
        now: impl Fn() -> Duration + Send + Sync + 'static,
    ) -> Self {
        let start = now();
        Self {
            cb,
            now: Box::new(now),
            start,
            last_sample: None,
            rate_ewma: None,
        }
    }

    fn elapsed(&self) -> Duration {
        (self.now)().saturating_sub(self.start)
    }

    /// Emit a non-tile milestone (parse, height map, finalize, …). Clears any
    /// tile-rate signal so `eta`/`rate` are `None`.
    pub fn phase(&mut self, progress: f32, message: &str) {
        self.last_sample = None;
        self.rate_ewma = None;
        self.emit(progress, message, None, None);
    }

    /// Emit a per-tile update. Updates the smoothed rate from the interval since
    /// the previous sample and derives `eta = remaining_tiles / rate`. Requires
    /// at least two samples before `rate`/`eta` are populated.
    pub fn tile(&mut self, tile_num: u64, total_tiles: u64, progress: f32, message: &str) {
        let elapsed = self.elapsed();
        if let Some((prev_elapsed, prev_tile)) = self.last_sample {
            let dt = elapsed.saturating_sub(prev_elapsed).as_secs_f32();
            let dn = tile_num.saturating_sub(prev_tile) as f32;
            if dt > 0.0 {
                let instantaneous = dn / dt;
                self.rate_ewma = Some(match self.rate_ewma {
                    Some(prev) => RATE_EWMA_ALPHA * instantaneous + (1.0 - RATE_EWMA_ALPHA) * prev,
                    None => instantaneous,
                });
            }
        }
        self.last_sample = Some((elapsed, tile_num));

        let (eta, rate) = match (self.rate_ewma, total_tiles.checked_sub(tile_num)) {
            (Some(r), Some(remaining)) if r > 0.0 && remaining > 0 => {
                (Some(Duration::from_secs_f32(remaining as f32 / r)), Some(r))
            }
            (Some(r), _) => (None, Some(r)),
            _ => (None, None),
        };
        self.emit(progress, message, eta, rate);
    }

    fn emit(&self, progress: f32, message: &str, eta: Option<Duration>, rate: Option<f32>) {
        (self.cb)(&ProgressReport {
            progress,
            message: message.to_string(),
            elapsed: self.elapsed(),
            eta,
            rate,
        });
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test pipeline::progress --lib`
Expected: all 8 tests PASS.

- [ ] **Step 5: Format + lint + commit**

```bash
make fmt
make lint
git add src/pipeline/progress.rs
git commit -m "feat(pipeline): add ProgressTracker with EWMA tile-rate + ETA"
```

---

## Task 3: Wire the tracker in and widen the callback signature (atomic migration)

This is the one task that must land atomically to keep the tree compiling, because the callback type is shared across the whole seam. Work through the files in order; do not commit until `make checkall` is green.

**Files:**
- Modify: `src/pipeline/mod.rs`, `src/pipeline/terrain.rs`, `src/pipeline/preview.rs`, `src/cli/convert.rs`, `src/server/state.rs`, `src/server/handlers.rs`

**Interfaces:**
- Consumes: `ProgressReport`, `ProgressTracker`, `format_duration`, `format_rate` (Tasks 1–2).
- Produces: every `progress_cb: &dyn Fn(f32, &str)` becomes `progress_cb: &dyn Fn(&ProgressReport)`; `JobState::Running` gains `eta_seconds: Option<f64>` and `rate: Option<f32>`; `/status` serializes them (omitted when `None`).

- [ ] **Step 1: `src/pipeline/mod.rs` — entry-point signatures + direct calls**

1. Ensure the `pub use` from Task 1 Step 2 includes `ProgressTracker` (add it now).
2. `run_conversion` (line ~87): change `progress_cb: &dyn Fn(f32, &str)` → `progress_cb: &dyn Fn(&ProgressReport)`.
   - Line ~93 `progress_cb(0.0, "Parsing OSM data");` → `progress_cb(&ProgressReport::simple(0.0, "Parsing OSM data"));`
   - Line ~116 `progress_cb(1.0, "Conversion complete");` → `progress_cb(&ProgressReport::simple(1.0, "Conversion complete"));`
   - (Line ~105 `run_pipeline_streaming(data, params, progress_cb)?;` stays as-is — pass-through, signature changed below.)
3. `run_conversion_from_data` (line ~175): same signature change.
   - Line ~194 `progress_cb(1.0, "Conversion complete");` → `progress_cb(&ProgressReport::simple(1.0, "Conversion complete"));`
4. `run_pipeline_streaming` (line ~209): same signature change. Immediately after the `let conv = ...` / setup (before `// Pass 1`), add a tracker + a wall-clock helper:

```rust
let mut tracker = ProgressTracker::new(progress_cb, wall_clock());
```

and near the top of the file (or just above `run_pipeline_streaming`) add a `pub(crate)` helper so `terrain.rs` can reuse it:

```rust
pub(crate) fn wall_clock() -> Box<dyn Fn() -> std::time::Duration + Send + Sync> {
    let start = std::time::Instant::now();
    Box::new(move || start.elapsed())
}
```

Then convert every `progress_cb(...)` inside `run_pipeline_streaming`:
- ~226 `progress_cb(0.10, "Computing terrain bounds");` → `tracker.phase(0.10, "Computing terrain bounds");`
- ~254 `progress_cb(0.20, "Computing height map");` → `tracker.phase(0.20, "Computing height map");`
- ~304 `progress_cb(0.30, "Building spatial index");` → `tracker.phase(0.30, "Building spatial index");`
- ~349 `progress_cb(0.35, "Converting in tiles");` → `tracker.phase(0.35, "Converting in tiles");`
- ~370–371 (the tile loop):
  ```rust
  let tile_progress = 0.35 + 0.50 * (tile_num as f32 / total_tiles as f32);
  tracker.tile(tile_num as u64, total_tiles as u64, tile_progress, &format!("Tile {tile_num}/{total_tiles}"));
  ```
- ~422 `progress_cb(0.88, finalize_msg);` → `tracker.phase(0.88, finalize_msg);` (adjust borrow if `finalize_msg` is `String` — use `&finalize_msg` or `.as_str()` as the original required; the existing call already compiled, so match its exact argument form).
- ~424 `progress_cb(0.95, "Writing level.dat");` → `tracker.phase(0.95, "Writing level.dat");`
- ~426 `progress_cb(0.99, "Streaming conversion complete");` → `tracker.phase(0.99, "Streaming conversion complete");`

- [ ] **Step 2: `src/pipeline/terrain.rs` — wire tracker into the terrain-only tile loop**

1. `run_terrain_only_to_disk` (line ~574): change signature → `progress_cb: &dyn Fn(&ProgressReport)`. At its top add `let mut tracker = ProgressTracker::new(progress_cb, crate::pipeline::wall_clock());` (make `wall_clock` `pub(crate)` in `mod.rs`, or duplicate the helper here — prefer reusing: mark `wall_clock` `pub(crate)` in Step 1).
2. Convert its calls:
   - ~514, ~538, ~565 (in `run_terrain_only`, the in-memory fn starting ~465) and ~704, ~811, ~815 (milestones) → `progress_cb(&ProgressReport::simple(<frac>, "<msg>"));`
   - ~650 (the tile loop with `total_tiles`) →
     ```rust
     tracker.tile(
         tile_idx as u64 + 1,
         total_tiles as u64,
         progress,
         &format!("Filling terrain tile {}/{total_tiles}", tile_idx + 1),
     );
     ```
   - ~740: read the surrounding lines (grep said `progress_cb(` here). If it is a second counted loop, use `tracker.tile(...)`; otherwise `progress_cb(&ProgressReport::simple(...))`.
3. `run_terrain_only` (signature at ~465): change → `&dyn Fn(&ProgressReport)`; calls ~514/538/565 → `progress_cb(&ProgressReport::simple(<frac>, "<msg>"));`.

- [ ] **Step 3: `src/pipeline/preview.rs` — signatures + `simple()`**

All four signatures (lines ~48, ~72, ~89, ~342) → `progress_cb: &dyn Fn(&ProgressReport)`. Every `progress_cb(<frac>, "<msg>");` call in this file (lines ~53, ~105, ~113, ~157, ~251, ~324, ~328, ~367, ~394, ~449, ~543) → `progress_cb(&ProgressReport::simple(<frac>, "<msg>"));`.

- [ ] **Step 4: `src/cli/convert.rs` — rewrite the two display functions**

Add to the `use crate::pipeline::{...}` import (line ~18): `ProgressReport`, `format_duration`, `format_rate`. Add `use std::time::Duration;` if not already imported.

Replace `log_progress` and `print_progress` (lines ~381–387):

```rust
fn log_progress(report: &ProgressReport) {
    let mut msg = format!("[progress] {}", report.message);
    if let Some(eta) = report.eta {
        msg.push_str(&format!(
            " (eta={}, rate={:.1} tiles/s)",
            format_duration(eta),
            report.rate.unwrap_or(0.0)
        ));
    }
    log::info!("{msg}");
}

fn print_progress(report: &ProgressReport) {
    let mut line = format!("[{:3.0}%] {}", report.progress * 100.0, report.message);
    if let Some(eta) = report.eta {
        line.push_str(&format!(" · ~ETA {}", format_duration(eta)));
    }
    if let Some(rate) = report.rate {
        line.push_str(&format!(" · {}", format_rate(rate)));
    }
    if report.elapsed > Duration::ZERO {
        line.push_str(&format!(" · {} elapsed", format_duration(report.elapsed)));
    }
    println!("{line}");
}
```

The call sites at lines ~68, ~122, ~253, ~324, ~378 (`&log_progress` / `&print_progress`) need **no change** — the function items coerce to the new `&dyn Fn(&ProgressReport)`.

- [ ] **Step 5: `src/server/state.rs` — extend `JobState::Running`**

Change the variant (lines ~34–37):

```rust
Running {
    progress: f32,
    message: String,
    eta_seconds: Option<f64>,
    rate: Option<f32>,
},
```

Update the two test constructions (lines ~325 and ~342) by adding `eta_seconds: None, rate: None,`.

- [ ] **Step 6: `src/server/handlers.rs` — forward fields + serialize**

a) `status_handler` (line ~1194): replace the `Running` arm with a `serde_json::Map` build so `None` fields are omitted:

```rust
JobState::Running { progress, message, eta_seconds, rate } => {
    let mut m = serde_json::Map::new();
    m.insert("state".into(), serde_json::json!("running"));
    m.insert("progress".into(), serde_json::json!(progress));
    m.insert("message".into(), serde_json::json!(message));
    if let Some(eta) = eta_seconds {
        m.insert("eta_seconds".into(), serde_json::json!(eta));
    }
    if let Some(r) = rate {
        m.insert("rate".into(), serde_json::json!(r));
    }
    Ok(Json(serde_json::Value::Object(m)))
}
```

b) The initial `Running` insert in `spawn_conversion_job` (line ~80): add `eta_seconds: None, rate: None,`.

c) Every closure that feeds **our pipeline** — the `run_conversion` closure (~484), the `run_terrain_only_to_disk` closure (~996), and any `run_conversion_from_data` closures (~1072, ~1161) — changes from `&|progress, msg| { ... }` to:

```rust
&|report: &crate::pipeline::ProgressReport| {
    jobs_for_progress.insert(
        jid_for_progress.clone(),
        JobState::Running {
            progress: report.progress,
            message: report.message.clone(),
            eta_seconds: report.eta.map(|d| d.as_secs_f64()),
            rate: report.rate,
        },
    );
}
```

(Adjust the cloned `jobs_*`/`jid_*` binding names to match each site exactly. Read each of the four sites; they all follow the `/convert` pattern at lines 484–492.)

d) Every closure that feeds an **external** library — `par_osm_rust::sources::fetch_map_data` (line ~822) and `crate::srtm::download_tiles_for_bbox` (line ~1337) — keeps its `(progress, msg)` / `(fraction, message)` signature (we do not control those APIs) and just gains `eta_seconds: None, rate: None,` inside its `JobState::Running { ... }`.

- [ ] **Step 7: Verify the full gate**

Run: `make checkall`
Expected: fmt + clippy + check + test + web-check all PASS. Fix every error (typical ones: a missed `progress_cb(...)` site, a `JobState::Running` literal missing the new fields, or an orphaned import).

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(pipeline): thread ProgressReport (ETA + rate) through pipeline, CLI, and server"
```

---

## Task 4: Surface ETA + rate in the web UI

**Files:**
- Modify: `web/src/lib/api.ts`, `web/src/hooks/useConversion.ts`, `web/src/components/DownloadProgress.tsx`, `web/src/components/ExportPanel.tsx`

**Interfaces:**
- Consumes: the new `eta_seconds?` / `rate?` fields on the `/status` JSON (Task 3).
- Produces: an ETA line in `DownloadProgress` next to the percentage, visible only when `etaSeconds != null`.

- [ ] **Step 1: Extend `JobStatus` (`web/src/lib/api.ts`)**

Add the two optional fields to the `JobStatus` interface (lines ~69–73):

```ts
export interface JobStatus {
  state: string;
  progress: number;
  message: string;
  eta_seconds?: number;
  rate?: number;
}
```

- [ ] **Step 2: Thread state through `useConversion` (`web/src/hooks/useConversion.ts`)**

1. In the inline cast inside `pollStatus` (lines ~202–206) add the fields:
   ```ts
   const data = (await res.json()) as {
     state: string;
     progress: number;
     message: string;
     eta_seconds?: number;
     rate?: number;
   };
   ```
2. After the existing `setProgress`/`setMessage`/`setStatus` calls (lines ~208–210), add:
   ```ts
   setEtaSeconds(typeof data.eta_seconds === 'number' ? data.eta_seconds : null);
   setRate(typeof data.rate === 'number' ? data.rate : null);
   ```
3. Add state (near the other `useState` declarations, ~lines 94–103):
   ```ts
   const [etaSeconds, setEtaSeconds] = useState<number | null>(null);
   const [rate, setRate] = useState<number | null>(null);
   ```
4. Add `etaSeconds` and `rate` to the `UseConversionReturn` return type (lines ~49–74), the returned object, and zero them in `reset()` (~lines 115–131): `setEtaSeconds(null); setRate(null);`.

- [ ] **Step 3: Render the ETA line (`web/src/components/DownloadProgress.tsx`)**

1. Add `etaSeconds: number | null` and `rate: number | null` to the component props.
2. Add helpers near the top of the file (module scope):
   ```ts
   function formatEta(totalSeconds: number): string {
     const s = Math.round(totalSeconds);
     if (s < 60) return `${s}s`;
     if (s < 3600) {
       const m = Math.floor(s / 60);
       const rem = s % 60;
       return rem === 0 ? `${m}m` : `${m}m ${String(rem).padStart(2, '0')}s`;
     }
     const h = Math.floor(s / 3600);
     const mm = Math.floor((s % 3600) / 60);
     return mm === 0 ? `${h}h` : `${h}h ${String(mm).padStart(2, '0')}m`;
   }
   function formatRate(r: number): string {
     return `${r.toFixed(1)} tiles/s`;
   }
   ```
3. In the conversion-progress JSX (lines ~47–70), next to the existing `{progressValue}%`, render the ETA line only when present, e.g.:
   ```tsx
   {etaSeconds != null && (
     <span className="ml-2 text-xs text-muted-foreground">
       ~{formatEta(etaSeconds)} left
       {rate != null && ` · ${formatRate(rate)}`}
     </span>
   )}
   ```
   (Match the surrounding element/className conventions — read the existing JSX at lines 47–70 and slot the span into the same row as the percentage.)

- [ ] **Step 4: Pass the props from `ExportPanel` (`web/src/components/ExportPanel.tsx`)**

Where the hook is destructured (lines ~89–103), add `etaSeconds` and `rate`. Where `<DownloadProgress … />` is rendered (lines ~283–294), add `etaSeconds={etaSeconds} rate={rate}`.

- [ ] **Step 5: Add vitest tests for the helpers**

Create or extend the component's test file (e.g. `web/src/components/DownloadProgress.test.ts`):

```ts
import { describe, it, expect } from 'vitest';

// If the helpers are not exported, copy them into the test or export them from
// DownloadProgress.tsx (preferred: `export function formatEta(...)`).
import { formatEta, formatRate } from './DownloadProgress';

describe('formatEta', () => {
  it('formats seconds, minutes, and hours compactly', () => {
    expect(formatEta(0)).toBe('0s');
    expect(formatEta(42)).toBe('42s');
    expect(formatEta(62)).toBe('1m 02s');
    expect(formatEta(120)).toBe('2m');
    expect(formatEta(3900)).toBe('1h 05m');
  });
});

describe('formatRate', () => {
  it('formats tiles/sec to one decimal', () => {
    expect(formatRate(4.3)).toBe('4.3 tiles/s');
    expect(formatRate(2)).toBe('2.0 tiles/s');
  });
});
```

Export `formatEta` and `formatRate` from `DownloadProgress.tsx` so the test can import them.

- [ ] **Step 6: Verify the web gate**

Run: `make web-check`
Expected: lint + unit tests + build-check PASS.

- [ ] **Step 7: Commit**

```bash
git add web/
git commit -m "feat(web): show ETA + tiles/sec next to conversion progress"
```

---

## Task 5: Mark the enhancement done + final verification

**Files:**
- Modify: `ENHANCEMENTS.md`

- [ ] **Step 1: Update ENHANCEMENTS.md**

Replace the "Progress Estimation" entry's `**Partial:**` paragraph (line ~26) with a `**Done:**` paragraph:

```markdown
**Done:** ETA and a smoothed tiles/sec rate are derived from the current run's observed tile throughput inside the pipeline (`ProgressTracker` in `src/pipeline/progress.rs`) and surfaced through the progress callback as `ProgressReport { progress, message, elapsed, eta, rate }`. The CLI prints `~ETA … · X.Y tiles/s · … elapsed` during the tile phase; the server serializes `eta_seconds`/`rate` through `/status`; the web UI shows `~… left · X.Y tiles/s` next to the percentage. `eta`/`rate` are `None` (hidden) outside the tile phase. Current-run rate only — no cross-run persistence.
```

- [ ] **Step 2: Final full gate**

Run: `make checkall`
Expected: everything PASS.

- [ ] **Step 3: Commit**

```bash
git add ENHANCEMENTS.md
git commit -m "docs: mark Progress Estimation (ETA) done in ENHANCEMENTS"
```

- [ ] **Step 4: Manual smoke (operator step, not blocking)**

Run a real conversion and confirm ETA/rate appear during the tile phase and disappear outside it:
```bash
RUST_LOG=debug cargo run --release -- convert --input <map.osm.pbf> --output /tmp/EtaSmoke/
```
Then `make dev`, run a conversion in the web UI, and confirm the `~… left · X.Y tiles/s` line appears next to the percentage during tiles.

---

## Verification Summary

- **Unit (Rust):** `ProgressTracker` rate/ETA/EWMA/edge-case tests + formatter tests (`cargo test pipeline::progress`).
- **Unit (web):** `formatEta`/`formatRate` (vitest).
- **Gate:** `make checkall` green after every task.
- **Manual:** CLI + web smoke confirming ETA appears/hides at the right phases.

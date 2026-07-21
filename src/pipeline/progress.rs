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

    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Shared elapsed-millis clock for deterministic timing. Returns the
    /// controllable handle and the `now` closure the tracker consumes.
    fn clock_pair() -> (
        Arc<AtomicU64>,
        impl Fn() -> Duration + Send + Sync + 'static,
    ) {
        let clock = Arc::new(AtomicU64::new(0));
        (clock.clone(), move || {
            Duration::from_millis(clock.load(Ordering::Relaxed))
        })
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
        for i in 1u64..=5 {
            clock.store(i * 1_000, Ordering::Relaxed);
            t.tile(i, 50, 0.36 + i as f32 * 0.01, "x");
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
}

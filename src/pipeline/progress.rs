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

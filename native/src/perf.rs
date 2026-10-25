//! Frame statistics for the performance HUD and the stress report: how
//! often frames come, how long each takes to build, and how many the
//! display had to skip. Pure arithmetic over timestamps, no UI.
//!
//! A frame counts towards the interval and drop figures only when it was
//! expected: something asked for it (an animation frame request, or data
//! that changed what is shown). An idle console draws nothing, so the long
//! gaps between idle frames are not drops.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

/// Frames kept for the rolling figures (about four seconds at 120 Hz).
const WINDOW: usize = 512;

/// Assumed until frames that follow each other reveal the display's rate.
const DEFAULT_REFRESH_MS: f64 = 1000.0 / 60.0;

/// Percentiles of a set of milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spread {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

impl Spread {
    /// Nearest-rank percentiles; all zero for no samples.
    pub fn of(samples: impl IntoIterator<Item = f64>) -> Self {
        let mut sorted: Vec<f64> = samples.into_iter().filter(|ms| ms.is_finite()).collect();
        if sorted.is_empty() {
            return Self::default();
        }
        sorted.sort_by(f64::total_cmp);
        let rank = |p: f64| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let index = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1;
            sorted[index]
        };
        Self {
            p50: rank(0.50),
            p95: rank(0.95),
            max: sorted[sorted.len() - 1],
        }
    }
}

impl fmt::Display for Spread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "p50 {:.2}  p95 {:.2}  max {:.2}",
            self.p50, self.p95, self.max
        )
    }
}

/// Frames the display skipped between a request and the frame that
/// answered it, `delay_ms` later: none within half a refresh of the next
/// vsync, one per refresh after that.
pub fn dropped(delay_ms: f64, refresh_ms: f64) -> u64 {
    if !(delay_ms.is_finite() && refresh_ms > 0.0) {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (delay_ms / refresh_ms).round() as u64;
    frames.saturating_sub(1)
}

/// Display refresh rates the estimate snaps to.
const RATES_HZ: [f64; 11] = [
    30.0, 48.0, 50.0, 60.0, 75.0, 90.0, 100.0, 120.0, 144.0, 165.0, 240.0,
];

/// The display's refresh interval: the lower quartile of the intervals
/// between frames drawn back to back (most land on consecutive vsyncs; a
/// few come early, outside the display link, or late), snapped to the
/// nearest common refresh rate.
pub fn refresh_interval(intervals: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut sorted: Vec<f64> = intervals
        .into_iter()
        .filter(|ms| ms.is_finite() && *ms > 0.0)
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let quartile = sorted[sorted.len().div_ceil(4) - 1];
    RATES_HZ
        .iter()
        .map(|hz| 1000.0 / hz)
        .min_by(|a, b| (a - quartile).abs().total_cmp(&(b - quartile).abs()))
}

/// Share of vsyncs missed while frames were due, in percent.
#[allow(clippy::cast_precision_loss)]
pub fn drop_percent(dropped: u64, expected: u64) -> f64 {
    let vsyncs = expected + dropped;
    if vsyncs == 0 {
        0.0
    } else {
        dropped as f64 * 100.0 / vsyncs as f64
    }
}

/// One drawn frame.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    /// Since the previous frame started, when this one followed it back to
    /// back (the previous frame asked for it).
    pub interval_ms: Option<f64>,
    /// From the request to this frame's start, when it was expected.
    pub delay_ms: Option<f64>,
    /// Building the frame: render, layout and paint.
    pub cpu_ms: f64,
}

#[derive(Debug, Default)]
pub struct FrameStats {
    frames: VecDeque<Frame>,
    /// Every frame since the start (or the last reset).
    pub total: u64,
    pub dropped: u64,
    /// Frames expected within the run: the drop rate's denominator.
    pub expected: u64,
    /// Every interval and build time since the start, for the report.
    all_intervals: Vec<f64>,
    all_cpu: Vec<f64>,
}

impl FrameStats {
    pub fn record(&mut self, frame: Frame) {
        let refresh = self.refresh_ms();
        if let Some(delay) = frame.delay_ms {
            self.expected += 1;
            self.dropped += dropped(delay, refresh);
        }
        if let Some(interval) = frame.interval_ms {
            self.all_intervals.push(interval);
        }
        self.all_cpu.push(frame.cpu_ms);
        self.total += 1;
        if self.frames.len() == WINDOW {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }

    /// The display's refresh interval as measured so far.
    pub fn refresh_ms(&self) -> f64 {
        refresh_interval(self.frames.iter().filter_map(|frame| frame.interval_ms))
            .unwrap_or(DEFAULT_REFRESH_MS)
    }

    /// Recent back-to-back frame intervals.
    pub fn intervals(&self) -> Spread {
        Spread::of(self.frames.iter().filter_map(|frame| frame.interval_ms))
    }

    /// Recent build times.
    pub fn cpu(&self) -> Spread {
        Spread::of(self.frames.iter().map(|frame| frame.cpu_ms))
    }

    pub fn last_cpu_ms(&self) -> Option<f64> {
        self.frames.back().map(|frame| frame.cpu_ms)
    }

    /// Intervals and build times since the start.
    pub fn whole_run(&self) -> (Spread, Spread) {
        (
            Spread::of(self.all_intervals.iter().copied()),
            Spread::of(self.all_cpu.iter().copied()),
        )
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// What a stress run measured, as printed on exit and shown in the HUD.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub duration: Duration,
    pub tags: usize,
    /// Updates per second the feed offered, and how many the store applied.
    pub rate: u32,
    pub applied: u64,
    pub frames: u64,
    pub expected: u64,
    pub dropped: u64,
    pub refresh_ms: f64,
    pub interval: Spread,
    pub cpu: Spread,
    /// From asking for another page to its first frame on screen.
    pub switches: usize,
    pub switch: Spread,
    pub peak_rss_bytes: Option<u64>,
}

impl fmt::Display for Report {
    #[allow(clippy::cast_precision_loss)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = self.duration.as_secs_f64().max(f64::EPSILON);
        let percent = drop_percent(self.dropped, self.expected);
        writeln!(f, "voltrus-native stress report")?;
        writeln!(f, "duration_s: {seconds:.1}")?;
        writeln!(
            f,
            "feed: {} tags, {} updates/s offered, {:.0} updates/s applied",
            self.tags,
            self.rate,
            self.applied as f64 / seconds
        )?;
        writeln!(
            f,
            "frames: {} ({:.1} fps), display {:.0} Hz",
            self.frames,
            self.frames as f64 / seconds,
            1000.0 / self.refresh_ms.max(f64::EPSILON)
        )?;
        writeln!(f, "frame_interval_ms: {}", self.interval)?;
        writeln!(f, "frame_cpu_ms: {}", self.cpu)?;
        writeln!(
            f,
            "dropped_frames: {} ({percent:.2}% of {} vsyncs while frames were due)",
            self.dropped,
            self.expected + self.dropped
        )?;
        writeln!(
            f,
            "page_switches: {}, to first frame ms: {}",
            self.switches, self.switch
        )?;
        match self.peak_rss_bytes {
            Some(bytes) => writeln!(f, "peak_rss_mb: {:.0}", bytes as f64 / 1_048_576.0),
            None => writeln!(f, "peak_rss_mb: unknown"),
        }
    }
}

/// The process's peak resident memory, from `getrusage`.
#[cfg(unix)]
pub fn peak_rss_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` fills the whole struct it is given and only reads
    // the `who` selector.
    let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    // SAFETY: zeroed is a valid `rusage`, and the call succeeded.
    let usage = unsafe { usage.assume_init() };
    let max = u64::try_from(usage.ru_maxrss).ok()?;
    // macOS reports bytes, Linux kilobytes.
    Some(if cfg!(target_os = "macos") {
        max
    } else {
        max * 1024
    })
}

#[cfg(not(unix))]
pub fn peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let spread = Spread::of((1..=100).map(f64::from));
        assert_eq!((spread.p50, spread.p95, spread.max), (50.0, 95.0, 100.0));
        assert_eq!(Spread::of([]), Spread::default());
        assert_eq!(Spread::of([f64::NAN, 3.0]).max, 3.0, "NaN is ignored");
    }

    #[test]
    fn a_late_frame_counts_the_vsyncs_it_missed() {
        let refresh = 1000.0 / 120.0;
        assert_eq!(dropped(refresh, refresh), 0, "on time");
        assert_eq!(dropped(refresh * 1.4, refresh), 0, "within half a frame");
        assert_eq!(dropped(refresh * 2.0, refresh), 1);
        assert_eq!(dropped(refresh * 4.1, refresh), 3);
        assert_eq!(dropped(0.5, refresh), 0, "a frame right after the request");
        assert_eq!(dropped(f64::NAN, refresh), 0);
    }

    #[test]
    fn the_refresh_rate_is_the_typical_interval_snapped_to_a_real_rate() {
        // 60 Hz with one early frame (outside the display link) and late ones.
        let sixty = [16.7, 16.6, 5.1, 16.7, 33.4, 16.6, 16.8, 50.0];
        assert_eq!(refresh_interval(sixty), Some(1000.0 / 60.0));
        assert_eq!(
            refresh_interval([8.4, 8.3, 8.3, 16.7]),
            Some(1000.0 / 120.0)
        );
        assert_eq!(refresh_interval([]), None);
        assert_eq!(
            refresh_interval([0.5]),
            Some(1000.0 / 240.0),
            "the fastest rate"
        );
        assert_eq!(drop_percent(20, 1980), 1.0);
        assert_eq!(drop_percent(0, 0), 0.0);
    }

    #[test]
    fn only_expected_frames_count_towards_drops() {
        let mut stats = FrameStats::default();
        // An animation at 120 Hz with one frame that took three vsyncs.
        for interval in [8.33, 8.33, 25.0, 8.33] {
            stats.record(Frame {
                interval_ms: Some(interval),
                delay_ms: Some(interval),
                cpu_ms: 2.0,
            });
        }
        // An idle second, then a frame for new data: not a drop.
        stats.record(Frame {
            interval_ms: None,
            delay_ms: None,
            cpu_ms: 1.0,
        });
        assert_eq!(stats.total, 5);
        assert_eq!(stats.expected, 4);
        assert_eq!(stats.dropped, 2);
        assert!((stats.refresh_ms() - 1000.0 / 120.0).abs() < 1e-9);
        assert_eq!(stats.intervals().max, 25.0);
        assert_eq!(stats.last_cpu_ms(), Some(1.0));
        let (interval, cpu) = stats.whole_run();
        assert_eq!((interval.p50, cpu.max), (8.33, 2.0));
    }

    #[test]
    fn the_report_names_its_figures() {
        let report = Report {
            duration: Duration::from_secs(20),
            tags: 10_000,
            rate: 20_000,
            applied: 400_000,
            frames: 2_400,
            expected: 1_980,
            dropped: 20,
            refresh_ms: 1000.0 / 120.0,
            interval: Spread {
                p50: 8.3,
                p95: 9.1,
                max: 25.0,
            },
            ..Report::default()
        };
        let text = report.to_string();
        assert!(text.contains("20000 updates/s applied"), "{text}");
        assert!(
            text.contains("frames: 2400 (120.0 fps), display 120 Hz"),
            "{text}"
        );
        assert!(
            text.contains("frame_interval_ms: p50 8.30  p95 9.10  max 25.00"),
            "{text}"
        );
        assert!(
            text.contains("dropped_frames: 20 (1.00% of 2000 vsyncs while frames were due)"),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_peak_memory_is_readable() {
        assert!(peak_rss_bytes().is_some_and(|bytes| bytes > 1_048_576));
    }
}

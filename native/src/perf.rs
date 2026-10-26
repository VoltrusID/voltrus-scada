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

/// Frames between two estimates of the display's refresh once the first
/// [`SETTLE`] frames have been seen (each estimate sorts the window).
const ESTIMATE_EVERY: u64 = 32;
const SETTLE: u64 = 64;

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
    /// The refresh interval the display reports, when the platform says.
    display_ms: Option<f64>,
    /// The refresh interval measured from back-to-back frames.
    estimate_ms: Option<f64>,
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
        if self.total < SETTLE || self.total.is_multiple_of(ESTIMATE_EVERY) {
            self.estimate_ms =
                refresh_interval(self.frames.iter().filter_map(|frame| frame.interval_ms));
        }
    }

    /// The display's refresh interval: as the platform reports it, else as
    /// measured so far.
    pub fn refresh_ms(&self) -> f64 {
        self.display_ms
            .or(self.estimate_ms)
            .unwrap_or(DEFAULT_REFRESH_MS)
    }

    /// Where [`Self::refresh_ms`] comes from, for the report.
    pub fn refresh_source(&self) -> &'static str {
        match (self.display_ms, self.estimate_ms) {
            (Some(_), _) => "reported by the display",
            (None, Some(_)) => "measured from frame intervals",
            (None, None) => "assumed",
        }
    }

    /// The display's refresh rate as the platform reports it (`None`: not
    /// known, measure it instead).
    pub fn set_display_hz(&mut self, hz: Option<f64>) {
        self.display_ms = hz
            .filter(|hz| hz.is_finite() && *hz > 0.0)
            .map(|hz| 1000.0 / hz);
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

    /// Share of the back-to-back frames since the start that came one
    /// vsync after the previous (interval under 1.5 refreshes), in percent.
    pub fn on_next_vsync(&self) -> f64 {
        within_percent(&self.all_intervals, self.refresh_ms() * 1.5)
    }

    /// Share of the frames since the start built within `budget_ms`, in
    /// percent.
    pub fn built_within(&self, budget_ms: f64) -> f64 {
        within_percent(&self.all_cpu, budget_ms)
    }

    /// Start afresh; what the display reports stays.
    pub fn reset(&mut self) {
        *self = Self {
            display_ms: self.display_ms,
            ..Self::default()
        };
    }
}

/// Share of `samples` at or under `budget_ms`, in percent (100 for none).
#[allow(clippy::cast_precision_loss)]
pub fn within_percent(samples: &[f64], budget_ms: f64) -> f64 {
    if samples.is_empty() {
        return 100.0;
    }
    let within = samples.iter().filter(|ms| **ms <= budget_ms).count();
    within as f64 * 100.0 / samples.len() as f64
}

/// One page's figures over a run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageFigures {
    pub id: &'static str,
    /// Frames drawn while the page was shown, and their build times.
    pub frames: usize,
    pub frame: Spread,
    /// The page's own render function (its element tree, before layout and
    /// paint), each time it ran on screen.
    pub render: Spread,
    /// The build time of the frame that first showed the page.
    pub first_ms: Option<f64>,
    /// The build time of the warm-up frame that drew the page out of sight,
    /// if it was warmed.
    pub warmed_ms: Option<f64>,
}

/// Where a frame's build time goes, and what the presentation after it
/// took, over a run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Phases {
    /// The root: rendering it and laying out the window's frame. Cached
    /// views take their place in the layout here and are drawn below.
    pub layout: Spread,
    /// The cached views (navbar, page, status bar, …) that changed:
    /// rendering, laying out and prepainting each (GPUI does all three for
    /// a cached view in its prepaint), and prepainting the rest.
    pub prepaint: Spread,
    pub paint: Spread,
    /// From the end of paint until the frame was handed to the GPU
    /// (finishing the scene, encoding, waiting for a drawable).
    pub present: Spread,
}

/// Frames that started over a millisecond after their vsync slot (the
/// previous frame's start plus a refresh), by the likeliest cause.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LateStarts {
    /// Back-to-back frames looked at.
    pub frames: u64,
    pub late: u64,
    /// The previous frame (build and present) ran past the slot.
    pub previous_frame: u64,
    /// A batch of events was being applied up to just before the start.
    pub event_batch: u64,
    /// Neither: the display link or the scheduler woke the frame late.
    pub other: u64,
    /// Frames after a pause in the data past a vsync: nothing was due at
    /// that vsync, so these are neither late nor drops (counted apart, not
    /// in `frames`).
    pub nothing_due: u64,
}

impl LateStarts {
    /// A frame `gap_ms` after the previous one, for data that came after
    /// the vsync it would otherwise have been owed at.
    pub fn record_unowed(&mut self, gap_ms: f64, refresh_ms: f64) {
        if gap_ms > refresh_ms + 1.0 {
            self.nothing_due += 1;
        }
    }

    /// Classify a back-to-back frame that started `interval_ms` after the
    /// previous one: `previous_ms`, the previous frame's build and
    /// present; `batch_before_ms`, how long before this start the last
    /// event batch ended (`None`: none since the previous frame).
    pub fn record(
        &mut self,
        interval_ms: f64,
        refresh_ms: f64,
        previous_ms: Option<f64>,
        batch_before_ms: Option<f64>,
    ) {
        self.frames += 1;
        if interval_ms <= refresh_ms + 1.0 {
            return;
        }
        self.late += 1;
        if previous_ms.is_some_and(|ms| ms > refresh_ms) {
            self.previous_frame += 1;
        } else if batch_before_ms.is_some_and(|ms| ms <= 1.5) {
            self.event_batch += 1;
        } else {
            self.other += 1;
        }
    }
}

/// One of the run's slowest frames.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlowFrame {
    pub cpu_ms: f64,
    pub page: &'static str,
    /// The page's render function within it (0 when the page replayed its
    /// last frame): the rest was layout, paint and the other views.
    pub render_ms: f64,
    /// What else the frame did: `first show`, `page switch`, or nothing.
    pub note: &'static str,
    /// The UI thread's own CPU time within the frame: far below `cpu_ms`,
    /// the thread was waiting or taken off the CPU, not working.
    pub on_cpu_ms: Option<f64>,
    /// Views (render, layout, prepaint) and paint within the frame.
    pub views_ms: Option<f64>,
    pub paint_ms: Option<f64>,
}

/// The calling thread's CPU time so far, in ms (user and system).
#[cfg(unix)]
pub fn thread_cpu_ms() -> Option<f64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes the struct it is given.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &raw mut time) };
    #[allow(clippy::cast_precision_loss)]
    (status == 0).then(|| time.tv_sec as f64 * 1000.0 + time.tv_nsec as f64 / 1_000_000.0)
}

#[cfg(not(unix))]
pub fn thread_cpu_ms() -> Option<f64> {
    None
}

/// Frames that took over [`STALL_MS`] longer than the UI thread worked in
/// them: it waited (a lock, the GPU, I/O) or was taken off the CPU.
pub const STALL_MS: f64 = 20.0;

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
    /// `stress`, or `profile` for the animation showcase.
    pub mode: &'static str,
    /// Where `refresh_ms` comes from.
    pub refresh_source: &'static str,
    /// Shares of the frames built within a 60 Hz and a 120 Hz frame, in
    /// percent.
    pub built_60: f64,
    pub built_120: f64,
    /// Applying a batch of runtime events to the store (on the UI thread,
    /// between frames).
    pub apply: Spread,
    /// Frames drawn while the pages were warmed after sign-in (each page
    /// drawn once, hidden), and their build times; not in the figures above.
    pub warm_up: Option<(usize, Spread)>,
    pub pages: Vec<PageFigures>,
    pub phases: Option<Phases>,
    pub late: LateStarts,
    /// Frames the UI thread spent over [`STALL_MS`] off the CPU or
    /// waiting, and the longest such wait.
    pub stalls: (usize, f64),
    /// Share of back-to-back frames that started one vsync after the
    /// previous, in percent.
    pub on_next_vsync: f64,
    /// Slowest first.
    pub slowest: Vec<SlowFrame>,
}

impl fmt::Display for Report {
    #[allow(clippy::cast_precision_loss)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = self.duration.as_secs_f64().max(f64::EPSILON);
        let percent = drop_percent(self.dropped, self.expected);
        writeln!(f, "voltrus-native stress report")?;
        if !self.mode.is_empty() {
            writeln!(f, "mode: {}", self.mode)?;
        }
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
        if !self.refresh_source.is_empty() {
            writeln!(f, "display_refresh: {}", self.refresh_source)?;
        }
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
        }?;
        if self.frames > 0 {
            writeln!(
                f,
                "frame_budget: {:.1}% of frames built within 16.7 ms (60 Hz), {:.1}% within 8.3 ms (120 Hz)",
                self.built_60, self.built_120
            )?;
        }
        if self.apply.max > 0.0 {
            writeln!(f, "apply_ms per event batch: {}", self.apply)?;
        }
        if let Some(phases) = &self.phases {
            writeln!(
                f,
                "frame_phases_ms: root layout {}  |  views render+layout+prepaint {}  |  paint {}  |  present {}",
                phases.layout, phases.prepaint, phases.paint, phases.present
            )?;
        }
        if self.late.frames > 0 {
            writeln!(
                f,
                "late_starts: {} of {} back-to-back frames started over 1 ms after their vsync slot: previous frame still drawing {}, event batch just applied {}, other (display link, scheduler) {}; {} more after a pause in the data with nothing due at the vsync",
                self.late.late,
                self.late.frames,
                self.late.previous_frame,
                self.late.event_batch,
                self.late.other,
                self.late.nothing_due
            )?;
            writeln!(
                f,
                "frame_pacing: {:.1}% of back-to-back frames came one vsync after the previous (interval under 1.5 refreshes); the rest started late or skipped vsyncs",
                self.on_next_vsync
            )?;
        }
        if self.stalls.0 > 0 {
            writeln!(
                f,
                "stalls: {} frames took over {STALL_MS:.0} ms longer than the UI thread worked in them (longest wait {:.2} ms)",
                self.stalls.0, self.stalls.1
            )?;
        }
        if let Some((frames, spread)) = &self.warm_up {
            writeln!(
                f,
                "warm_up: {frames} frames from the start until every page was drawn once, build ms {spread} (not counted above)"
            )?;
        }
        if !self.pages.is_empty() {
            writeln!(
                f,
                "pages (frame = whole frame while shown, render = the page's element tree, first = its first frame on screen, warmed = the warm-up frame that drew it out of sight), ms:"
            )?;
            for page in &self.pages {
                write!(
                    f,
                    "  {:<12} frames {:>5}  frame {}  render {}",
                    page.id, page.frames, page.frame, page.render
                )?;
                if let Some(first) = page.first_ms {
                    write!(f, "  first {first:.2}")?;
                }
                match page.warmed_ms {
                    Some(warmed) => writeln!(f, "  warmed {warmed:.2}")?,
                    None => writeln!(f)?,
                }
            }
        }
        if !self.slowest.is_empty() {
            let slowest: Vec<String> = self
                .slowest
                .iter()
                .map(|frame| {
                    let page = if frame.page.is_empty() {
                        "sign-in"
                    } else {
                        frame.page
                    };
                    let mut text =
                        format!("{:.2} {page} (render {:.2}", frame.cpu_ms, frame.render_ms);
                    if let (Some(views), Some(paint)) = (frame.views_ms, frame.paint_ms) {
                        text.push_str(&format!(", views {views:.2}, paint {paint:.2}"));
                    }
                    if let Some(on_cpu) = frame.on_cpu_ms {
                        text.push_str(&format!(", on-CPU {on_cpu:.2}"));
                    }
                    if !frame.note.is_empty() {
                        text.push_str(", ");
                        text.push_str(frame.note);
                    }
                    text.push(')');
                    text
                })
                .collect();
            writeln!(f, "slowest_frames_ms: {}", slowest.join(", "))?;
        }
        Ok(())
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

/// The refresh rate, in Hz, of the display a window is on (`display`, a
/// `CGDirectDisplayID` on macOS), as the platform reports it; GPUI does not
/// expose it. A ProMotion panel reports its top rate (120 Hz), which the
/// display link runs at while frames keep coming.
pub fn display_refresh_hz(display: Option<u32>) -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        best_rate(display.and_then(mac::mode_hz), mac::main_screen_max_fps())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = display;
        None
    }
}

/// Of the display mode's rate and the screen's top rate, the higher one
/// that is plausible (the mode reports 0 for some built-in panels, and the
/// screen's rate is that of the screen with the key window).
pub fn best_rate(mode_hz: Option<f64>, screen_hz: Option<f64>) -> Option<f64> {
    [mode_hz, screen_hz]
        .into_iter()
        .flatten()
        .filter(|hz| hz.is_finite() && (20.0..=1000.0).contains(hz))
        .reduce(f64::max)
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void};

    type Id = *mut c_void;
    type Sel = *mut c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
        fn CGDisplayModeGetRefreshRate(mode: *mut c_void) -> f64;
        fn CGDisplayModeRelease(mode: *mut c_void);
    }

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    /// The refresh rate of the display's current mode.
    pub fn mode_hz(display: u32) -> Option<f64> {
        // SAFETY: the mode is checked for null, read once and released; an
        // unknown display id yields null.
        unsafe {
            let mode = CGDisplayCopyDisplayMode(display);
            if mode.is_null() {
                return None;
            }
            let hz = CGDisplayModeGetRefreshRate(mode);
            CGDisplayModeRelease(mode);
            Some(hz)
        }
    }

    /// `NSScreen.mainScreen.maximumFramesPerSecond` (macOS 12 and later):
    /// 120 on a ProMotion panel.
    pub fn main_screen_max_fps() -> Option<f64> {
        // SAFETY: `objc_msgSend` is called through the exact signature of
        // each message (an object, a BOOL, an NSInteger); every receiver is
        // checked for nil and the selector for support first.
        unsafe {
            let send_id = std::mem::transmute::<
                unsafe extern "C" fn(),
                unsafe extern "C" fn(Id, Sel) -> Id,
            >(objc_msgSend);
            let send_responds = std::mem::transmute::<
                unsafe extern "C" fn(),
                unsafe extern "C" fn(Id, Sel, Sel) -> u8,
            >(objc_msgSend);
            let send_integer = std::mem::transmute::<
                unsafe extern "C" fn(),
                unsafe extern "C" fn(Id, Sel) -> isize,
            >(objc_msgSend);
            let class = objc_getClass(c"NSScreen".as_ptr());
            if class.is_null() {
                return None;
            }
            let screen = send_id(class, sel_registerName(c"mainScreen".as_ptr()));
            if screen.is_null() {
                return None;
            }
            let max_fps = sel_registerName(c"maximumFramesPerSecond".as_ptr());
            let responds = sel_registerName(c"respondsToSelector:".as_ptr());
            if send_responds(screen, responds, max_fps) == 0 {
                return None;
            }
            #[allow(clippy::cast_precision_loss)]
            let fps = send_integer(screen, max_fps) as f64;
            (fps > 0.0).then_some(fps)
        }
    }
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

    #[test]
    fn a_reported_refresh_rate_beats_the_estimate_and_survives_a_reset() {
        let mut stats = FrameStats::default();
        for _ in 0..10 {
            stats.record(Frame {
                interval_ms: Some(16.7),
                delay_ms: Some(16.7),
                cpu_ms: 3.0,
            });
        }
        assert!((stats.refresh_ms() - 1000.0 / 60.0).abs() < 1e-9);
        assert_eq!(stats.refresh_source(), "measured from frame intervals");
        // A ProMotion panel: frames 16.7 ms apart now miss a vsync each.
        stats.set_display_hz(Some(120.0));
        assert!((stats.refresh_ms() - 1000.0 / 120.0).abs() < 1e-9);
        stats.record(Frame {
            interval_ms: Some(16.7),
            delay_ms: Some(16.7),
            cpu_ms: 3.0,
        });
        assert_eq!(stats.dropped, 1);
        stats.reset();
        assert_eq!((stats.total, stats.dropped), (0, 0));
        assert_eq!(stats.refresh_source(), "reported by the display");
        stats.set_display_hz(Some(f64::NAN));
        assert_eq!(stats.refresh_source(), "assumed");
    }

    #[test]
    fn frame_budgets_count_frames_built_in_time() {
        assert_eq!(within_percent(&[], 8.3), 100.0);
        assert_eq!(within_percent(&[2.0, 8.0, 8.4, 20.0], 8.33), 50.0);
        let mut stats = FrameStats::default();
        for cpu in [1.0, 5.0, 9.0, 17.0] {
            stats.record(Frame {
                interval_ms: None,
                delay_ms: None,
                cpu_ms: cpu,
            });
        }
        assert_eq!(stats.built_within(1000.0 / 60.0), 75.0);
        assert_eq!(stats.built_within(1000.0 / 120.0), 50.0);
    }

    #[test]
    fn platform_rates_take_the_higher_plausible_one() {
        assert_eq!(best_rate(Some(60.0), Some(120.0)), Some(120.0));
        assert_eq!(
            best_rate(Some(0.0), Some(120.0)),
            Some(120.0),
            "built-in panels report 0"
        );
        assert_eq!(best_rate(Some(59.94), None), Some(59.94));
        assert_eq!(best_rate(None, None), None);
        assert_eq!(best_rate(Some(f64::NAN), Some(5.0)), None);
        // Whatever this machine has, it is a plausible rate or unknown.
        assert!(display_refresh_hz(None).is_none_or(|hz| (20.0..=1000.0).contains(&hz)));
    }

    #[test]
    fn the_report_breaks_frames_down_by_page() {
        let report = Report {
            duration: Duration::from_secs(10),
            frames: 600,
            refresh_ms: 1000.0 / 60.0,
            mode: "profile",
            refresh_source: "reported by the display",
            built_60: 99.5,
            built_120: 97.3,
            apply: Spread {
                p50: 0.1,
                p95: 0.3,
                max: 1.0,
            },
            warm_up: Some((
                5,
                Spread {
                    p50: 4.0,
                    p95: 9.0,
                    max: 9.0,
                },
            )),
            pages: vec![PageFigures {
                id: "overview",
                frames: 590,
                frame: Spread {
                    p50: 2.0,
                    p95: 4.0,
                    max: 7.0,
                },
                render: Spread {
                    p50: 0.5,
                    p95: 0.9,
                    max: 1.5,
                },
                first_ms: Some(6.5),
                warmed_ms: None,
            }],
            phases: Some(Phases {
                layout: Spread {
                    p50: 1.0,
                    p95: 2.0,
                    max: 3.0,
                },
                ..Phases::default()
            }),
            late: LateStarts {
                frames: 100,
                late: 4,
                previous_frame: 1,
                event_batch: 2,
                other: 1,
                nothing_due: 3,
            },
            on_next_vsync: 97.5,
            stalls: (2, 41.5),
            slowest: vec![
                SlowFrame {
                    cpu_ms: 7.0,
                    page: "overview",
                    render_ms: 1.5,
                    note: "first show",
                    on_cpu_ms: Some(6.5),
                    views_ms: Some(3.0),
                    paint_ms: Some(1.0),
                },
                SlowFrame {
                    cpu_ms: 6.0,
                    page: "tags",
                    render_ms: 0.25,
                    note: "",
                    ..SlowFrame::default()
                },
            ],
            ..Report::default()
        };
        let text = report.to_string();
        for line in [
            "mode: profile",
            "display_refresh: reported by the display",
            "frame_budget: 99.5% of frames built within 16.7 ms (60 Hz), 97.3% within 8.3 ms (120 Hz)",
            "apply_ms per event batch: p50 0.10  p95 0.30  max 1.00",
            "warm_up: 5 frames from the start until every page was drawn once",
            "  overview     frames   590  frame p50 2.00  p95 4.00  max 7.00  render p50 0.50  p95 0.90  max 1.50  first 6.50",
            "slowest_frames_ms: 7.00 overview (render 1.50, views 3.00, paint 1.00, on-CPU 6.50, first show), 6.00 tags (render 0.25)",
            "frame_phases_ms: root layout p50 1.00  p95 2.00  max 3.00  |  views render+layout+prepaint p50 0.00",
            "late_starts: 4 of 100 back-to-back frames started over 1 ms after their vsync slot: previous frame still drawing 1, event batch just applied 2, other (display link, scheduler) 1; 3 more after a pause in the data",
            "frame_pacing: 97.5% of back-to-back frames came one vsync after the previous",
            "stalls: 2 frames took over 20 ms longer than the UI thread worked in them (longest wait 41.50 ms)",
        ] {
            assert!(text.contains(line), "missing “{line}” in\n{text}");
        }
    }

    #[test]
    fn late_starts_are_put_down_to_their_likeliest_cause() {
        let refresh = 1000.0 / 60.0;
        let mut late = LateStarts::default();
        late.record(16.7, refresh, Some(30.0), Some(0.1));
        assert_eq!(
            (late.frames, late.late),
            (1, 0),
            "on time whatever came before"
        );
        late.record(18.9, refresh, Some(17.5), Some(0.2));
        late.record(18.9, refresh, Some(6.0), Some(0.4));
        late.record(18.9, refresh, Some(6.0), Some(9.0));
        late.record(18.9, refresh, None, None);
        assert_eq!(
            late,
            LateStarts {
                frames: 5,
                late: 4,
                previous_frame: 1,
                event_batch: 1,
                other: 2,
                nothing_due: 0,
            }
        );
        late.record_unowed(33.3, refresh);
        late.record_unowed(16.9, refresh);
        assert_eq!((late.frames, late.nothing_due), (5, 1));
    }
}

//! Measures the frames the window draws, for the HUD (⌘⇧P), the
//! Performance page and the stress report.
//!
//! The root view's render marks a frame's start and a zero-size canvas
//! painted last marks its end, so a frame's time is the CPU work of
//! building it (render, layout, paint). Whoever makes a frame due tells the
//! meter ([`FrameMeter::expect_frame`]): an animation asking for the next
//! frame, or data that changed what is shown. Only expected frames count
//! towards drops, so an idle console's long gaps are not drops.
//!
//! Each frame is put down to the page on screen, and the page's own render
//! time is kept beside it, so the report can say where a frame's time goes.
//! The display's refresh rate comes from the platform when it says (a
//! ProMotion panel's 120 Hz), else from the intervals between frames.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, Window};
use voltrus_scada_native::perf::{
    self, Frame, FrameStats, LateStarts, PageFigures, Phases, SlowFrame, Spread,
};
use vt_ui::chart::History;

pub type Meter = Rc<RefCell<FrameMeter>>;

/// Frame samples the HUD's charts keep.
const TIMELINE: usize = 2048;

/// A due frame this soon after the previous one continues a run of frames
/// (a busy stream); a longer gap was a pause, not dropped frames.
const CONTINUOUS_MS: f64 = 100.0;

/// How often the display's refresh rate is asked again (the window may
/// move to another display).
const DISPLAY_CHECK: Duration = Duration::from_secs(1);

/// Samples kept per page and for batches: a run of hours stays bounded.
const SAMPLES: usize = 1 << 16;

/// New data owes a frame at the vsync after the previous frame when it
/// came within this share of a refresh after that frame started (later,
/// it may have missed the vsync without any fault of the console's).
const OWED_WITHIN: f64 = 0.75;

/// Slowest frames the report names.
const SLOWEST: usize = 5;

/// One page's frames over a run.
#[derive(Default)]
struct PageLog {
    id: &'static str,
    /// Build times of the frames drawn while the page was shown.
    frames: Vec<f64>,
    /// Its render function's time, each time it ran on screen.
    renders: Vec<f64>,
    first_ms: Option<f64>,
    /// The warm-up frame that drew it out of sight.
    warmed_ms: Option<f64>,
}

/// One frame's phase marks, as the root's first and last elements see them.
#[derive(Default, Clone, Copy)]
struct Marks {
    /// The first element's prepaint: every view rendered, layout done.
    laid_out: Option<Instant>,
    /// The first element's paint: prepaint done.
    painting: Option<Instant>,
}

pub struct FrameMeter {
    pub stats: FrameStats,
    /// Build time per frame and back-to-back intervals over wall time, for
    /// the Performance page's chart.
    pub cpu_line: Rc<History>,
    pub interval_line: Rc<History>,
    /// The frame being built: its start.
    building: Option<Instant>,
    last_start: Option<Instant>,
    /// The earliest moment a frame became due since the last one started,
    /// and whether that happened while a frame was being built (an
    /// animation asking for the next one).
    due: Option<(Instant, bool)>,
    /// When the frame being built became due, and whether it follows the
    /// previous frame back to back.
    current: Option<(Option<f64>, Option<f64>)>,
    /// A page switch waiting for its first frame.
    switch: Option<Instant>,
    pub switches: Vec<f64>,
    /// Frame starts of the last second, for the frame rate.
    recent: std::collections::VecDeque<Instant>,
    /// The page on screen, as its view last reported.
    page: &'static str,
    /// The frame being built shows its page for the first time.
    first_show: bool,
    /// The page's render time within the frame being built.
    frame_render: f64,
    pages: Vec<PageLog>,
    slowest: Vec<SlowFrame>,
    /// Applying a batch of runtime events, in ms.
    applies: Vec<f64>,
    /// Pages are being drawn hidden after sign-in: these frames go to
    /// `warm_up` instead of the figures.
    warming: bool,
    warm_up: Vec<f64>,
    /// The page drawn out of sight in the frame being built.
    warming_page: Option<&'static str>,
    display_checked: Option<Instant>,
    marks: Marks,
    /// Per frame, in ms: render and layout, prepaint, paint, present.
    phases: [Vec<f64>; 4],
    /// The last frame's start and the end of its paint, until its present
    /// is done.
    presenting: Option<(Instant, Instant)>,
    /// The last frame's build and present, once presented.
    last_total: Option<f64>,
    /// When the last batch of events was applied, since the last frame.
    last_batch: Option<Instant>,
    late: LateStarts,
    /// The UI thread's CPU time when the frame being built started.
    thread_start: Option<f64>,
    /// Frames the UI thread spent over `perf::STALL_MS` waiting or off
    /// the CPU, and the longest such wait.
    stalls: (usize, f64),
}

impl Default for FrameMeter {
    fn default() -> Self {
        Self {
            stats: FrameStats::default(),
            cpu_line: History::new(TIMELINE),
            interval_line: History::new(TIMELINE),
            building: None,
            last_start: None,
            due: None,
            current: None,
            switch: None,
            switches: Vec::new(),
            recent: std::collections::VecDeque::new(),
            page: "",
            first_show: false,
            frame_render: 0.0,
            pages: Vec::new(),
            slowest: Vec::new(),
            applies: Vec::new(),
            warming: false,
            warm_up: Vec::new(),
            warming_page: None,
            display_checked: None,
            marks: Marks::default(),
            phases: Default::default(),
            presenting: None,
            last_total: None,
            last_batch: None,
            late: LateStarts::default(),
            thread_start: None,
            stalls: (0, 0.0),
        }
    }
}

/// Milliseconds from `from` to `to`, to the microsecond.
#[allow(clippy::cast_precision_loss)]
pub fn ms(from: Instant, to: Instant) -> f64 {
    to.saturating_duration_since(from).as_micros() as f64 / 1000.0
}

fn keep(samples: &mut Vec<f64>, ms: f64) {
    if samples.len() < SAMPLES {
        samples.push(ms);
    }
}

impl FrameMeter {
    /// A frame is due: something will look different in the next one.
    pub fn expect_frame(&mut self, now: Instant) {
        if self.due.is_none() {
            self.due = Some((now, self.building.is_some()));
        }
    }

    /// The page changed; the next frame end measures the switch.
    pub fn page_switch(&mut self, now: Instant) {
        self.switch = Some(now);
        self.expect_frame(now);
    }

    /// The root view starts building a frame.
    pub fn frame_start(&mut self, now: Instant) {
        let gap = self.last_start.map(|last| ms(last, now));
        let (delay, interval) = match (self.due.take(), gap) {
            // Asked for while the previous frame was built (an animation),
            // or while frames follow each other (a busy stream): the gap
            // between the two frames is the frame interval.
            (Some((_, true)), Some(gap)) => (Some(gap), Some(gap)),
            // New data while frames follow each other: it was owed at the
            // vsync after the previous frame only if it came in time for
            // it. Data that came later (the feed paused past a vsync) was
            // owed one vsync later: that is not a drop.
            (Some((at, false)), Some(gap)) if gap < CONTINUOUS_MS => {
                let last = self.last_start.unwrap_or(now);
                if ms(last, at) < self.stats.refresh_ms() * OWED_WITHIN {
                    (Some(gap), Some(gap))
                } else {
                    if !self.warming {
                        self.late.record_unowed(gap, self.stats.refresh_ms());
                    }
                    (Some(ms(at, now)), None)
                }
            }
            // The first frame after a pause: how long it took to come.
            (Some((at, _)), _) => (Some(ms(at, now)), None),
            (None, _) => (None, None),
        };
        if let Some(interval) = interval.filter(|_| !self.warming) {
            let refresh = self.stats.refresh_ms();
            let batch = self.last_batch.map(|at| ms(at, now));
            self.late.record(interval, refresh, self.last_total, batch);
        }
        self.last_batch = None;
        self.last_total = None;
        self.marks = Marks::default();
        self.current = Some((delay, interval));
        self.thread_start = perf::thread_cpu_ms();
        self.building = Some(now);
        self.last_start = Some(now);
        self.recent.push_back(now);
        while self
            .recent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at).as_secs_f64() > 1.0)
        {
            self.recent.pop_front();
        }
    }

    fn page_log(&mut self, id: &'static str) -> &mut PageLog {
        let index = match self.pages.iter().position(|page| page.id == id) {
            Some(index) => index,
            None => {
                self.pages.push(PageLog {
                    id,
                    ..PageLog::default()
                });
                self.pages.len() - 1
            }
        };
        &mut self.pages[index]
    }

    /// The page on screen ran its render function in `render_ms`.
    pub fn page_built(&mut self, id: &'static str, render_ms: f64) {
        self.page = id;
        self.frame_render += render_ms;
        let log = self.page_log(id);
        if log.first_ms.is_none() && log.frames.is_empty() {
            self.first_show = true;
        }
        if !self.warming {
            keep(&mut self.page_log(id).renders, render_ms);
        }
    }

    /// A page was drawn out of sight to warm it; its render took
    /// `render_ms`.
    pub fn page_warmed(&mut self, id: &'static str, _render_ms: f64) {
        self.warming_page = Some(id);
    }

    /// Every view of the frame rendered and laid out: its prepaint begins.
    pub fn laid_out(&mut self, now: Instant) {
        if self.building.is_some() {
            self.marks.laid_out = Some(now);
        }
    }

    /// Prepaint done: its paint begins.
    pub fn painting(&mut self, now: Instant) {
        if self.building.is_some() {
            self.marks.painting = Some(now);
        }
    }

    /// The frame painted last was handed to the GPU.
    pub fn presented(&mut self, now: Instant) {
        let Some((start, painted)) = self.presenting.take() else {
            return;
        };
        self.last_total = Some(ms(start, now));
        if !self.warming {
            keep(&mut self.phases[3], ms(painted, now));
        }
    }

    /// Drawing the pages hidden after sign-in, so that their first show
    /// finds fonts, text and views ready.
    pub fn set_warming(&mut self, warming: bool) {
        self.warming = warming;
    }

    /// A batch of runtime events took `ms` to apply.
    pub fn batch_applied(&mut self, ms: f64) {
        self.last_batch = Some(Instant::now());
        if !self.warming {
            keep(&mut self.applies, ms);
        }
    }

    /// The last element of the frame painted.
    pub fn frame_end(&mut self, now: Instant, wall_ms: f64) {
        let Some(start) = self.building.take() else {
            return;
        };
        let (delay_ms, interval_ms) = self.current.take().unwrap_or((None, None));
        let cpu_ms = ms(start, now);
        let on_cpu_ms = self
            .thread_start
            .take()
            .zip(perf::thread_cpu_ms())
            .map(|(from, to)| (to - from).max(0.0));
        let first_show = std::mem::take(&mut self.first_show);
        let render_ms = std::mem::take(&mut self.frame_render);
        let switched = self.switch.take();
        let warmed = self.warming_page.take();
        self.presenting = Some((start, now));
        if self.warming {
            keep(&mut self.warm_up, cpu_ms);
            if first_show {
                self.page_log(self.page).first_ms = Some(cpu_ms);
            }
            if let Some(id) = warmed {
                self.page_log(id).warmed_ms = Some(cpu_ms);
            }
            return;
        }
        let marks = self.marks;
        let mut split = (None, None);
        if let (Some(laid_out), Some(painting)) = (marks.laid_out, marks.painting) {
            keep(&mut self.phases[0], ms(start, laid_out));
            keep(&mut self.phases[1], ms(laid_out, painting));
            keep(&mut self.phases[2], ms(painting, now));
            split = (Some(ms(laid_out, painting)), Some(ms(painting, now)));
        }
        if let Some(on_cpu) = on_cpu_ms {
            let waited = cpu_ms - on_cpu;
            if waited > perf::STALL_MS {
                self.stalls.0 += 1;
                self.stalls.1 = self.stalls.1.max(waited);
            }
        }
        self.stats.record(Frame {
            interval_ms,
            delay_ms,
            cpu_ms,
        });
        self.cpu_line.push(wall_ms, cpu_ms);
        if let Some(interval) = interval_ms {
            self.interval_line.push(wall_ms, interval);
        }
        if let Some(asked) = switched {
            self.switches.push(ms(asked, now));
        }
        if !self.page.is_empty() {
            let page = self.page;
            let log = self.page_log(page);
            keep(&mut log.frames, cpu_ms);
            if first_show {
                log.first_ms = Some(cpu_ms);
            }
        }
        let warmed_before = first_show
            && self
                .pages
                .iter()
                .any(|page| page.id == self.page && page.warmed_ms.is_some());
        let note = if warmed_before {
            "first show, warmed"
        } else if first_show {
            "first show, not warmed"
        } else if switched.is_some() {
            "page switch"
        } else {
            ""
        };
        self.note_slow(SlowFrame {
            cpu_ms,
            page: self.page,
            render_ms,
            note,
            on_cpu_ms,
            views_ms: split.0,
            paint_ms: split.1,
        });
    }

    fn note_slow(&mut self, frame: SlowFrame) {
        if self.slowest.len() == SLOWEST
            && self
                .slowest
                .last()
                .is_some_and(|last| last.cpu_ms >= frame.cpu_ms)
        {
            return;
        }
        let at = self
            .slowest
            .partition_point(|kept| kept.cpu_ms >= frame.cpu_ms);
        self.slowest.insert(at, frame);
        self.slowest.truncate(SLOWEST);
    }

    /// Frames drawn in the last second.
    pub fn fps(&self) -> usize {
        self.recent.len()
    }

    pub fn switch_spread(&self) -> Spread {
        Spread::of(self.switches.iter().copied())
    }

    /// Each page's frames, in the order they were first shown.
    pub fn page_figures(&self) -> Vec<PageFigures> {
        self.pages
            .iter()
            .filter(|page| !page.frames.is_empty() || page.first_ms.is_some())
            .map(|page| PageFigures {
                id: page.id,
                frames: page.frames.len(),
                frame: Spread::of(page.frames.iter().copied()),
                render: Spread::of(page.renders.iter().copied()),
                first_ms: page.first_ms,
                warmed_ms: page.warmed_ms,
            })
            .collect()
    }

    /// Where the frames' time went, once frames were measured.
    pub fn phases(&self) -> Option<Phases> {
        let spread = |index: usize| Spread::of(self.phases[index].iter().copied());
        (!self.phases[0].is_empty()).then(|| Phases {
            layout: spread(0),
            prepaint: spread(1),
            paint: spread(2),
            present: spread(3),
        })
    }

    /// Frames that waited over `perf::STALL_MS`, and the longest wait.
    pub fn stalls(&self) -> (usize, f64) {
        self.stalls
    }

    pub fn late_starts(&self) -> LateStarts {
        self.late
    }

    pub fn slowest(&self) -> Vec<SlowFrame> {
        self.slowest.clone()
    }

    pub fn apply_spread(&self) -> Spread {
        Spread::of(self.applies.iter().copied())
    }

    /// The warm-up's frames and build times, if there was one.
    pub fn warm_up(&self) -> Option<(usize, Spread)> {
        (!self.warm_up.is_empty())
            .then(|| (self.warm_up.len(), Spread::of(self.warm_up.iter().copied())))
    }

    /// Ask the platform for the refresh rate of the window's display, at
    /// most once a second.
    pub fn sync_display(&mut self, window: &Window, cx: &App, now: Instant) {
        if self
            .display_checked
            .is_some_and(|at| now.saturating_duration_since(at) < DISPLAY_CHECK)
        {
            return;
        }
        self.display_checked = Some(now);
        let display = window.display(cx).map(|display| u32::from(display.id()));
        self.stats.set_display_hz(perf::display_refresh_hz(display));
    }

    /// Start counting afresh (a stress run begins).
    pub fn reset(&mut self) {
        self.stats.reset();
        self.switches.clear();
        self.cpu_line.clear();
        self.interval_line.clear();
        self.pages.clear();
        self.slowest.clear();
        self.applies.clear();
        self.warm_up.clear();
        self.phases = Default::default();
        self.late = LateStarts::default();
        self.stalls = (0, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_frames_measure_intervals_and_data_frames_their_delay() {
        let mut meter = FrameMeter::default();
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        // An idle frame: nothing asked for it.
        meter.frame_start(at(0));
        meter.frame_end(at(2), 0.0);
        // Data arrives 500 ms later; its frame comes 4 ms after.
        meter.expect_frame(at(500));
        meter.frame_start(at(504));
        // While building, an animation asks for the next frame.
        meter.expect_frame(at(505));
        meter.frame_end(at(507), 0.0);
        // The animation's frame comes one 120 Hz refresh later.
        meter.frame_start(at(512));
        meter.frame_end(at(514), 0.0);
        assert_eq!(meter.stats.total, 3);
        assert_eq!(meter.stats.expected, 2);
        assert_eq!(meter.stats.dropped, 0);
        let intervals = meter.stats.intervals();
        assert_eq!(
            intervals.max, 8.0,
            "only the back-to-back frame has an interval"
        );
        assert_eq!(meter.stats.cpu().max, 3.0);
        assert_eq!(meter.interval_line.len(), 1);
        assert_eq!(meter.fps(), 3);
    }

    #[test]
    fn a_page_switch_is_timed_to_its_first_frame() {
        let mut meter = FrameMeter::default();
        let t0 = Instant::now();
        meter.page_switch(t0);
        meter.frame_start(t0 + Duration::from_millis(3));
        meter.frame_end(t0 + Duration::from_millis(9), 0.0);
        assert_eq!(meter.switches, [9.0]);
        assert_eq!(meter.stats.expected, 1);
    }

    /// One frame of `cpu` ms showing `page`, whose render took `render` ms.
    fn frame(meter: &mut FrameMeter, at: &mut Instant, page: &'static str, render: f64, cpu: u64) {
        meter.frame_start(*at);
        meter.page_built(page, render);
        *at += Duration::from_millis(cpu);
        meter.frame_end(*at, 0.0);
        *at += Duration::from_millis(16);
    }

    #[test]
    fn frames_are_put_down_to_the_page_on_screen() {
        let mut meter = FrameMeter::default();
        let mut at = Instant::now();
        frame(&mut meter, &mut at, "overview", 1.0, 9);
        frame(&mut meter, &mut at, "overview", 0.5, 2);
        meter.page_switch(at);
        frame(&mut meter, &mut at, "tags", 0.25, 6);
        // A cached frame: the page did not render, it is still on screen.
        meter.frame_start(at);
        at += Duration::from_millis(1);
        meter.frame_end(at, 0.0);
        let pages = meter.page_figures();
        assert_eq!(
            pages
                .iter()
                .map(|page| (page.id, page.frames, page.first_ms))
                .collect::<Vec<_>>(),
            [("overview", 2, Some(9.0)), ("tags", 2, Some(6.0))]
        );
        assert_eq!(pages[0].render.max, 1.0);
        assert_eq!(pages[1].render.p50, 0.25, "only frames that rendered it");
        let slowest = meter.slowest();
        assert_eq!(
            slowest
                .iter()
                .map(|frame| (frame.cpu_ms, frame.page, frame.note, frame.render_ms))
                .collect::<Vec<_>>(),
            [
                (9.0, "overview", "first show, not warmed", 1.0),
                (6.0, "tags", "first show, not warmed", 0.25),
                (2.0, "overview", "", 0.5),
                (1.0, "tags", "", 0.0),
            ]
        );
    }

    #[test]
    fn only_the_five_slowest_frames_are_named() {
        let mut meter = FrameMeter::default();
        let mut at = Instant::now();
        for cpu in [3, 7, 1, 9, 4, 8, 2, 6] {
            frame(&mut meter, &mut at, "overview", 0.1, cpu);
        }
        let named: Vec<f64> = meter.slowest().iter().map(|frame| frame.cpu_ms).collect();
        assert_eq!(named, [9.0, 8.0, 7.0, 6.0, 4.0]);
    }

    #[test]
    fn warm_up_frames_stay_out_of_the_figures() {
        let mut meter = FrameMeter::default();
        let mut at = Instant::now();
        meter.set_warming(true);
        frame(&mut meter, &mut at, "overview", 5.0, 40);
        meter.batch_applied(3.0);
        meter.set_warming(false);
        frame(&mut meter, &mut at, "overview", 0.5, 3);
        meter.batch_applied(0.2);
        assert_eq!(meter.stats.total, 1);
        assert_eq!(meter.stats.cpu().max, 3.0);
        assert_eq!(
            meter.warm_up().map(|(frames, spread)| (frames, spread.max)),
            Some((1, 40.0))
        );
        assert_eq!(meter.apply_spread().max, 0.2);
        let overview = &meter.page_figures()[0];
        assert_eq!(
            (overview.frames, overview.first_ms, overview.render.max),
            (1, Some(40.0), 0.5),
            "its first frame was in the warm-up"
        );
        meter.reset();
        assert!(meter.page_figures().is_empty() && meter.warm_up().is_none());
    }

    #[test]
    fn a_warmed_page_is_named_so_on_its_first_show() {
        let mut meter = FrameMeter::default();
        let mut at = Instant::now();
        meter.set_warming(true);
        // The overview on screen, the tags page drawn out of sight.
        meter.frame_start(at);
        meter.page_built("overview", 1.0);
        meter.page_warmed("tags", 2.0);
        at += Duration::from_millis(30);
        meter.frame_end(at, 0.0);
        at += Duration::from_millis(16);
        meter.set_warming(false);
        meter.page_switch(at);
        frame(&mut meter, &mut at, "tags", 0.5, 4);
        frame(&mut meter, &mut at, "alarms", 0.5, 3);
        let tags = meter
            .page_figures()
            .into_iter()
            .find(|page| page.id == "tags")
            .unwrap();
        assert_eq!((tags.warmed_ms, tags.first_ms), (Some(30.0), Some(4.0)));
        let notes: Vec<_> = meter.slowest().iter().map(|frame| frame.note).collect();
        assert_eq!(notes, ["first show, warmed", "first show, not warmed"]);
    }

    #[test]
    fn a_frame_splits_into_phases_and_its_present_is_timed() {
        let mut meter = FrameMeter::default();
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        meter.frame_start(at(0));
        meter.laid_out(at(3));
        meter.painting(at(4));
        meter.frame_end(at(6), 0.0);
        meter.presented(at(9));
        let phases = meter.phases().unwrap();
        assert_eq!(
            (
                phases.layout.max,
                phases.prepaint.max,
                phases.paint.max,
                phases.present.max
            ),
            (3.0, 1.0, 2.0, 3.0)
        );
        meter.presented(at(12));
        assert_eq!(
            meter.phases().unwrap().present.max,
            3.0,
            "one present per frame"
        );
    }

    #[test]
    fn a_late_frame_after_a_long_one_is_put_down_to_it() {
        let mut meter = FrameMeter::default();
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        // Frames asked for back to back (an animation) on a 60 Hz display.
        meter.stats.set_display_hz(Some(60.0));
        meter.frame_start(at(0));
        meter.expect_frame(at(1));
        meter.frame_end(at(15), 0.0);
        meter.presented(at(19));
        meter.frame_start(at(20));
        meter.expect_frame(at(21));
        meter.frame_end(at(22), 0.0);
        meter.presented(at(23));
        meter.batch_applied(0.5);
        meter.frame_start(at(39));
        let late = meter.late_starts();
        assert_eq!((late.frames, late.late, late.previous_frame), (2, 2, 1));
        assert_eq!(late.event_batch + late.other, 1);
    }

    #[test]
    fn data_after_a_pause_past_a_vsync_is_not_a_drop() {
        let mut meter = FrameMeter::default();
        meter.stats.set_display_hz(Some(60.0));
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        meter.frame_start(at(0));
        meter.frame_end(at(3), 0.0);
        // Data in time for the next vsync, drawn two vsyncs later: a drop.
        meter.expect_frame(at(5));
        meter.frame_start(at(33));
        meter.frame_end(at(36), 0.0);
        assert_eq!(meter.stats.dropped, 1);
        // The feed paused: data came only after the next vsync and was
        // drawn at the one after it.
        meter.expect_frame(at(53));
        meter.frame_start(at(67));
        meter.frame_end(at(70), 0.0);
        assert_eq!((meter.stats.dropped, meter.stats.expected), (1, 2));
        assert_eq!(meter.late_starts().nothing_due, 1);
        assert_eq!(
            meter.stats.intervals().max,
            33.0,
            "the pause is no interval"
        );
    }

    #[test]
    fn a_long_frame_the_thread_did_not_work_through_is_a_stall() {
        let mut meter = FrameMeter::default();
        let mut at = Instant::now();
        // 60 ms of wall time, next to no CPU time on this thread.
        frame(&mut meter, &mut at, "tags", 0.1, 60);
        frame(&mut meter, &mut at, "tags", 0.1, 5);
        let (count, longest) = meter.stalls();
        assert_eq!(count, 1);
        assert!(longest > 50.0 && longest <= 60.0, "{longest}");
        let slowest = &meter.slowest()[0];
        assert!(slowest.on_cpu_ms.is_some_and(|ms| ms < 10.0));
        meter.reset();
        assert_eq!(meter.stalls(), (0, 0.0));
    }
}

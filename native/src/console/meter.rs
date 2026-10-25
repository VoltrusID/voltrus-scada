//! Measures the frames the window draws, for the HUD (⌘⇧P), the
//! Performance page and the stress report.
//!
//! The root view's render marks a frame's start and a zero-size canvas
//! painted last marks its end, so a frame's time is the CPU work of
//! building it (render, layout, paint). Whoever makes a frame due tells the
//! meter ([`FrameMeter::expect_frame`]): an animation asking for the next
//! frame, or data that changed what is shown. Only expected frames count
//! towards drops, so an idle console's long gaps are not drops.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use voltrus_scada_native::perf::{Frame, FrameStats, Spread};
use vt_ui::chart::History;

pub type Meter = Rc<RefCell<FrameMeter>>;

/// Frame samples the HUD's charts keep.
const TIMELINE: usize = 2048;

/// A due frame this soon after the previous one continues a run of frames
/// (a busy stream); a longer gap was a pause, not dropped frames.
const CONTINUOUS_MS: f64 = 100.0;

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
        }
    }
}

/// Milliseconds from `from` to `to`, to the microsecond.
#[allow(clippy::cast_precision_loss)]
fn ms(from: Instant, to: Instant) -> f64 {
    to.saturating_duration_since(from).as_micros() as f64 / 1000.0
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
            (Some(_), Some(gap)) if gap < CONTINUOUS_MS => (Some(gap), Some(gap)),
            // The first frame after a pause: how long it took to come.
            (Some((at, _)), _) => (Some(ms(at, now)), None),
            (None, _) => (None, None),
        };
        self.current = Some((delay, interval));
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

    /// The last element of the frame painted.
    pub fn frame_end(&mut self, now: Instant, wall_ms: f64) {
        let Some(start) = self.building.take() else {
            return;
        };
        let (delay_ms, interval_ms) = self.current.take().unwrap_or((None, None));
        let cpu_ms = ms(start, now);
        self.stats.record(Frame {
            interval_ms,
            delay_ms,
            cpu_ms,
        });
        self.cpu_line.push(wall_ms, cpu_ms);
        if let Some(interval) = interval_ms {
            self.interval_line.push(wall_ms, interval);
        }
        if let Some(asked) = self.switch.take() {
            self.switches.push(ms(asked, now));
        }
    }

    /// Frames drawn in the last second.
    pub fn fps(&self) -> usize {
        self.recent.len()
    }

    pub fn switch_spread(&self) -> Spread {
        Spread::of(self.switches.iter().copied())
    }

    /// Start counting afresh (a stress run begins).
    pub fn reset(&mut self) {
        self.stats.reset();
        self.switches.clear();
        self.cpu_line.clear();
        self.interval_line.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
}

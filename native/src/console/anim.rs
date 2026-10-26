//! Animation helpers. The rule: a view asks for the next frame only while
//! something it draws is moving ([`request_frame`]), so an idle console
//! draws nothing between data updates and the once-a-second tick. GPUI's
//! `with_animation` follows the same rule for one-shot transitions; pair it
//! with [`super::meter::Meter::expect_frame`] so the HUD counts its frames.

use std::time::{Duration, Instant};

use gpui::{Hsla, Window};

use super::meter::Meter;

/// A value change lights its tile up for this long.
pub const FLASH: Duration = Duration::from_millis(700);
/// A level bar glides to a new value in this long.
pub const TWEEN: Duration = Duration::from_millis(280);
/// A page fades and slides in this long after a switch.
pub const PAGE_IN: Duration = Duration::from_millis(140);
/// A toast slides in this long.
pub const TOAST_IN: Duration = Duration::from_millis(180);

/// Fast start, gentle stop.
pub fn ease_out(t: f32) -> f32 {
    let inverse = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inverse * inverse * inverse
}

/// How far, 0 to 1, an animation that started at `since` has run at `now`;
/// `None` once it has ended (or before it started).
pub fn progress(since: Instant, now: Instant, duration: Duration) -> Option<f32> {
    let elapsed = now.checked_duration_since(since)?;
    (elapsed < duration).then(|| elapsed.as_secs_f32() / duration.as_secs_f32())
}

/// `from` blended towards `to` by `t`.
pub fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let from = from.to_rgb();
    let to = to.to_rgb();
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    gpui::Rgba {
        r: lerp(from.r, to.r),
        g: lerp(from.g, to.g),
        b: lerp(from.b, to.b),
        a: lerp(from.a, to.a),
    }
    .into()
}

pub fn lerp(from: f64, to: f64, t: f32) -> f64 {
    from + (to - from) * f64::from(t.clamp(0.0, 1.0))
}

/// Ask for the next frame for the view being drawn, and tell the meter a
/// frame is due. Call it from a render function while something moves.
pub fn request_frame(window: &mut Window, meter: &Meter) {
    meter.borrow_mut().expect_frame(Instant::now());
    window.request_animation_frame();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_runs_from_zero_and_ends() {
        let start = Instant::now();
        let second = Duration::from_secs(1);
        assert_eq!(progress(start, start, second), Some(0.0));
        let half = progress(start, start + Duration::from_millis(500), second).unwrap();
        assert!((half - 0.5).abs() < 1e-6);
        assert_eq!(progress(start, start + second, second), None, "ended");
        assert_eq!(progress(start + second, start, second), None, "not started");
    }

    #[test]
    fn easing_and_blending_hit_their_ends() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5, "ease out runs ahead");
        assert_eq!(lerp(10.0, 20.0, 0.25), 12.5);
        let black = gpui::black();
        let white = gpui::white();
        let grey = mix(black, white, 0.5).to_rgb();
        assert!((grey.r - 0.5).abs() < 0.01 && (grey.a - 1.0).abs() < 1e-6);
    }
}

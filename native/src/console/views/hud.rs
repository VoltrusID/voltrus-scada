//! The performance HUD (⌘⇧P): frame rate, frame times, drops against the
//! display's refresh, the last frame's build time, events, tags and memory.
//! It reads the meter and redraws only itself, a few times a second, while
//! shown (see `Console::set_hud`); hidden, it is not drawn at all.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, Pixels, Window, div, px};
use voltrus_scada_native::perf;
use vt_ui::theme::*;

use crate::console::Console;
use crate::console::keys::{TogglePerfHud, hint};

pub(crate) const WIDTH: Pixels = px(350.0);
pub(crate) const HEIGHT: Pixels = px(232.0);

pub fn render(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let rate = console.rate.per_second(Instant::now());
    let meter = console.meter.borrow();
    let stats = &meter.stats;
    let refresh = stats.refresh_ms();
    let intervals = stats.intervals();
    let cpu = stats.cpu();
    let percent = perf::drop_percent(stats.dropped, stats.expected);
    let memory = perf::peak_rss_bytes().map_or_else(
        || "—".to_owned(),
        |bytes| format!("{:.0} MB peak", bytes as f64 / 1_048_576.0),
    );
    let budget_color = |ms: f64| {
        if ms > refresh * 1.5 {
            theme.danger
        } else if ms > refresh * 0.8 {
            theme.warning
        } else {
            theme.fg
        }
    };
    let rows = [
        (
            "Frames",
            format!("{} fps · {:.0} Hz display", meter.fps(), 1000.0 / refresh),
            theme.fg,
        ),
        (
            "Frame time",
            format!(
                "p50 {:.1} · p95 {:.1} · max {:.1} ms",
                intervals.p50, intervals.p95, intervals.max
            ),
            budget_color(intervals.p95),
        ),
        (
            "Build time",
            format!(
                "p50 {:.1} · p95 {:.1} · max {:.1} ms",
                cpu.p50, cpu.p95, cpu.max
            ),
            budget_color(cpu.p95),
        ),
        (
            "Last frame",
            stats
                .last_cpu_ms()
                .map_or_else(|| "—".to_owned(), |ms| format!("{ms:.2} ms to build")),
            theme.fg,
        ),
        (
            "Dropped",
            format!("{} ({percent:.1}% of vsyncs)", stats.dropped),
            if stats.dropped == 0 {
                theme.fg
            } else {
                theme.warning
            },
        ),
        ("Events", format!("{rate} updates/s"), theme.fg),
        ("Tags", console.store.tags.len().to_string(), theme.fg),
        ("Memory", memory, theme.fg),
    ];
    let stress = console.stress.as_ref().map(|run| {
        format!(
            "Stress {:.0} s · {} tags · {}/s",
            run.started.elapsed().as_secs_f64(),
            run.plan.options.tags,
            run.plan.options.rate
        )
    });

    // Opaque, like every popover: the page under it must not show through
    // its figures.
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(SPACE_2)
        .p(SPACE_6)
        .overflow_hidden()
        .rounded(RADIUS_LG)
        .bg(theme.elevated)
        .border_1()
        .border_color(theme.border_strong)
        .shadow(theme.shadow(Elevation::Popover))
        .font_family(MONO)
        .text_size(FONT_SMALL)
        .child(
            div()
                .flex()
                .justify_between()
                .pb(SPACE_2)
                .font_family(SANS)
                .font_weight(FontWeight::SEMIBOLD)
                .child("Performance")
                .child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(theme.fg_dim)
                        .child(stress.unwrap_or_else(|| {
                            format!("{} to hide", hint(&TogglePerfHud).unwrap_or_default())
                        })),
                ),
        )
        .children(rows.into_iter().map(|(label, value, color)| {
            div()
                .flex()
                .gap(SPACE_4)
                .child(
                    div()
                        .w(px(76.0))
                        .flex_none()
                        .text_color(theme.fg_dim)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(color)
                        .child(value),
                )
        }))
        .into_any_element()
}

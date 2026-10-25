//! The Performance page: frame timing over the last seconds, the stress
//! test's controls, and the last stress report. A page added the registry
//! way: this module and one row in `pages::PAGES`; its settings live in its
//! own state type (`Console::state`).

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, SharedString, Window, div, px};
use voltrus_scada_native::perf;
use voltrus_scada_native::stress::StressOptions;
use vt_ui::Icon;
use vt_ui::chart::{Series, trend_chart};
use vt_ui::components::{Button, Segment, section_label, segmented};
use vt_ui::theme::*;

use super::{LIST_TAIL, card, card_title, now_ms, page_header};
use crate::console::keys::{StartStress, StopStress};
use crate::console::{Console, StressPlan};

/// The stress test's settings, as the page's controls set them.
pub struct Settings {
    rate: u32,
    tags: usize,
    /// Seconds between automatic page switches; 0 stays put.
    switch_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        let options = StressOptions::default();
        Self {
            rate: options.rate,
            tags: options.tags,
            switch_secs: 2,
        }
    }
}

/// The stress run the palette's "Start the stress test" begins.
pub fn plan(console: &mut Console) -> StressPlan {
    let settings = console.state::<Settings>();
    StressPlan {
        options: StressOptions {
            tags: settings.tags,
            rate: settings.rate,
            ..StressOptions::default()
        },
        switch_every: (settings.switch_secs > 0).then(|| Duration::from_secs(settings.switch_secs)),
        stop_after: None,
        report: None,
    }
}

const RATES: [u32; 4] = [5_000, 20_000, 50_000, 100_000];
const TAG_COUNTS: [usize; 3] = [1_000, 10_000, 25_000];
const SWITCHES: [u64; 4] = [0, 1, 2, 5];
/// The frame chart's span.
const CHART_MS: f64 = 10_000.0;

pub fn render(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let (rate, tags, switch_secs) = {
        let settings = console.state::<Settings>();
        (settings.rate, settings.tags, settings.switch_secs)
    };
    let running = console.stress.is_some();
    let meter = console.meter.borrow();
    let stats = &meter.stats;
    let intervals = stats.intervals();
    let cpu = stats.cpu();
    let refresh = stats.refresh_ms();
    let figures = [
        (
            "Frames",
            format!("{} fps", meter.fps()),
            format!("display {:.0} Hz", 1000.0 / refresh),
        ),
        (
            "Frame time p95",
            format!("{:.1} ms", intervals.p95),
            format!("p50 {:.1} · max {:.1}", intervals.p50, intervals.max),
        ),
        (
            "Build time p95",
            format!("{:.1} ms", cpu.p95),
            format!("p50 {:.1} · max {:.1}", cpu.p50, cpu.max),
        ),
        (
            "Dropped frames",
            stats.dropped.to_string(),
            format!(
                "{:.1}% of vsyncs while due",
                perf::drop_percent(stats.dropped, stats.expected)
            ),
        ),
        (
            "Page switch p95",
            format!("{:.1} ms", meter.switch_spread().p95),
            format!("{} switches, to first frame", meter.switches.len()),
        ),
        (
            "Memory",
            perf::peak_rss_bytes().map_or_else(
                || "—".to_owned(),
                |bytes| format!("{:.0} MB", bytes as f64 / 1_048_576.0),
            ),
            "peak resident".to_owned(),
        ),
    ];
    let series = vec![
        Series {
            label: "Frame interval".into(),
            color: theme.series(0),
            history: meter.interval_line.clone(),
            stepped: false,
        },
        Series {
            label: "Build time".into(),
            color: theme.series(1),
            history: meter.cpu_line.clone(),
            stepped: false,
        },
    ];
    drop(meter);
    let report = match console.stress_report() {
        Some(report) => Some(("Running", report)),
        None => console
            .last_report
            .clone()
            .map(|report| ("Last run", report)),
    };
    let status = console.stress.as_ref().map_or_else(
        || "Stopped".to_owned(),
        |run| {
            format!(
                "Running {:.0} s · {} updates applied",
                Instant::now().duration_since(run.started).as_secs_f64(),
                run.applied
            )
        },
    );

    let choice = |label: String, selected: bool, set: fn(&mut Settings, usize), index: usize| {
        Segment::new(
            label,
            selected,
            cx.listener(move |console, _, _, cx| {
                set(console.state::<Settings>(), index);
                cx.notify();
            }),
        )
    };
    let rates = RATES
        .iter()
        .enumerate()
        .map(|(index, value)| {
            choice(
                format!("{}k/s", value / 1000),
                *value == rate,
                |settings, index| settings.rate = RATES[index],
                index,
            )
        })
        .collect();
    let counts = TAG_COUNTS
        .iter()
        .enumerate()
        .map(|(index, value)| {
            choice(
                format!("{}k", value / 1000),
                *value == tags,
                |settings, index| settings.tags = TAG_COUNTS[index],
                index,
            )
        })
        .collect();
    let switches = SWITCHES
        .iter()
        .enumerate()
        .map(|(index, value)| {
            choice(
                if *value == 0 {
                    "Off".to_owned()
                } else {
                    format!("{value} s")
                },
                *value == switch_secs,
                |settings, index| settings.switch_secs = SWITCHES[index],
                index,
            )
        })
        .collect();
    let control = |label: &'static str, control: AnyElement| {
        div()
            .flex()
            .items_center()
            .gap(SPACE_5)
            .child(div().w(px(120.0)).text_color(theme.fg_muted).child(label))
            .child(control)
    };

    div()
        .id("performance")
        .flex()
        .flex_col()
        .size_full()
        .overflow_y_scroll()
        .child(page_header(
            "Performance",
            "Frame timing, the stress test and what the console is doing",
            vec![if running {
                Button::new("stress-stop", "Stop the stress test")
                    .danger()
                    .icon(Icon::Square)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(StopStress), cx))
                    .into_any_element()
            } else {
                Button::new("stress-start", "Start the stress test")
                    .primary()
                    .icon(Icon::Zap)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(StartStress), cx))
                    .into_any_element()
            }],
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_8)
                .p(SPACE_9)
                .pb(LIST_TAIL)
                .child(div().grid().grid_cols(3).gap(SPACE_6).children(figures.into_iter().map(
                    |(label, value, detail)| {
                        card(&theme)
                            .gap(SPACE_2)
                            .p(SPACE_7)
                            .child(div().text_size(FONT_SMALL).text_color(theme.fg_muted).child(label))
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(FONT_DISPLAY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(value),
                            )
                            .child(div().truncate().text_size(FONT_SMALL).text_color(theme.fg_dim).child(detail))
                    },
                )))
                .child(
                    card(&theme)
                        .h(px(220.0))
                        .child(card_title("Frames, last 10 s (ms)", None, None, &theme))
                        .child(div().flex_1().min_h_0().p(SPACE_6).child(trend_chart(
                            series,
                            now_ms(),
                            CHART_MS,
                            cx,
                        ))),
                )
                .child(
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap(SPACE_6)
                        .child(
                            card(&theme)
                                .child(card_title("Stress test", None, None, &theme))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(SPACE_5)
                                        .p(SPACE_7)
                                        .child(control("Updates", segmented("stress-rate", rates, cx)))
                                        .child(control("Tags", segmented("stress-tags", counts, cx)))
                                        .child(control(
                                            "Switch pages",
                                            segmented("stress-switch", switches, cx),
                                        ))
                                        .child(
                                            div()
                                                .text_size(FONT_SMALL)
                                                .text_color(if running { theme.info } else { theme.fg_dim })
                                                .child(status),
                                        ),
                                ),
                        )
                        .child(
                            card(&theme)
                                .child(card_title(
                                    report.as_ref().map_or("Report", |(title, _)| *title),
                                    None,
                                    None,
                                    &theme,
                                ))
                                .child(
                                    div()
                                        .p(SPACE_7)
                                        .flex()
                                        .flex_col()
                                        .gap(SPACE_2)
                                        .font_family(MONO)
                                        .text_size(FONT_SMALL)
                                        .map(|this| match report {
                                            Some((_, report)) => this.children(
                                                report
                                                    .to_string()
                                                    .lines()
                                                    .map(|line| SharedString::from(line.to_owned()))
                                                    .collect::<Vec<_>>(),
                                            ),
                                            None => this
                                                .text_color(theme.fg_dim)
                                                .child("No stress run yet. Start one above, from ⌘K, or with --stress."),
                                        }),
                                )
                                .child(div().px(SPACE_7).pb(SPACE_6).child(section_label(
                                    "Printed to stdout when a run ends",
                                    cx,
                                ))),
                        ),
                ),
        )
        .into_any_element()
}

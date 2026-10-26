//! System › System: the host, its CPU, memory and disk, the database file
//! and the server process (`GET /api/v1/system`), as the web panel's
//! `SystemOverview`, polled every 5 s while on screen.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, Window, div, px};
use voltrus_scada_native::api::eng_system::{
    Level, SYSTEM_PATH, SystemInfo, db_size, fill, uptime,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::theme::*;

use super::{level_color, meter};
use crate::console::Console;
use crate::console::views::ops_kit::notice;
use crate::console::views::{LIST_TAIL, card, value_with_unit};

pub(super) const EVERY: Duration = Duration::from_secs(5);
const SCROLL: &str = "data.system.overview";

#[derive(Default)]
struct State {
    info: Option<Result<SystemInfo, String>>,
    asked: bool,
    seq: i64,
}

pub(super) fn open(console: &mut Console) {
    if !console.state::<State>().asked {
        fetch(console);
    }
}

pub(super) fn fetch(console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.asked = true;
    state.seq += 1;
    let seq = state.seq;
    console.call(
        CallKey::new("data.system.overview").with(seq),
        Call::get::<SystemInfo>(SYSTEM_PATH),
    );
}

pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    _: &mut Context<Console>,
) {
    let state = console.state::<State>();
    if key.arg != state.seq {
        return;
    }
    match reply.take::<SystemInfo>() {
        Ok(info) => state.info = Some(Ok(info)),
        // A failed poll keeps the last figures on screen.
        Err(error) => {
            if !matches!(state.info, Some(Ok(_))) {
                state.info = Some(Err(error));
            }
        }
    }
}

/// A usage card: the figure over its total, the percentage and a bar.
fn usage(
    label: &'static str,
    used: String,
    total: String,
    percent: f64,
    theme: &Theme,
) -> AnyElement {
    let color = level_color(Level::bar(percent), theme);
    card(theme)
        .p(SPACE_7)
        .gap(SPACE_3)
        .child(caption(label, theme))
        .child(value_with_unit(used, &total, FONT_DISPLAY, theme.fg, theme))
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(format!("{percent:.1}%")),
        )
        .child(meter(fill(percent), color, theme))
        .into_any_element()
}

fn caption(label: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(label.to_uppercase())
}

pub(super) fn render(
    console: &mut Console,
    _: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let info = console.state::<State>().info.clone();
    let info = match info {
        None => return notice("Connecting to the server…", false, &theme),
        Some(Err(error)) => return notice(format!("System figures: {error}"), true, &theme),
        Some(Ok(info)) => info,
    };
    let scroll = console.scroll(SCROLL);
    let cpu_color = level_color(Level::cpu(info.cpu_usage), &theme);
    let (db, db_detail) = db_size(info.db_size_mb);
    let db_color = if info.db_size_mb > 512.0 {
        theme.warning
    } else {
        theme.info
    };
    let process = &info.process;

    let host = card(&theme)
        .p(SPACE_7)
        .gap(SPACE_3)
        .child(caption("Host", &theme))
        .child(
            div()
                .truncate()
                .font_family(MONO)
                .text_size(FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .child(info.hostname.clone()),
        )
        .child(
            div()
                .flex()
                .gap(SPACE_3)
                .min_w_0()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(div().truncate().child(info.os.clone()))
                .child("· up")
                .child(div().font_family(MONO).child(uptime(info.uptime_secs))),
        );
    let cpu = card(&theme)
        .p(SPACE_7)
        .gap(SPACE_3)
        .child(caption("CPU usage", &theme))
        .child(value_with_unit(
            format!("{:.1}", info.cpu_usage),
            "%",
            FONT_DISPLAY,
            cpu_color,
            &theme,
        ))
        .child(div().h(px(16.0)))
        .child(meter(
            fill(info.cpu_usage),
            level_color(Level::bar(info.cpu_usage), &theme),
            &theme,
        ));
    let memory = usage(
        "Memory",
        format!("{:.0}", info.memory_used_mb),
        format!("/ {:.0} MB", info.memory_total_mb),
        info.memory_percent,
        &theme,
    );
    let disk = usage(
        "Disk",
        format!("{:.1}", info.disk_used_gb),
        format!("/ {:.1} GB", info.disk_total_gb),
        info.disk_percent,
        &theme,
    );
    let database = card(&theme)
        .p(SPACE_7)
        .gap(SPACE_3)
        .child(caption("Database", &theme))
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(db_color)
                .child(db),
        )
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(db_detail),
        );
    let figures = [
        ("RSS memory", format!("{:.1} MB", process.memory_mb)),
        ("CPU usage", format!("{:.1}%", process.cpu_percent)),
        ("Uptime", uptime(process.uptime_secs)),
        ("Binary size", format!("{:.1} MB", process.binary_size_mb)),
    ];
    let service = card(&theme)
        .p(SPACE_7)
        .gap(SPACE_5)
        .border_color(theme.success)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.success)
                .child("RUST SERVICE")
                .child(
                    div()
                        .font_family(MONO)
                        .child(format!("PID {}", process.pid)),
                ),
        )
        .child(
            div()
                .grid()
                .grid_cols(4)
                .gap(SPACE_6)
                .children(figures.into_iter().map(|(label, value)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_1)
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(FONT_DISPLAY)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.success)
                                .whitespace_nowrap()
                                .child(value),
                        )
                        .child(
                            div()
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(label),
                        )
                })),
        );

    div()
        .id(SCROLL)
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .p(SPACE_9)
                .pb(LIST_TAIL)
                .child(
                    div()
                        .grid()
                        .grid_cols(3)
                        .gap(SPACE_6)
                        .child(host)
                        .child(cpu)
                        .child(memory)
                        .child(disk)
                        .child(database),
                )
                .child(service),
        )
        .into_any_element()
}

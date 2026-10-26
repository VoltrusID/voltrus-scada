//! System › Health, after the web client's `HealthCheckPanel.tsx`: the
//! project health check (`GET /api/v1/wizard/health-check`), its score and
//! counters, and the issues found, each opened with ↵ (or a click) to its
//! suggestion and the items it affects. r (or the button) runs it again.

use std::collections::HashSet;

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, Window, div, px};
use voltrus_scada_native::api::eng_system::{
    HEALTH_PATH, HealthReport, HealthSeverity, health_verdict,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, pill};
use vt_ui::theme::*;

use super::{query, severity_color};
use crate::console::Console;
use crate::console::views::ops_kit::notice;
use crate::console::views::{LIST_TAIL, ListKeys, card, focus_ring, keyboard_list};

pub(super) const LIST: &str = "data.system.health";
const SCROLL: &str = "data.system.health.issues";
/// Affected items shown before "…and N more", as the web panel.
const AFFECTED: usize = 15;

#[derive(Default)]
struct State {
    report: Option<Result<HealthReport, String>>,
    asked: bool,
    running: bool,
    seq: i64,
    /// The issues opened, by their place in the report.
    expanded: HashSet<usize>,
}

pub(super) fn open(console: &mut Console) {
    if !console.state::<State>().asked {
        run(console);
    }
}

/// Run the check (again).
pub(super) fn run(console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.asked = true;
    state.running = true;
    state.seq += 1;
    let seq = state.seq;
    console.call(
        CallKey::new("data.system.health").with(seq),
        Call::get::<HealthReport>(HEALTH_PATH),
    );
}

pub(super) fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    console.focus_list(LIST, window, cx);
    true
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
    state.running = false;
    let report = reply.take::<HealthReport>();
    if report.is_ok() {
        state.expanded.clear();
    }
    state.report = Some(report);
}

pub(super) fn run_button(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let running = console.state::<State>().running;
    Button::new(
        "system-health-run",
        if running {
            "Running…"
        } else {
            "Run health check"
        },
    )
    .primary()
    .icon(Icon::RefreshCw)
    .tooltip("R")
    .disabled(running)
    .on_click(cx.listener(|console, _, _, cx| {
        run(console);
        cx.notify();
    }))
    .into_any_element()
}

/// The issues the search leaves, by their place in the report.
fn shown(report: &HealthReport, search: &str) -> Vec<usize> {
    report
        .issues
        .iter()
        .enumerate()
        .filter(|(_, issue)| issue.matches(search))
        .map(|(index, _)| index)
        .collect()
}

/// ↵ or Space on an issue: open or close it.
fn toggle(console: &mut Console, row: usize, _: bool, _: &mut Window, cx: &mut Context<Console>) {
    let search = query(console, cx);
    let state = console.state::<State>();
    let Some(Ok(report)) = &state.report else {
        return;
    };
    let Some(index) = shown(report, &search).get(row).copied() else {
        return;
    };
    if !state.expanded.remove(&index) {
        state.expanded.insert(index);
    }
    console.scroll(SCROLL).scroll_to_item(row);
    cx.notify();
}

/// The cursor moved: keep its issue in view.
fn moved(console: &mut Console, row: usize, _: &mut Window, _: &mut Context<Console>) {
    console.scroll(SCROLL).scroll_to_item(row);
}

#[allow(clippy::too_many_lines)]
pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let (report, running, expanded) = (state.report.clone(), state.running, state.expanded.clone());
    let report = match report {
        None => return notice("Running the health check…", false, &theme),
        Some(Err(error)) => return notice(format!("Health check: {error}"), true, &theme),
        Some(Ok(report)) => report,
    };
    let search = query(console, cx);
    let issues = shown(&report, &search);
    let summary = &report.summary;
    let (verdict, verdict_level) = health_verdict(report.overall_score);
    let score_color = severity_color(verdict_level, &theme);

    let count = |label: &'static str, value: u32, severity: HealthSeverity| {
        div()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_TITLE)
                    .font_weight(FontWeight::BOLD)
                    .text_color(severity_color(severity, &theme))
                    .child(value.to_string()),
            )
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(label),
            )
    };
    let score = card(&theme)
        .flex_row()
        .items_center()
        .gap(SPACE_9)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(72.0))
                .rounded(px(36.0))
                .border_2()
                .border_color(score_color)
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_VALUE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(score_color)
                        .child(report.overall_score.to_string()),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(SPACE_2)
                .child(
                    div()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if running {
                            format!("{verdict} · running again…")
                        } else {
                            verdict.to_owned()
                        }),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(format!(
                            "{} devices · {} tags · {} alarms · {} flows · {} screens",
                            summary.total_devices,
                            summary.total_tags,
                            summary.total_alarms,
                            summary.total_flows,
                            summary.total_screens
                        )),
                ),
        )
        .child(count(
            "Critical",
            summary.critical_issues,
            HealthSeverity::Critical,
        ))
        .child(count(
            "Warnings",
            summary.warning_issues,
            HealthSeverity::Warning,
        ))
        .child(count("Info", summary.info_issues, HealthSeverity::Info));
    let counters =
        div()
            .grid()
            .grid_cols(6)
            .gap(SPACE_5)
            .children(
                summary
                    .counters()
                    .into_iter()
                    .map(|(label, value, severity)| {
                        card(&theme)
                            .p(SPACE_6)
                            .gap(SPACE_1)
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(FONT_TITLE)
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(severity_color(severity, &theme))
                                    .child(value.to_string()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(FONT_CAPTION)
                                    .text_color(theme.fg_dim)
                                    .child(label),
                            )
                    }),
            );

    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: Some(moved),
            on_activate: toggle,
        },
        issues.len(),
        5,
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    let scroll = console.scroll(SCROLL);

    let body = if report.issues.is_empty() {
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap(SPACE_3)
            .p(SPACE_10)
            .text_color(theme.success)
            .font_weight(FontWeight::SEMIBOLD)
            .child(vt_ui::icon(Icon::CircleCheck, ICON_LG, theme.success))
            .child("No issues found — your project is in great shape!")
            .into_any_element()
    } else if issues.is_empty() {
        notice("No issue matches the search", false, &theme)
    } else {
        div()
            .id(SCROLL)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .px(SPACE_9)
            .pb(LIST_TAIL)
            .children(issues.iter().enumerate().map(|(row, &index)| {
                let issue = &report.issues[index];
                let open = expanded.contains(&index);
                let color = severity_color(issue.severity, &theme);
                let hover = theme.hover;
                let current = cursor == Some(row);
                let more = issue.affected_items.len().saturating_sub(AFFECTED);
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .rounded(RADIUS_LG)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .overflow_hidden()
                    .child(
                        div()
                            .id(ElementId::NamedInteger(
                                "system-health-issue".into(),
                                u64::try_from(row).unwrap_or(0),
                            ))
                            .relative()
                            .flex()
                            .items_center()
                            .gap(SPACE_5)
                            .px(SPACE_7)
                            .py(SPACE_5)
                            .cursor_pointer()
                            .when(current, |this| this.bg(theme.selected))
                            .when(!current, |this| this.hover(move |style| style.bg(hover)))
                            .when(focused && current, |this| this.child(focus_ring(&theme)))
                            .on_click(cx.listener(move |console, _, window, cx| {
                                let nav = console.list(LIST, cx);
                                nav.select(row);
                                let focus = nav.focus.clone();
                                window.focus(&focus);
                                toggle(console, row, false, window, cx);
                            }))
                            .child(div().flex_none().size(px(10.0)).rounded(px(5.0)).bg(color))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(issue.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(FONT_SMALL)
                                            .text_color(theme.fg_muted)
                                            .child(issue.description.clone()),
                                    ),
                            )
                            .child(pill(issue.severity.id().to_uppercase(), color))
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(format!("{} affected", issue.count)),
                            )
                            .child(vt_ui::icon(
                                if open {
                                    Icon::ChevronUp
                                } else {
                                    Icon::ChevronDown
                                },
                                ICON_MD,
                                theme.fg_dim,
                            )),
                    )
                    .when(open, |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(SPACE_3)
                                .px(SPACE_8)
                                .pt(SPACE_4)
                                .pb(SPACE_7)
                                .bg(theme.bg)
                                .border_t_1()
                                .border_color(theme.border)
                                .text_size(FONT_SMALL)
                                .child(issue.description.clone())
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme.accent)
                                        .child("Suggestion"),
                                )
                                .child(
                                    div()
                                        .text_color(theme.fg_muted)
                                        .child(issue.suggestion.clone()),
                                )
                                .when(!issue.affected_items.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .pt(SPACE_3)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Affected items"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap(SPACE_2)
                                            .children(
                                                issue.affected_items.iter().take(AFFECTED).map(
                                                    |item| {
                                                        div()
                                                            .px(SPACE_3)
                                                            .rounded(RADIUS_SM)
                                                            .bg(theme.surface)
                                                            .border_1()
                                                            .border_color(theme.border)
                                                            .font_family(MONO)
                                                            .text_size(FONT_CAPTION)
                                                            .child(item.clone())
                                                    },
                                                ),
                                            )
                                            .when(more > 0, |this| {
                                                this.child(
                                                    div()
                                                        .text_size(FONT_CAPTION)
                                                        .text_color(theme.fg_dim)
                                                        .child(format!("…and {more} more")),
                                                )
                                            }),
                                    )
                                }),
                        )
                    })
            }))
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(SPACE_6)
                .p(SPACE_9)
                .pb(SPACE_6)
                .child(score)
                .child(counters)
                .when(!report.issues.is_empty(), |this| {
                    this.child(
                        div()
                            .text_size(FONT_EMPHASIS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Issues found · {}", report.issues.len())),
                    )
                }),
        )
        .child(list.flex().flex_col().flex_1().min_h_0().child(body))
        .into_any_element()
}

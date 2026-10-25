//! The Alarms page's overlays: the shelve dialog (a duration and a
//! reason), the acknowledge-all confirmation, and the detail sheet with the
//! alarm's ISA-18.2 journal.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, FontWeight, SharedString, Window, div, px};
use voltrus_scada_native::api::Alarm;
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{Button, kbd, pill, section_label};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::super::{age, alarm_source, local_time};
use super::list::{self, acknowledge, direction, shelve_or_unshelve, state_line};
use super::{LIST, State, fetch_journal, priority_color};
use crate::console::alarms::{self, DEFAULT_SHELVE, SHELVE_DURATIONS, is_resolved};
use crate::console::keys::{
    ALARM_LIST, AlarmsAcknowledge, AlarmsShelve, LIST as LIST_CONTEXT, ListActivate, ListDown,
    ListUp, MENU,
};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::{Console, is_shelved, severity_rank, time_now_secs};

pub(super) const SHELVE: &str = "alarms.shelve";
pub(super) const CONFIRM: &str = "alarms.confirm";
pub(super) const DETAIL: &str = "alarms.detail";

/// The alarm `id`, from the open alarms or the history.
fn find(console: &Console, id: i64) -> Option<&Alarm> {
    console
        .alarms
        .iter()
        .find(|alarm| alarm.id == id)
        .or_else(|| {
            console
                .state_ref::<State>()
                .and_then(|state| state.history.as_ref())
                .and_then(|history| history.as_ref().ok())
                .and_then(|history| history.iter().find(|alarm| alarm.id == id))
        })
}

/// A dialog's title row: a title, a line about it, and its key.
fn title(title: &'static str, subtitle: String, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .flex_col()
        .gap(SPACE_1)
        .px(SPACE_8)
        .pt(SPACE_7)
        .pb(SPACE_6)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(subtitle),
        )
}

/// A dialog's bottom row: key hints on the left, buttons on the right.
fn footer(
    hints: &[(&'static str, &'static str)],
    buttons: Vec<AnyElement>,
    theme: &Theme,
    cx: &Context<Console>,
) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .px(SPACE_8)
        .py(SPACE_6)
        .border_t_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_dim)
        .children(hints.iter().map(|(keys, label)| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(kbd(*keys, cx))
                .child(*label)
        }))
        .child(div().flex_1())
        .children(buttons)
}

// ── Shelve ────────────────────────────────────────────────────────────────

/// The shelve dialog for alarm `id`: the duration (↑/↓ or a click) and an
/// optional reason; Enter shelves, Esc cancels.
pub(super) fn open_shelve(
    console: &mut Console,
    id: i64,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !console.is_operator() {
        return;
    }
    if console.state::<State>().reason.is_none() {
        let field = cx.new(|cx| TextInput::new(cx).placeholder("Reason (optional)"));
        let subscription = cx.subscribe_in(
            &field,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => submit_shelve(console, window, cx),
                InputEvent::Cancelled => console.close_overlay(window, cx),
                InputEvent::Changed | InputEvent::Blurred => {}
            },
        );
        let state = console.state::<State>();
        state.reason = Some(field);
        state.subscriptions.push(subscription);
    }
    let state = console.state::<State>();
    state.shelving = Some((id, DEFAULT_SHELVE));
    let field = state.reason.clone();
    let overlay = Overlay::modal(SHELVE, px(440.0), shelve_dialog, cx);
    console.open_overlay(overlay, window, cx);
    if let Some(field) = field {
        field.update(cx, |field, cx| {
            field.set_text("", cx);
            field.focus(window);
        });
    }
}

fn step_duration(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    if let Some((_, duration)) = &mut console.state::<State>().shelving {
        *duration = if forward {
            (*duration + 1).min(SHELVE_DURATIONS.len() - 1)
        } else {
            duration.saturating_sub(1)
        };
    }
    cx.notify();
}

fn submit_shelve(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some((id, duration)) = state.shelving.take() else {
        return;
    };
    let reason = state
        .reason
        .as_ref()
        .map(|field| field.read(cx).value().trim().to_owned())
        .unwrap_or_default();
    let (seconds, label) = SHELVE_DURATIONS[duration.min(SHELVE_DURATIONS.len() - 1)];
    console.shelve(id, seconds, reason);
    console.close_overlay(window, cx);
    console.show_toast(format!("Shelving alarm {id} for {label}"), false, cx);
}

fn shelve_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some((id, chosen)) = console.state::<State>().shelving else {
        return div().into_any_element();
    };
    let field = console.state::<State>().reason.clone();
    let alarm = find(console, id);
    let subtitle = alarm.map_or_else(
        || format!("Alarm {id}"),
        |alarm| {
            format!(
                "{} · {} · raised {}",
                alarm_source(console, alarm),
                alarms::priority(alarm),
                local_time(alarm.triggered_at, console.utc_offset)
            )
        },
    );
    let until = time_now_secs()
        + i64::try_from(SHELVE_DURATIONS[chosen.min(SHELVE_DURATIONS.len() - 1)].0).unwrap_or(0);
    let options = SHELVE_DURATIONS
        .iter()
        .enumerate()
        .map(|(index, (_, label))| {
            let selected = index == chosen;
            let hover = theme.hover;
            div()
                .id(("shelve-duration", index))
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(28.0))
                .px(SPACE_5)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .when(selected, |this| this.bg(theme.selected))
                .when(!selected, |this| this.hover(move |style| style.bg(hover)))
                .on_click(cx.listener(move |console, _, _, cx| {
                    if let Some((_, duration)) = &mut console.state::<State>().shelving {
                        *duration = index;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(12.0))
                        .rounded_full()
                        .border_1()
                        .border_color(if selected {
                            theme.accent
                        } else {
                            theme.border_strong
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |this| {
                            this.child(div().size(px(6.0)).rounded_full().bg(theme.accent))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .font_weight(if selected {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(*label),
                )
        })
        .collect::<Vec<_>>();
    let label = SHELVE_DURATIONS[chosen.min(SHELVE_DURATIONS.len() - 1)].1;

    div()
        .key_context(MENU)
        .on_action(cx.listener(|console, _: &ListUp, _, cx| {
            step_duration(console, false, cx);
        }))
        .on_action(cx.listener(|console, _: &ListDown, _, cx| {
            step_duration(console, true, cx);
        }))
        .flex()
        .flex_col()
        .child(title("Shelve the alarm", subtitle, &theme))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_2)
                        .child(section_label("For", cx))
                        .children(options),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_3)
                        .child(section_label("Reason", cx))
                        .child(
                            div()
                                .px(SPACE_4)
                                .py(SPACE_3)
                                .rounded(RADIUS_MD)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.bg)
                                .children(field),
                        ),
                )
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(format!(
                            "Hidden from the open alarms and silent until {}; it comes back by itself.",
                            local_time(until, console.utc_offset)
                        )),
                ),
        )
        .child(footer(
            &[("↑↓", "Duration"), ("↵", "Shelve"), ("Esc", "Cancel")],
            vec![
                Button::new("shelve-cancel", "Cancel")
                    .ghost()
                    .on_click(cx.listener(|console, _, window, cx| {
                        console.close_overlay(window, cx);
                    }))
                    .into_any_element(),
                Button::new("shelve-submit", format!("Shelve for {label}"))
                    .primary()
                    .icon(Icon::Clock)
                    .on_click(cx.listener(|console, _, window, cx| {
                        submit_shelve(console, window, cx);
                    }))
                    .into_any_element(),
            ],
            &theme,
            cx,
        ))
        .into_any_element()
}

// ── Acknowledge all ───────────────────────────────────────────────────────

/// ⇧A, ⇧⌘A and the button: ask before acknowledging every alarm.
pub(crate) fn confirm_acknowledge_all(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !console.is_signed_in() {
        return;
    }
    let focus = console
        .state::<State>()
        .confirm_focus
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    let overlay = Overlay::modal(CONFIRM, px(420.0), confirm_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn acknowledge_all_now(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let count = console.unacknowledged();
    console.close_overlay(window, cx);
    if count > 0 {
        console.acknowledge_all();
        console.show_toast(
            format!(
                "Acknowledging {count} alarm{}",
                if count == 1 { "" } else { "s" }
            ),
            false,
            cx,
        );
    }
}

fn confirm_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(focus) = console.state::<State>().confirm_focus.clone() else {
        return div().into_any_element();
    };
    let waiting: Vec<&Alarm> = console
        .alarms
        .iter()
        .filter(|alarm| {
            alarm.acknowledged_at.is_none() && !is_shelved(alarm) && !is_resolved(alarm)
        })
        .collect();
    let count = waiting.len();
    let by_priority = [("critical", 0..=0), ("warning", 1..=2), ("info", 3..=3)]
        .into_iter()
        .filter_map(|(label, ranks)| {
            let n = waiting
                .iter()
                .filter(|alarm| ranks.contains(&severity_rank(alarms::priority(alarm))))
                .count();
            (n > 0).then(|| format!("{n} {label}"))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let (heading, text) = if count == 0 {
        (
            "Nothing to acknowledge",
            "Every open alarm is acknowledged or shelved.".to_owned(),
        )
    } else {
        (
            "Acknowledge all alarms?",
            format!(
                "{count} alarm{} wait{} for acknowledgement ({by_priority}). Each is recorded as acknowledged by you; the sound stops.",
                if count == 1 { "" } else { "s" },
                if count == 1 { "s" } else { "" },
            ),
        )
    };
    let hints: &[(&'static str, &'static str)] = if count == 0 {
        &[("Esc", "Close")]
    } else {
        &[("↵", "Acknowledge"), ("Esc", "Cancel")]
    };
    let mut buttons = vec![
        Button::new(
            "confirm-cancel",
            if count == 0 { "Close" } else { "Cancel" },
        )
        .ghost()
        .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx)))
        .into_any_element(),
    ];
    if count > 0 {
        buttons.push(
            Button::new("confirm-ack-all", format!("Acknowledge {count}"))
                .primary()
                .icon(Icon::Check)
                .on_click(cx.listener(|console, _, window, cx| {
                    acknowledge_all_now(console, window, cx);
                }))
                .into_any_element(),
        );
    }
    div()
        .track_focus(&focus)
        .key_context(LIST_CONTEXT)
        .on_action(cx.listener(|console, _: &ListActivate, window, cx| {
            acknowledge_all_now(console, window, cx);
        }))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_8)
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(32.0))
                        .rounded(RADIUS_LG)
                        .bg(if count == 0 {
                            theme.success.alpha(0.14)
                        } else {
                            theme.warning.alpha(0.14)
                        })
                        .child(vt_ui::icon(
                            if count == 0 {
                                Icon::CircleCheck
                            } else {
                                Icon::CircleAlert
                            },
                            ICON_LG,
                            if count == 0 {
                                theme.success
                            } else {
                                theme.warning
                            },
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_3)
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(FONT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(heading),
                        )
                        .child(
                            div()
                                .text_size(FONT_BODY)
                                .text_color(theme.fg_muted)
                                .child(text),
                        ),
                ),
        )
        .child(footer(hints, buttons, &theme, cx))
        .into_any_element()
}

// ── Detail ────────────────────────────────────────────────────────────────

/// The sheet for alarm `id`: everything about it, its actions, and (for
/// operators) its ISA-18.2 journal. j/k step through the list beneath.
pub(super) fn open_detail(
    console: &mut Console,
    id: i64,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let focus = {
        let state = console.state::<State>();
        if state.detail != Some(id) {
            state.journal = None;
        }
        state.detail = Some(id);
        state
            .sheet_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    };
    fetch_journal(console, id);
    if !console.overlay_is(DETAIL) {
        let overlay = Overlay::sheet(DETAIL, px(460.0), detail_sheet, cx);
        console.open_overlay(overlay, window, cx);
    }
    window.focus(&focus);
    cx.notify();
}

/// j/k in the sheet: the list's cursor moves and the sheet follows it.
fn step_detail(console: &mut Console, step: Move, window: &mut Window, cx: &mut Context<Console>) {
    if console.list_move(LIST, step, cx).is_some()
        && let Some(alarm) = list::cursor_alarm(console, cx)
    {
        open_detail(console, alarm.id, window, cx);
    }
}

/// A label and its value, one line of the sheet's facts.
fn fact(label: &'static str, value: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(SPACE_5)
        .min_h(px(22.0))
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(value.into()))
}

fn detail_sheet(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let (Some(id), Some(focus)) = (
        console.state::<State>().detail,
        console.state::<State>().sheet_focus.clone(),
    ) else {
        return div().into_any_element();
    };
    let scroll = console.scroll("alarms.detail");
    let operator = console.is_operator();
    let now = time_now_secs();
    let Some(alarm) = find(console, id).cloned() else {
        return div()
            .track_focus(&focus)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme.fg_muted)
            .child(format!("Alarm {id} is no longer listed"))
            .into_any_element();
    };
    let offset = console.utc_offset;
    let source = alarm_source(console, &alarm);
    let priority = alarms::priority(&alarm).to_owned();
    let (state_icon, state_text, state_color) = state_line(console, &alarm, &theme);
    let unit = console
        .store
        .by_key(&alarm.source_key)
        .map(|index| console.store.tags[index].info.unit.clone())
        .unwrap_or_default();
    let with_unit = |value: f64| {
        if unit.is_empty() {
            format_value(value)
        } else {
            format!("{} {unit}", format_value(value))
        }
    };
    let shelved = is_shelved(&alarm);
    let resolved = is_resolved(&alarm);

    let mut facts = vec![
        fact("Value", with_unit(alarm.value), &theme).font_family(MONO),
        fact(
            "Limit",
            format!(
                "{} ({})",
                with_unit(alarm.threshold),
                if alarm.alarm_type.eq_ignore_ascii_case("low") {
                    "alarms at or below"
                } else {
                    "alarms at or above"
                }
            ),
            &theme,
        ),
        fact(
            "Raised",
            format!(
                "{} · {} ago",
                local_time(alarm.triggered_at, offset),
                age(now - alarm.triggered_at)
            ),
            &theme,
        ),
    ];
    if alarm.deadband > 0.0 {
        facts.push(fact("Deadband", with_unit(alarm.deadband), &theme));
    }
    if let Some(at) = alarm.acknowledged_at {
        facts.push(fact(
            "Acknowledged",
            format!(
                "{} by {}",
                local_time(at, offset),
                alarm.acknowledged_by.as_deref().unwrap_or("someone")
            ),
            &theme,
        ));
    }
    if shelved {
        if let Some(until) = alarm.shelved_until {
            facts.push(fact(
                "Shelved until",
                format!(
                    "{} by {}",
                    local_time(until, offset),
                    alarm.shelved_by.as_deref().unwrap_or("someone")
                ),
                &theme,
            ));
        }
        if let Some(reason) = alarm.shelved_reason.as_ref().filter(|r| !r.is_empty()) {
            facts.push(fact("Reason", reason.clone(), &theme));
        }
    }
    if let Some(at) = alarm.resolved_at {
        facts.push(fact("Resolved", local_time(at, offset), &theme));
    }
    if let Some(until) = alarm.cooldown_until.filter(|until| *until > now) {
        facts.push(fact("Cooldown until", local_time(until, offset), &theme));
    }
    if !alarm.message.is_empty() {
        facts.push(fact("Message", alarm.message.clone(), &theme));
    }
    facts.push(fact("Alarm", format!("#{}", alarm.id), &theme).font_family(MONO));

    let mut actions: Vec<AnyElement> = Vec::new();
    if !resolved && !shelved && alarm.acknowledged_at.is_none() {
        let target = alarm.clone();
        actions.push(
            Button::new("detail-ack", "Acknowledge")
                .primary()
                .icon(Icon::Check)
                .tooltip("A")
                .on_click(cx.listener(move |console, _, _, _| acknowledge(console, &target)))
                .into_any_element(),
        );
    }
    if operator && !resolved {
        let target = alarm.clone();
        actions.push(
            Button::new(
                "detail-shelve",
                if shelved { "Unshelve" } else { "Shelve…" },
            )
            .icon(Icon::Clock)
            .tooltip("S")
            .on_click(cx.listener(move |console, _, window, cx| {
                shelve_or_unshelve(console, &target, window, cx);
            }))
            .into_any_element(),
        );
    }

    let journal = journal(console, id, operator, &theme, cx);
    let hints: &[(&'static str, &'static str)] = if operator {
        &[
            ("J K", "Next"),
            ("A", "Acknowledge"),
            ("S", "Shelve"),
            ("Esc", "Close"),
        ]
    } else {
        &[("J K", "Next"), ("A", "Acknowledge"), ("Esc", "Close")]
    };

    div()
        .track_focus(&focus)
        .key_context(format!("{LIST_CONTEXT} {ALARM_LIST}").as_str())
        .on_action(cx.listener(|console, _: &ListDown, window, cx| {
            step_detail(console, Move::Down, window, cx);
        }))
        .on_action(cx.listener(|console, _: &ListUp, window, cx| {
            step_detail(console, Move::Up, window, cx);
        }))
        .on_action(cx.listener(|console, _: &AlarmsAcknowledge, _, cx| {
            let detail = console.state::<State>().detail;
            if let Some(alarm) = detail.and_then(|id| find(console, id).cloned()) {
                acknowledge(console, &alarm);
            }
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &AlarmsShelve, window, cx| {
            let detail = console.state::<State>().detail;
            if let Some(alarm) = detail.and_then(|id| find(console, id).cloned()) {
                shelve_or_unshelve(console, &alarm, window, cx);
            }
        }))
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_none()
                .flex_col()
                .gap(SPACE_3)
                .px(SPACE_8)
                .pt(SPACE_7)
                .pb(SPACE_6)
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(pill(
                            priority.to_uppercase(),
                            priority_color(&priority, &theme),
                        ))
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(direction(&alarm)),
                        )
                        .child(div().flex_1())
                        .child(kbd("Esc", cx)),
                )
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(source),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(alarm.source_key.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .text_size(FONT_SMALL)
                        .text_color(state_color)
                        .child(vt_ui::icon(state_icon, ICON_SM, state_color))
                        .child(state_text),
                )
                .when(!actions.is_empty(), |this| {
                    this.child(div().flex().gap(SPACE_4).pt(SPACE_3).children(actions))
                }),
        )
        .child(
            div()
                .id("alarm-detail-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .flex()
                .flex_col()
                .gap(SPACE_8)
                .px(SPACE_8)
                .py(SPACE_7)
                .child(div().flex().flex_col().gap(SPACE_1).children(facts))
                .child(journal),
        )
        .child(footer(hints, Vec::new(), &theme, cx))
        .into_any_element()
}

/// The ISA-18.2 lifecycle: each event, when and by whom.
fn journal(
    console: &Console,
    id: i64,
    operator: bool,
    theme: &Theme,
    cx: &Context<Console>,
) -> Div {
    let section = div()
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .child(section_label("Lifecycle (ISA-18.2 journal)", cx));
    let note = |text: String| {
        div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_muted)
            .child(text)
    };
    if !operator {
        return section.child(note(
            "The journal is kept for operators (admin, demo, integration).".into(),
        ));
    }
    let journal = console
        .state_ref::<State>()
        .and_then(|state| state.journal.as_ref())
        .filter(|(alarm, _)| *alarm == id)
        .map(|(_, journal)| journal);
    match journal {
        None => section.child(note("Loading…".into())),
        Some(Err(error)) => section.child(note(format!("The server did not send it: {error}"))),
        Some(Ok(entries)) if entries.is_empty() => {
            section.child(note("No lifecycle events recorded yet.".into()))
        }
        Some(Ok(entries)) => {
            let last = entries.len() - 1;
            section.children(entries.iter().enumerate().map(|(index, entry)| {
                let color = match entry.event.as_str() {
                    "triggered" | "raised" => theme.danger,
                    "acknowledged" => theme.success,
                    "shelved" | "unshelved" => theme.info,
                    "resolved" | "cleared" => theme.success,
                    _ => theme.fg_muted,
                };
                div()
                    .flex()
                    .gap(SPACE_5)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .flex_col()
                            .items_center()
                            .w(px(10.0))
                            .child(div().mt(px(5.0)).size(px(8.0)).rounded_full().bg(color))
                            .when(index < last, |this| {
                                this.child(
                                    div().w(px(1.0)).flex_1().min_h(px(14.0)).bg(theme.border),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .pb(SPACE_4)
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(super::super::capitalized(&entry.event)),
                            )
                            .child(div().text_size(FONT_SMALL).text_color(theme.fg_dim).child(
                                match &entry.actor {
                                    Some(actor) if !actor.is_empty() => format!(
                                        "{} · {actor}",
                                        local_time(entry.at, console.utc_offset)
                                    ),
                                    _ => local_time(entry.at, console.utc_offset),
                                },
                            )),
                    )
            }))
        }
    }
}

//! The Alarms page (⌘3): the alarm list with its filters, search and
//! sort, a summary strip, and for operators the analytics and the audit
//! trail; dialogs to shelve with a duration and a reason, to confirm
//! acknowledging all, and a detail sheet with the ISA-18.2 journal. The
//! audible annunciation and its mute live in `sound` and
//! `console::audio`; the rules (filters, counts, …) in `console::alarms`.

mod dialogs;
mod insights;
mod list;
mod sound;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, FontWeight, Hsla, Stateful, Subscription, Window,
    div, px,
};
use voltrus_scada_native::api::{
    Alarm, AlarmAnalyticsEntry, AlarmAuditEntry, AlarmJournalEntry, AlarmSummary,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, Tooltip, segmented};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::page_header;
use crate::console::alarms::{
    ANALYTICS_SECS, Counts, Filter, HISTORY_LIMIT, Launch, Sort, View, duration,
};
use crate::console::keys::{
    AcknowledgeAll, AlarmsShowAnalytics, AlarmsShowAudit, AlarmsShowList, AlarmsToggleMute, hint,
};
use crate::console::nav::Move;
use crate::console::{Console, Page, Phase, severity_rank, time_now_secs};

pub(crate) use dialogs::confirm_acknowledge_all;
pub(crate) use sound::mute_button;

/// The alarm list's keyboard list.
pub(crate) const LIST: &str = "alarms";
/// The analytics ranking's and the audit trail's keyboard lists.
const ANALYTICS_LIST: &str = "alarms.analytics";
const AUDIT_LIST: &str = "alarms.audit";

/// A request still unanswered after this long is asked again.
const PENDING_FOR: Duration = Duration::from_secs(30);

/// The page's state (`console.state::<State>()`).
#[derive(Default)]
pub(crate) struct State {
    view: View,
    filter: Filter,
    sort: Sort,
    /// The list's search field, made on the first show.
    search: Option<Entity<TextInput>>,
    /// `GET /alarms/summary`, for operators.
    summary: Option<Result<AlarmSummary, String>>,
    /// `GET /alarms?status=all`, for the History filter.
    history: Option<Result<Vec<Alarm>, String>>,
    analytics: Option<Result<Vec<AlarmAnalyticsEntry>, String>>,
    audit: Option<Result<Vec<AlarmAuditEntry>, String>>,
    /// The alarm in the detail sheet, and its journal once fetched.
    detail: Option<i64>,
    journal: Option<(i64, Result<Vec<AlarmJournalEntry>, String>)>,
    /// The shelve dialog: the alarm, the chosen duration (an index into
    /// `SHELVE_DURATIONS`) and the reason field.
    shelving: Option<(i64, usize)>,
    reason: Option<Entity<TextInput>>,
    /// Focus inside the detail sheet and the confirmation, for their keys.
    sheet_focus: Option<FocusHandle>,
    confirm_focus: Option<FocusHandle>,
    /// What `--alarms` asked for, waiting for the alarm list.
    launch: Option<Launch>,
    /// Requests in flight, by key name, so a slow server is asked once.
    pending: HashMap<&'static str, Instant>,
    subscriptions: Vec<Subscription>,
}

/// The colour of a priority pill: the web console's critical/warning/info.
pub(super) fn priority_color(priority: &str, theme: &Theme) -> Hsla {
    match severity_rank(priority) {
        0 => theme.danger,
        1 | 2 => theme.warning,
        _ => theme.info,
    }
}

fn request(console: &mut Console, name: &'static str, call: Call) {
    let now = Instant::now();
    let pending = &mut console.state::<State>().pending;
    if pending
        .get(name)
        .is_some_and(|since| now.duration_since(*since) < PENDING_FOR)
    {
        return;
    }
    pending.insert(name, now);
    console.call(CallKey::new(name), call);
}

/// Live requests only: a stress run has no server.
fn online(console: &Console) -> bool {
    console.stress.is_none() && console.phase == Phase::Console
}

fn fetch_summary(console: &mut Console) {
    if online(console) && console.is_operator() {
        request(
            console,
            "alarms.summary",
            Call::get::<AlarmSummary>("/api/v1/alarms/summary"),
        );
    }
}

pub(super) fn fetch_history(console: &mut Console) {
    if online(console) {
        request(
            console,
            "alarms.history",
            Call::get::<Vec<Alarm>>(format!("/api/v1/alarms?status=all&limit={HISTORY_LIMIT}")),
        );
    }
}

fn fetch_analytics(console: &mut Console) {
    if online(console) && console.is_operator() {
        let now = time_now_secs();
        request(
            console,
            "alarms.analytics",
            Call::get::<Vec<AlarmAnalyticsEntry>>(format!(
                "/api/v1/alarms/analytics?from={}&to={now}",
                now - ANALYTICS_SECS
            )),
        );
    }
}

fn fetch_audit(console: &mut Console) {
    if online(console) && console.is_operator() {
        request(
            console,
            "alarms.audit",
            Call::get::<Vec<AlarmAuditEntry>>("/api/v1/alarms/audit?limit=200"),
        );
    }
}

pub(super) fn fetch_journal(console: &mut Console, alarm: i64) {
    if online(console) && console.is_operator() {
        console.call(
            CallKey::new("alarms.journal").with(alarm),
            Call::get::<Vec<AlarmJournalEntry>>(format!("/api/v1/alarms/{alarm}/journal")),
        );
    }
}

/// Fetch what the shown view needs, again.
fn refresh(console: &mut Console) {
    fetch_summary(console);
    let (view, filter) = {
        let state = console.state::<State>();
        (state.view, state.filter)
    };
    match view {
        View::List if filter == Filter::History => fetch_history(console),
        View::List => {}
        // The history's last five minutes make the flood warning.
        View::Analytics => {
            fetch_analytics(console);
            fetch_history(console);
        }
        View::Audit => fetch_audit(console),
    }
}

/// The search field, made once with its keys: typing filters, Enter or Esc
/// goes back to the list with the cursor on the first match.
fn ensure_search(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().search.is_some() {
        return;
    }
    let field = cx.new(|cx| TextInput::new(cx).placeholder("Filter alarms  /"));
    let subscription = cx.subscribe_in(
        &field,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                console.list(LIST, cx).cursor = None;
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                let nav = console.list(LIST, cx);
                nav.step(Move::First);
                window.focus(&nav.focus);
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.search = Some(field);
    state.subscriptions.push(subscription);
}

/// The keyboard list of the shown view.
fn view_list(view: View) -> &'static str {
    match view {
        View::List => LIST,
        View::Analytics => ANALYTICS_LIST,
        View::Audit => AUDIT_LIST,
    }
}

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    ensure_search(console, window, cx);
    // A role without the view's route sees the list.
    let view = console.state::<State>().view;
    if !console.allows(view.role()) {
        console.state::<State>().view = View::List;
    }
    let (view, filter, summary, history, analytics, audit) = {
        let state = console.state::<State>();
        (
            state.view,
            state.filter,
            state.summary.is_some(),
            state.history.is_some(),
            state.analytics.is_some(),
            state.audit.is_some(),
        )
    };
    // Fetch once; the alarm refresh keeps it current while shown.
    if !summary {
        fetch_summary(console);
    }
    match view {
        View::List if filter == Filter::History && !history => fetch_history(console),
        View::Analytics => {
            if !analytics {
                fetch_analytics(console);
            }
            if !history {
                fetch_history(console);
            }
        }
        View::Audit if !audit => fetch_audit(console),
        _ => {}
    }
    console.focus_list(view_list(view), window, cx);
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console
        .state_ref::<State>()
        .filter(|state| state.view == View::List)
        .and_then(|state| state.search.clone())
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.pending.remove(key.name);
    match key.name {
        "alarms.summary" => state.summary = Some(reply.take()),
        "alarms.history" => state.history = Some(reply.take()),
        "alarms.analytics" => state.analytics = Some(reply.take()),
        "alarms.audit" => state.audit = Some(reply.take()),
        "alarms.journal" if state.detail == Some(key.arg) => {
            state.journal = Some((key.arg, reply.take()));
        }
        _ => {}
    }
    cx.notify();
}

/// The alarm list changed (every refresh, about every 10 s, and after any
/// alarm action): sound follows it, and the shown view's figures refresh.
pub(crate) fn alarms_changed(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    sound::sync(console);
    if console.page == Page::Alarms {
        refresh(console);
    }
    let detail = console.state::<State>().detail;
    if let Some(alarm) = detail
        && console.overlay_is(dialogs::DETAIL)
    {
        fetch_journal(console, alarm);
    }
    let waiting = console.state::<State>().launch;
    if let Some(launch) = waiting
        && !console.alarms.is_empty()
    {
        console.state::<State>().launch = None;
        open_launch(console, launch, window, cx);
    }
}

/// Signed out: silence, and forget what this session fetched.
pub(crate) fn signed_out(console: &mut Console) {
    sound::sync(console);
    *console.state::<State>() = State::default();
}

/// `--alarms <view>`: show the page with that view or dialog open.
pub(crate) fn launch(
    console: &mut Console,
    launch: Launch,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    console.show(Page::Alarms, window, cx);
    if launch.needs_alarm() && console.alarms.is_empty() {
        console.state::<State>().launch = Some(launch);
    } else {
        open_launch(console, launch, window, cx);
    }
}

fn open_launch(
    console: &mut Console,
    launch: Launch,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match launch {
        Launch::History => {
            show_view(console, View::List, window, cx);
            list::set_filter(console, Filter::History, cx);
        }
        Launch::Analytics => show_view(console, View::Analytics, window, cx),
        Launch::Audit => show_view(console, View::Audit, window, cx),
        Launch::Confirm => confirm_acknowledge_all(console, window, cx),
        Launch::Detail | Launch::Shelve => {
            let rows = list::rows(console, cx);
            let Some(alarm) = rows
                .first()
                .and_then(|index| list::source(console).get(*index))
                .map(|alarm| alarm.id)
            else {
                return;
            };
            console.list(LIST, cx).cursor = Some(0);
            if launch == Launch::Detail {
                dialogs::open_detail(console, alarm, window, cx);
            } else {
                dialogs::open_shelve(console, alarm, window, cx);
            }
        }
    }
}

/// Show the Alarms page with `view` (the palette's Alarm list, analytics
/// and audit trail).
pub(crate) fn show_view(
    console: &mut Console,
    view: View,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !console.allows(view.role()) {
        console.show_toast(format!("{} needs an operator role", view.title()), true, cx);
        return;
    }
    console.state::<State>().view = view;
    if console.page == Page::Alarms {
        console.focus_page(window, cx);
    } else {
        console.show(Page::Alarms, window, cx);
    }
    // Fresh figures each time a view is chosen.
    refresh(console);
    cx.notify();
}

/// The page's commands that work from anywhere: the sound's mute and the
/// palette's views. Hooked on the console's root.
pub(crate) fn root_actions(
    root: Stateful<gpui::Div>,
    cx: &mut Context<Console>,
) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|console, _: &AlarmsToggleMute, _, cx| {
        sound::toggle_mute(console, cx);
    }))
    .on_action(cx.listener(|console, _: &AlarmsShowList, window, cx| {
        if console.phase == Phase::Console {
            show_view(console, View::List, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &AlarmsShowAnalytics, window, cx| {
        if console.phase == Phase::Console {
            show_view(console, View::Analytics, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &AlarmsShowAudit, window, cx| {
        if console.phase == Phase::Console {
            show_view(console, View::Audit, window, cx);
        }
    }))
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let unacknowledged = console.unacknowledged();
    let view = console.state::<State>().view;
    let views = View::allowed(console.role());

    let mut controls: Vec<AnyElement> = Vec::new();
    if views.len() > 1 {
        let segments = views
            .into_iter()
            .map(|choice| {
                Segment::new(
                    match choice {
                        View::List => "List",
                        View::Analytics => "Analytics",
                        View::Audit => "Audit",
                    },
                    view == choice,
                    cx.listener(move |console, _, window, cx| {
                        show_view(console, choice, window, cx);
                    }),
                )
                .icon(match choice {
                    View::List => Icon::List,
                    View::Analytics => Icon::ChartLine,
                    View::Audit => Icon::ScrollText,
                })
            })
            .collect();
        controls.push(segmented("alarm-view", segments, cx));
    }
    controls.push(
        Button::new("ack-all", "Acknowledge all")
            .icon(Icon::Check)
            .disabled(unacknowledged == 0)
            .tooltip(hint(&AcknowledgeAll).unwrap_or_default())
            .on_click(cx.listener(|console, _, window, cx| {
                confirm_acknowledge_all(console, window, cx);
            }))
            .into_any_element(),
    );

    let subtitle = match view {
        View::List => format!(
            "{unacknowledged} to acknowledge · {} open · ↵ details, A acknowledge, S shelve, ← → filter, O sort",
            console
                .alarms
                .iter()
                .filter(|alarm| Filter::Open.admits(alarm))
                .count()
        ),
        View::Analytics => "The sources that alarmed most in the last 24 hours".into(),
        View::Audit => "Every acknowledge, shelve and unshelve, newest first".into(),
    };

    let body = match view {
        View::List => list::render(console, window, cx),
        View::Analytics => insights::analytics(console, window, cx),
        View::Audit => insights::audit(console, window, cx),
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("Alarms", subtitle, controls, cx))
        .child(summary_strip(console, &theme))
        .child(body)
        .into_any_element()
}

/// Open alarms by priority, the shelf and the last day: the server's
/// summary for operators, counted from the open alarms otherwise.
fn summary_strip(console: &Console, theme: &Theme) -> Stateful<gpui::Div> {
    let summary = console
        .state_ref::<State>()
        .and_then(|state| state.summary.as_ref());
    let counts = match summary {
        Some(Ok(summary)) if console.is_operator() => Counts::from_server(summary),
        _ => Counts::local(&console.alarms),
    };
    let note = match summary {
        Some(Err(error)) if console.is_operator() => {
            format!("Summary unavailable ({error}); counted from the open alarms")
        }
        _ if counts.from_server => "Server summary".to_owned(),
        _ => "Counted from the open alarms".to_owned(),
    };
    let chip = |value: i64, label: &'static str, color: Hsla| {
        let color = if value == 0 { theme.fg_dim } else { color };
        div()
            .flex()
            .flex_none()
            .items_baseline()
            .gap(SPACE_2)
            .px(SPACE_4)
            .py(px(2.0))
            .rounded(RADIUS_SM)
            .bg(color.alpha(if value == 0 { 0.06 } else { 0.12 }))
            .child(
                div()
                    .font_family(MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(FONT_SMALL)
                    .text_color(color)
                    .child(value.to_string()),
            )
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(color.alpha(0.85))
                    .child(label),
            )
    };
    let muted = sound::muted(console);
    let sounding = sound::sounding(console);
    div()
        .id("alarm-summary")
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(36.0))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border)
        .child(chip(counts.critical, "critical", theme.danger))
        .child(chip(counts.warning, "warning", theme.warning))
        .child(chip(counts.info, "info", theme.info))
        .child(chip(counts.shelved, "shelved", theme.fg_muted))
        .when_some(counts.recent_24h, |this, recent| {
            this.child(chip(recent, "in the last 24 h", theme.fg_muted))
        })
        .when_some(counts.avg_ack_secs, |this, secs| {
            this.child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_muted)
                    .child(format!("acknowledged in {} on average", duration(secs))),
            )
        })
        .child(div().flex_1())
        .when(sounding.is_some() || muted, |this| {
            let (text, color) = match sounding {
                Some(tone) => (format!("Sounding: {} tone", tone.label()), theme.warning),
                None => ("Alarm sound muted".to_owned(), theme.fg_dim),
            };
            this.child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(color)
                    .child(format!(
                        "{text}  {}",
                        hint(&AlarmsToggleMute).unwrap_or_default()
                    )),
            )
        })
        .child(
            div()
                .id("alarm-summary-note")
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(note.clone())
                .tooltip(move |_, cx| Tooltip::view(note.clone().into(), cx)),
        )
}

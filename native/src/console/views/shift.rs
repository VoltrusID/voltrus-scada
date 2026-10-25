//! Shift handover: the log of what each shift handed to the next, filtered
//! by date and shift, with new entries written in a dialog (multi-line
//! notes; @mentions notify the named users), and the server's automatic
//! report of the current shift, fetched again every 30 s while it is on
//! screen. Every route is an operator route, so the page is too.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, AppContext as _, Context, ElementId, Entity, FontWeight,
    HighlightStyle, SharedString, StyledText, Subscription, Window, div, px,
};
use time::{Duration as Days, OffsetDateTime};
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::ops::{
    self, Created, Handover, HandoverList, NewHandover, SHIFT_PERIODS, ShiftReport, mention_spans,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, empty_state, kbd, pill, segmented};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::ops_kit::{
    close_overlay_in, filter_bar, labelled, notice, on_field, poll_while_shown, text_field, value,
};
use super::{
    LIST_TAIL, ListKeys, capitalized, focus_ring, keyboard_list, local_time, page_header,
    severity_color,
};
use crate::console::keys::{OpsNextFilter, OpsNextTab, OpsPreviousFilter, OpsRefresh};
use crate::console::overlay::Overlay;
use crate::console::text_area::TextArea;
use crate::console::{Console, Page};

/// The handover log's keyboard list.
pub(crate) const LIST: &str = "shift";
const SCROLL: &str = "shift.log";
/// The report's sections, as a keyboard list.
const REPORT_LIST: &str = "shift.report";
const REPORT_SCROLL: &str = "shift.report";
const KEY_CONTEXT: &str = "OpsShift OpsTabbed OpsFiltered OpsRefreshable";
const REPORT_EVERY: Duration = Duration::from_secs(30);
/// The new-entry dialog's overlay id.
const DIALOG: &str = "shift-new";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Log,
    Report,
}

/// The page's own fields, made on its first show.
struct Fields {
    date: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

/// The new entry being written; kept as a draft when the dialog closes.
struct Form {
    date: Entity<TextInput>,
    /// Index into `SHIFT_PERIODS`.
    period: usize,
    happened: Entity<TextArea>,
    pending: Entity<TextArea>,
    watch: Entity<TextArea>,
    saving: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) tab: Tab,
    /// 0 for every shift, else `SHIFT_PERIODS[period - 1]`.
    pub(crate) period: usize,
    fields: Option<Fields>,
    log: Rc<Vec<Handover>>,
    log_loaded: bool,
    log_error: Option<String>,
    log_seq: i64,
    report: Option<Result<Rc<ShiftReport>, String>>,
    report_seq: i64,
    /// When the report last came, Unix seconds.
    report_at: Option<i64>,
    form: Option<Form>,
    /// The window the dialog opened in, to close it once the entry saved.
    window: Option<AnyWindowHandle>,
}

impl State {
    fn period_filter(&self) -> Option<&'static str> {
        self.period
            .checked_sub(1)
            .and_then(|index| SHIFT_PERIODS.get(index).copied())
    }
}

/// Today's date and the shift of this hour, on the operator's clock.
fn now_shift(console: &Console) -> (String, usize) {
    let now = OffsetDateTime::now_utc().to_offset(console.utc_offset);
    let period = ops::shift_of_hour(now.hour());
    (
        ops::format_date(now.date()),
        SHIFT_PERIODS
            .iter()
            .position(|known| *known == period)
            .unwrap_or(0),
    )
}

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().fields.is_none() {
        let date = text_field("Any date  YYYY-MM-DD", true, cx);
        let subscriptions = vec![on_field(&date, date_event, window, cx)];
        console.state::<State>().fields = Some(Fields {
            date,
            _subscriptions: subscriptions,
        });
    }
    let state = console.state::<State>();
    let (log, report) = (!state.log_loaded, state.report.is_none());
    if log {
        fetch_log(console, cx);
    }
    if report {
        fetch_report(console);
    }
    poll_while_shown(console, Page::Shift, REPORT_EVERY, tick, cx);
    focus_tab(console, window, cx);
}

/// `/`: the date filter.
pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console
        .state_ref::<State>()?
        .fields
        .as_ref()
        .map(|fields| fields.date.clone())
}

/// Focus the keyboard list of the tab on screen: the log's cards or the
/// report's sections.
fn focus_tab(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let list = match console.state::<State>().tab {
        Tab::Log => LIST,
        Tab::Report => REPORT_LIST,
    };
    console.focus_list(list, window, cx);
}

/// Every 30 s on screen: the report again, when it is the tab shown.
fn tick(console: &mut Console, _: &mut Context<Console>) {
    if console.state::<State>().tab == Tab::Report {
        fetch_report(console);
    }
}

fn date_event(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        // Fetch once the date reads as one (or the field is cleared).
        InputEvent::Changed => {
            let text = date_filter_text(console, cx);
            if text.is_empty() || ops::parse_date(&text).is_some() {
                fetch_log(console, cx);
            }
            cx.notify();
        }
        InputEvent::Submitted | InputEvent::Cancelled => focus_tab(console, window, cx),
        InputEvent::Blurred => {}
    }
}

fn date_filter_text(console: &mut Console, cx: &gpui::App) -> String {
    console
        .state::<State>()
        .fields
        .as_ref()
        .map(|fields| value(&fields.date, cx))
        .unwrap_or_default()
}

fn fetch_log(console: &mut Console, cx: &gpui::App) {
    let date = date_filter_text(console, cx);
    let date = ops::parse_date(&date).map(ops::format_date);
    let state = console.state::<State>();
    state.log_seq += 1;
    let (seq, period) = (state.log_seq, state.period_filter());
    console.call(
        CallKey::new("shift.log").with(seq),
        Call::get::<HandoverList>(ops::handovers_path(date.as_deref(), period)),
    );
}

fn fetch_report(console: &mut Console) {
    let state = console.state::<State>();
    state.report_seq += 1;
    let seq = state.report_seq;
    console.call(
        CallKey::new("shift.report").with(seq),
        Call::get::<ShiftReport>(ops::SHIFT_REPORT_PATH),
    );
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    match key.name {
        "shift.log" if key.arg == state.log_seq => match reply.take::<HandoverList>() {
            Ok(list) => {
                if *state.log != list.handovers || !state.log_loaded {
                    state.log = Rc::new(list.handovers);
                }
                state.log_loaded = true;
                state.log_error = None;
            }
            Err(error) => state.log_error = Some(error),
        },
        "shift.report" if key.arg == state.report_seq => {
            state.report = Some(reply.take::<ShiftReport>().map(Rc::new));
            state.report_at = Some(crate::console::time_now_secs());
        }
        "shift.create" => match reply.take::<Created>() {
            Ok(_) => {
                state.form = None;
                if let Some(window) = state.window {
                    close_overlay_in(window, DIALOG, cx);
                }
                console.show_toast("Handover entry saved", false, cx);
                fetch_log(console, cx);
                fetch_report(console);
            }
            Err(error) => {
                if let Some(form) = &mut state.form {
                    form.saving = false;
                    form.error = Some(error);
                }
            }
        },
        _ => return,
    }
    cx.notify();
}

fn set_tab(console: &mut Console, tab: Tab, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.tab != tab {
        state.tab = tab;
        if tab == Tab::Report {
            fetch_report(console);
        }
    }
    focus_tab(console, window, cx);
    cx.notify();
}

fn set_period(
    console: &mut Console,
    period: usize,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    state.period = period % (SHIFT_PERIODS.len() + 1);
    state.tab = Tab::Log;
    fetch_log(console, cx);
    console.list(LIST, cx).cursor = Some(0);
    focus_tab(console, window, cx);
    cx.notify();
}

fn step_period(
    console: &mut Console,
    forward: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let count = SHIFT_PERIODS.len() + 1;
    let period = console.state::<State>().period;
    set_period(
        console,
        if forward {
            period + 1
        } else {
            period + count - 1
        },
        window,
        cx,
    );
}

/// Move the date filter by `days`, from today when it holds none.
fn step_date(console: &mut Console, days: i64, cx: &mut Context<Console>) {
    let text = date_filter_text(console, cx);
    let date = match ops::parse_date(&text) {
        Some(date) => date.checked_add(Days::days(days)),
        None => ops::parse_date(&now_shift(console).0),
    };
    set_date(console, date.map(ops::format_date).unwrap_or_default(), cx);
}

fn set_date(console: &mut Console, date: String, cx: &mut Context<Console>) {
    if let Some(fields) = &console.state::<State>().fields {
        fields.date.update(cx, |field, cx| field.set_text(date, cx));
    }
    // `set_text` emits no change event.
    fetch_log(console, cx);
    cx.notify();
}

fn refresh(console: &mut Console, cx: &mut Context<Console>) {
    fetch_log(console, cx);
    fetch_report(console);
    cx.notify();
}

/// The cursor moved to card `row`: scroll it into view.
fn reveal_card(console: &mut Console, row: usize, _: &mut Window, _: &mut Context<Console>) {
    console.scroll(SCROLL).scroll_to_item(row);
}

/// The cursor moved to report section `row`: scroll it into view (the
/// report's first child is its header).
fn reveal_section(console: &mut Console, row: usize, _: &mut Window, _: &mut Context<Console>) {
    console.scroll(REPORT_SCROLL).scroll_to_item(row + 1);
}

/// Enter or Space on a card: nothing to open; the card shows it all.
fn activate_card(_: &mut Console, _: usize, _: bool, _: &mut Window, _: &mut Context<Console>) {}

// ── The new-entry dialog ─────────────────────────────────────────────────

/// Open the dialog for a new entry (`n`, the palette, the New button):
/// on the Shift page, with the draft kept from last time, if any.
pub fn new_entry(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast("Shift handover needs an operator role", true, cx);
        return;
    }
    if console.page != Page::Shift {
        console.show(Page::Shift, window, cx);
    }
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    if state.form.is_none() {
        let form = new_form(console, window, cx);
        console.state::<State>().form = Some(form);
    }
    let overlay = Overlay::modal(DIALOG, px(580.0), dialog, cx);
    console.open_overlay(overlay, window, cx);
    if let Some(form) = &console.state::<State>().form {
        form.happened.update(cx, |area, _| area.focus(window));
    }
}

fn new_form(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> Form {
    let (today, period) = now_shift(console);
    let date = text_field("YYYY-MM-DD", true, cx);
    date.update(cx, |field, cx| field.set_text(today, cx));
    let area = |placeholder: &'static str, rows: usize, cx: &mut Context<Console>| {
        cx.new(|cx| TextArea::new(rows, cx).placeholder(placeholder))
    };
    let happened = area(
        "What happened during the shift. Mention someone with @name.",
        4,
        cx,
    );
    let pending = area("Tasks or issues still open", 2, cx);
    let watch = area("What the next shift should keep an eye on", 2, cx);
    let mut subscriptions = vec![on_field(&date, form_event, window, cx)];
    for notes in [&happened, &pending, &watch] {
        subscriptions.push(cx.subscribe_in(
            notes,
            window,
            |console, _, event: &InputEvent, window, cx| form_event(console, event, window, cx),
        ));
    }
    Form {
        date,
        period,
        happened,
        pending,
        watch,
        saving: false,
        error: None,
        _subscriptions: subscriptions,
    }
}

/// Changes redraw (the mentions line); ⌘↵ (Enter in the date) saves; Esc
/// closes the dialog and keeps the draft.
fn form_event(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Changed => {
            if let Some(form) = &mut console.state::<State>().form {
                form.error = None;
            }
            cx.notify();
        }
        InputEvent::Submitted => save(console, cx),
        InputEvent::Cancelled => console.close_overlay(window, cx),
        InputEvent::Blurred => {}
    }
}

/// The entry the form holds.
fn entry(form: &Form, cx: &gpui::App) -> NewHandover {
    NewHandover {
        shift_date: value(&form.date, cx),
        shift_period: SHIFT_PERIODS[form.period.min(SHIFT_PERIODS.len() - 1)].to_owned(),
        what_happened: form.happened.read(cx).value().trim().to_owned(),
        whats_pending: form.pending.read(cx).value().trim().to_owned(),
        what_to_watch: form.watch.read(cx).value().trim().to_owned(),
    }
}

fn save(console: &mut Console, cx: &mut Context<Console>) {
    let Some(form) = &mut console.state::<State>().form else {
        return;
    };
    if form.saving {
        return;
    }
    let entry = entry(form, cx);
    if let Some(problem) = entry.problem() {
        form.error = Some(problem.to_owned());
        cx.notify();
        return;
    }
    form.saving = true;
    form.error = None;
    let body = serde_json::to_value(&entry).unwrap_or_default();
    console.call(
        CallKey::new("shift.create"),
        Call::send::<Created>(Method::POST, ops::HANDOVER_PATH, body),
    );
    cx.notify();
}

/// Who an entry's notes mention, each once: the users the server notifies.
fn mentioned(texts: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for text in texts {
        for span in mention_spans(text) {
            let name = text[span].to_owned();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// The dialog's content; the overlay draws its frame.
fn dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(form) = &console.state::<State>().form else {
        return div().into_any_element();
    };
    let (date, period, saving, error) = (
        form.date.clone(),
        form.period,
        form.saving,
        form.error.clone(),
    );
    let (happened, pending, watch) = (
        form.happened.clone(),
        form.pending.clone(),
        form.watch.clone(),
    );
    let names = mentioned(&[
        happened.read(cx).value(),
        pending.read(cx).value(),
        watch.read(cx).value(),
    ]);
    let periods = SHIFT_PERIODS
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Segment::new(
                capitalized(name),
                period == index,
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = &mut console.state::<State>().form {
                        form.period = index;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();
    let hours = ops::shift_hours(SHIFT_PERIODS[period])
        .map(|(start, end)| format!("{start:02}:00–{end:02}:00"))
        .unwrap_or_default();

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(48.0))
                .px(SPACE_8)
                .border_b_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::Pencil, ICON_MD, theme.accent))
                .child(
                    div()
                        .flex_1()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("New handover entry"),
                )
                .child(
                    IconButton::new("shift-dialog-close", Icon::X)
                        .tooltip("Close (keeps the draft)  Esc")
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.close_overlay(window, cx);
                        })),
                ),
        )
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
                        .items_end()
                        .gap(SPACE_6)
                        .child(labelled(
                            "Shift date",
                            div().w(px(140.0)).child(date),
                            &theme,
                        ))
                        .child(labelled(
                            "Shift",
                            segmented("shift-period", periods, cx),
                            &theme,
                        ))
                        .child(
                            div()
                                .pb(SPACE_2)
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(hours),
                        ),
                )
                .child(labelled("What happened", happened, &theme))
                .child(labelled("What's pending", pending, &theme))
                .child(labelled("What to watch", watch, &theme))
                .child(div().min_h(px(18.0)).text_size(FONT_SMALL).map(|this| {
                    match &error {
                        Some(error) => this.text_color(theme.danger).child(error.clone()),
                        None if names.is_empty() => this
                            .text_color(theme.fg_dim)
                            .child("@name in a note notifies that user."),
                        None => this
                            .text_color(theme.fg_muted)
                            .child(format!("Notifies {}", names.join(", "))),
                    }
                })),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .px(SPACE_8)
                .py(SPACE_6)
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div().flex_1().flex().gap(SPACE_3).items_center().children([
                        kbd("Tab", cx).into_any_element(),
                        div()
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child("next field · ↵ new line")
                            .into_any_element(),
                    ]),
                )
                .child(
                    Button::new("shift-dialog-cancel", "Cancel")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new(
                        "shift-dialog-save",
                        if saving { "Saving…" } else { "Save  ⌘↵" },
                    )
                    .primary()
                    .icon(Icon::Check)
                    .disabled(saving)
                    .on_click(cx.listener(|console, _, _, cx| save(console, cx))),
                ),
        )
        .into_any_element()
}

// ── The page ─────────────────────────────────────────────────────────────

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let (tab, period) = (state.tab, state.period);
    let date_field = state.fields.as_ref().map(|fields| fields.date.clone());
    let log = state.log.clone();
    let (log_loaded, log_error) = (state.log_loaded, state.log_error.clone());
    let report = state.report.clone();
    let report_at = state.report_at;
    let filtered = period != 0
        || date_field
            .as_ref()
            .is_some_and(|field| !field.read(cx).value().trim().is_empty());

    let tabs = segmented(
        "shift-tab",
        vec![
            Segment::new(
                format!("Handover log  {}", log.len()),
                tab == Tab::Log,
                cx.listener(|console, _, window, cx| set_tab(console, Tab::Log, window, cx)),
            ),
            Segment::new(
                "Auto report",
                tab == Tab::Report,
                cx.listener(|console, _, window, cx| set_tab(console, Tab::Report, window, cx)),
            ),
        ],
        cx,
    );
    let controls = vec![
        tabs,
        IconButton::new("shift-refresh", Icon::RefreshCw)
            .tooltip("Refresh  R")
            .on_click(cx.listener(|console, _, _, cx| refresh(console, cx)))
            .into_any_element(),
        Button::new("shift-new", "New entry")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| new_entry(console, window, cx)))
            .into_any_element(),
    ];
    let subtitle = match tab {
        Tab::Log => {
            "What each shift handed to the next · N writes one, [ ] filter the shift, T switches tab"
        }
        Tab::Report => "The current shift as the server sees it · refreshed every 30 s while shown",
    };

    let body = match tab {
        Tab::Log => {
            let periods = std::iter::once("All")
                .chain(SHIFT_PERIODS)
                .enumerate()
                .map(|(index, name)| {
                    Segment::new(
                        capitalized(name),
                        period == index,
                        cx.listener(move |console, _, window, cx| {
                            set_period(console, index, window, cx);
                        }),
                    )
                })
                .collect();
            let date_controls = div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(
                    IconButton::new("shift-day-back", Icon::ChevronLeft)
                        .tooltip("The day before")
                        .on_click(cx.listener(|console, _, _, cx| step_date(console, -1, cx))),
                )
                .child(div().w(px(150.0)).children(date_field))
                .child(
                    IconButton::new("shift-day-next", Icon::ChevronRight)
                        .tooltip("The day after")
                        .on_click(cx.listener(|console, _, _, cx| step_date(console, 1, cx))),
                )
                .child(
                    Button::new("shift-today", "Today")
                        .compact()
                        .ghost()
                        .on_click(cx.listener(|console, _, _, cx| {
                            let today = now_shift(console).0;
                            set_date(console, today, cx);
                        })),
                )
                .child(
                    Button::new("shift-any-day", "Any day")
                        .compact()
                        .ghost()
                        .on_click(cx.listener(|console, _, _, cx| {
                            set_date(console, String::new(), cx);
                        })),
                );
            let bar = filter_bar(&theme)
                .child(labelled("Date", date_controls, &theme))
                .child(labelled(
                    "Shift",
                    segmented("shift-filter", periods, cx),
                    &theme,
                ));
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(bar)
                .child(log_list(
                    console,
                    &log,
                    (log_loaded, log_error, filtered),
                    window,
                    cx,
                ))
                .into_any_element()
        }
        Tab::Report => report_view(console, report, report_at, window, cx),
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .key_context(KEY_CONTEXT)
        .on_action(cx.listener(|console, _: &OpsNextTab, window, cx| {
            let next = match console.state::<State>().tab {
                Tab::Log => Tab::Report,
                Tab::Report => Tab::Log,
            };
            set_tab(console, next, window, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsNextFilter, window, cx| {
            step_period(console, true, window, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsPreviousFilter, window, cx| {
            step_period(console, false, window, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsRefresh, _, cx| refresh(console, cx)))
        .child(page_header("Shift handover", subtitle, controls, cx))
        .child(body)
        .into_any_element()
}

fn log_list(
    console: &mut Console,
    log: &Rc<Vec<Handover>>,
    (loaded, error, filtered): (bool, Option<String>, bool),
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: Some(reveal_card),
            on_activate: activate_card,
        },
        log.len(),
        3,
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    let body = if log.is_empty() {
        if let Some(error) = error {
            notice(format!("Could not load the log: {error}"), true, &theme)
        } else if !loaded {
            notice("Loading the handover log…", false, &theme)
        } else if filtered {
            empty_state(
                Icon::Funnel,
                "No entries for this filter",
                "Try another date or shift, or clear the date with Any day.",
                Vec::new(),
                cx,
            )
            .into_any_element()
        } else {
            empty_state(
                Icon::Inbox,
                "No handover entries yet",
                "Press N to write what this shift hands to the next.",
                Vec::new(),
                cx,
            )
            .into_any_element()
        }
    } else {
        let offset = console.utc_offset;
        div()
            .id("shift-log")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&console.scroll(SCROLL))
            .px(SPACE_9)
            .pt(SPACE_7)
            .pb(LIST_TAIL)
            .children(log.iter().enumerate().map(|(row, entry)| {
                handover_card(entry, row, focused && cursor == Some(row), offset, &theme)
            }))
            .into_any_element()
    };
    list.flex_1().min_h_0().child(body).into_any_element()
}

/// Text with its @mentions in the accent colour.
fn mention_text(text: &str, theme: &Theme) -> StyledText {
    let accent = theme.accent;
    StyledText::new(text.to_owned()).with_highlights(mention_spans(text).into_iter().map(
        move |span: Range<usize>| {
            (
                span,
                HighlightStyle {
                    color: Some(accent),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..HighlightStyle::default()
                },
            )
        },
    ))
}

fn note(label: &'static str, color: gpui::Hsla, text: &str, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_2)
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(label),
        )
        .child(
            div()
                .text_size(FONT_BODY)
                .line_height(px(19.0))
                .child(mention_text(text, theme)),
        )
}

fn handover_card(
    entry: &Handover,
    row: usize,
    ring: bool,
    offset: time::UtcOffset,
    theme: &Theme,
) -> AnyElement {
    div()
        .id(ElementId::NamedInteger(
            "handover".into(),
            u64::try_from(row).unwrap_or(0),
        ))
        .relative()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .mb(SPACE_6)
        .p(SPACE_7)
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .when(ring, |this| this.child(focus_ring(theme)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(pill(entry.shift_date.clone(), theme.success))
                .child(pill(capitalized(&entry.shift_period), theme.info))
                .child(
                    div()
                        .flex_1()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(format!("by user #{}", entry.author_id)),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(ops::format_local(entry.created_at, offset)),
                ),
        )
        .child(note(
            "WHAT HAPPENED",
            theme.fg_dim,
            &entry.what_happened,
            theme,
        ))
        .when(!entry.whats_pending.is_empty(), |this| {
            this.child(note("PENDING", theme.warning, &entry.whats_pending, theme))
        })
        .when(!entry.what_to_watch.is_empty(), |this| {
            this.child(note("WATCH OUT", theme.danger, &entry.what_to_watch, theme))
        })
        .into_any_element()
}

/// One section of the report: a title with its count, then its lines.
fn section(
    title: &'static str,
    color: gpui::Hsla,
    lines: Vec<AnyElement>,
    ring: bool,
    theme: &Theme,
) -> AnyElement {
    let count = lines.len();
    div()
        .relative()
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .mb(SPACE_6)
        .p(SPACE_7)
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .when(ring, |this| this.child(focus_ring(theme)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(pill(
                    count.to_string(),
                    if count == 0 { theme.fg_dim } else { color },
                )),
        )
        .map(|this| {
            if lines.is_empty() {
                this.child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child("None this shift."),
                )
            } else {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_3)
                        .pl(SPACE_5)
                        .border_l_2()
                        .border_color(color.alpha(0.35))
                        .children(lines),
                )
            }
        })
        .into_any_element()
}

/// A line of the report: a leading tag, the text, the time.
fn report_line(
    lead: Option<AnyElement>,
    text: AnyElement,
    at: String,
    theme: &Theme,
) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(SPACE_4)
        .children(lead)
        .child(div().flex_1().min_w_0().text_size(FONT_BODY).child(text))
        .child(
            div()
                .flex_none()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(at),
        )
        .into_any_element()
}

fn report_view(
    console: &mut Console,
    report: Option<Result<Rc<ShiftReport>, String>>,
    report_at: Option<i64>,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let offset = console.utc_offset;
    let list = keyboard_list(
        ListKeys {
            id: REPORT_LIST,
            on_move: Some(reveal_section),
            on_activate: activate_card,
        },
        if matches!(report, Some(Ok(_))) { 4 } else { 0 },
        1,
        console,
        cx,
    );
    let nav = console.list(REPORT_LIST, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    let ring = |row: usize| focused && cursor == Some(row);
    let report = match report {
        None => {
            return list
                .flex_1()
                .min_h_0()
                .child(notice("Loading the shift report…", false, &theme))
                .into_any_element();
        }
        Some(Err(error)) => {
            return list
                .flex_1()
                .min_h_0()
                .child(notice(
                    format!("Could not load the report: {error}"),
                    true,
                    &theme,
                ))
                .into_any_element();
        }
        Some(Ok(report)) => report,
    };
    let text = |text: String| div().child(text).into_any_element();
    // The server reads shift hours on its UTC clock; show them on the
    // operator's too.
    let hours = ops::parse_date(&report.shift_date)
        .and_then(|date| ops::shift_window(date, &report.shift_period, time::UtcOffset::UTC));
    let span = hours.map_or_else(String::new, |(start, end)| {
        format!(
            "{} – {} (server hours, UTC)",
            ops::format_local(start, offset),
            ops::format_local(end, offset)
        )
    });

    let alarms = report
        .alarms
        .iter()
        .map(|alarm| {
            let color = severity_color(&alarm.severity, &theme);
            let what = if alarm.message.is_empty() {
                alarm.source_name.clone()
            } else {
                alarm.message.clone()
            };
            report_line(
                Some(pill(alarm.severity.to_uppercase(), color).into_any_element()),
                text(what),
                local_time(alarm.triggered_at, offset),
                &theme,
            )
        })
        .collect();
    let annotations = report
        .annotations
        .iter()
        .map(|note| {
            report_line(
                Some(pill(note.tag_id.clone(), theme.success).into_any_element()),
                text(format!(
                    "{} · user #{}",
                    note.annotation_text, note.author_id
                )),
                local_time(note.created_at, offset),
                &theme,
            )
        })
        .collect();
    let handovers = report
        .handovers
        .iter()
        .map(|entry| {
            report_line(
                Some(pill(format!("#{}", entry.author_id), theme.info).into_any_element()),
                div()
                    .child(mention_text(&entry.what_happened, &theme))
                    .into_any_element(),
                local_time(entry.created_at, offset),
                &theme,
            )
        })
        .collect();
    let security = report
        .security_events
        .iter()
        .map(|event| {
            let what = event
                .details
                .clone()
                .unwrap_or_else(|| event.event_type.clone());
            let who: Vec<String> = [event.username.clone(), event.source_ip.clone()]
                .into_iter()
                .flatten()
                .collect();
            let what = if who.is_empty() {
                what
            } else {
                format!("{what} · {}", who.join(" · "))
            };
            report_line(
                Some(
                    pill(
                        event.severity.to_uppercase(),
                        severity_color(&event.severity, &theme),
                    )
                    .into_any_element(),
                ),
                text(what),
                local_time(event.created_at, offset),
                &theme,
            )
        })
        .collect();

    let body = div()
        .id("shift-report-scroll")
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&console.scroll(REPORT_SCROLL))
        .px(SPACE_9)
        .pt(SPACE_7)
        .pb(LIST_TAIL)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .mb(SPACE_6)
                .child(pill(report.shift_date.clone(), theme.success))
                .child(pill(capitalized(&report.shift_period), theme.info))
                .child(
                    div()
                        .flex_1()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(span),
                )
                .when_some(report_at, |this, at| {
                    this.child(
                        div()
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(SharedString::from(format!(
                                "Updated {} · every 30 s",
                                local_time(at, offset)
                            ))),
                    )
                }),
        )
        .child(section("Alarms", theme.danger, alarms, ring(0), &theme))
        .child(section(
            "Annotations",
            theme.success,
            annotations,
            ring(1),
            &theme,
        ))
        .child(section(
            "Handover notes",
            theme.info,
            handovers,
            ring(2),
            &theme,
        ))
        .child(section(
            "Security events",
            theme.warning,
            security,
            ring(3),
            &theme,
        ));
    list.flex_1().min_h_0().child(body).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_listed_once_across_the_notes() {
        assert_eq!(
            mentioned(&[
                "@maria check DC string B",
                "ask @bob and @maria",
                "mail ops@plant.id"
            ]),
            ["@maria", "@bob"]
        );
        assert!(mentioned(&["", ""]).is_empty());
    }

    #[test]
    fn the_period_filter_names_a_shift_or_none() {
        let mut state = State::default();
        assert_eq!(state.period_filter(), None);
        state.period = 3;
        assert_eq!(state.period_filter(), Some("night"));
        state.period = 4;
        assert_eq!(state.period_filter(), None);
    }
}

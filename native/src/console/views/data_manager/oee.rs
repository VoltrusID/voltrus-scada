//! Data Manager › OEE, the web client's `OeePanel.tsx`: a device's overall
//! equipment effectiveness over 8 h, 24 h or 7 d (the four figures as
//! cards with bars), what each figure is made of (Enter on a card drills
//! into it), the downtime events (open one with a machine state, close an
//! open one with a reason) and the shift schedule (add, delete).
//!
//! Reading is open to every user; the writes are operator routes, so their
//! buttons say why they are off for others.
//!
//! Keys: ←/→ move between the figures and Enter drills in, ↓ goes to the
//! list below; `,`/`.` step through the devices, `[`/`]` the ranges, `t`
//! switches Downtime and Shifts, `n` records downtime or adds a shift, `r`
//! fetches again. In the downtime list Enter closes the open event; in the
//! shift list ⌫ deletes the shift.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, Context, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent,
    SharedString, Subscription, Window, actions, div, px, uniform_list,
};
use serde::de::IgnoredAny;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::insights::{
    self, Band, CloseDowntime, DAYS, DOWNTIME_PATH, DOWNTIME_STATES, DowntimeEvent, NewDowntime,
    NewShift, OEE_RANGES, OeeMetrics, SHIFTS_PATH, ShiftConfig, WEEKDAYS, band, days_label,
    downtime_by_state, duration_label, percent,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, ButtonKind, IconButton, Segment, field, kbd, pill, segmented};
use vt_ui::text_input::{InputKind, TextInput};
use vt_ui::theme::*;

use super::insights_kit::{self, bar, chips, devices, every_while_shown, figure};
use super::sidebar_width;
use crate::console::overlay::Overlay;
use crate::console::views::ops_kit::{close_overlay_in, notice};
use crate::console::views::tag_detail::{dialog, note_line, on_field, text_field};
use crate::console::views::{
    ListKeys, card, focus_ring, keyboard_list, local_time, page_header, rows_on_screen, tail_row,
    tail_rows,
};
use crate::console::{Console, time_now_secs};

/// The section's id: its calls are `data.oee.*`.
pub(crate) const ID: &str = "oee";
/// The section root's key context: its keys never fire while typing.
pub const OEE: &str = "Oee && !TextInput";
/// The figure cards' key context.
pub const OEE_CARDS: &str = "OeeCards";
const EVENTS: &str = "data.oee.events";
const SHIFTS: &str = "data.oee.shifts";
const RECORD: &str = "oee-record";
const CLOSE: &str = "oee-close";
const NEW_SHIFT: &str = "oee-new-shift";
const DELETE_SHIFT: &str = "oee-delete-shift";
const ROW_HEIGHT_PX: f32 = 34.0;
const DETAIL: f32 = 340.0;
/// Fetched again this often while on screen: downtime opens and closes
/// without a WebSocket message.
const REFRESH: Duration = Duration::from_secs(15);

actions!(
    voltrus_console,
    [
        OeePreviousFigure,
        OeeNextFigure,
        OeeDrill,
        OeeToList,
        OeePreviousDevice,
        OeeNextDevice,
        OeePreviousRange,
        OeeNextRange,
        OeeSwitchTab,
        OeeNew,
        OeeRefresh,
        OeeDeleteShift,
    ]
);

/// The four figures, in the web gauges' order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Figure {
    Oee,
    Availability,
    Performance,
    Quality,
}

impl Figure {
    pub(crate) const ALL: [Self; 4] = [
        Self::Oee,
        Self::Availability,
        Self::Performance,
        Self::Quality,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Oee => "OEE",
            Self::Availability => "Availability",
            Self::Performance => "Performance",
            Self::Quality => "Quality",
        }
    }

    fn value(self, metrics: &OeeMetrics) -> f64 {
        match self {
            Self::Oee => metrics.oee,
            Self::Availability => metrics.availability,
            Self::Performance => metrics.performance,
            Self::Quality => metrics.quality,
        }
    }

    fn color(self, metrics: &OeeMetrics, theme: &Theme) -> Hsla {
        match self {
            Self::Oee => match band(metrics.oee) {
                Band::Good => theme.success,
                Band::Fair => theme.warning,
                Band::Poor => theme.danger,
            },
            Self::Availability => theme.series[0],
            Self::Performance => theme.series[1],
            Self::Quality => theme.series[2],
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Downtime,
    Shifts,
}

/// One answer as last fetched, numbered so a slow answer never overwrites
/// a newer one.
struct Fetched<T> {
    value: Option<Result<T, String>>,
    seq: i64,
}

impl<T> Default for Fetched<T> {
    fn default() -> Self {
        Self {
            value: None,
            seq: 0,
        }
    }
}

struct RecordForm {
    device: Option<SharedString>,
    state: &'static str,
    error: Option<String>,
    sending: bool,
    focus: FocusHandle,
}

struct CloseForm {
    event: DowntimeEvent,
    reason: Entity<TextInput>,
    error: Option<String>,
    sending: bool,
    _subscription: Subscription,
}

struct ShiftForm {
    name: Entity<TextInput>,
    start: Entity<TextInput>,
    hours: Entity<TextInput>,
    days: u32,
    error: Option<String>,
    sending: bool,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(crate) struct State {
    /// The device the figures are for; `None` lists every device's
    /// downtime and shows no figures.
    device: Option<SharedString>,
    /// Whether the device was chosen (a first look picks the first one).
    device_chosen: bool,
    /// Index into [`OEE_RANGES`].
    range: usize,
    /// The `[from, to]` of the last fetch.
    span: (i64, i64),
    metrics: Fetched<OeeMetrics>,
    events: Fetched<Rc<Vec<DowntimeEvent>>>,
    shifts: Fetched<Rc<Vec<ShiftConfig>>>,
    asked: bool,
    /// The figure card under the keyboard cursor, and the one drilled into.
    pub(crate) figure: usize,
    pub(crate) drilled: usize,
    cards: Option<FocusHandle>,
    pub(crate) tab: Tab,
    record: Option<RecordForm>,
    close: Option<CloseForm>,
    shift: Option<ShiftForm>,
    delete: Option<(ShiftConfig, FocusHandle)>,
    window: Option<AnyWindowHandle>,
}

fn cards_focus(console: &mut Console, cx: &mut Context<Console>) -> FocusHandle {
    console
        .state::<State>()
        .cards
        .get_or_insert_with(|| cx.focus_handle())
        .clone()
}

/// The section came on screen: fetch what it has not got.
pub fn on_open(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        return;
    }
    if !console.state::<State>().device_chosen {
        let first = devices(console).first().cloned();
        let state = console.state::<State>();
        state.device = first;
        state.device_chosen = true;
    }
    if !console.state::<State>().asked {
        console.state::<State>().asked = true;
        fetch(console);
        fetch_shifts(console);
    }
    every_while_shown(console, ID, REFRESH, tick, cx);
    cx.notify();
}

fn tick(console: &mut Console, _: &mut Context<Console>) -> bool {
    fetch(console);
    true
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let focus = cards_focus(console, cx);
    window.focus(&focus);
    cx.notify();
}

/// The figures and the downtime for the device and range chosen.
fn fetch(console: &mut Console) {
    let now = time_now_secs();
    let state = console.state::<State>();
    let (from, to) = (now - OEE_RANGES[state.range].1, now);
    state.span = (from, to);
    let device = state.device.clone();
    state.events.seq += 1;
    let events_seq = state.events.seq;
    if device.is_some() {
        state.metrics.seq += 1;
    } else {
        state.metrics.value = None;
    }
    let metrics_seq = state.metrics.seq;
    console.call(
        CallKey::new("data.oee.events").with(events_seq),
        Call::get::<Vec<DowntimeEvent>>(insights::downtime_path(device.as_deref(), from, to)),
    );
    if let Some(device) = device {
        console.call(
            CallKey::new("data.oee.metrics").with(metrics_seq),
            Call::get::<OeeMetrics>(insights::oee_metrics_path(&device, from, to)),
        );
    }
}

fn fetch_shifts(console: &mut Console) {
    let state = console.state::<State>();
    state.shifts.seq += 1;
    let seq = state.shifts.seq;
    console.call(
        CallKey::new("data.oee.shifts").with(seq),
        Call::get::<Vec<ShiftConfig>>(SHIFTS_PATH),
    );
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    match key.name {
        "data.oee.metrics" if key.arg == state.metrics.seq => {
            state.metrics.value = Some(reply.take::<OeeMetrics>());
        }
        "data.oee.events" if key.arg == state.events.seq => {
            state.events.value = Some(reply.take::<Vec<DowntimeEvent>>().map(Rc::new));
        }
        "data.oee.shifts" if key.arg == state.shifts.seq => {
            state.shifts.value = Some(reply.take::<Vec<ShiftConfig>>().map(Rc::new));
        }
        "data.oee.record" => match reply.take::<DowntimeEvent>() {
            Ok(event) => {
                state.record = None;
                close_dialog(console, RECORD, cx);
                console.show_toast(
                    format!("Downtime opened: {} {}", event.device_id, event.state),
                    false,
                    cx,
                );
                fetch(console);
            }
            Err(error) => {
                if let Some(form) = state.record.as_mut() {
                    form.sending = false;
                    form.error = Some(error);
                }
            }
        },
        "data.oee.close" => match reply.take::<IgnoredAny>() {
            Ok(_) => {
                state.close = None;
                close_dialog(console, CLOSE, cx);
                console.show_toast("Downtime closed", false, cx);
                fetch(console);
            }
            Err(error) => {
                if let Some(form) = state.close.as_mut() {
                    form.sending = false;
                    form.error = Some(error);
                }
            }
        },
        "data.oee.shift_new" => match reply.take::<ShiftConfig>() {
            Ok(shift) => {
                state.shift = None;
                close_dialog(console, NEW_SHIFT, cx);
                console.show_toast(format!("Shift {} added", shift.name), false, cx);
                fetch_shifts(console);
            }
            Err(error) => {
                if let Some(form) = state.shift.as_mut() {
                    form.sending = false;
                    form.error = Some(error);
                }
            }
        },
        "data.oee.shift_delete" => {
            match reply.error() {
                None => console.show_toast("Shift deleted", false, cx),
                Some(error) => console.show_toast(format!("Delete failed: {error}"), true, cx),
            }
            fetch_shifts(console);
        }
        _ => return,
    }
    cx.notify();
}

fn close_dialog(console: &mut Console, id: &'static str, cx: &mut Context<Console>) {
    if let Some(window) = console.state::<State>().window {
        close_overlay_in(window, id, cx);
    }
}

// ── Choices ──────────────────────────────────────────────────────────────

fn pick_device(console: &mut Console, device: Option<SharedString>, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.device = device;
    state.device_chosen = true;
    state.metrics.value = None;
    fetch(console);
    cx.notify();
}

fn step_device(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let options = devices(console);
    let current = console.state::<State>().device.clone();
    let next = insights_kit::step(&options, current.as_deref(), forward).cloned();
    if next.is_some() {
        pick_device(console, next, cx);
    }
}

fn set_range(console: &mut Console, range: usize, cx: &mut Context<Console>) {
    console.state::<State>().range = range % OEE_RANGES.len();
    fetch(console);
    cx.notify();
}

fn move_figure(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let len = Figure::ALL.len();
    state.figure = if forward {
        (state.figure + 1).min(len - 1)
    } else {
        state.figure.saturating_sub(1)
    };
    cx.notify();
}

fn drill(console: &mut Console, figure: usize, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.figure = figure;
    state.drilled = figure;
    cx.notify();
}

fn list_id(tab: Tab) -> &'static str {
    match tab {
        Tab::Downtime => EVENTS,
        Tab::Shifts => SHIFTS,
    }
}

fn set_tab(console: &mut Console, tab: Tab, cx: &mut Context<Console>) {
    console.state::<State>().tab = tab;
    cx.notify();
}

fn new_item(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    match console.state::<State>().tab {
        Tab::Downtime => open_record(console, window, cx),
        Tab::Shifts => open_new_shift(console, window, cx),
    }
}

/// The writes are operator routes; a viewer hears why the button is off.
fn operator_or_toast(console: &mut Console, what: &str, cx: &mut Context<Console>) -> bool {
    if console.is_operator() {
        return true;
    }
    console.show_toast(
        format!("{what} needs the operator role (admin, demo or integration)"),
        true,
        cx,
    );
    false
}

// ── Dialogs ──────────────────────────────────────────────────────────────

fn open_record(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !operator_or_toast(console, "Recording downtime", cx) {
        return;
    }
    let focus = cx.focus_handle();
    let device = console
        .state::<State>()
        .device
        .clone()
        .or_else(|| devices(console).first().cloned());
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.record = Some(RecordForm {
        device,
        state: "idle",
        error: None,
        sending: false,
        focus: focus.clone(),
    });
    let overlay = Overlay::modal(RECORD, px(460.0), record_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn submit_record(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = console.state::<State>().record.as_mut() else {
        return;
    };
    if form.sending {
        return;
    }
    let body = match NewDowntime::new(form.device.as_deref().unwrap_or(""), form.state) {
        Ok(body) => body,
        Err(error) => {
            form.error = Some(error);
            cx.notify();
            return;
        }
    };
    form.sending = true;
    form.error = None;
    console.call(
        CallKey::new("data.oee.record"),
        Call::new(move |api| async move {
            api.send_json::<_, DowntimeEvent>(Method::POST, DOWNTIME_PATH, &body)
                .await
        }),
    );
    cx.notify();
}

fn record_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let options = devices(console);
    let Some(form) = console.state::<State>().record.as_ref() else {
        return div().into_any_element();
    };
    let (device, chosen, error, sending, focus) = (
        form.device.clone(),
        form.state,
        form.error.clone(),
        form.sending,
        form.focus.clone(),
    );
    let states = div().flex().flex_wrap().gap(SPACE_2).children(
        DOWNTIME_STATES
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                Button::new(("oee-record-state", index), name)
                    .compact()
                    .kind(if chosen == name {
                        ButtonKind::Primary
                    } else {
                        ButtonKind::Secondary
                    })
                    .on_click(cx.listener(move |console, _, _, cx| {
                        if let Some(form) = console.state::<State>().record.as_mut() {
                            form.state = name;
                        }
                        cx.notify();
                    }))
            }),
    );
    let device_row = if options.is_empty() {
        note_line(
            Icon::Info,
            "The plant's tags name no device.".into(),
            theme.fg_muted,
        )
        .into_any_element()
    } else {
        chips(
            "oee-record-device",
            &options,
            device.as_deref(),
            None,
            |console, device, cx| {
                if let Some(form) = console.state::<State>().record.as_mut() {
                    form.device = device;
                }
                cx.notify();
            },
            cx,
        )
    };
    let mut body = vec![
        field("Device", device_row, cx).into_any_element(),
        field("Machine state", states, cx).into_any_element(),
        note_line(
            Icon::Info,
            "The event starts now and stays open until it is closed with a reason.".into(),
            theme.fg_muted,
        )
        .into_any_element(),
    ];
    if let Some(error) = error {
        body.push(note_line(Icon::CircleAlert, error, theme.danger).into_any_element());
    }
    let buttons = vec![
        Button::new("oee-record-cancel", "Cancel")
            .ghost()
            .on_click(cx.listener(|console, _, window, cx| {
                console.state::<State>().record = None;
                console.close_overlay(window, cx);
            }))
            .into_any_element(),
        Button::new(
            "oee-record-save",
            if sending { "Opening…" } else { "Open event" },
        )
        .primary()
        .icon(Icon::Clock)
        .disabled(sending)
        .on_click(cx.listener(|console, _, window, cx| submit_record(console, window, cx)))
        .into_any_element(),
    ];
    div()
        .track_focus(&focus)
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "enter" && event.keystroke.modifiers.platform {
                submit_record(console, window, cx);
                cx.stop_propagation();
            }
        }))
        .child(dialog(
            "Record downtime",
            "A machine stopped: open a downtime event for it (⌘↵ opens).",
            body,
            buttons,
            cx,
        ))
        .into_any_element()
}

fn ask_close(
    console: &mut Console,
    event: DowntimeEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !event.is_open() {
        console.show_toast("That downtime event is already closed", false, cx);
        return;
    }
    if !operator_or_toast(console, "Closing downtime", cx) {
        return;
    }
    let reason = text_field("What stopped it (optional)", InputKind::Text, false, cx);
    let subscription = on_field(&reason, submit_close, window, cx);
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.close = Some(CloseForm {
        event,
        reason: reason.clone(),
        error: None,
        sending: false,
        _subscription: subscription,
    });
    let overlay = Overlay::modal(CLOSE, px(440.0), close_dialog_view, cx);
    console.open_overlay(overlay, window, cx);
    reason.read(cx).focus(window);
}

fn submit_close(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = console.state::<State>().close.as_mut() else {
        return;
    };
    if form.sending {
        return;
    }
    form.sending = true;
    form.error = None;
    let body = CloseDowntime::new(form.reason.read(cx).value());
    let path = insights::close_downtime_path(form.event.id);
    console.call(
        CallKey::new("data.oee.close"),
        Call::new(move |api| async move {
            api.send_json::<_, IgnoredAny>(Method::PUT, &path, &body)
                .await
        }),
    );
    cx.notify();
}

fn close_dialog_view(
    console: &mut Console,
    _: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let offset = console.utc_offset;
    let Some(form) = console.state::<State>().close.as_ref() else {
        return div().into_any_element();
    };
    let event = &form.event;
    let summary = format!(
        "{} {} since {} ({})",
        event.device_id,
        event.state,
        local_time(event.started_at, offset),
        duration_label(event.duration_at(time_now_secs()))
    );
    let mut body = vec![
        note_line(Icon::Clock, summary, theme.fg_muted).into_any_element(),
        field("Reason", form.reason.clone(), cx).into_any_element(),
    ];
    if let Some(error) = form.error.clone() {
        body.push(note_line(Icon::CircleAlert, error, theme.danger).into_any_element());
    }
    let sending = form.sending;
    let buttons = vec![
        Button::new("oee-close-cancel", "Cancel")
            .ghost()
            .on_click(cx.listener(|console, _, window, cx| {
                console.state::<State>().close = None;
                console.close_overlay(window, cx);
            }))
            .into_any_element(),
        Button::new(
            "oee-close-save",
            if sending { "Closing…" } else { "Close event" },
        )
        .primary()
        .icon(Icon::Check)
        .disabled(sending)
        .on_click(cx.listener(|console, _, window, cx| submit_close(console, window, cx)))
        .into_any_element(),
    ];
    dialog(
        "Close downtime",
        "The machine runs again: the event ends now (↵ closes it).",
        body,
        buttons,
        cx,
    )
}

fn open_new_shift(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !operator_or_toast(console, "Adding a shift", cx) {
        return;
    }
    let name = text_field("Morning", InputKind::Text, false, cx);
    let start = text_field("06:00", InputKind::Text, true, cx);
    let hours = text_field("8", InputKind::Unsigned, true, cx);
    start.update(cx, |field, cx| field.set_text("06:00".to_owned(), cx));
    hours.update(cx, |field, cx| field.set_text("8".to_owned(), cx));
    let subscriptions = [&name, &start, &hours]
        .into_iter()
        .map(|input| on_field(input, submit_shift, window, cx))
        .collect();
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.shift = Some(ShiftForm {
        name: name.clone(),
        start,
        hours,
        days: WEEKDAYS,
        error: None,
        sending: false,
        _subscriptions: subscriptions,
    });
    let overlay = Overlay::modal(NEW_SHIFT, px(480.0), shift_dialog, cx);
    console.open_overlay(overlay, window, cx);
    name.read(cx).focus(window);
}

fn submit_shift(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = console.state::<State>().shift.as_mut() else {
        return;
    };
    if form.sending {
        return;
    }
    let shift = NewShift::parse(
        form.name.read(cx).value(),
        form.start.read(cx).value(),
        form.hours.read(cx).value(),
        form.days,
    );
    let body = match shift {
        Ok(body) => body,
        Err(error) => {
            form.error = Some(error);
            cx.notify();
            return;
        }
    };
    form.sending = true;
    form.error = None;
    console.call(
        CallKey::new("data.oee.shift_new"),
        Call::new(move |api| async move {
            api.send_json::<_, ShiftConfig>(Method::POST, SHIFTS_PATH, &body)
                .await
        }),
    );
    cx.notify();
}

fn shift_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(form) = console.state::<State>().shift.as_ref() else {
        return div().into_any_element();
    };
    let days = form.days;
    let day_buttons = div()
        .flex()
        .gap(SPACE_2)
        .children(DAYS.iter().enumerate().map(|(bit, day)| {
            let on = days & (1 << bit) != 0;
            Button::new(("oee-shift-day", bit), *day)
                .compact()
                .kind(if on {
                    ButtonKind::Primary
                } else {
                    ButtonKind::Secondary
                })
                .on_click(cx.listener(move |console, _, _, cx| {
                    if let Some(form) = console.state::<State>().shift.as_mut() {
                        form.days ^= 1 << bit;
                    }
                    cx.notify();
                }))
        }));
    let mut body = vec![
        field("Name", form.name.clone(), cx).into_any_element(),
        div()
            .grid()
            .grid_cols(2)
            .gap(SPACE_6)
            .child(field("Starts at (HH:MM)", form.start.clone(), cx))
            .child(field("Lasts (hours, 1–24)", form.hours.clone(), cx))
            .into_any_element(),
        field(
            format!("Days · {}", days_label(days)),
            day_buttons.into_any_element(),
            cx,
        )
        .into_any_element(),
    ];
    if let Some(error) = form.error.clone() {
        body.push(note_line(Icon::CircleAlert, error, theme.danger).into_any_element());
    }
    let sending = form.sending;
    let buttons = vec![
        Button::new("oee-shift-cancel", "Cancel")
            .ghost()
            .on_click(cx.listener(|console, _, window, cx| {
                console.state::<State>().shift = None;
                console.close_overlay(window, cx);
            }))
            .into_any_element(),
        Button::new(
            "oee-shift-save",
            if sending { "Adding…" } else { "Add shift" },
        )
        .primary()
        .icon(Icon::Plus)
        .disabled(sending)
        .on_click(cx.listener(|console, _, window, cx| submit_shift(console, window, cx)))
        .into_any_element(),
    ];
    dialog(
        "New shift",
        "When a crew works: the schedule OEE planning counts on (↵ adds it).",
        body,
        buttons,
        cx,
    )
}

fn ask_delete_shift(
    console: &mut Console,
    shift: ShiftConfig,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !operator_or_toast(console, "Deleting a shift", cx) {
        return;
    }
    let focus = cx.focus_handle();
    console.state::<State>().delete = Some((shift, focus.clone()));
    let overlay = Overlay::modal(DELETE_SHIFT, px(420.0), delete_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn delete_confirmed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some((shift, _)) = console.state::<State>().delete.take() else {
        return;
    };
    console.close_overlay(window, cx);
    console.call(
        CallKey::new("data.oee.shift_delete").with(shift.id),
        Call::send_empty::<IgnoredAny>(Method::DELETE, insights::shift_path(shift.id)),
    );
}

fn delete_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let Some((shift, focus)) = console.state::<State>().delete.clone() else {
        return div().into_any_element();
    };
    div()
        .track_focus(&focus)
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "enter" {
                delete_confirmed(console, window, cx);
                cx.stop_propagation();
            }
        }))
        .child(dialog(
            format!("Delete the {} shift?", shift.name),
            format!(
                "{} · {}. This cannot be undone (↵ deletes, Esc keeps).",
                shift.window_label(),
                days_label(shift.days_mask)
            ),
            Vec::new(),
            vec![
                Button::new("oee-delete-keep", "Keep")
                    .ghost()
                    .on_click(cx.listener(|console, _, window, cx| {
                        console.state::<State>().delete = None;
                        console.close_overlay(window, cx);
                    }))
                    .into_any_element(),
                Button::new("oee-delete-confirm", "Delete")
                    .danger()
                    .icon(Icon::Trash)
                    .on_click(cx.listener(|console, _, window, cx| {
                        delete_confirmed(console, window, cx);
                    }))
                    .into_any_element(),
            ],
            cx,
        ))
        .into_any_element()
}

// ── Lists ────────────────────────────────────────────────────────────────

fn events_of(console: &Console) -> Rc<Vec<DowntimeEvent>> {
    console
        .state_ref::<State>()
        .and_then(|state| state.events.value.as_ref())
        .and_then(|value| value.as_ref().ok())
        .cloned()
        .unwrap_or_default()
}

fn shifts_of(console: &Console) -> Rc<Vec<ShiftConfig>> {
    console
        .state_ref::<State>()
        .and_then(|state| state.shifts.value.as_ref())
        .and_then(|value| value.as_ref().ok())
        .cloned()
        .unwrap_or_default()
}

fn activate_event(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(event) = events_of(console).get(row).cloned() {
        ask_close(console, event, window, cx);
    }
}

fn activate_shift(_: &mut Console, _: usize, _: bool, _: &mut Window, _: &mut Context<Console>) {}

fn delete_cursor_shift(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let row = console.list(SHIFTS, cx).cursor;
    if let Some(shift) = row.and_then(|row| shifts_of(console).get(row).cloned()) {
        ask_delete_shift(console, shift, window, cx);
    }
}

pub(crate) fn state_color(state: &str, theme: &Theme) -> Hsla {
    match state {
        "faulted" => theme.danger,
        "idle" => theme.warning,
        "setup" => theme.info,
        "maintenance" => theme.accent,
        "running" => theme.success,
        _ => theme.fg_dim,
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let options = devices(console);
    let cards = cards_focus(console, cx);
    let state = console.state::<State>();
    let (device, range, span) = (state.device.clone(), state.range, state.span);
    let metrics = state.metrics.value.clone();
    let events = state.events.value.clone();
    let shift_count = state
        .shifts
        .value
        .as_ref()
        .and_then(|value| value.as_ref().ok())
        .map_or(0, |shifts| shifts.len());
    let (figure, drilled, tab) = (state.figure, state.drilled, state.tab);
    let cards_focused = cards.is_focused(window);
    let now = time_now_secs();

    let ranges = segmented(
        "oee-range",
        OEE_RANGES
            .iter()
            .enumerate()
            .map(|(index, (label, _))| {
                Segment::new(
                    *label,
                    index == range,
                    cx.listener(move |console, _, _, cx| set_range(console, index, cx)),
                )
            })
            .collect(),
        cx,
    );
    let controls = vec![
        ranges,
        IconButton::new("oee-refresh", Icon::RefreshCw)
            .tooltip("Fetch again  R")
            .on_click(cx.listener(|console, _, _, cx| {
                fetch(console);
                fetch_shifts(console);
                cx.notify();
            }))
            .into_any_element(),
    ];
    let subtitle = match &device {
        Some(device) => format!(
            "{device} · last {} · ←→ figure · ↵ drill in · , . device · [ ] range",
            OEE_RANGES[range].0
        ),
        None => "Every device's downtime · choose a device for its figures".to_owned(),
    };

    let device_bar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .px(SPACE_9)
        .py(SPACE_4)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_none()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child("DEVICE"),
        )
        .child(chips(
            "oee-device",
            &options,
            device.as_deref(),
            Some("All devices"),
            pick_device,
            cx,
        ));

    // The four figures.
    let figures: AnyElement = match (&device, &metrics) {
        (None, _) => notice("Choose a device above (, .) to see its OEE.", false, &theme),
        (Some(_), None) => notice("Fetching the figures…", false, &theme),
        (Some(_), Some(Err(error))) => notice(format!("OEE: {error}"), true, &theme),
        (Some(_), Some(Ok(metrics))) => div()
            .flex()
            .gap(SPACE_6)
            .children(Figure::ALL.iter().enumerate().map(|(index, which)| {
                figure_card(
                    *which,
                    metrics,
                    index,
                    cards_focused && figure == index,
                    drilled == index,
                    &theme,
                    cx,
                )
            }))
            .into_any_element(),
    };
    let figures = div()
        .id("oee-figures")
        .key_context(OEE_CARDS)
        .track_focus(&cards)
        .flex_none()
        .px(SPACE_9)
        .pt(SPACE_7)
        .min_h(px(112.0))
        .on_action(cx.listener(|console, _: &OeePreviousFigure, _, cx| {
            move_figure(console, false, cx);
        }))
        .on_action(cx.listener(|console, _: &OeeNextFigure, _, cx| {
            move_figure(console, true, cx);
        }))
        .on_action(cx.listener(|console, _: &OeeDrill, _, cx| {
            let figure = console.state::<State>().figure;
            drill(console, figure, cx);
        }))
        .on_action(cx.listener(|console, _: &OeeToList, window, cx| {
            let tab = console.state::<State>().tab;
            console.focus_list(list_id(tab), window, cx);
        }))
        .child(figures);

    let detail = match (&device, &metrics) {
        (Some(_), Some(Ok(metrics))) => detail_card(
            Figure::ALL[drilled.min(Figure::ALL.len() - 1)],
            metrics,
            events.as_ref().and_then(|events| events.as_ref().ok()),
            now,
            &theme,
        ),
        _ => card(&theme)
            .p(SPACE_7)
            .text_size(FONT_SMALL)
            .text_color(theme.fg_muted)
            .child("What a figure is made of shows here: pick a device, then ↵ on a figure.")
            .into_any_element(),
    };

    // Downtime and shifts.
    let width = f32::from(window.viewport_size().width - sidebar_width()) - DETAIL - 80.0;
    let tabs = segmented(
        "oee-tab",
        vec![
            Segment::new(
                format!(
                    "Downtime{}",
                    events
                        .as_ref()
                        .and_then(|events| events.as_ref().ok())
                        .map_or(String::new(), |events| format!(" · {}", events.len()))
                ),
                tab == Tab::Downtime,
                cx.listener(|console, _, _, cx| set_tab(console, Tab::Downtime, cx)),
            )
            .icon(Icon::Clock),
            Segment::new(
                format!("Shifts · {shift_count}"),
                tab == Tab::Shifts,
                cx.listener(|console, _, _, cx| set_tab(console, Tab::Shifts, cx)),
            )
            .icon(Icon::Rows),
        ],
        cx,
    );
    let add = match tab {
        Tab::Downtime => Button::new("oee-record", "Record downtime")
            .compact()
            .icon(Icon::Plus)
            .tooltip(if operator {
                "N"
            } else {
                "Needs the operator role (admin, demo or integration)"
            })
            .disabled(!operator)
            .on_click(cx.listener(|console, _, window, cx| open_record(console, window, cx))),
        Tab::Shifts => Button::new("oee-new-shift", "New shift")
            .compact()
            .icon(Icon::Plus)
            .tooltip(if operator {
                "N"
            } else {
                "Needs the operator role (admin, demo or integration)"
            })
            .disabled(!operator)
            .on_click(cx.listener(|console, _, window, cx| open_new_shift(console, window, cx))),
    };
    let list = match tab {
        Tab::Downtime => events_list(console, device.is_none(), width, span, window, cx),
        Tab::Shifts => shifts_list(console, operator, window, cx),
    };
    let lists = card(&theme)
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .h(px(44.0))
                .px(SPACE_6)
                .border_b_1()
                .border_color(theme.border)
                .child(tabs)
                .child(kbd("T", cx))
                .child(div().flex_1())
                .child(add),
        )
        .child(list);

    div()
        .key_context("Oee")
        .flex()
        .flex_col()
        .size_full()
        .on_action(cx.listener(|console, _: &OeePreviousDevice, _, cx| {
            step_device(console, false, cx);
        }))
        .on_action(cx.listener(|console, _: &OeeNextDevice, _, cx| {
            step_device(console, true, cx);
        }))
        .on_action(cx.listener(|console, _: &OeePreviousRange, _, cx| {
            let range = console.state::<State>().range;
            set_range(console, range + OEE_RANGES.len() - 1, cx);
        }))
        .on_action(cx.listener(|console, _: &OeeNextRange, _, cx| {
            let range = console.state::<State>().range;
            set_range(console, range + 1, cx);
        }))
        .on_action(cx.listener(|console, _: &OeeSwitchTab, window, cx| {
            let tab = match console.state::<State>().tab {
                Tab::Downtime => Tab::Shifts,
                Tab::Shifts => Tab::Downtime,
            };
            set_tab(console, tab, cx);
            console.focus_list(list_id(tab), window, cx);
        }))
        .on_action(cx.listener(|console, _: &OeeNew, window, cx| {
            new_item(console, window, cx);
        }))
        .on_action(cx.listener(|console, _: &OeeRefresh, _, cx| {
            fetch(console);
            fetch_shifts(console);
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &OeeDeleteShift, window, cx| {
            if console.state::<State>().tab == Tab::Shifts {
                delete_cursor_shift(console, window, cx);
            }
        }))
        .child(page_header("OEE", subtitle, controls, cx))
        .child(device_bar)
        .child(figures)
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .gap(SPACE_6)
                .px(SPACE_9)
                .py(SPACE_7)
                .child(div().flex_none().w(px(DETAIL)).child(detail))
                .child(div().flex().flex_col().flex_1().min_w_0().child(lists)),
        )
        .into_any_element()
}

/// One figure: its name, the percentage, a bar, and what it is.
fn figure_card(
    which: Figure,
    metrics: &OeeMetrics,
    index: usize,
    ring: bool,
    drilled: bool,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let value = which.value(metrics);
    let color = which.color(metrics, theme);
    let line = match which {
        Figure::Oee => "Availability × Performance × Quality".to_owned(),
        Figure::Availability => format!(
            "Ran {} of {}",
            duration_label(metrics.total_running_secs),
            duration_label(metrics.total_planned_secs)
        ),
        Figure::Performance | Figure::Quality => "No production counts yet".to_owned(),
    };
    let hover = theme.hover;
    card(theme)
        .id(("oee-figure", index))
        .relative()
        .flex_1()
        .gap(SPACE_3)
        .p(SPACE_6)
        .cursor_pointer()
        .when(drilled, |this| this.border_color(color.alpha(0.6)))
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |console, _, window, cx| {
            drill(console, index, cx);
            let focus = cards_focus(console, cx);
            window.focus(&focus);
        }))
        .when(ring, |this| this.child(focus_ring(theme)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child(which.title().to_uppercase())
                .child(div().flex_1())
                .when(drilled, |this| {
                    this.child(vt_ui::icon(Icon::ChevronDown, ICON_SM, color))
                }),
        )
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(format!("{}%", percent(value))),
        )
        .child(div().flex().child(bar(value, color, px(6.0), theme)))
        .child(
            div()
                .truncate()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(line),
        )
        .into_any_element()
}

/// One labelled bar row: name, bar, percentage (the web's BreakdownBar).
fn breakdown_row(
    label: String,
    value: f64,
    color: Hsla,
    right: String,
    theme: &Theme,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .child(
            div()
                .w(px(92.0))
                .flex_none()
                .truncate()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(label),
        )
        .child(bar(value, color, px(8.0), theme))
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_right()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .child(right),
        )
        .into_any_element()
}

/// What the drilled figure is made of.
#[allow(clippy::cast_precision_loss)]
fn detail_card(
    which: Figure,
    metrics: &OeeMetrics,
    events: Option<&Rc<Vec<DowntimeEvent>>>,
    now: i64,
    theme: &Theme,
) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    let pct = |value: f64| format!("{}%", percent(value));
    match which {
        Figure::Oee => {
            rows.push(
                div()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(format!(
                        "{} × {} × {} = {}",
                        pct(metrics.availability),
                        pct(metrics.performance),
                        pct(metrics.quality),
                        pct(metrics.oee)
                    ))
                    .into_any_element(),
            );
            for part in [Figure::Availability, Figure::Performance, Figure::Quality] {
                let value = part.value(metrics);
                rows.push(breakdown_row(
                    part.title().to_owned(),
                    value,
                    part.color(metrics, theme),
                    pct(value),
                    theme,
                ));
            }
            rows.push(
                div()
                    .grid()
                    .grid_cols(3)
                    .gap(SPACE_5)
                    .pt(SPACE_3)
                    .child(figure(
                        "Running",
                        duration_label(metrics.total_running_secs),
                        theme.fg,
                        theme,
                    ))
                    .child(figure(
                        "Planned",
                        duration_label(metrics.total_planned_secs),
                        theme.fg,
                        theme,
                    ))
                    .child(figure(
                        "Downtime events",
                        metrics.downtime_count.to_string(),
                        theme.fg,
                        theme,
                    ))
                    .into_any_element(),
            );
        }
        Figure::Availability => {
            let down = (metrics.total_planned_secs - metrics.total_running_secs).max(0);
            rows.push(breakdown_row(
                "Running".into(),
                metrics.availability,
                theme.success,
                duration_label(metrics.total_running_secs),
                theme,
            ));
            rows.push(breakdown_row(
                "Down".into(),
                1.0 - metrics.availability,
                theme.danger,
                duration_label(down),
                theme,
            ));
            let by_state = events.map(|events| downtime_by_state(events, now));
            match by_state {
                Some(by_state) if !by_state.is_empty() => {
                    let longest = by_state.first().map_or(1, |(_, secs, _)| (*secs).max(1));
                    rows.push(
                        div()
                            .pt(SPACE_3)
                            .text_size(FONT_CAPTION)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.fg_dim)
                            .child("DOWNTIME BY STATE")
                            .into_any_element(),
                    );
                    for (name, secs, count) in by_state {
                        rows.push(breakdown_row(
                            format!("{name} ×{count}"),
                            secs as f64 / longest as f64,
                            state_color(&name, theme),
                            duration_label(secs),
                            theme,
                        ));
                    }
                }
                _ => rows.push(
                    note_line(
                        Icon::CircleCheck,
                        "No downtime in this range.".into(),
                        theme.fg_muted,
                    )
                    .into_any_element(),
                ),
            }
        }
        Figure::Performance | Figure::Quality => {
            let value = which.value(metrics);
            rows.push(breakdown_row(
                which.title().to_owned(),
                value,
                which.color(metrics, theme),
                pct(value),
                theme,
            ));
            rows.push(
                note_line(
                    Icon::Info,
                    format!(
                        "The server has no production counts to rate {} by yet, so it reports 100 %. OEE follows availability until it does.",
                        which.title().to_lowercase()
                    ),
                    theme.fg_muted,
                )
                .into_any_element(),
            );
        }
    }
    card(theme)
        .gap(SPACE_5)
        .p(SPACE_7)
        .child(
            div()
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::SEMIBOLD)
                .child(match which {
                    Figure::Oee => "OEE breakdown",
                    Figure::Availability => "Availability: where the time went",
                    Figure::Performance => "Performance",
                    Figure::Quality => "Quality",
                }),
        )
        .children(rows)
        .into_any_element()
}

const STATE_W: f32 = 96.0;
const DEVICE_W: f32 = 110.0;
const TIME_W: f32 = 96.0;
const DURATION_W: f32 = 72.0;
const ACTION_W: f32 = 64.0;

#[allow(clippy::too_many_lines)]
fn events_list(
    console: &mut Console,
    every_device: bool,
    width: f32,
    span: (i64, i64),
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let offset = console.utc_offset;
    let value = console.state::<State>().events.value.clone();
    let rows = match value {
        None => return notice("Fetching the downtime…", false, &theme),
        Some(Err(error)) => return notice(format!("Downtime: {error}"), true, &theme),
        Some(Ok(rows)) => rows,
    };
    let shown = rows.len();
    let list = keyboard_list(
        ListKeys {
            id: EVENTS,
            on_move: None,
            on_activate: activate_event,
        },
        shown,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    if shown == 0 {
        return list
            .flex_1()
            .child(notice(
                format!("No downtime started since {}.", local_time(span.0, offset)),
                false,
                &theme,
            ))
            .into_any_element();
    }
    let nav = console.list(EVENTS, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let device_column = every_device && width > 620.0;
    let ended_column = width > 540.0;
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(28.0))
        .px(SPACE_6)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(div().w(px(STATE_W)).flex_none().child("STATE"))
        .when(device_column, |this| {
            this.child(div().w(px(DEVICE_W)).flex_none().child("DEVICE"))
        })
        .child(div().w(px(TIME_W)).flex_none().child("STARTED"))
        .when(ended_column, |this| {
            this.child(div().w(px(TIME_W)).flex_none().child("ENDED"))
        })
        .child(
            div()
                .w(px(DURATION_W))
                .flex_none()
                .text_right()
                .child("DURATION"),
        )
        .child(div().flex_1().min_w_0().child("REASON"))
        .child(div().w(px(ACTION_W)).flex_none());
    let now = time_now_secs();
    let body = uniform_list(
        "oee-events",
        shown + tail_rows(shown, ROW_HEIGHT_PX),
        cx.processor(
            move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                range
                    .map(|row| match rows.get(row) {
                        None => tail_row(ROW_HEIGHT_PX),
                        Some(event) => {
                            let current = cursor == Some(row);
                            let hover = theme.hover;
                            let open = event.is_open();
                            let color = state_color(&event.state, &theme);
                            let target = event.clone();
                            div()
                                .id(("oee-event", row))
                                .relative()
                                .flex()
                                .items_center()
                                .gap(SPACE_5)
                                .h(px(ROW_HEIGHT_PX))
                                .px(SPACE_6)
                                .border_b_1()
                                .border_color(theme.border.alpha(0.6))
                                .text_size(FONT_SMALL)
                                .when(current, |this| this.bg(theme.selected))
                                .when(!current, |this| this.hover(move |style| style.bg(hover)))
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    console.list(EVENTS, cx).select(row);
                                    let focus = console.list(EVENTS, cx).focus.clone();
                                    window.focus(&focus);
                                    cx.notify();
                                }))
                                .when(focused && current, |this| this.child(focus_ring(&theme)))
                                .child(
                                    div()
                                        .w(px(STATE_W))
                                        .flex_none()
                                        .flex()
                                        .child(pill(event.state.clone(), color)),
                                )
                                .when(device_column, |this| {
                                    this.child(
                                        div()
                                            .w(px(DEVICE_W))
                                            .flex_none()
                                            .truncate()
                                            .font_family(MONO)
                                            .text_color(theme.fg_muted)
                                            .child(event.device_id.clone()),
                                    )
                                })
                                .child(
                                    div()
                                        .w(px(TIME_W))
                                        .flex_none()
                                        .font_family(MONO)
                                        .text_color(theme.fg_muted)
                                        .child(local_time(event.started_at, offset)),
                                )
                                .when(ended_column, |this| {
                                    this.child(
                                        div().w(px(TIME_W)).flex_none().flex().child(
                                            match event.ended_at {
                                                Some(ended) => div()
                                                    .font_family(MONO)
                                                    .text_color(theme.fg_muted)
                                                    .child(local_time(ended, offset))
                                                    .into_any_element(),
                                                None => {
                                                    pill("open", theme.warning).into_any_element()
                                                }
                                            },
                                        ),
                                    )
                                })
                                .child(
                                    div()
                                        .w(px(DURATION_W))
                                        .flex_none()
                                        .text_right()
                                        .font_family(MONO)
                                        .text_color(if open { theme.warning } else { theme.fg })
                                        .child(duration_label(event.duration_at(now))),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(if event.reason.is_some() {
                                            theme.fg
                                        } else {
                                            theme.fg_dim
                                        })
                                        .child(event.reason.clone().unwrap_or_else(|| "—".into())),
                                )
                                .child(div().w(px(ACTION_W)).flex_none().flex().justify_end().when(
                                    open,
                                    |this| {
                                        this.child(
                                            Button::new(("oee-close", row), "Close")
                                                .compact()
                                                .tooltip(if operator {
                                                    "Close with a reason  ↵"
                                                } else {
                                                    "Needs the operator role"
                                                })
                                                .disabled(!operator)
                                                .on_click(cx.listener(
                                                    move |console, _, window, cx| {
                                                        ask_close(
                                                            console,
                                                            target.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                        )
                                    },
                                ))
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .size_full();
    list.flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(header)
        .child(div().flex_1().min_h_0().child(body))
        .into_any_element()
}

fn shifts_list(
    console: &mut Console,
    operator: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let value = console.state::<State>().shifts.value.clone();
    let rows = match value {
        None => return notice("Fetching the shifts…", false, &theme),
        Some(Err(error)) => return notice(format!("Shifts: {error}"), true, &theme),
        Some(Ok(rows)) => rows,
    };
    let shown = rows.len();
    let list = keyboard_list(
        ListKeys {
            id: SHIFTS,
            on_move: None,
            on_activate: activate_shift,
        },
        shown,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    if shown == 0 {
        return list
            .flex_1()
            .child(notice("No shifts yet: add one with N.", false, &theme))
            .into_any_element();
    }
    let nav = console.list(SHIFTS, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    let scroll = nav.scroll.clone();
    let body = uniform_list(
        "oee-shifts",
        shown + tail_rows(shown, ROW_HEIGHT_PX),
        cx.processor(
            move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                range
                    .map(|row| match rows.get(row) {
                        None => tail_row(ROW_HEIGHT_PX),
                        Some(shift) => {
                            let current = cursor == Some(row);
                            let hover = theme.hover;
                            let target = shift.clone();
                            div()
                                .id(("oee-shift", row))
                                .relative()
                                .flex()
                                .items_center()
                                .gap(SPACE_5)
                                .h(px(ROW_HEIGHT_PX))
                                .px(SPACE_6)
                                .border_b_1()
                                .border_color(theme.border.alpha(0.6))
                                .text_size(FONT_SMALL)
                                .when(current, |this| this.bg(theme.selected))
                                .when(!current, |this| this.hover(move |style| style.bg(hover)))
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    console.list(SHIFTS, cx).select(row);
                                    let focus = console.list(SHIFTS, cx).focus.clone();
                                    window.focus(&focus);
                                    cx.notify();
                                }))
                                .when(focused && current, |this| this.child(focus_ring(&theme)))
                                .child(
                                    div()
                                        .w(px(140.0))
                                        .flex_none()
                                        .truncate()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(shift.name.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(132.0))
                                        .flex_none()
                                        .font_family(MONO)
                                        .child(shift.window_label()),
                                )
                                .child(
                                    div()
                                        .w(px(52.0))
                                        .flex_none()
                                        .font_family(MONO)
                                        .text_color(theme.fg_muted)
                                        .child(format!("{} h", shift.duration_hours)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme.fg_muted)
                                        .child(days_label(shift.days_mask)),
                                )
                                .child(
                                    IconButton::new(("oee-shift-delete", row), Icon::Trash)
                                        .tooltip(if operator {
                                            "Delete  ⌫"
                                        } else {
                                            "Needs the operator role"
                                        })
                                        .color(theme.danger)
                                        .on_click(cx.listener(move |console, _, window, cx| {
                                            ask_delete_shift(console, target.clone(), window, cx);
                                        })),
                                )
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .size_full();
    list.flex_1().min_h_0().child(body).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_figures_read_their_part_of_the_metrics() {
        let metrics = OeeMetrics {
            availability: 0.5,
            performance: 0.9,
            quality: 0.8,
            oee: 0.36,
            ..OeeMetrics::default()
        };
        let values: Vec<f64> = Figure::ALL.iter().map(|f| f.value(&metrics)).collect();
        assert_eq!(values, [0.36, 0.5, 0.9, 0.8]);
        assert_eq!(
            Figure::ALL.map(Figure::title),
            ["OEE", "Availability", "Performance", "Quality"]
        );
        assert_eq!(list_id(Tab::Downtime), EVENTS);
        assert_eq!(list_id(Tab::Shifts), SHIFTS);
    }
}

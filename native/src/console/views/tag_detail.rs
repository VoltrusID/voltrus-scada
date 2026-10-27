//! The Tags page's detail panel: the tag's value and facts, a chart with
//! its own span (a preset that runs live, or a custom stretch in the past),
//! history resolution and aggregation, overlays of other tags, annotations
//! (operators), CSV export of what the chart shows, and the write section
//! (`tag_write`). The chart scrolls at the display's rate while live; the
//! history comes from one `GET /api/v1/history` per span and setting, kept
//! apart from the store's own so it does not fight the Trends page.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, FontWeight, MouseButton, MouseDownEvent,
    MouseMoveEvent, SharedString, Subscription, Window, div, px, uniform_list,
};
use reqwest::Method;
use serde::de::IgnoredAny;
use voltrus_scada_native::runtime::{Call, CallKey, Link, Reply};
use voltrus_scada_native::tag_api::{
    self, Aggregation, Annotation, Annotations, Created, CsvColumn, Detail, NewAnnotation, SPANS,
    Span,
};
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{
    Button, IconButton, Segment, empty_state, field, kbd, pill, section_label, segmented,
};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::tag_chart::{self, ChartSpec, Line, PlotCell, Samples};
use super::{
    ListKeys, age, card, focus_ring, keyboard_list, local_time, now_ms, status_color, value_color,
    value_with_unit,
};
use crate::console::keys::{
    ListDown, ListUp, MENU, TagsAddNote, TagsExportCsv, TagsWrite, display, hint,
};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::store::Status;
use crate::console::{Console, Page, anim, time_now_secs};

/// The panel's width beside the table.
pub(crate) const PANEL: f32 = 400.0;
/// Overlays on one chart at most.
const MAX_OVERLAYS: usize = 5;
/// Full detail asks for every second: a day of it at most.
const FULL_DETAIL_MAX_MS: f64 = 24.0 * 3_600_000.0;
/// The notes list and the compare picker's list.
const NOTES: &str = "tag-notes";
const PICKER: &str = "tag-compare";
const PICK_ROW: f32 = 30.0;

/// A dialog of the detail panel, also openable from the command line
/// (`--overlay tag-write`, …) for screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagDialog {
    Write,
    Policy,
    Note,
    Compare,
    Range,
    /// Not a dialog: the detail across the whole page.
    Expand,
}

impl TagDialog {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "tag-write" => Self::Write,
            "tag-policy" => Self::Policy,
            "tag-note" => Self::Note,
            "tag-compare" => Self::Compare,
            "tag-range" => Self::Range,
            "tag-expand" => Self::Expand,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub(crate) struct DetailState {
    query: tag_api::HistoryQuery,
    /// The answer to `fetched_for`, by tag id: the lines it was asked for
    /// (the open tag and its overlays; the server answers only those).
    fetched: HashMap<i64, Samples>,
    /// The query `fetched` answers, and the lines it was asked for.
    fetched_for: Option<(tag_api::HistoryQuery, Vec<i64>)>,
    /// The server's bucket for `fetched_for`, ms.
    interval: u64,
    /// The query in flight, its lines and its number; older answers are
    /// dropped.
    asked: Option<(tag_api::HistoryQuery, Vec<i64>)>,
    seq: i64,
    history_error: Option<String>,
    /// Each line's samples (fetched, then live), rebuilt only when either
    /// changes: tag id → (fetched length, newest live time, samples).
    lines: HashMap<i64, (usize, u64, Samples)>,
    overlays: Vec<i64>,
    cursor: Option<f64>,
    plot: PlotCell,
    pub expanded: bool,
    /// Annotations of the tag with this key, newest first.
    notes: Option<(String, Vec<Annotation>)>,
    notes_asked: Option<String>,
    /// The number of the newest notes call; older answers are dropped.
    notes_seq: i64,
    notes_error: Option<String>,
    launch: Option<TagDialog>,
    // Dialog fields, made on first use.
    from: Option<Entity<TextInput>>,
    to: Option<Entity<TextInput>>,
    range_error: Option<String>,
    note_time: Option<Entity<TextInput>>,
    note_text: Option<Entity<TextInput>>,
    note_error: Option<String>,
    pick: Option<Entity<TextInput>>,
    subscriptions: Vec<Subscription>,
}

/// The position of the tag open in the detail panel, when the Tags page
/// shows one.
pub(crate) fn open_index(console: &Console) -> Option<usize> {
    if console.page != Page::Tags {
        return None;
    }
    console.selected.and_then(|id| console.store.position(id))
}

/// Remember a dialog to open once a tag is open (`--overlay tag-…`).
pub(crate) fn launch(console: &mut Console, dialog: TagDialog) {
    console.state::<DetailState>().launch = Some(dialog);
}

/// A text field for a dialog.
pub(crate) fn text_field(
    placeholder: &'static str,
    kind: InputKind,
    mono: bool,
    cx: &mut Context<Console>,
) -> Entity<TextInput> {
    cx.new(|cx| {
        let field = TextInput::new(cx).placeholder(placeholder).kind(kind);
        if mono { field.mono() } else { field }
    })
}

/// Enter in `field` runs `submit`; Esc closes the dialog.
pub(crate) fn on_field(
    field: &Entity<TextInput>,
    submit: fn(&mut Console, &mut Window, &mut Context<Console>),
    window: &mut Window,
    cx: &mut Context<Console>,
) -> Subscription {
    cx.subscribe_in(
        field,
        window,
        move |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => submit(console, window, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Changed | InputEvent::Blurred => cx.notify(),
        },
    )
}

/// The chart's `[from, to]` at `now`.
fn bounds_of(span: Span, now: f64) -> (f64, f64) {
    match span {
        #[allow(clippy::cast_precision_loss)]
        Span::Between { start_ms, end_ms } => (start_ms as f64, end_ms as f64),
        Span::Last(_) => (now - span.length_ms(), now),
    }
}

/// Ask for what the open tag's panel shows (its history and notes): on
/// selection and when the Tags page shows, never from render, which the
/// warm-up runs for every page.
pub(crate) fn fetch_open(console: &mut Console) {
    let Some(index) = open_index(console) else {
        return;
    };
    let key = console.store.tags[index].info.key.clone();
    ensure_history(console);
    ensure_notes(console, &key);
}

/// Ask for the history of the open tag and its overlays unless it is there
/// or on its way.
fn ensure_history(console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    let Some(index) = open_index(console) else {
        return;
    };
    let open = console.store.tags[index].info.id;
    let ids = line_ids(console, open);
    let keys: Vec<String> = ids
        .iter()
        .filter_map(|id| console.store.position(*id))
        .map(|index| console.store.tags[index].info.key.clone())
        .collect();
    let state = console.state::<DetailState>();
    let query = state.query;
    let covers = |asked: &Option<(tag_api::HistoryQuery, Vec<i64>)>| {
        asked
            .as_ref()
            .is_some_and(|(of, lines)| *of == query && ids.iter().all(|id| lines.contains(id)))
    };
    if covers(&state.fetched_for) || covers(&state.asked) {
        return;
    }
    state.asked = Some((query, ids));
    state.seq += 1;
    let seq = state.seq;
    let path = tag_api::history_path(&query, &keys);
    console.call(
        CallKey::new("tags.history").with(seq),
        Call::new(move |api| async move {
            let answer = api.get_json::<tag_api::HistoryAnswer>(&path).await?;
            let series: HashMap<String, Vec<(f64, f64)>> = answer
                .data
                .iter()
                .map(|(key, series)| (key.clone(), tag_api::chart_samples(series)))
                .collect();
            Ok::<_, anyhow::Error>((series, answer.interval))
        }),
    );
}

/// The samples of the lines `wanted` (tag id, key) out of a history answer
/// by key. Tags sharing a key share its samples.
fn wanted_lines(
    mut series: HashMap<String, Vec<(f64, f64)>>,
    wanted: &[(i64, &str)],
) -> HashMap<i64, Samples> {
    let mut lines: HashMap<i64, Samples> = HashMap::new();
    for (position, &(id, key)) in wanted.iter().enumerate() {
        let shared = wanted[..position]
            .iter()
            .find(|(_, other)| *other == key)
            .and_then(|(other, _)| lines.get(other).cloned());
        if let Some(samples) = shared.or_else(|| series.remove(key).map(Rc::new)) {
            lines.insert(id, samples);
        }
    }
    lines
}

/// Ask for the open tag's annotations (operators: the server's route is
/// `demo_protected`, reads included).
fn ensure_notes(console: &mut Console, key: &str) {
    if console.stress.is_some() || !console.is_operator() {
        return;
    }
    let state = console.state::<DetailState>();
    let have = state.notes.as_ref().is_some_and(|(of, _)| of == key);
    if have || state.notes_asked.as_deref() == Some(key) {
        return;
    }
    fetch_notes(console, key.to_owned());
}

fn fetch_notes(console: &mut Console, key: String) {
    let path = tag_api::annotations_path(&key);
    let state = console.state::<DetailState>();
    state.notes_asked = Some(key);
    state.notes_seq += 1;
    let seq = state.notes_seq;
    console.call(
        CallKey::new("tags.notes").with(seq),
        Call::get::<Annotations>(path),
    );
}

/// The samples of tag `id`'s line: the fetched history, then the live
/// samples after it.
fn line_samples(console: &mut Console, id: i64) -> Samples {
    let Some(index) = console.store.position(id) else {
        return Rc::default();
    };
    let latest = console.store.tags[index]
        .history
        .latest()
        .map_or(0, |(at, _)| at.to_bits());
    let state = console.state::<DetailState>();
    let fetched = state.fetched.get(&id).cloned();
    let fetched_len = fetched.as_ref().map_or(0, |samples| samples.len());
    if let Some((len, at, samples)) = state.lines.get(&id)
        && *len == fetched_len
        && *at == latest
    {
        return Rc::clone(samples);
    }
    let tag = &console.store.tags[index];
    let merged = Rc::new(match &fetched {
        Some(fetched) => tag_api::merge(fetched, tag.samples()),
        None => tag.samples().collect(),
    });
    console
        .state::<DetailState>()
        .lines
        .insert(id, (fetched_len, latest, Rc::clone(&merged)));
    merged
}

/// The tag first, then each overlay still in the store.
fn line_ids(console: &mut Console, id: i64) -> Vec<i64> {
    let overlays = console.state::<DetailState>().overlays.clone();
    std::iter::once(id)
        .chain(
            overlays
                .into_iter()
                .filter(|overlay| *overlay != id && console.store.position(*overlay).is_some()),
        )
        .collect()
}

pub(crate) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    match key.name {
        "tags.history" => {
            let state = console.state::<DetailState>();
            if key.arg != state.seq {
                return;
            }
            let asked = state.asked.take();
            match reply.take::<(HashMap<String, Vec<(f64, f64)>>, u64)>() {
                Ok((series, interval)) => {
                    let store = &console.store;
                    let wanted: Vec<(i64, &str)> = asked
                        .iter()
                        .flat_map(|(_, ids)| ids)
                        .filter_map(move |id| {
                            let index = store.position(*id)?;
                            Some((*id, store.tags[index].info.key.as_str()))
                        })
                        .collect();
                    let fetched = wanted_lines(series, &wanted);
                    let state = console.state::<DetailState>();
                    state.fetched = fetched;
                    state.fetched_for = asked;
                    state.interval = interval;
                    state.history_error = None;
                    state.lines.clear();
                }
                Err(error) => {
                    let state = console.state::<DetailState>();
                    // Asked again only when the span or a setting changes.
                    state.fetched_for = asked;
                    state.fetched.clear();
                    state.lines.clear();
                    state.history_error = Some(error);
                }
            }
        }
        "tags.notes" => {
            let state = console.state::<DetailState>();
            // An answer for a tag opened before is not this tag's.
            if key.arg != state.notes_seq {
                return;
            }
            let Some(of) = state.notes_asked.clone() else {
                return;
            };
            match reply.take::<Annotations>() {
                Ok(list) => {
                    state.notes = Some((of, list.annotations));
                    state.notes_error = None;
                }
                Err(error) => {
                    state.notes = Some((of, Vec::new()));
                    state.notes_error = Some(error);
                }
            }
        }
        "tags.note_add" | "tags.note_delete" => {
            let added = key.name == "tags.note_add";
            let error = reply.error().map(str::to_owned);
            match error {
                Some(error) => {
                    let what = if added {
                        "Add the note"
                    } else {
                        "Delete the note"
                    };
                    console.state::<DetailState>().note_error = Some(error.clone());
                    console.show_toast(format!("{what} failed: {error}"), true, cx);
                }
                None => {
                    if added {
                        if let Some(field) = console.state::<DetailState>().note_text.clone() {
                            field.update(cx, |field, cx| field.set_text("", cx));
                        }
                        console.show_toast("Note added", false, cx);
                    }
                    let of = console
                        .state::<DetailState>()
                        .notes
                        .as_ref()
                        .map(|(of, _)| of.clone());
                    if let Some(of) = of {
                        fetch_notes(console, of);
                    }
                }
            }
        }
        _ => {}
    }
    cx.notify();
}

// ── Actions ───────────────────────────────────────────────────────────────

fn set_span(console: &mut Console, span: Span, cx: &mut Context<Console>) {
    let state = console.state::<DetailState>();
    state.query.span = span;
    // A day of every second is the most worth asking for.
    if span.length_ms() > FULL_DETAIL_MAX_MS {
        state.query.detail = Detail::Auto;
    }
    state.cursor = None;
    ensure_history(console);
    cx.notify();
}

fn set_detail(console: &mut Console, detail: Detail, cx: &mut Context<Console>) {
    let state = console.state::<DetailState>();
    if detail == Detail::Full && state.query.span.length_ms() > FULL_DETAIL_MAX_MS {
        console.show_toast("Full detail covers a day at most", true, cx);
        return;
    }
    state.query.detail = detail;
    ensure_history(console);
    cx.notify();
}

fn set_agg(console: &mut Console, agg: Aggregation, cx: &mut Context<Console>) {
    console.state::<DetailState>().query.agg = agg;
    ensure_history(console);
    cx.notify();
}

fn toggle_overlay(console: &mut Console, id: i64, cx: &mut Context<Console>) {
    let state = console.state::<DetailState>();
    if let Some(position) = state.overlays.iter().position(|overlay| *overlay == id) {
        state.overlays.remove(position);
    } else if state.overlays.len() < MAX_OVERLAYS {
        state.overlays.push(id);
    } else {
        console.show_toast(
            format!("A chart takes {MAX_OVERLAYS} overlays at most"),
            true,
            cx,
        );
    }
    ensure_history(console);
    cx.notify();
}

pub(crate) fn toggle_expanded(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<DetailState>();
    state.expanded = !state.expanded;
    cx.notify();
}

/// Open a dialog of the detail panel for the open tag.
pub(crate) fn open_dialog(
    console: &mut Console,
    dialog: TagDialog,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if open_index(console).is_none() {
        console.show_toast("Open a tag on the Tags page first", true, cx);
        return;
    }
    match dialog {
        TagDialog::Write => super::tag_write::open_write(console, window, cx),
        TagDialog::Policy => super::tag_write::open_policy(console, window, cx),
        TagDialog::Note => {
            let at = console.state::<DetailState>().cursor;
            open_note(console, at, window, cx);
        }
        TagDialog::Compare => open_compare(console, window, cx),
        TagDialog::Range => open_range(console, window, cx),
        TagDialog::Expand => {
            console.state::<DetailState>().expanded = true;
            cx.notify();
        }
    }
}

fn open_range(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let offset = console.utc_offset;
    if console.state::<DetailState>().from.is_none() {
        let from = text_field("2026-10-25 13:00", InputKind::Text, true, cx);
        let to = text_field("2026-10-25 14:00", InputKind::Text, true, cx);
        let subscriptions = [
            on_field(&from, apply_range, window, cx),
            on_field(&to, apply_range, window, cx),
        ];
        let state = console.state::<DetailState>();
        state.subscriptions.extend(subscriptions);
        state.from = Some(from);
        state.to = Some(to);
    }
    let state = console.state::<DetailState>();
    #[allow(clippy::cast_possible_truncation)]
    let (start, end) = match state.query.span {
        Span::Between { start_ms, end_ms } => (start_ms, end_ms),
        span => {
            let now = now_ms() as i64;
            (now - span.length_ms() as i64, now)
        }
    };
    state.range_error = None;
    let (Some(from), Some(to)) = (state.from.clone(), state.to.clone()) else {
        return;
    };
    from.update(cx, |field, cx| {
        field.set_text(tag_api::format_local(start, offset), cx);
    });
    to.update(cx, |field, cx| {
        field.set_text(tag_api::format_local(end, offset), cx);
    });
    let overlay = Overlay::modal("tag-range", px(420.0), range_dialog, cx);
    console.open_overlay(overlay, window, cx);
    from.update(cx, |field, _| field.focus(window));
}

fn apply_range(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<DetailState>();
    let (Some(from), Some(to)) = (state.from.clone(), state.to.clone()) else {
        return;
    };
    let from = from.read(cx).value().to_owned();
    let to = to.read(cx).value().to_owned();
    match tag_api::parse_between(&from, &to, console.utc_offset) {
        Ok(span) => {
            set_span(console, span, cx);
            console.close_overlay(window, cx);
        }
        Err(error) => {
            console.state::<DetailState>().range_error = Some(error);
            cx.notify();
        }
    }
}

fn open_note(
    console: &mut Console,
    at: Option<f64>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !console.is_operator() {
        console.show_toast(
            "Notes need the operator role (admin, demo or integration)",
            true,
            cx,
        );
        return;
    }
    if console.state::<DetailState>().note_text.is_none() {
        let time = text_field("2026-10-25 14:05:09", InputKind::Text, true, cx);
        let text = text_field("What happened here?", InputKind::Text, false, cx);
        let subscriptions = [
            on_field(&time, save_note, window, cx),
            on_field(&text, save_note, window, cx),
        ];
        let state = console.state::<DetailState>();
        state.subscriptions.extend(subscriptions);
        state.note_time = Some(time);
        state.note_text = Some(text);
    }
    #[allow(clippy::cast_possible_truncation)]
    let at = at.unwrap_or_else(now_ms) as i64;
    let offset = console.utc_offset;
    let state = console.state::<DetailState>();
    state.note_error = None;
    let (Some(time), Some(text)) = (state.note_time.clone(), state.note_text.clone()) else {
        return;
    };
    time.update(cx, |field, cx| {
        field.set_text(tag_api::format_local(at, offset), cx);
    });
    let overlay = Overlay::modal("tag-note", px(440.0), note_dialog, cx);
    console.open_overlay(overlay, window, cx);
    text.update(cx, |field, _| field.focus(window));
}

/// Send the note dialog's note; the dialog closes at once, a failure comes
/// back as a toast with the text kept for another try.
fn save_note(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(index) = open_index(console) else {
        return;
    };
    let key = console.store.tags[index].info.key.clone();
    let offset = console.utc_offset;
    let state = console.state::<DetailState>();
    let (Some(time), Some(text)) = (state.note_time.clone(), state.note_text.clone()) else {
        return;
    };
    let text = text.read(cx).value().trim().to_owned();
    let time = time.read(cx).value().to_owned();
    let error = if text.is_empty() {
        Some("Write the note first".to_owned())
    } else {
        None
    };
    let at = tag_api::parse_local(&time, offset);
    match (error, at) {
        (Some(error), _) => state.note_error = Some(error),
        (None, None) => {
            state.note_error = Some(format!("“{}” is not a time", time.trim()));
        }
        (None, Some(at_ms)) => {
            state.note_error = None;
            let body = NewAnnotation {
                tag_id: key,
                annotation_text: text,
                data_timestamp: at_ms.div_euclid(1000),
            };
            console.call(
                CallKey::new("tags.note_add"),
                Call::new(move |api| async move {
                    api.send_json::<_, Created>(Method::POST, "/api/v1/annotations", &body)
                        .await
                }),
            );
            console.close_overlay(window, cx);
        }
    }
    cx.notify();
}

/// Delete note `id` when it is one of the open tag's notes as shown.
fn delete_note(console: &mut Console, id: i64) {
    let Some(index) = open_index(console) else {
        return;
    };
    let key = console.store.tags[index].info.key.clone();
    let state = console.state::<DetailState>();
    let shown = state
        .notes
        .as_ref()
        .is_some_and(|(of, notes)| *of == key && notes.iter().any(|note| note.id == id));
    if !shown {
        return;
    }
    console.call(
        CallKey::new("tags.note_delete").with(id),
        Call::send_empty::<IgnoredAny>(Method::DELETE, format!("/api/v1/annotations/{id}")),
    );
}

fn open_compare(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<DetailState>().pick.is_none() {
        let pick = text_field("Find a tag to compare", InputKind::Text, false, cx);
        let subscription = cx.subscribe_in(
            &pick,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => {
                    console.list(PICKER, cx).cursor = Some(0);
                    cx.notify();
                }
                InputEvent::Submitted => pick_row(console, None, cx),
                InputEvent::Cancelled => console.close_overlay(window, cx),
                InputEvent::Blurred => {}
            },
        );
        let state = console.state::<DetailState>();
        state.subscriptions.push(subscription);
        state.pick = Some(pick);
    }
    let Some(pick) = console.state::<DetailState>().pick.clone() else {
        return;
    };
    pick.update(cx, |field, cx| field.set_text("", cx));
    let overlay = Overlay::modal("tag-compare", px(460.0), compare_dialog, cx);
    console.open_overlay(overlay, window, cx);
    console.list(PICKER, cx).cursor = Some(0);
    pick.update(cx, |field, _| field.focus(window));
}

/// The compare picker's tags (store positions): every other tag matching
/// its filter.
fn picks(console: &mut Console, cx: &gpui::App) -> Vec<usize> {
    let open = console.selected;
    let query = console
        .state::<DetailState>()
        .pick
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    // The store keeps filter results per query: no tag is lowercased here.
    console
        .store
        .filter(&query)
        .iter()
        .copied()
        .filter(|index| Some(console.store.tags[*index].info.id) != open)
        .collect()
}

/// Put the picker's row (the cursor's by default) on the chart or take it
/// off.
fn pick_row(console: &mut Console, row: Option<usize>, cx: &mut Context<Console>) {
    let Some(row) = row.or_else(|| console.list(PICKER, cx).cursor) else {
        return;
    };
    let rows = picks(console, cx);
    if let Some(index) = rows.get(row).copied() {
        let id = console.store.tags[index].info.id;
        toggle_overlay(console, id, cx);
    }
}

/// ⇧⌘S: save what the chart shows (the tag and its overlays, within the
/// chart's span) as CSV, to a file the operator picks.
pub(crate) fn export_csv(console: &mut Console, cx: &mut Context<Console>) {
    let Some(index) = open_index(console) else {
        console.show_toast("Open a tag on the Tags page first", true, cx);
        return;
    };
    let id = console.store.tags[index].info.id;
    let span = console.state::<DetailState>().query.span;
    let (from, to) = bounds_of(span, now_ms());
    let columns: Vec<(String, Samples)> = line_ids(console, id)
        .into_iter()
        .filter_map(|id| {
            let key = console.store.tags[console.store.position(id)?]
                .info
                .key
                .clone();
            Some((key, line_samples(console, id)))
        })
        .collect();
    let csv_columns: Vec<CsvColumn<'_>> = columns
        .iter()
        .map(|(key, samples)| CsvColumn {
            key,
            samples: samples.as_slice(),
        })
        .collect();
    let (text, rows) = tag_api::csv(&csv_columns, from, to, console.utc_offset);
    if rows == 0 {
        console.show_toast("The chart shows no samples to export", true, cx);
        return;
    }
    #[allow(clippy::cast_possible_truncation)]
    let stamp = tag_api::format_local(now_ms() as i64, console.utc_offset).replace([' ', ':'], "-");
    let name = format!("{}-{stamp}.csv", console.store.tags[index].info.key);
    let directory = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let epoch = console.session_epoch();
    let chosen = cx.prompt_for_new_path(&directory, Some(name.as_str()));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(path))) = chosen.await else {
            return;
        };
        // A session that ended while the panel was open keeps its data.
        let same = this
            .update(cx, |console, _| console.session_epoch() == epoch)
            .unwrap_or(false);
        if !same {
            return;
        }
        let target = path.clone();
        let written = cx
            .background_executor()
            .spawn(async move { std::fs::write(target, text) })
            .await;
        let _ = this.update(cx, |console, cx| match written {
            Ok(()) => console.show_toast(
                format!("Saved {rows} rows to {}", path.display()),
                false,
                cx,
            ),
            Err(error) => console.show_toast(
                format!("Could not save {}: {error}", path.display()),
                true,
                cx,
            ),
        });
    })
    .detach();
}

// ── The panel ─────────────────────────────────────────────────────────────

/// The open tag in detail: beside the table, or across the page when
/// expanded.
#[allow(clippy::too_many_lines)]
pub(crate) fn panel(
    console: &mut Console,
    index: usize,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let id = console.store.tags[index].info.id;
    let key = console.store.tags[index].info.key.clone();
    if let Some(dialog) = console.state::<DetailState>().launch.take() {
        cx.defer_in(window, move |console, window, cx| {
            open_dialog(console, dialog, window, cx);
        });
    }

    let state = console.state::<DetailState>();
    let query = state.query;
    let expanded = state.expanded;
    let cursor = state.cursor;
    let plot = Rc::clone(&state.plot);
    let loading = state.asked.is_some();
    let history_error = state.history_error.clone();
    let interval = state.interval;
    let live = query.span.is_live() && console.link == Link::Live;
    let now = now_ms();
    if live {
        anim::request_frame(window, &console.meter);
    }
    let (from, to) = bounds_of(query.span, now);

    let ids = line_ids(console, id);
    let lines: Vec<(i64, Samples)> = ids
        .iter()
        .map(|line| (*line, line_samples(console, *line)))
        .collect();
    let tag = &console.store.tags[index];
    let color = status_color(tag.status, &theme);
    let main_color = if tag.status == Status::Normal {
        theme.accent
    } else {
        color
    };
    let line_color = |position: usize| {
        if position == 0 {
            main_color
        } else {
            theme.series(position)
        }
    };
    let stats = lines
        .first()
        .and_then(|(_, samples)| tag_api::stats(samples, from, to));

    let unit = tag.info.unit.clone();
    let with_unit = |value: f64| {
        format!("{} {unit}", format_value(value))
            .trim_end()
            .to_owned()
    };
    let threshold = |value: f64| {
        if value > 0.0 {
            with_unit(value)
        } else {
            "Off".to_owned()
        }
    };
    let updated = if tag.at_secs > 0 {
        format!("Updated {} ago", age(time_now_secs() - tag.at_secs))
    } else {
        "No value yet".to_owned()
    };
    let device = if tag.info.device_id.is_empty() {
        "—".to_owned()
    } else {
        match console.devices.get(&tag.info.device_id) {
            Some(link) if link.online => match link.latency_ms {
                Some(ms) => format!("{} · online, {ms} ms", tag.info.device_id),
                None => format!("{} · online", tag.info.device_id),
            },
            Some(_) => format!("{} · offline", tag.info.device_id),
            None => tag.info.device_id.clone(),
        }
    };
    let facts = [
        (
            "Points".to_owned(),
            stats.map_or_else(|| "0".to_owned(), |stats| stats.points.to_string()),
        ),
        (
            "Lowest".to_owned(),
            stats.map_or_else(|| "—".to_owned(), |stats| with_unit(stats.min)),
        ),
        (
            "Highest".to_owned(),
            stats.map_or_else(|| "—".to_owned(), |stats| with_unit(stats.max)),
        ),
        (
            "Average".to_owned(),
            stats.map_or_else(|| "—".to_owned(), |stats| with_unit(stats.mean)),
        ),
        ("Warning at".to_owned(), threshold(tag.info.warn_val)),
        ("Critical at".to_owned(), threshold(tag.info.crit_val)),
        (
            "Alarm delays".to_owned(),
            format!(
                "on {} s · off {} s",
                tag.info.on_delay_secs, tag.info.off_delay_secs
            ),
        ),
        (
            "Source".to_owned(),
            if tag.info.source_type.is_empty() {
                "—".to_owned()
            } else {
                tag.info.source_type.clone()
            },
        ),
        ("Device".to_owned(), device),
        (
            "Path".to_owned(),
            tag.info.path.clone().unwrap_or_else(|| "—".to_owned()),
        ),
    ];

    // The legend: each line's value at the crosshair, or its latest.
    let legend: Vec<AnyElement> = lines
        .iter()
        .enumerate()
        .filter_map(|(position, (line, samples))| {
            let tag = &console.store.tags[console.store.position(*line)?];
            let value = match cursor {
                Some(at) => tag_chart::nearest(samples, at).map(|(_, value)| value),
                None => samples.last().map(|(_, value)| *value),
            };
            let line = *line;
            Some(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_3)
                    .pl(SPACE_4)
                    .pr(if position == 0 { SPACE_4 } else { SPACE_2 })
                    .h(px(24.0))
                    .rounded(RADIUS_MD)
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().size(px(8.0)).rounded_full().bg(line_color(position)))
                    .child(
                        div()
                            .max_w(px(140.0))
                            .truncate()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(tag.info.name.clone()),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .font_weight(FontWeight::MEDIUM)
                            .child(value.map_or_else(|| "—".to_owned(), format_value)),
                    )
                    .when(position > 0, |this| {
                        this.child(
                            IconButton::new(
                                ElementId::NamedInteger("uncompare".into(), line.unsigned_abs()),
                                Icon::X,
                            )
                            .size(px(18.0))
                            .tooltip("Take off the chart")
                            .on_click(cx.listener(
                                move |console, _, _, cx| {
                                    toggle_overlay(console, line, cx);
                                },
                            )),
                        )
                    })
                    .into_any_element(),
            )
        })
        .collect();

    let marks: Vec<f64> = console
        .state::<DetailState>()
        .notes
        .as_ref()
        .filter(|(of, _)| *of == key)
        .map(|(_, notes)| {
            notes
                .iter()
                .map(|note| note.data_timestamp as f64 * 1000.0)
                .collect()
        })
        .unwrap_or_default();
    let tag = &console.store.tags[index];
    let mut limits = Vec::new();
    if tag.info.warn_val > 0.0 {
        limits.push((tag.info.warn_val, theme.warning));
    }
    if tag.info.crit_val > 0.0 {
        limits.push((tag.info.crit_val, theme.danger));
    }
    let spec = ChartSpec {
        lines: lines
            .iter()
            .enumerate()
            .map(|(position, (_, samples))| Line {
                color: line_color(position),
                samples: Rc::clone(samples),
            })
            .collect(),
        from_ms: from,
        to_ms: to,
        limits,
        marks,
        cursor,
        offset: console.utc_offset,
        plot: Rc::clone(&plot),
    };
    let span_caption = match query.span {
        Span::Last(span) => {
            let label = SPANS
                .iter()
                .find(|(id, _, _)| *id == span)
                .map_or(span, |(_, label, _)| *label);
            format!("Last {label}{}", if live { " · live" } else { "" })
        }
        #[allow(clippy::cast_possible_truncation)]
        Span::Between { start_ms, end_ms } => format!(
            "{} → {}",
            tag_api::format_local(start_ms, console.utc_offset),
            tag_api::format_local(end_ms, console.utc_offset)
        ),
    };
    let resolution = if loading {
        "Loading history…".to_owned()
    } else if let Some(error) = &history_error {
        format!("History: {error}")
    } else if interval > 0 {
        format!("Server bucket {}", super::span_label(interval as f64))
    } else {
        String::new()
    };

    let mut spans: Vec<Segment> = SPANS
        .iter()
        .map(|(span, _, _)| {
            let span = *span;
            Segment::new(
                span,
                query.span == Span::Last(span),
                cx.listener(move |console, _, _, cx| set_span(console, Span::Last(span), cx)),
            )
        })
        .collect();
    spans.push(Segment::new(
        "Custom…",
        !query.span.is_live(),
        cx.listener(|console, _, window, cx| open_dialog(console, TagDialog::Range, window, cx)),
    ));
    let details = [(Detail::Auto, "Auto"), (Detail::Full, "Full")]
        .into_iter()
        .map(|(detail, label)| {
            Segment::new(
                label,
                query.detail == detail,
                cx.listener(move |console, _, _, cx| set_detail(console, detail, cx)),
            )
        })
        .collect();
    let aggs = Aggregation::ALL
        .into_iter()
        .map(|agg| {
            Segment::new(
                agg.label(),
                query.agg == agg,
                cx.listener(move |console, _, _, cx| set_agg(console, agg, cx)),
            )
        })
        .collect();

    let chart = card(&theme)
        .id("tag-chart")
        .relative()
        .p(SPACE_4)
        .map(|this| {
            if expanded {
                this.flex_1().min_h(px(320.0))
            } else {
                this.h(px(220.0)).flex_none()
            }
        })
        .on_mouse_move(cx.listener(move |console, event: &MouseMoveEvent, _, cx| {
            let span = console.state::<DetailState>().query.span;
            let (from, to) = bounds_of(span, now_ms());
            let state = console.state::<DetailState>();
            let at = tag_chart::time_at(&state.plot, event.position.x, from, to);
            if at != state.cursor {
                state.cursor = at;
                cx.notify();
            }
        }))
        .on_hover(cx.listener(|console, hovered: &bool, _, cx| {
            if !*hovered {
                console.state::<DetailState>().cursor = None;
                cx.notify();
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|console, event: &MouseDownEvent, window, cx| {
                let span = console.state::<DetailState>().query.span;
                let (from, to) = bounds_of(span, now_ms());
                let plot = Rc::clone(&console.state::<DetailState>().plot);
                if let Some(at) = tag_chart::time_at(&plot, event.position.x, from, to) {
                    open_note(console, Some(at), window, cx);
                }
            }),
        )
        .child(if lines.iter().all(|(_, samples)| samples.is_empty()) {
            empty_state(
                Icon::ChartLine,
                if loading {
                    "Loading history"
                } else {
                    "No samples yet"
                },
                "Values appear here as the server records them.",
                Vec::new(),
                cx,
            )
            .into_any_element()
        } else {
            tag_chart::chart(spec, cx).into_any_element()
        });

    let tag = &console.store.tags[index];
    let trended = console.trend.contains(&id);
    let header = div()
        .flex()
        .items_start()
        .gap(SPACE_3)
        .px(SPACE_8)
        .pt(SPACE_8)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(SPACE_1)
                .child(
                    div()
                        .truncate()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tag.info.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(tag.info.key.clone()),
                ),
        )
        .child(
            IconButton::new("tag-compare", Icon::Layers)
                .tooltip("Compare with another tag")
                .on_click(cx.listener(|console, _, window, cx| {
                    open_dialog(console, TagDialog::Compare, window, cx);
                })),
        )
        .when(console.is_operator(), |this| {
            this.child(
                IconButton::new("tag-note", Icon::Pencil)
                    .tooltip(format!(
                        "Add a note ({})",
                        hint(&TagsAddNote).unwrap_or_default()
                    ))
                    .on_click(cx.listener(|console, _, window, cx| {
                        open_dialog(console, TagDialog::Note, window, cx);
                    })),
            )
        })
        .child(
            IconButton::new("tag-csv", Icon::Download)
                .tooltip(format!(
                    "Save the chart as CSV ({})",
                    hint(&TagsExportCsv).unwrap_or_default()
                ))
                .on_click(cx.listener(|console, _, _, cx| export_csv(console, cx))),
        )
        .child(
            IconButton::new(
                "tag-expand",
                if expanded {
                    Icon::PanelRight
                } else {
                    Icon::PanelLeft
                },
            )
            .tooltip(if expanded {
                "Show the table"
            } else {
                "Widen the detail"
            })
            .on_click(cx.listener(|console, _, _, cx| toggle_expanded(console, cx))),
        )
        .child(
            IconButton::new("close-detail", Icon::X)
                .tooltip("Close")
                .on_click(cx.listener(|console, _, _, cx| {
                    console.selected = None;
                    cx.notify();
                })),
        );

    let value = div()
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .px(SPACE_8)
        .py(SPACE_6)
        .child(value_with_unit(
            tag.display(),
            &tag.info.unit,
            px(34.0),
            value_color(tag.status, &theme),
            &theme,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(pill(tag.status.label(), color))
                .when(tag.simulated, |this| {
                    this.child(pill("SIMULATED", theme.info))
                })
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(updated),
                ),
        );

    let chart_section = div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .px(SPACE_8)
        .pb(SPACE_7)
        .when(expanded, |this| this.flex_1().min_h_0())
        .child(segmented("tag-span", spans, cx))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(SPACE_3)
                .child(segmented("tag-detail-level", details, cx))
                .child(segmented("tag-aggregation", aggs, cx)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(div().flex_1().min_w_0().truncate().child(span_caption))
                .child(div().font_family(MONO).child(resolution)),
        )
        .child(div().flex().flex_wrap().gap(SPACE_3).children(legend))
        .child(chart)
        .when(console.is_operator(), |this| {
            this.child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child("Right-click the chart to note a moment there"),
            )
        });

    let facts = div()
        .grid()
        .grid_cols(2)
        .gap(SPACE_5)
        .px(SPACE_8)
        .pb(SPACE_7)
        .children(facts.into_iter().map(|(label, value)| {
            div()
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .min_w_0()
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(label),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(value),
                )
        }));

    let notes = notes_section(console, &key, window, cx);
    let write = super::tag_write::section(console, id, cx);
    let trend_button = div().px(SPACE_8).pb(SPACE_8).child(
        Button::new(
            "toggle-trend",
            if trended {
                "Remove from Trends"
            } else {
                "Add to Trends"
            },
        )
        .icon(Icon::ChartLine)
        .on_click(cx.listener(move |console, _, _, cx| {
            console.toggle_trend(id, cx);
        })),
    );

    if expanded {
        // Widened: the chart takes the page's height on the left; the
        // facts, notes and write section scroll in a column on the right.
        return div()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(theme.chrome)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(header)
                    .child(value)
                    .child(chart_section),
            )
            .child(
                div()
                    .id("tag-detail-side")
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(px(PANEL))
                    .h_full()
                    .overflow_y_scroll()
                    .pt(SPACE_8)
                    .border_l_1()
                    .border_color(theme.border)
                    .child(facts)
                    .child(notes)
                    .child(write)
                    .child(trend_button),
            )
            .into_any_element();
    }
    div()
        .id("tag-detail")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(PANEL))
        .h_full()
        .overflow_y_scroll()
        .bg(theme.chrome)
        .border_l_1()
        .border_color(theme.border)
        .child(header)
        .child(value)
        .child(chart_section)
        .child(facts)
        .child(notes)
        .child(write)
        .child(trend_button)
        .into_any_element()
}

/// Enter on a note: show its moment on the chart, moving the span to it
/// when it lies outside.
fn show_note(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<DetailState>();
    let Some(at) = state
        .notes
        .as_ref()
        .and_then(|(_, notes)| notes.get(row))
        .map(|note| note.data_timestamp.saturating_mul(1000))
    else {
        return;
    };
    let (from, to) = bounds_of(state.query.span, now_ms());
    #[allow(clippy::cast_precision_loss)]
    let at_ms = at as f64;
    if !(from..=to).contains(&at_ms) {
        state.query.span = Span::Between {
            start_ms: at - 30 * 60_000,
            end_ms: at + 30 * 60_000,
        };
        state.query.detail = Detail::Auto;
    }
    state.cursor = Some(at_ms);
    ensure_history(console);
    cx.notify();
}

fn follow_note(console: &mut Console, row: usize, window: &mut Window, cx: &mut Context<Console>) {
    show_note(console, row, false, window, cx);
}

fn notes_section(
    console: &mut Console,
    key: &str,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let section = div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .px(SPACE_8)
        .py(SPACE_7)
        .border_t_1()
        .border_color(theme.border);
    if !console.is_operator() {
        return section
            .child(section_label("Notes", cx))
            .child(note_line(
                Icon::Lock,
                "Notes need the operator role (admin, demo or integration).".to_owned(),
                theme.fg_dim,
            ))
            .into_any_element();
    }
    let state = console.state::<DetailState>();
    let notes: Vec<Annotation> = state
        .notes
        .as_ref()
        .filter(|(of, _)| of == key)
        .map(|(_, notes)| notes.clone())
        .unwrap_or_default();
    let error = state.notes_error.clone();
    let offset = console.utc_offset;
    let list = keyboard_list(
        ListKeys {
            id: NOTES,
            on_move: Some(follow_note),
            on_activate: show_note,
        },
        notes.len(),
        notes.len(),
        console,
        cx,
    );
    let nav = console.list(NOTES, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    let rows: Vec<AnyElement> = notes
        .iter()
        .enumerate()
        .map(|(row, note)| {
            let id = note.id;
            let ring = focused && cursor == Some(row);
            let hover = theme.hover;
            div()
                .id(("tag-note-row", row))
                .relative()
                .flex()
                .items_start()
                .gap(SPACE_4)
                .px(SPACE_4)
                .py(SPACE_3)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |console, _, window, cx| {
                    console.list(NOTES, cx).select(row);
                    show_note(console, row, false, window, cx);
                }))
                .child(
                    div()
                        .pt(px(5.0))
                        .child(div().size(px(6.0)).rounded_full().bg(theme.info)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(SPACE_1)
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(local_time(note.data_timestamp, offset)),
                        )
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .child(note.annotation_text.clone()),
                        ),
                )
                .child(
                    IconButton::new(
                        ElementId::NamedInteger("delete-note".into(), id.unsigned_abs()),
                        Icon::Trash,
                    )
                    .size(px(20.0))
                    .tooltip("Delete the note")
                    .on_click(cx.listener(move |console, _, _, _| delete_note(console, id))),
                )
                .when(ring, |this| this.child(focus_ring(&theme)))
                .into_any_element()
        })
        .collect();
    section
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(section_label("Notes", cx))
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(notes.len().to_string()),
                )
                .child(div().flex_1())
                .child(
                    Button::new("add-note", "Add note")
                        .compact()
                        .icon(Icon::Plus)
                        .on_click(cx.listener(|console, _, window, cx| {
                            open_dialog(console, TagDialog::Note, window, cx);
                        })),
                ),
        )
        .when_some(error, |this, error| {
            this.child(note_line(Icon::CircleAlert, error, theme.danger))
        })
        .child(if rows.is_empty() {
            note_line(
                Icon::Info,
                "No notes on this tag yet.".to_owned(),
                theme.fg_dim,
            )
            .into_any_element()
        } else {
            list.flex()
                .flex_col()
                .gap(SPACE_1)
                .children(rows)
                .into_any_element()
        })
        .into_any_element()
}

pub(crate) fn note_line(glyph: Icon, text: String, color: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .gap(SPACE_3)
        .text_size(FONT_SMALL)
        .text_color(color)
        .child(div().pt(px(2.0)).child(vt_ui::icon(glyph, ICON_SM, color)))
        .child(div().flex_1().min_w_0().child(text))
}

// ── Dialogs ───────────────────────────────────────────────────────────────

/// A dialog's frame: title, body, then the buttons on the right.
pub(crate) fn dialog(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    body: Vec<AnyElement>,
    buttons: Vec<AnyElement>,
    cx: &Context<Console>,
) -> AnyElement {
    let theme = theme(cx);
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .px(SPACE_8)
                .pt(SPACE_8)
                .pb(SPACE_6)
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(subtitle.into()),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .children(body),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(SPACE_4)
                .px(SPACE_8)
                .py(SPACE_6)
                .border_t_1()
                .border_color(theme.border)
                .children(buttons),
        )
        .into_any_element()
}

fn close_button(cx: &mut Context<Console>) -> AnyElement {
    Button::new("dialog-cancel", "Cancel")
        .ghost()
        .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx)))
        .into_any_element()
}

fn range_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<DetailState>();
    let (Some(from), Some(to)) = (state.from.clone(), state.to.clone()) else {
        return div().into_any_element();
    };
    let error = state.range_error.clone();
    dialog(
        "Custom chart span",
        "On your clock, as YYYY-MM-DD HH:MM[:SS]. A past span does not run live.",
        vec![
            div()
                .flex()
                .gap(SPACE_5)
                .child(div().flex_1().child(field("From", from, cx)))
                .child(div().flex_1().child(field("To", to, cx)))
                .into_any_element(),
            match error {
                Some(error) => note_line(Icon::CircleAlert, error, theme.danger).into_any_element(),
                None => note_line(
                    Icon::Info,
                    "Enter applies; a preset span on the panel goes back to live.".to_owned(),
                    theme.fg_dim,
                )
                .into_any_element(),
            },
        ],
        vec![
            close_button(cx),
            Button::new("apply-range", "Show this span")
                .primary()
                .icon(Icon::Clock)
                .on_click(cx.listener(|console, _, window, cx| apply_range(console, window, cx)))
                .into_any_element(),
        ],
        cx,
    )
}

fn note_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let name = open_index(console)
        .map(|index| console.store.tags[index].info.name.clone())
        .unwrap_or_default();
    let state = console.state::<DetailState>();
    let (Some(time), Some(text)) = (state.note_time.clone(), state.note_text.clone()) else {
        return div().into_any_element();
    };
    let error = state.note_error.clone();
    dialog(
        format!("Note on {name}"),
        "Everyone with the operator role sees it on this tag's chart; @name mentions notify.",
        vec![
            field("At", time, cx).into_any_element(),
            field("Note", text, cx).into_any_element(),
            match error {
                Some(error) => note_line(Icon::CircleAlert, error, theme.danger).into_any_element(),
                None => note_line(Icon::Info, "Enter saves.".to_owned(), theme.fg_dim)
                    .into_any_element(),
            },
        ],
        vec![
            close_button(cx),
            Button::new("save-note", "Save note")
                .primary()
                .icon(Icon::Check)
                .on_click(cx.listener(|console, _, window, cx| save_note(console, window, cx)))
                .into_any_element(),
        ],
        cx,
    )
}

fn compare_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(pick) = console.state::<DetailState>().pick.clone() else {
        return div().into_any_element();
    };
    let rows = Rc::new(picks(console, cx));
    let count = rows.len();
    let chosen = console.state::<DetailState>().overlays.clone();
    let nav = console.list(PICKER, cx);
    nav.set_len(count);
    let scroll = nav.scroll.clone();
    let list =
        uniform_list(
            "tag-compare-rows",
            count,
            cx.processor(
                move |console: &mut Console, range: std::ops::Range<usize>, _, cx| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let cursor = console.list(PICKER, cx).cursor;
                    range
                        .filter_map(|row| Some((row, *rows.get(row)?)))
                        .map(|(row, index)| {
                            let tag = &console.store.tags[index];
                            let position = chosen.iter().position(|id| *id == tag.info.id);
                            let active = cursor == Some(row);
                            let hover = theme.hover;
                            div()
                                .id(("tag-compare-row", row))
                                .flex()
                                .items_center()
                                .gap(SPACE_4)
                                .h(px(PICK_ROW))
                                .px(SPACE_5)
                                .rounded(RADIUS_MD)
                                .cursor_pointer()
                                .when(active, |this| this.bg(theme.selected))
                                .when(!active, |this| this.hover(move |style| style.bg(hover)))
                                .on_click(cx.listener(move |console, _, _, cx| {
                                    console.list(PICKER, cx).select(row);
                                    pick_row(console, Some(row), cx);
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .size(px(14.0))
                                        .rounded(RADIUS_SM)
                                        .border_1()
                                        .map(|this| match position {
                                            Some(position) => {
                                                let color = theme.series(position + 1);
                                                this.bg(color).border_color(color).child(
                                                    vt_ui::icon(Icon::Check, px(10.0), theme.bg),
                                                )
                                            }
                                            None => this.border_color(theme.border_strong),
                                        }),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(tag.info.name.clone()),
                                )
                                .child(
                                    div()
                                        .max_w(px(140.0))
                                        .truncate()
                                        .font_family(MONO)
                                        .text_size(FONT_CAPTION)
                                        .text_color(theme.fg_dim)
                                        .child(tag.info.key.clone()),
                                )
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .px(SPACE_3)
        .h(px(PICK_ROW * 9.0));
    let chosen_count = console.state::<DetailState>().overlays.len();
    div()
        .key_context(MENU)
        .on_action(cx.listener(|console, _: &ListUp, _, cx| {
            console.list_move(PICKER, Move::Up, cx);
        }))
        .on_action(cx.listener(|console, _: &ListDown, _, cx| {
            console.list_move(PICKER, Move::Down, cx);
        }))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .p(SPACE_6)
                .border_b_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::Layers, ICON_LG, theme.fg_dim))
                .child(div().flex_1().child(pick)),
        )
        .child(div().py(SPACE_3).child(if count == 0 {
            div()
                .h(px(PICK_ROW * 2.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_dim)
                .child("No tag matches")
                .into_any_element()
        } else {
            list.into_any_element()
        }))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_6)
                .px(SPACE_6)
                .h(px(34.0))
                .border_t_1()
                .border_color(theme.border)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .child(kbd("↑↓", cx))
                        .child("Move"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .child(kbd("↵", cx))
                        .child("On or off the chart"),
                )
                .child(div().flex_1())
                .child(format!("{chosen_count} of {MAX_OVERLAYS} overlays")),
        )
        .into_any_element()
}

/// The write hint for the panel, e.g. `⇧⌘W`.
pub(crate) fn write_keys() -> String {
    hint(&TagsWrite).unwrap_or_else(|| display("secondary-shift-w"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::rc::Rc;

    use gpui::TestAppContext;
    use voltrus_scada_native::runtime::{CallKey, Event, Reply};
    use voltrus_scada_native::tag_api::{Annotation, Annotations};

    use super::{DetailState, delete_note, toggle_overlay, wanted_lines};
    use crate::console::Page;
    use crate::console::testing::Harness;

    type Series = HashMap<String, Vec<(f64, f64)>>;

    /// The argument of the one call named `name` among `calls`.
    fn arg(calls: &[CallKey], name: &str) -> i64 {
        let found: Vec<i64> = calls
            .iter()
            .filter(|key| key.name == name)
            .map(|key| key.arg)
            .collect();
        assert_eq!(found.len(), 1, "one {name} call in {calls:?}");
        found[0]
    }

    fn signed_in(cx: &mut TestAppContext) -> Harness {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.event(Event::Tags(vec![
            Harness::tag(7, "a.p"),
            Harness::tag(8, "b.p"),
            Harness::tag(9, "c.p"),
        ]));
        h.calls();
        h
    }

    fn series(keys: &[&str]) -> Series {
        keys.iter()
            .map(|key| ((*key).to_owned(), vec![(1000.0, 1.0)]))
            .collect()
    }

    /// The tag ids the detail keeps fetched history for.
    fn kept(h: &mut Harness) -> Vec<i64> {
        let mut ids: Vec<i64> = h.update(|console, _, _| {
            console
                .state::<DetailState>()
                .fetched
                .keys()
                .copied()
                .collect()
        });
        ids.sort_unstable();
        ids
    }

    fn note(id: i64, key: &str) -> Annotation {
        Annotation {
            id,
            tag_id: key.into(),
            annotation_text: format!("note {id}"),
            ..Annotation::default()
        }
    }

    #[gpui::test]
    fn a_late_notes_answer_for_another_tag_is_dropped(cx: &mut TestAppContext) {
        let mut h = signed_in(cx);
        h.update(|console, window, cx| console.open_tag(7, window, cx));
        let first = arg(&h.calls(), "tags.notes");
        h.update(|console, window, cx| console.open_tag(8, window, cx));
        let second = arg(&h.calls(), "tags.notes");
        // Tag a's answer arrives after tag b was opened.
        let late = Annotations {
            annotations: vec![note(1, "a.p")],
            count: 1,
        };
        h.reply(CallKey::new("tags.notes").with(first), Reply::ok(late));
        let shown = h.update(|console, _, _| console.state::<DetailState>().notes.clone());
        assert_eq!(shown, None, "tag b shows nothing of tag a's notes");
        let own = Annotations {
            annotations: vec![note(2, "b.p")],
            count: 1,
        };
        h.reply(CallKey::new("tags.notes").with(second), Reply::ok(own));
        let shown = h.update(|console, _, _| console.state::<DetailState>().notes.clone());
        assert_eq!(shown, Some(("b.p".to_owned(), vec![note(2, "b.p")])));
        // Delete acts on the open tag's notes only.
        h.update(|console, _, _| delete_note(console, 1));
        assert!(h.calls().is_empty());
        h.update(|console, _, _| delete_note(console, 2));
        assert_eq!(h.calls(), vec![CallKey::new("tags.note_delete").with(2)]);
    }

    /// The warm-up draws every page; drawing the detail panel must not ask
    /// the server for anything. Selecting a tag does.
    #[gpui::test]
    fn drawing_the_panel_asks_the_server_nothing(cx: &mut TestAppContext) {
        let mut h = signed_in(cx);
        h.draw();
        assert!(h.calls().is_empty());
        h.update(|console, window, cx| console.show(Page::Tags, window, cx));
        // An open tag the draw finds without a selection having asked.
        h.update(|console, _, _| console.selected = Some(7));
        h.calls();
        h.draw();
        h.draw();
        let calls = h.calls();
        assert!(
            !calls.iter().any(|key| key.name.starts_with("tags.")),
            "render asked {calls:?}"
        );
        h.update(|console, _, cx| console.select(8, cx));
        let mut names: Vec<&str> = h.calls().into_iter().map(|key| key.name).collect();
        names.sort_unstable();
        assert_eq!(names, ["tags.history", "tags.notes"]);
        h.draw();
        assert!(h.calls().is_empty());
    }

    #[gpui::test]
    fn the_history_answer_keeps_only_the_charted_tags(cx: &mut TestAppContext) {
        let mut h = signed_in(cx);
        h.update(|console, window, cx| console.open_tag(7, window, cx));
        let seq = arg(&h.calls(), "tags.history");
        h.reply(
            CallKey::new("tags.history").with(seq),
            Reply::ok((series(&["a.p", "b.p", "c.p"]), 1000_u64)),
        );
        assert_eq!(kept(&mut h), vec![7]);
        // An overlay the answer did not keep is asked for.
        h.update(|console, _, cx| toggle_overlay(console, 9, cx));
        let seq = arg(&h.calls(), "tags.history");
        h.reply(
            CallKey::new("tags.history").with(seq),
            Reply::ok((series(&["a.p", "b.p", "c.p"]), 1000_u64)),
        );
        assert_eq!(kept(&mut h), vec![7, 9]);
        // Taking it off again needs nothing new.
        h.update(|console, _, cx| toggle_overlay(console, 9, cx));
        assert!(h.calls().is_empty());
    }

    #[test]
    fn wanted_lines_drop_the_rest_and_share_a_key() {
        let lines = wanted_lines(
            series(&["a.p", "b.p", "c.p"]),
            &[(7, "a.p"), (8, "a.p"), (9, "x.p")],
        );
        let mut ids: Vec<i64> = lines.keys().copied().collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![7, 8]);
        assert!(Rc::ptr_eq(&lines[&7], &lines[&8]));
    }
}

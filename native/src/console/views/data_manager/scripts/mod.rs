//! Data Manager › Scripts: the JavaScript (QuickJS) scripts the flow
//! engine's Script nodes run, as the web client's `ScriptEditor.tsx` and
//! `ScriptExecutionHistory.tsx` edit them. A searchable, sortable list of
//! scripts beside the open script: its name, limits and state, its source
//! in a [`code_editor::CodeEditor`], and below it a test panel (a JSON
//! input run against the saved script, `POST /api/v1/scripts/{id}/test`)
//! and the script's runs (`GET …/executions`, polled every 10 s while the
//! section is on screen, as the web history does).
//!
//! Keys: in the list, j/k move and open, ↵ goes into the editor, N new,
//! E enable/disable, ⌫ delete, R refresh, S/⇧S sort, T test, H history;
//! anywhere in the section ⌘S saves and ⌘↵ runs the test; Esc in a field
//! comes back to the list. Edits to a script survive opening another one
//! (marked ● in the list) until saved.

pub(crate) mod code_editor;
pub(crate) mod js_syntax;
mod model;

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EntityInputHandler, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, SharedString, Subscription, Window, div, px, uniform_list,
};
use voltrus_scada_native::api::eng_scripts::{self as api, Script, ScriptExecution, TestResult};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use voltrus_scada_native::zone::LocalZone;
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, empty_state, kbd, pill, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use self::code_editor::CodeEditor;
use self::model::{Draft, Outcome, SortKey};
use crate::console::Console;
use crate::console::keys::{
    ENG_SCRIPTS, EngScriptsDelete, EngScriptsNew, EngScriptsRefresh, EngScriptsRun, EngScriptsSave,
    EngScriptsShowHistory, EngScriptsShowTest, EngScriptsSortNext, EngScriptsSortReverse,
    EngScriptsToggleEnabled,
};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::pages::Page;
use crate::console::text_area::TextArea;
use crate::console::views::{
    ListKeys, focus_ring, keyboard_list, local_time, ops_kit, page_header, rows_on_screen,
    tail_row, tail_rows,
};

const ID: &str = "scripts";
/// The scripts list.
const LIST: &str = "data.scripts";
/// The runs list.
const HISTORY: &str = "data.scripts.history";
const DELETE: &str = "eng-script-delete";
const ROW_HEIGHT: f32 = 34.0;
const HISTORY_ROW: f32 = 30.0;
const LIST_WIDTH: f32 = 300.0;
const PANEL_HEIGHT: f32 = 290.0;
const UPDATED: f32 = 76.0;
const STATE: f32 = 40.0;
const TIME: f32 = 104.0;
const STATUS: f32 = 76.0;
const DURATION: f32 = 70.0;
/// The web history polls every 10 s.
const POLL: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Panel {
    #[default]
    Test,
    History,
}

/// The script in the editor.
struct Editor {
    /// `None` for a script not created yet.
    id: Option<i64>,
    name: Entity<TextInput>,
    cpu: Entity<TextInput>,
    memory: Entity<TextInput>,
    source: Entity<CodeEditor>,
    enabled: bool,
    /// Whether it holds edits the server does not have, as of the last
    /// change.
    dirty: bool,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(crate) struct State {
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    scripts: Option<Result<Vec<Script>, String>>,
    asked: bool,
    /// The number of the last list request (later answers win).
    list_seq: i64,
    loading: bool,
    sort: SortKey,
    descending: bool,
    editor: Option<Editor>,
    /// Edits to scripts not saved, by id, while another one is open.
    unsaved: HashMap<i64, Draft>,
    /// A new script's form, while a saved one is open.
    new_draft: Option<Draft>,
    saving: bool,
    /// Why the last save, test or delete failed.
    error: Option<String>,
    test_input: Option<Entity<TextArea>>,
    _test_input: Option<Subscription>,
    testing: bool,
    /// The last test: the script, its answer, and whether the editor held
    /// unsaved edits (the server runs the saved source).
    test: Option<(i64, TestResult, bool)>,
    panel: Panel,
    /// The runs of a script, newest first.
    executions: Option<(i64, Result<Vec<ScriptExecution>, String>)>,
    /// The script the last runs request was for, and its number (later
    /// answers win).
    executions_for: Option<i64>,
    executions_seq: i64,
    /// The run whose output shows.
    open_execution: Option<i64>,
    confirm: Option<(i64, String)>,
    confirm_focus: Option<FocusHandle>,
    /// Put the list's cursor on this script once listed.
    focus_id: Option<i64>,
}

// ── Hooks ────────────────────────────────────────────────────────────────

pub fn on_open(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        let state = console.state::<State>();
        if state.scripts.is_none() {
            state.scripts = Some(Err("A stress test has no server to ask".into()));
        }
        return;
    }
    if !console.state::<State>().asked {
        fetch_list(console);
    }
    ops_kit::poll_while_shown(console, Page::DataManager, POLL, tick, cx);
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
    // Coming in with nothing open: the script under the cursor opens.
    let cursor = console.list(LIST, cx).cursor;
    if console.state::<State>().editor.is_none()
        && let Some(row) = cursor
    {
        moved(console, row, window, cx);
    }
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console.state_ref::<State>()?.search.clone()
}

/// The runs of the open script, every [`POLL`] while the section shows.
fn tick(console: &mut Console, _: &mut Context<Console>) {
    if super::current_id(console) != Some(ID) {
        return;
    }
    if let Some(id) = console
        .state::<State>()
        .editor
        .as_ref()
        .and_then(|editor| editor.id)
    {
        fetch_executions(console, id);
    }
}

// ── Fetching ─────────────────────────────────────────────────────────────

fn fetch_list(console: &mut Console) {
    let state = console.state::<State>();
    state.asked = true;
    state.loading = true;
    state.list_seq += 1;
    let seq = state.list_seq;
    console.call(
        CallKey::new("data.scripts.list").with(seq),
        Call::get::<Vec<Script>>(api::SCRIPTS),
    );
}

fn fetch_executions(console: &mut Console, id: i64) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.executions_seq += 1;
    state.executions_for = Some(id);
    let seq = state.executions_seq;
    console.call(
        CallKey::new("data.scripts.executions").with(seq),
        Call::new(move |api| async move {
            let list = api.script_executions(id, api::HISTORY_LIMIT).await?;
            anyhow::Ok((id, list))
        }),
    );
}

fn refresh(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        return;
    }
    fetch_list(console);
    if let Some(id) = editor_id(console) {
        fetch_executions(console, id);
    }
    cx.notify();
}

// ── Reading the state ────────────────────────────────────────────────────

fn scripts(console: &Console) -> &[Script] {
    match console
        .state_ref::<State>()
        .and_then(|state| state.scripts.as_ref())
    {
        Some(Ok(scripts)) => scripts.as_slice(),
        _ => &[],
    }
}

fn script(console: &Console, id: i64) -> Option<Script> {
    scripts(console)
        .iter()
        .find(|script| script.id == id)
        .cloned()
}

fn editor_id(console: &mut Console) -> Option<i64> {
    console
        .state::<State>()
        .editor
        .as_ref()
        .and_then(|editor| editor.id)
}

/// The list's rows: positions in the scripts, searched and sorted.
fn table(console: &mut Console, cx: &App) -> Vec<usize> {
    let query = console
        .state::<State>()
        .search
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    let state = console.state::<State>();
    let (sort, descending) = (state.sort, state.descending);
    model::rows(scripts(console), &query, sort, descending)
}

fn row_id(console: &mut Console, row: usize, cx: &App) -> Option<i64> {
    let rows = table(console, cx);
    let index = *rows.get(row)?;
    scripts(console).get(index).map(|script| script.id)
}

/// The editor's fields as typed.
fn draft_of(editor: &Editor, cx: &App) -> Draft {
    Draft {
        name: editor.name.read(cx).value().to_owned(),
        source: editor.source.read(cx).value().to_owned(),
        cpu_ms: editor.cpu.read(cx).value().to_owned(),
        memory_mb: editor.memory.read(cx).value().to_owned(),
        enabled: editor.enabled,
    }
}

/// Work out again whether the editor holds unsaved edits; redraw when
/// that changed.
fn recheck_dirty(console: &mut Console, cx: &mut Context<Console>) {
    let Some(editor) = console.state::<State>().editor.as_ref() else {
        return;
    };
    let draft = draft_of(editor, cx);
    let id = editor.id;
    let original = id.and_then(|id| script(console, id));
    let dirty = model::is_dirty(&draft, original.as_ref());
    if let Some(editor) = console.state::<State>().editor.as_mut()
        && editor.dirty != dirty
    {
        editor.dirty = dirty;
        cx.notify();
    }
}

// ── Opening ──────────────────────────────────────────────────────────────

/// Keep the open script's unsaved edits aside before another one opens.
fn stash(console: &mut Console, cx: &App) {
    let Some(editor) = console.state::<State>().editor.take() else {
        return;
    };
    let draft = draft_of(&editor, cx);
    match editor.id {
        Some(id) => {
            let original = script(console, id);
            let state = console.state::<State>();
            if original.is_some_and(|original| model::is_dirty(&draft, Some(&original))) {
                state.unsaved.insert(id, draft);
            } else {
                state.unsaved.remove(&id);
            }
        }
        None => {
            console.state::<State>().new_draft = model::is_dirty(&draft, None).then_some(draft);
        }
    }
}

/// Open script `id` (or a new script) in the editor.
fn open(console: &mut Console, id: Option<i64>, window: &mut Window, cx: &mut Context<Console>) {
    if console
        .state::<State>()
        .editor
        .as_ref()
        .is_some_and(|editor| editor.id == id && id.is_some())
    {
        return;
    }
    let original = match id {
        Some(id) => match script(console, id) {
            Some(script) => Some(script),
            None => return,
        },
        None => None,
    };
    stash(console, cx);
    let state = console.state::<State>();
    let draft = match (&original, id) {
        (Some(original), Some(id)) => state
            .unsaved
            .get(&id)
            .cloned()
            .unwrap_or_else(|| Draft::of(original)),
        _ => state.new_draft.take().unwrap_or_else(Draft::new_script),
    };
    let dirty = model::is_dirty(&draft, original.as_ref());

    let name = cx.new(|cx| {
        TextInput::new(cx)
            .placeholder("Script name")
            .text(draft.name.clone())
    });
    let cpu = cx.new(|cx| {
        TextInput::new(cx)
            .kind(InputKind::Unsigned)
            .mono()
            .placeholder("5000")
            .text(draft.cpu_ms.clone())
    });
    let memory = cx.new(|cx| {
        TextInput::new(cx)
            .kind(InputKind::Unsigned)
            .mono()
            .placeholder("32")
            .text(draft.memory_mb.clone())
    });
    let source = cx.new(|cx| {
        let mut editor = CodeEditor::new(cx).placeholder("function main(input) { return input; }");
        editor.set_text(&draft.source, cx);
        editor
    });
    let mut subscriptions: Vec<Subscription> = [&name, &cpu, &memory]
        .into_iter()
        .map(|field| ops_kit::on_field(field, on_field_event, window, cx))
        .collect();
    subscriptions.push(cx.subscribe_in(
        &source,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => recheck_dirty(console, cx),
            InputEvent::Submitted => run_test(console, cx),
            InputEvent::Cancelled => console.focus_list(LIST, window, cx),
            InputEvent::Blurred => {}
        },
    ));

    let state = console.state::<State>();
    state.editor = Some(Editor {
        id,
        name,
        cpu,
        memory,
        source,
        enabled: draft.enabled,
        dirty,
        _subscriptions: subscriptions,
    });
    state.error = None;
    if state.executions_for != id {
        state.open_execution = None;
    }
    if let Some(id) = id {
        let fetched = matches!(&state.executions, Some((for_id, _)) if *for_id == id);
        if !fetched || state.executions_for != Some(id) {
            fetch_executions(console, id);
        }
    }
    cx.notify();
}

fn on_field_event(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Changed => recheck_dirty(console, cx),
        InputEvent::Submitted => save(console, cx),
        InputEvent::Cancelled => console.focus_list(LIST, window, cx),
        InputEvent::Blurred => {}
    }
}

/// N, the button: a new script, its name field focused.
fn new_script(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    open(console, None, window, cx);
    if let Some(editor) = console.state::<State>().editor.as_ref() {
        editor.name.read(cx).focus(window);
    }
}

/// Moving in the list opens the script under the cursor.
fn moved(console: &mut Console, row: usize, window: &mut Window, cx: &mut Context<Console>) {
    if let Some(id) = row_id(console, row, cx) {
        open(console, Some(id), window, cx);
    }
}

/// ↵ in the list: into the open script's source.
fn activate(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    moved(console, row, window, cx);
    if let Some(editor) = console.state::<State>().editor.as_ref() {
        editor.source.read(cx).focus(window);
    }
}

fn ensure_search(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().search.is_some() {
        return;
    }
    let field = cx.new(|cx| TextInput::new(cx).placeholder("Search scripts  /"));
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
                let cursor = nav.cursor;
                if let Some(row) = cursor {
                    moved(console, row, window, cx);
                }
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.search = Some(field);
    state._search = Some(subscription);
}

fn ensure_test_input(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().test_input.is_some() {
        return;
    }
    let area = cx.new(|cx| TextArea::new(6, cx).placeholder("{\"value\": 42}   ⌘↵ run"));
    area.update(cx, |area, cx| {
        area.replace_text_in_range(None, model::SAMPLE_INPUT, window, cx);
    });
    let subscription = cx.subscribe_in(
        &area,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => run_test(console, cx),
            InputEvent::Cancelled => console.focus_list(LIST, window, cx),
            InputEvent::Changed | InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.test_input = Some(area);
    state._test_input = Some(subscription);
}

// ── Actions ──────────────────────────────────────────────────────────────

/// ⌘S, Enter in a field, the button: create or save the open script.
fn save(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.saving {
        return;
    }
    let Some(editor) = state.editor.as_ref() else {
        return;
    };
    let id = editor.id;
    let draft = draft_of(editor, cx);
    match id {
        None => match model::create_body(&draft) {
            Err(error) => console.state::<State>().error = Some(error),
            Ok(body) => {
                let state = console.state::<State>();
                state.saving = true;
                state.error = None;
                console.call(
                    CallKey::new("data.scripts.create"),
                    Call::new(move |api| async move { api.create_script(&body).await }),
                );
            }
        },
        Some(id) => {
            let Some(original) = script(console, id) else {
                return;
            };
            match model::update_body(&draft, &original) {
                Err(error) => console.state::<State>().error = Some(error),
                Ok(None) => console.show_toast(
                    format!("{} has no changes to save", original.name),
                    false,
                    cx,
                ),
                Ok(Some(body)) => {
                    let state = console.state::<State>();
                    state.saving = true;
                    state.error = None;
                    console.call(
                        CallKey::new("data.scripts.save").with(id),
                        Call::new(move |api| async move { api.update_script(id, &body).await }),
                    );
                }
            }
        }
    }
    cx.notify();
}

/// ⌘↵, the button: run the saved script with the test input.
fn run_test(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.testing {
        return;
    }
    let Some(editor) = state.editor.as_ref() else {
        return;
    };
    let Some(id) = editor.id else {
        state.error = Some("Create the script first: a test runs what the server has saved".into());
        cx.notify();
        return;
    };
    let dirty = editor.dirty;
    let text = state.test_input.as_ref().map_or_else(
        || model::SAMPLE_INPUT.to_owned(),
        |area| area.read(cx).value().to_owned(),
    );
    let input = match model::parse_input(&text) {
        Ok(input) => input,
        Err(error) => {
            state.error = Some(error);
            state.panel = Panel::Test;
            cx.notify();
            return;
        }
    };
    state.testing = true;
    state.error = None;
    state.panel = Panel::Test;
    console.call(
        CallKey::new("data.scripts.test").with(id),
        Call::new(move |api| async move {
            let result = api.test_script(id, input).await?;
            anyhow::Ok((result, dirty))
        }),
    );
    cx.notify();
}

/// E, the switch: enable or disable the open script (saved with ⌘S, as
/// the web editor's toggle).
fn toggle_enabled(console: &mut Console, cx: &mut Context<Console>) {
    if let Some(editor) = console.state::<State>().editor.as_mut()
        && editor.id.is_some()
    {
        editor.enabled = !editor.enabled;
    }
    recheck_dirty(console, cx);
    cx.notify();
}

fn set_enabled(console: &mut Console, enabled: bool, cx: &mut Context<Console>) {
    let current = console
        .state::<State>()
        .editor
        .as_ref()
        .map(|editor| editor.enabled);
    if current.is_some_and(|current| current != enabled) {
        toggle_enabled(console, cx);
    }
}

fn ask_delete(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(id) = editor_id(console) else {
        return;
    };
    let Some(original) = script(console, id) else {
        return;
    };
    let focus = cx.focus_handle();
    let state = console.state::<State>();
    state.confirm = Some((id, original.name));
    state.confirm_focus = Some(focus.clone());
    let overlay = Overlay::modal(DELETE, px(420.0), confirm_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn delete_confirmed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some((id, name)) = console.state::<State>().confirm.take() else {
        return;
    };
    console.close_overlay(window, cx);
    console.call(
        CallKey::new("data.scripts.delete").with(id),
        Call::new(move |api| async move {
            api.delete_script(id).await?;
            anyhow::Ok(name)
        }),
    );
}

fn set_sort(console: &mut Console, key: SortKey, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.sort == key {
        state.descending = !state.descending;
    } else {
        state.sort = key;
        state.descending = false;
    }
    if let Some(id) = editor_id(console) {
        console.state::<State>().focus_id = Some(id);
    }
    cx.notify();
}

fn set_panel(console: &mut Console, panel: Panel, cx: &mut Context<Console>) {
    console.state::<State>().panel = panel;
    cx.notify();
}

/// A run's row picked: its output shows (again: hidden).
fn pick_execution(console: &mut Console, row: usize, toggle: bool, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let id = match &state.executions {
        Some((_, Ok(list))) => list.get(row).map(|execution| execution.id),
        _ => None,
    };
    state.open_execution = if toggle && state.open_execution == id {
        None
    } else {
        id
    };
    cx.notify();
}

fn history_moved(console: &mut Console, row: usize, _: &mut Window, cx: &mut Context<Console>) {
    pick_execution(console, row, false, cx);
}

fn history_activate(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    pick_execution(console, row, true, cx);
}

// ── Answers ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.scripts.list" => {
            let state = console.state::<State>();
            if key.arg != state.list_seq {
                return;
            }
            let answer = reply.take::<Vec<Script>>();
            state.loading = false;
            match answer {
                Ok(list) => {
                    state
                        .unsaved
                        .retain(|id, _| list.iter().any(|script| script.id == *id));
                    let gone = state
                        .editor
                        .as_ref()
                        .and_then(|editor| editor.id)
                        .is_some_and(|id| list.iter().all(|script| script.id != id));
                    if gone {
                        state.editor = None;
                    }
                    state.scripts = Some(Ok(list));
                    recheck_dirty(console, cx);
                }
                Err(error) => {
                    // Keep a list already shown; say why it did not refresh.
                    if matches!(state.scripts, Some(Ok(_))) {
                        console.show_toast(format!("Scripts: {error}"), true, cx);
                    } else {
                        state.scripts = Some(Err(error));
                    }
                }
            }
        }
        "data.scripts.create" => {
            let state = console.state::<State>();
            // Not asked by this session's state (a sign-out came between).
            if !state.saving {
                return;
            }
            let answer = reply.take::<Script>();
            state.saving = false;
            match answer {
                Ok(created) => {
                    let (id, name) = (created.id, created.name.clone());
                    let open = state
                        .editor
                        .as_mut()
                        .filter(|editor| editor.id.is_none())
                        .map(|editor| {
                            editor.id = Some(id);
                            editor.name.clone()
                        });
                    if let Some(Ok(list)) = state.scripts.as_mut() {
                        let at = list.partition_point(|script| {
                            model::by_server_order(script, &created).is_le()
                        });
                        list.insert(at, created);
                    }
                    state.new_draft = None;
                    // The server keeps the name trimmed.
                    if let Some(field) = open
                        && field.read(cx).value() != name
                    {
                        field.update(cx, |field, cx| field.set_text(name.clone(), cx));
                    }
                    state.focus_id = Some(id);
                    recheck_dirty(console, cx);
                    // Only for the open script (another may have opened since).
                    if editor_id(console) == Some(id) {
                        fetch_executions(console, id);
                    }
                    console.show_toast(format!("Created {name}"), false, cx);
                }
                Err(error) => state.error = Some(error),
            }
        }
        "data.scripts.save" => {
            let state = console.state::<State>();
            if !state.saving {
                return;
            }
            let answer = reply.take::<Script>();
            state.saving = false;
            match answer {
                Ok(saved) => {
                    let name = saved.name.clone();
                    state.unsaved.remove(&saved.id);
                    let open = state
                        .editor
                        .as_ref()
                        .filter(|editor| editor.id == Some(saved.id))
                        .map(|editor| editor.name.clone());
                    if let Some(Ok(list)) = state.scripts.as_mut()
                        && let Some(slot) = list.iter_mut().find(|script| script.id == saved.id)
                    {
                        *slot = saved.clone();
                    }
                    // The server keeps the name trimmed.
                    if let Some(field) = open
                        && field.read(cx).value() != saved.name
                    {
                        field.update(cx, |field, cx| field.set_text(saved.name.clone(), cx));
                    }
                    recheck_dirty(console, cx);
                    console.show_toast(format!("Saved {name}"), false, cx);
                }
                Err(error) => state.error = Some(error),
            }
        }
        "data.scripts.delete" => {
            let answer = reply.take::<String>();
            match answer {
                Ok(name) => {
                    let id = key.arg;
                    let state = console.state::<State>();
                    if let Some(Ok(list)) = state.scripts.as_mut() {
                        list.retain(|script| script.id != id);
                    }
                    state.unsaved.remove(&id);
                    if state
                        .editor
                        .as_ref()
                        .is_some_and(|editor| editor.id == Some(id))
                    {
                        state.editor = None;
                    }
                    if state
                        .test
                        .as_ref()
                        .is_some_and(|(test_id, _, _)| *test_id == id)
                    {
                        state.test = None;
                    }
                    if state.executions_for == Some(id) {
                        state.executions = None;
                        state.executions_for = None;
                    }
                    console.show_toast(format!("Deleted {name}"), false, cx);
                }
                Err(error) => {
                    console.state::<State>().error = Some(error.clone());
                    console.show_toast(format!("Delete failed: {error}"), true, cx);
                }
            }
        }
        "data.scripts.test" => {
            let state = console.state::<State>();
            if !state.testing {
                return;
            }
            let answer = reply.take::<(TestResult, bool)>();
            state.testing = false;
            match answer {
                Ok((result, dirty)) => {
                    state.test = Some((key.arg, result, dirty));
                    if editor_id(console) == Some(key.arg) {
                        fetch_executions(console, key.arg);
                    }
                }
                Err(error) => state.error = Some(format!("Test failed: {error}")),
            }
        }
        "data.scripts.executions" => {
            let state = console.state::<State>();
            if key.arg != state.executions_seq {
                return;
            }
            match reply.take::<(i64, Vec<ScriptExecution>)>() {
                Ok((id, list)) => {
                    let previous = match &state.executions {
                        Some((for_id, Ok(previous))) if *for_id == id => Some(previous.as_slice()),
                        _ => None,
                    };
                    let unchanged = previous.is_some_and(|previous| previous == list.as_slice());
                    if unchanged {
                        // A poll that brought nothing new draws nothing.
                        return;
                    }
                    if let Some(newest) =
                        previous.and_then(|previous| model::newest_failure(previous, &list))
                    {
                        state.open_execution = Some(newest);
                        console.list(HISTORY, cx).select(0);
                    }
                    console.state::<State>().executions = Some((id, Ok(list)));
                }
                Err(error) => {
                    if let Some(id) = state.executions_for {
                        state.executions = Some((id, Err(error)));
                    }
                }
            }
        }
        _ => {}
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn outcome_color(status: &str, theme: &Theme) -> Hsla {
    match model::outcome(status) {
        Outcome::Success => theme.success,
        Outcome::Timeout => theme.warning,
        Outcome::Failed => theme.danger,
    }
}

/// A block of monospace text that scrolls (an output, an error).
fn mono_block(id: impl Into<SharedString>, text: String, color: Hsla, theme: &Theme) -> AnyElement {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .p(SPACE_4)
        .rounded(RADIUS_MD)
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .font_family(MONO)
        .text_size(FONT_SMALL)
        .text_color(color)
        .child(text)
        .into_any_element()
}

fn caption(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(text.into())
}

fn banner(text: String, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(SPACE_3)
        .mx(SPACE_9)
        .mt(SPACE_4)
        .px(SPACE_5)
        .py(SPACE_3)
        .rounded(RADIUS_MD)
        .bg(theme.danger.alpha(0.1))
        .border_1()
        .border_color(theme.danger.alpha(0.3))
        .text_size(FONT_SMALL)
        .text_color(theme.danger)
        .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
        .child(div().flex_1().min_w_0().font_family(MONO).child(text))
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    if !console.is_operator() {
        return empty_state(
            Icon::Lock,
            "Scripts need the operator role",
            "The server lets administrators, demo and integration users edit and run scripts.",
            Vec::new(),
            cx,
        )
        .into_any_element();
    }
    ensure_search(console, window, cx);
    ensure_test_input(console, window, cx);
    let theme = theme(cx).clone();
    let rows = table(console, cx);
    let shown = rows.len();
    let total = scripts(console).len();

    // A new or re-sorted list: the cursor follows the open script.
    let focus_id = console.state::<State>().focus_id;
    if let Some(id) = focus_id
        && let Some(row) = rows.iter().position(|index| {
            scripts(console)
                .get(*index)
                .is_some_and(|script| script.id == id)
        })
    {
        console.state::<State>().focus_id = None;
        console.list(LIST, cx).select(row);
    }

    let state = console.state::<State>();
    let (sort, descending, loading) = (state.sort, state.descending, state.loading);
    let search_field = state.search.clone();
    let filtered = search_field
        .as_ref()
        .is_some_and(|field| !field.read(cx).value().trim().is_empty());
    let list_error = match &state.scripts {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let loaded = state.scripts.is_some();
    let open_id = state.editor.as_ref().and_then(|editor| editor.id);
    let open_dirty = state.editor.as_ref().is_some_and(|editor| editor.dirty);
    let unsaved: Vec<i64> = state.unsaved.keys().copied().collect();

    let arrow = if descending { "↓" } else { "↑" };
    let header_cell =
        |label: &'static str, key: SortKey, width: Option<f32>, cx: &mut Context<Console>| {
            let active = key == sort;
            div()
                .id(SharedString::from(format!(
                    "eng-scripts-sort-{}",
                    key.label()
                )))
                .flex()
                .items_center()
                .gap(SPACE_1)
                .map(|this| match width {
                    Some(width) => this.w(px(width)).flex_none().justify_end(),
                    None => this.flex_1().min_w_0(),
                })
                .cursor_pointer()
                .text_color(if active { theme.fg } else { theme.fg_dim })
                .hover(|style| style.text_color(theme.fg))
                .on_click(cx.listener(move |console, _, _, cx| set_sort(console, key, cx)))
                .child(label)
                .when(active, |this| this.child(arrow))
        };
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(30.0))
        .px(SPACE_6)
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .child(header_cell("NAME", SortKey::Name, None, cx))
        .child(header_cell("UPDATED", SortKey::Updated, Some(UPDATED), cx))
        .child(header_cell("ON", SortKey::State, Some(STATE), cx));

    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: Some(moved),
            on_activate: activate,
        },
        shown,
        rows_on_screen(window, ROW_HEIGHT),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let zone = console.zone.clone();

    let body = if let Some(error) = list_error {
        ops_kit::notice(format!("Scripts: {error}"), true, &theme)
    } else if !loaded || (loading && total == 0) {
        ops_kit::notice("Loading scripts…", false, &theme)
    } else if shown == 0 {
        div()
            .p(SPACE_7)
            .text_size(FONT_SMALL)
            .text_color(theme.fg_muted)
            .child(if total == 0 {
                "No scripts yet. N makes one."
            } else {
                "No script has this text in its name."
            })
            .into_any_element()
    } else {
        let rows: std::rc::Rc<[usize]> = rows.into();
        uniform_list(
            "eng-scripts",
            shown + tail_rows(shown, ROW_HEIGHT),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    range
                        .map(|row| match rows.get(row).copied() {
                            Some(index) => script_row(
                                console,
                                index,
                                row,
                                RowLook {
                                    cursor,
                                    focused,
                                    open_id,
                                    open_dirty,
                                    zone: &zone,
                                },
                                &unsaved,
                                &theme,
                                cx,
                            ),
                            None => tail_row(ROW_HEIGHT),
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .size_full()
        .into_any_element()
    };

    let subtitle = if filtered {
        format!("{shown} of {total} scripts match")
    } else {
        format!(
            "{total} scripts · sorted by {}{arrow} · N new · ↵ edit · E on/off · ⌫ delete · ⌘S save · ⌘↵ test · R refresh",
            sort.label()
        )
    };
    let controls = vec![
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .w(px(220.0))
            .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
            .children(search_field.map(|field| div().flex_1().child(field)))
            .into_any_element(),
        Button::new("eng-scripts-refresh", "Refresh")
            .ghost()
            .icon(Icon::RefreshCw)
            .tooltip("R")
            .on_click(cx.listener(|console, _, _, cx| refresh(console, cx)))
            .into_any_element(),
        Button::new("eng-scripts-new", "New script")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| new_script(console, window, cx)))
            .into_any_element(),
    ];

    let detail = detail(console, window, cx);

    div()
        .key_context(ENG_SCRIPTS)
        .flex()
        .flex_col()
        .size_full()
        .on_action(cx.listener(|console, _: &EngScriptsNew, window, cx| {
            new_script(console, window, cx);
        }))
        .on_action(cx.listener(|console, _: &EngScriptsDelete, window, cx| {
            ask_delete(console, window, cx);
        }))
        .on_action(cx.listener(|console, _: &EngScriptsToggleEnabled, _, cx| {
            toggle_enabled(console, cx);
        }))
        .on_action(cx.listener(|console, _: &EngScriptsRefresh, _, cx| refresh(console, cx)))
        .on_action(cx.listener(|console, _: &EngScriptsSortNext, _, cx| {
            let next = console.state::<State>().sort.next();
            set_sort(console, next, cx);
        }))
        .on_action(cx.listener(|console, _: &EngScriptsSortReverse, _, cx| {
            let sort = console.state::<State>().sort;
            set_sort(console, sort, cx);
        }))
        .on_action(cx.listener(|console, _: &EngScriptsShowTest, _, cx| {
            set_panel(console, Panel::Test, cx);
        }))
        .on_action(cx.listener(|console, _: &EngScriptsShowHistory, _, cx| {
            set_panel(console, Panel::History, cx);
        }))
        .on_action(cx.listener(|console, _: &EngScriptsSave, _, cx| save(console, cx)))
        .on_action(cx.listener(|console, _: &EngScriptsRun, _, cx| run_test(console, cx)))
        .child(page_header("Scripts", subtitle, controls, cx))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .w(px(LIST_WIDTH))
                        .h_full()
                        .border_r_1()
                        .border_color(theme.border)
                        .child(header)
                        .child(list.flex_1().min_h_0().child(body)),
                )
                .child(detail),
        )
        .into_any_element()
}

/// What every list row of one render shares.
#[derive(Clone, Copy)]
struct RowLook<'a> {
    cursor: Option<usize>,
    focused: bool,
    open_id: Option<i64>,
    open_dirty: bool,
    zone: &'a LocalZone,
}

#[allow(clippy::too_many_arguments)]
fn script_row(
    console: &Console,
    index: usize,
    row: usize,
    look: RowLook<'_>,
    unsaved: &[i64],
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let Some(script) = scripts(console).get(index) else {
        return tail_row(ROW_HEIGHT);
    };
    let id = script.id;
    let is_open = look.open_id == Some(id);
    let edited = if is_open {
        look.open_dirty
    } else {
        unsaved.contains(&id)
    };
    let hover = theme.hover;
    let dot = if script.syntax_error.is_some() {
        theme.danger
    } else if script.enabled {
        theme.success
    } else {
        theme.fg_dim
    };
    div()
        .id(gpui::ElementId::NamedInteger(
            "eng-script".into(),
            u64::try_from(id).unwrap_or(0),
        ))
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .h(px(ROW_HEIGHT))
        .px(SPACE_6)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(is_open, |this| this.bg(theme.selected))
        .when(!is_open, |this| this.hover(move |style| style.bg(hover)))
        .on_click(cx.listener(move |console, _, window, cx| {
            console.list(LIST, cx).select(row);
            open(console, Some(id), window, cx);
            let focus = console.list(LIST, cx).focus.clone();
            window.focus(&focus);
            cx.notify();
        }))
        .when(look.focused && look.cursor == Some(row), |this| {
            this.child(focus_ring(theme))
        })
        .child(div().flex_none().size(px(7.0)).rounded_full().bg(dot))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .flex_1()
                .min_w_0()
                .child(div().min_w_0().truncate().child(script.name.clone()))
                .when(edited, |this| {
                    this.child(div().flex_none().text_color(theme.warning).child("●"))
                })
                .when(script.syntax_error.is_some(), |this| {
                    this.child(pill("ERROR", theme.danger))
                }),
        )
        .child(
            div()
                .w(px(UPDATED))
                .flex_none()
                .truncate()
                .text_right()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(local_time(script.updated_at, look.zone)),
        )
        .child(
            div()
                .w(px(STATE))
                .flex_none()
                .text_right()
                .text_size(FONT_SMALL)
                .text_color(if script.enabled {
                    theme.success
                } else {
                    theme.fg_dim
                })
                .child(if script.enabled { "On" } else { "Off" }),
        )
        .into_any_element()
}

/// The open script: its fields, its source, and the test and history
/// panel.
#[allow(clippy::too_many_lines)]
fn detail(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let Some(editor) = state.editor.as_ref() else {
        return div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(empty_state(
                Icon::Terminal,
                "No script open",
                "Pick a script in the list (↵ edits its source), or make one with N.",
                vec![
                    Button::new("eng-scripts-first", "New script")
                        .primary()
                        .icon(Icon::Plus)
                        .on_click(
                            cx.listener(|console, _, window, cx| new_script(console, window, cx)),
                        )
                        .into_any_element(),
                ],
                cx,
            ))
            .into_any_element();
    };
    let id = editor.id;
    let (name, cpu, memory, source) = (
        editor.name.clone(),
        editor.cpu.clone(),
        editor.memory.clone(),
        editor.source.clone(),
    );
    let (enabled, dirty) = (editor.enabled, editor.dirty);
    let (saving, error) = (state.saving, state.error.clone());
    let original = id.and_then(|id| script(console, id));
    let lines = source.read(cx).line_count();

    let toolbar = ops_kit::filter_bar(&theme)
        .child(ops_kit::labelled(
            "Name",
            div().w(px(220.0)).child(name),
            &theme,
        ))
        .child(ops_kit::labelled(
            "CPU limit (ms)",
            div().w(px(96.0)).child(cpu),
            &theme,
        ))
        .child(ops_kit::labelled(
            "Memory (MB)",
            div().w(px(76.0)).child(memory),
            &theme,
        ))
        .when(id.is_some(), |this| {
            this.child(ops_kit::labelled(
                "State",
                segmented(
                    "eng-script-enabled",
                    vec![
                        Segment::new(
                            "Enabled",
                            enabled,
                            cx.listener(|console, _, _, cx| set_enabled(console, true, cx)),
                        )
                        .icon(Icon::CircleCheck),
                        Segment::new(
                            "Disabled",
                            !enabled,
                            cx.listener(|console, _, _, cx| set_enabled(console, false, cx)),
                        )
                        .icon(Icon::CircleX),
                    ],
                    cx,
                ),
                &theme,
            ))
        })
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .when(dirty && id.is_some(), |this| {
                    this.child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.warning)
                            .child("● unsaved"),
                    )
                })
                .child(
                    Button::new(
                        "eng-script-save",
                        match (id, saving) {
                            (None, true) => "Creating…",
                            (None, false) => "Create script",
                            (Some(_), true) => "Saving…",
                            (Some(_), false) => "Save",
                        },
                    )
                    .primary()
                    .icon(Icon::Check)
                    .tooltip("⌘S")
                    .disabled(saving || (id.is_some() && !dirty))
                    .on_click(cx.listener(|console, _, _, cx| save(console, cx))),
                )
                .when(id.is_some(), |this| {
                    this.child(
                        Button::new("eng-script-delete", "Delete")
                            .danger()
                            .icon(Icon::Trash)
                            .tooltip("⌫ in the list")
                            .on_click(cx.listener(|console, _, window, cx| {
                                ask_delete(console, window, cx);
                            })),
                    )
                }),
        );

    let syntax_error = original
        .as_ref()
        .and_then(|script| script.syntax_error.clone());

    let source_head = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .px(SPACE_9)
        .pt(SPACE_5)
        .pb(SPACE_3)
        .child(caption("SOURCE · JAVASCRIPT", &theme))
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(format!("{lines} lines")),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(kbd("Tab", cx))
                .child("indent")
                .child(kbd("⌘/", cx))
                .child("comment")
                .child(kbd("⌘Z", cx))
                .child("undo")
                .child(kbd("Esc", cx))
                .child("list"),
        );

    let panel = panel(console, id, window, cx);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(toolbar)
        .when_some(error, |this, error| this.child(banner(error, &theme)))
        .when_some(syntax_error, |this, error| {
            this.child(banner(format!("Saved with a syntax error, so it does not run:\n{error}"), &theme))
        })
        .when(id.is_none(), |this| {
            this.child(
                div()
                    .px(SPACE_9)
                    .pt(SPACE_4)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child("A new script: the server checks its syntax and creates it enabled. ⌘S or Enter in a field creates it."),
            )
        })
        .child(source_head)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(160.0))
                .px(SPACE_9)
                .pb(SPACE_5)
                .child(source),
        )
        .child(panel)
        .into_any_element()
}

/// The test panel or the runs, below the source.
#[allow(clippy::too_many_lines)]
fn panel(
    console: &mut Console,
    id: Option<i64>,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let frame = div()
        .flex()
        .flex_col()
        .flex_none()
        .h(px(PANEL_HEIGHT))
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.chrome);
    let Some(id) = id else {
        return frame
            .child(ops_kit::notice(
                "Create the script to test it and see its runs.",
                false,
                &theme,
            ))
            .into_any_element();
    };
    let state = console.state::<State>();
    let current = state.panel;
    let testing = state.testing;
    let runs = match &state.executions {
        Some((for_id, Ok(list))) if *for_id == id => Some(list.len()),
        _ => None,
    };
    let tabs = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .px(SPACE_9)
        .py(SPACE_4)
        .child(segmented(
            "eng-script-panel",
            vec![
                Segment::new(
                    "Test",
                    current == Panel::Test,
                    cx.listener(|console, _, _, cx| set_panel(console, Panel::Test, cx)),
                )
                .icon(Icon::Play),
                Segment::new(
                    match runs {
                        Some(count) => format!("History ({count})"),
                        None => "History".to_owned(),
                    },
                    current == Panel::History,
                    cx.listener(|console, _, _, cx| set_panel(console, Panel::History, cx)),
                )
                .icon(Icon::History),
            ],
            cx,
        ))
        .child(
            div()
                .flex_1()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child("T test · H history · runs refresh every 10 s"),
        )
        .child(
            Button::new(
                "eng-script-run",
                if testing { "Running…" } else { "Run test" },
            )
            .icon(Icon::Play)
            .tooltip("⌘↵ runs the saved script with this input")
            .disabled(testing)
            .on_click(cx.listener(|console, _, _, cx| run_test(console, cx))),
        );

    let content = match current {
        Panel::Test => test_panel(console, id, &theme),
        Panel::History => history_panel(console, id, window, cx),
    };
    frame
        .child(tabs)
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .gap(SPACE_6)
                .px(SPACE_9)
                .pb(SPACE_6)
                .child(content),
        )
        .into_any_element()
}

fn test_panel(console: &mut Console, id: i64, theme: &Theme) -> AnyElement {
    let state = console.state::<State>();
    let input = state.test_input.clone();
    let testing = state.testing;
    let test = state
        .test
        .as_ref()
        .filter(|(test_id, _, _)| *test_id == id)
        .map(|(_, result, dirty)| (result.clone(), *dirty));

    let result: AnyElement = match test {
        _ if testing => ops_kit::notice("Running the saved script…", false, theme),
        None => ops_kit::notice(
            "⌘↵ runs the saved script with this input; its main(input) returns the output.",
            false,
            theme,
        ),
        Some((result, dirty)) => {
            let color = if result.success {
                theme.success
            } else {
                theme.danger
            };
            let headline = format!(
                "{}{}",
                if result.success { "Success" } else { "Error" },
                result
                    .duration_ms
                    .map(|ms| format!(
                        " · {}",
                        model::duration_label(i64::try_from(ms).unwrap_or(i64::MAX))
                    ))
                    .unwrap_or_default()
            );
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .size_full()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(vt_ui::icon(
                            if result.success {
                                Icon::CircleCheck
                            } else {
                                Icon::CircleX
                            },
                            ICON_MD,
                            color,
                        ))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color)
                                .child(headline),
                        ),
                )
                .when(dirty, |this| {
                    this.child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.warning)
                            .child("Ran the saved version: your edits are not saved yet (⌘S)."),
                    )
                })
                .when_some(result.output, |this, output| {
                    this.child(mono_block("eng-script-output", output, theme.fg, theme))
                })
                .when_some(result.error, |this, error| {
                    this.child(mono_block("eng-script-error", error, theme.danger, theme))
                })
                .into_any_element()
        }
    };

    div()
        .flex()
        .flex_1()
        .min_w_0()
        .gap(SPACE_6)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .w(gpui::relative(0.5))
                .min_w_0()
                .child(caption("SAMPLE INPUT (JSON)", theme))
                .children(input),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(caption("RESULT", theme))
                .child(div().flex().flex_col().flex_1().min_h_0().child(result)),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn history_panel(
    console: &mut Console,
    id: i64,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let zone = console.zone.clone();
    let state = console.state::<State>();
    let list = match &state.executions {
        Some((for_id, result)) if *for_id == id => Some(result.clone()),
        _ => None,
    };
    let open = state.open_execution;
    let list = match list {
        None => return ops_kit::notice("Loading the runs…", false, &theme),
        Some(Err(error)) => return ops_kit::notice(format!("Runs: {error}"), true, &theme),
        Some(Ok(list)) if list.is_empty() => {
            return ops_kit::notice(
                "No runs yet. A test or a flow's Script node adds one.",
                false,
                &theme,
            );
        }
        Some(Ok(list)) => list,
    };
    let len = list.len();
    let keys = keyboard_list(
        ListKeys {
            id: HISTORY,
            on_move: Some(history_moved),
            on_activate: history_activate,
        },
        len,
        rows_on_screen(window, HISTORY_ROW),
        console,
        cx,
    );
    let nav = console.list(HISTORY, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());

    let cell = |text: &'static str, width: Option<f32>| {
        div()
            .map(|this| match width {
                Some(width) => this.w(px(width)).flex_none(),
                None => this.flex_1().min_w_0(),
            })
            .child(text)
    };
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(26.0))
        .px(SPACE_5)
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(cell("TIME", Some(TIME)))
        .child(cell("STATUS", Some(STATUS)))
        .child(cell("DURATION", Some(DURATION)))
        .child(cell("TRIGGER", None));

    let detail = list
        .iter()
        .find(|execution| Some(execution.id) == open)
        .cloned();
    let rows: std::rc::Rc<[ScriptExecution]> = list.into();
    let table = uniform_list(
        "eng-script-runs",
        len + tail_rows(len, HISTORY_ROW),
        cx.processor(
            move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                range
                    .map(|row| match rows.get(row) {
                        Some(execution) => {
                            execution_row(execution, row, cursor, focused, open, &zone, &theme, cx)
                        }
                        None => tail_row(HISTORY_ROW),
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .size_full();

    let detail = match detail {
        None => ops_kit::notice("↵ or a click shows a run's output.", false, &theme),
        Some(execution) => {
            let mut parts: Vec<AnyElement> = Vec::new();
            parts.push(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(pill(
                        execution.status.to_uppercase(),
                        outcome_color(&execution.status, &theme),
                    ))
                    .child(format!(
                        "#{} · {} · {}",
                        execution.id,
                        execution.trigger(),
                        model::duration_label(execution.duration_ms)
                    ))
                    .when_some(execution.flow_id.clone(), |this, flow| {
                        this.child(div().font_family(MONO).child(format!("flow {flow}")))
                    })
                    .into_any_element(),
            );
            if let Some(error) = execution.error_message.clone() {
                parts.push(
                    caption("ERROR", &theme)
                        .text_color(theme.danger)
                        .into_any_element(),
                );
                parts.push(mono_block(
                    format!("eng-run-error-{}", execution.id),
                    error,
                    theme.danger,
                    &theme,
                ));
            }
            if let Some(output) = execution.output.clone() {
                parts.push(caption("OUTPUT", &theme).into_any_element());
                parts.push(mono_block(
                    format!("eng-run-output-{}", execution.id),
                    output,
                    theme.fg,
                    &theme,
                ));
            }
            if execution.error_message.is_none() && execution.output.is_none() {
                parts.push(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child("No output or error details available.")
                        .into_any_element(),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .size_full()
                .children(parts)
                .into_any_element()
        }
    };

    div()
        .flex()
        .flex_1()
        .min_w_0()
        .gap(SPACE_6)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .child(header)
                .child(keys.flex_1().min_h_0().child(table)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .w(gpui::relative(0.5))
                .min_w_0()
                .min_h_0()
                .child(detail),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn execution_row(
    execution: &ScriptExecution,
    row: usize,
    cursor: Option<usize>,
    focused: bool,
    open: Option<i64>,
    zone: &LocalZone,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let hover = theme.hover;
    let shown = open == Some(execution.id);
    div()
        .id(gpui::ElementId::NamedInteger(
            "eng-script-run".into(),
            u64::try_from(execution.id).unwrap_or(0),
        ))
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .h(px(HISTORY_ROW))
        .px(SPACE_5)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(shown, |this| this.bg(theme.selected))
        .when(!shown, |this| this.hover(move |style| style.bg(hover)))
        .on_click(cx.listener(move |console, _, window, cx| {
            console.list(HISTORY, cx).select(row);
            let focus = console.list(HISTORY, cx).focus.clone();
            window.focus(&focus);
            pick_execution(console, row, true, cx);
        }))
        .when(focused && cursor == Some(row), |this| {
            this.child(focus_ring(theme))
        })
        .child(
            div()
                .w(px(TIME))
                .flex_none()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .child(local_time(execution.created_at, zone)),
        )
        .child(div().w(px(STATUS)).flex_none().child(pill(
            execution.status.to_uppercase(),
            outcome_color(&execution.status, theme),
        )))
        .child(
            div()
                .w(px(DURATION))
                .flex_none()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(model::duration_label(execution.duration_ms)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(execution.trigger().to_owned()),
        )
        .into_any_element()
}

/// The delete confirmation: Enter or the button deletes, Esc keeps.
fn confirm_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let Some((_, name)) = state.confirm.clone() else {
        return div().into_any_element();
    };
    let focus = state.confirm_focus.clone();
    div()
        .when_some(focus, |this, focus| this.track_focus(&focus))
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            // Enter on the dialog itself; on a focused button it is only
            // that button's.
            let own = console
                .state::<State>()
                .confirm_focus
                .as_ref()
                .is_some_and(|focus| focus.is_focused(window));
            if own && event.keystroke.key == "enter" {
                delete_confirmed(console, window, cx);
                cx.stop_propagation();
            }
        }))
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(vt_ui::icon(Icon::Trash, ICON_LG, theme.danger))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Delete {name}?")),
                ),
        )
        .child(div().text_color(theme.fg_muted).child(
            "Its runs go with it, and a flow's Script node that calls it fails. This cannot be undone.",
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(kbd("↵", cx))
                        .child("delete")
                        .child(kbd("Esc", cx))
                        .child("keep"),
                )
                .child(
                    Button::new("eng-script-delete-cancel", "Keep")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.state::<State>().confirm = None;
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new("eng-script-delete-confirm", "Delete")
                        .danger()
                        .icon(Icon::Trash)
                        .on_click(cx.listener(|console, _, window, cx| {
                            delete_confirmed(console, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::testing::Harness;
    use gpui::TestAppContext;

    fn saved(id: i64, source: &str) -> Script {
        Script {
            id,
            name: format!("Script {id}"),
            source: source.into(),
            syntax_error: None,
            cpu_limit_ms: 5000,
            memory_limit_bytes: 32 * 1024 * 1024,
            enabled: true,
            created_at: 1_761_390_000,
            updated_at: 1_761_390_000,
        }
    }

    /// Signed in with scripts 1 and 2 listed and `open` in the editor.
    fn with_open(cx: &mut TestAppContext, open_id: Option<i64>, source: &str) -> Harness {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        let source = source.to_owned();
        h.update(move |console, window, cx| {
            console.state::<State>().scripts = Some(Ok(vec![
                saved(1, &source),
                saved(2, "function main(i) { return i; }"),
            ]));
            open(console, open_id, window, cx);
        });
        h.calls();
        h
    }

    fn asked_executions(h: &mut Harness) -> bool {
        h.calls()
            .iter()
            .any(|key| key.name == "data.scripts.executions")
    }

    #[gpui::test]
    fn a_test_answer_refetches_runs_only_for_the_open_script(cx: &mut TestAppContext) {
        let mut h = with_open(cx, Some(1), "function main(i) { return i; }");
        h.update(|console, _, _| console.state::<State>().testing = true);
        h.reply(
            CallKey::new("data.scripts.test").with(2),
            Reply::ok((TestResult::default(), false)),
        );
        assert!(!asked_executions(&mut h), "script 2 is not open");

        h.update(|console, _, _| console.state::<State>().testing = true);
        h.reply(
            CallKey::new("data.scripts.test").with(1),
            Reply::ok((TestResult::default(), false)),
        );
        assert!(asked_executions(&mut h), "script 1 is open");
    }

    #[gpui::test]
    fn a_create_answer_refetches_runs_only_for_the_open_script(cx: &mut TestAppContext) {
        // Script 1 opened while the new one was being created.
        let mut h = with_open(cx, Some(1), "function main(i) { return i; }");
        h.update(|console, _, _| console.state::<State>().saving = true);
        h.reply(
            CallKey::new("data.scripts.create"),
            Reply::ok(saved(3, "function main(i) { return i; }")),
        );
        assert!(!asked_executions(&mut h), "script 3 is not open");

        let mut h = with_open(cx, None, "function main(i) { return i; }");
        h.update(|console, _, _| console.state::<State>().saving = true);
        h.reply(
            CallKey::new("data.scripts.create"),
            Reply::ok(saved(4, "function main(i) { return i; }")),
        );
        assert!(asked_executions(&mut h), "the new script is open as 4");
    }

    #[gpui::test]
    fn a_crlf_script_opens_unedited(cx: &mut TestAppContext) {
        let mut h = with_open(cx, Some(1), "function main(i) {\r\n  return i;\r\n}\r\n");
        let dirty = h.update(|console, _, cx| {
            // As any field's change works it out: the editor's text
            // against the server's.
            recheck_dirty(console, cx);
            console
                .state::<State>()
                .editor
                .as_ref()
                .map(|editor| editor.dirty)
        });
        assert_eq!(dirty, Some(false));
    }
}

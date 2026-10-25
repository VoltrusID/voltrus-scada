//! The mimic's edit mode (E, or Edit in the bar; operators): the web
//! client's SCADA editor (`client/src/scada2/ScadaView.tsx` with its
//! palette, inspector and canvas) over the rules of
//! [`voltrus_scada_native::mimic::edit`].
//!
//! - **Stage** ([`stage`]): the grid (a dot every 20 stage pixels, moves on
//!   10), the selection with its handles and, on a symbol, the quarter-turn
//!   knob; click, ⇧-click, a rubber band on the empty stage (⌥ starts one
//!   anywhere), ⌘A. Drags move every selected widget, handles resize, both
//!   snapping to the grid and lining up with other widgets (guide lines;
//!   ⌘ held: free), boxes kept on the 1920×1080 stage. Frames come from the
//!   pointer only while something is dragged.
//! - **Connect tool** (C; V back): click the source, then the target; the
//!   route follows the pointer meanwhile. A connection is selected by
//!   clicking its line.
//! - **Palette** ([`palette`]): the library by category and the widget
//!   types, searchable (/); click adds at the stage's centre, a drag drops
//!   at the pointer.
//! - **Inspector** ([`inspector`]): position and size, the type's settings,
//!   bindings with a tag search, arrange; a connection's type; the screen's
//!   summary when nothing is selected.
//! - **History**: ⌘Z / ⇧⌘Z, ⌘C / ⌘V / ⌘D, ⌫, arrows (⇧: 10), [ and ], R /
//!   ⇧R turn, H mirrors.
//! - **Saving**: ⌘S puts the rows (`PUT scada-equipment` and
//!   `scada-connections`); a failure keeps the edits. Unsaved edits are
//!   never dropped silently: Done, another screen, another page and
//!   signing out ask first (Save, Discard, Cancel). The web client saves
//!   on a screen switch without asking; the console asks, so an operator
//!   never writes a plant screen by accident. A screen loading meanwhile
//!   never replaces the edits.

mod inspector;
mod palette;
mod stage;

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, Context, CursorStyle, DispatchPhase, Div,
    DragMoveEvent, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, Stateful,
    Subscription, WeakEntity, Window, actions, canvas, div, px,
};
use serde::de::IgnoredAny;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::mimic::edit::{
    self, Clip, Edit, Guide, Handle, History, PASTE_OFFSET, Placing, Rect, Snapping,
};
use voltrus_scada_native::mimic::{self, Items, Layout, connections_path, equipment_path};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, empty_state, segmented};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

pub(super) use self::stage::paint_grid;
use super::paint::{Hmi, Tessellations};
use super::scene::StoreValues;
use super::{Drawn, Frame, Loaded, State, Wire, active, focus_handle, redraw};
use crate::console::keys::Dismiss;
use crate::console::overlay::Overlay;
use crate::console::screens::Screens;
use crate::console::views::cached::CachedView;
use crate::console::{Console, Page, Phase, anim};

/// The editor's key context, on its root.
pub const EDIT: &str = "MimicEdit";
/// Its keys that stay off while typing in a field.
pub const KEYS: &str = "MimicEdit && !TextInput";
/// Its keys that work in its fields too (⌘S).
pub const KEYS_ANYWHERE: &str = "MimicEdit";

actions!(
    voltrus_console,
    [
        MimicEdit,
        MimicEditSave,
        MimicEditUndo,
        MimicEditRedo,
        MimicEditCopy,
        MimicEditPaste,
        MimicEditDuplicate,
        MimicEditSelectAll,
        MimicEditDelete,
        MimicEditFront,
        MimicEditBack,
        MimicEditSelectTool,
        MimicEditConnectTool,
        MimicEditRotate,
        MimicEditRotateBack,
        MimicEditMirror,
        MimicEditGrid,
        MimicEditFind,
        MimicEditNext,
        MimicEditPrevious,
    ]
);

/// Move the selection by `(dx, dy)` stage pixels (the arrows: 1, with ⇧ 10).
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = voltrus_console, no_json)]
pub struct MimicEditNudge(pub i32, pub i32);

/// The palette's width (`PANEL_WIDTH`).
const PALETTE_WIDTH: Pixels = px(252.0);
/// The inspector's width (`INSPECTOR_WIDTH`).
pub(crate) const INSPECTOR_WIDTH: Pixels = px(288.0);
/// Window pixels around a handle that grab it.
const HANDLE_HIT: f64 = 7.0;
/// Window pixels from a box's top to its rotation knob.
const KNOB_GAP: f64 = 22.0;
/// Window pixels around a connection's line that select it.
const LINE_HIT: f64 = 6.0;
/// Window pixels within which an edge lines up with another widget's.
const GUIDE_SNAP: f64 = 6.0;
/// The save's call key.
const SAVE: &str = "mimic.save";
/// The unsaved-changes dialog.
const DIALOG: &str = "mimic-unsaved";

/// What a press on the stage does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Select,
    Connect,
}

/// The inspector's text fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    X,
    Y,
    Width,
    Height,
    Config(&'static str),
}

/// What the inspector's tag search binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    /// The widget's (only) tag.
    Primary,
    /// One more tag of a multi-tag widget.
    Add,
    /// A tag setting (`faultSensorId`, `runSensorId`).
    Config(&'static str),
}

/// A drag on the stage in progress.
#[derive(Debug, Clone)]
enum Gesture {
    /// Pressed on a widget: a click (it selects on release) or, once the
    /// pointer moves, a move of the selection.
    Move {
        at: Point<Pixels>,
        moved: bool,
        ids: Vec<String>,
        starts: Vec<(f64, f64)>,
        area: Rect,
        key: String,
        /// Select only this widget on a click.
        click: Option<String>,
        /// Drop this widget from the selection on a click (⇧).
        toggle: Option<String>,
    },
    Resize {
        at: Point<Pixels>,
        id: String,
        start: Rect,
        handle: Handle,
        key: String,
    },
    Band {
        from: (f64, f64),
        base: Vec<String>,
    },
    /// The rotation knob: a quarter turn on release.
    Knob,
}

/// The pointer's shape over the stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pointer {
    Plain,
    Grab,
    Knob,
    Resize(Handle),
}

/// Why the editor is being left.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Leave {
    /// Done (E): back to the viewer.
    Done,
    /// Another screen.
    Screen(String),
    /// Another page.
    Page(Page),
    /// ⌘[ or ⌘]: back or forward through the pages.
    History(bool),
    SignOut,
}

/// What a command does; the actions and the toolbar run them.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Cmd {
    Toggle,
    Save,
    Undo,
    Redo,
    Copy,
    Paste,
    Duplicate,
    SelectAll,
    Delete,
    Nudge(i32, i32),
    Front,
    Back,
    Tool(Tool),
    Rotate(i32),
    Mirror,
    Grid,
    Find,
    Step(bool),
}

/// The edit mode asked for before the screen was in (`--edit`, E, the
/// palette), and the commands that came meanwhile (`--overlay`).
#[derive(Default)]
pub(crate) struct Launch {
    edit: bool,
    queued: Vec<Cmd>,
}

/// The editor of the active screen.
pub(crate) struct Editor {
    screen: String,
    history: History,
    /// Selected widgets, in the order they were picked.
    selection: Vec<String>,
    /// The selected connection.
    line: Option<String>,
    tool: Tool,
    /// The type new connections take.
    line_type: &'static str,
    /// The connect tool's source.
    from: Option<String>,
    /// The pointer on the stage, for the connect tool's route.
    pointer: Option<(f64, f64)>,
    hover: Option<String>,
    cursor: Pointer,
    grid: bool,
    gesture: Option<Gesture>,
    guides: Vec<Guide>,
    band: Option<Rect>,
    /// A palette item dragged over the stage: where it would land.
    ghost: Option<(Placing, f64, f64)>,
    clip: Clip,
    drags: u64,
    /// The save in flight, what it sent (screen, layout, rows' layout) and
    /// what to do once it is in.
    saving: Option<i64>,
    next_save: i64,
    sent: Option<(String, Layout, Layout)>,
    after: Option<Leave>,
    /// The dialog's question.
    asking: Option<Leave>,
    window: Option<AnyWindowHandle>,
    query: Entity<TextInput>,
    tag_query: Entity<TextInput>,
    picking: Option<Pick>,
    /// The inspector's fields and the widget they were made for.
    fields: Vec<(Slot, Entity<TextInput>)>,
    fields_for: Option<String>,
    palette_view: Entity<CachedView>,
    inspector_view: Entity<CachedView>,
    dialog_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    _field_subscriptions: Vec<Subscription>,
}

impl Editor {
    fn layout(&self) -> &Layout {
        self.history.present()
    }

    /// Forget selections of what is gone (after an undo, a delete).
    fn prune(&mut self) {
        let layout = self.history.present();
        let known = |id: &String| layout.widget(id).is_some();
        self.selection.retain(known);
        if self.from.as_ref().is_some_and(|id| !known(id)) {
            self.from = None;
        }
        if let Some(line) = &self.line
            && !layout.connections.iter().any(|c| c.id == *line)
        {
            self.line = None;
        }
    }

    /// The single selected widget.
    fn single(&self) -> Option<&mimic::Widget> {
        match self.selection.as_slice() {
            [id] => self.history.present().widget(id),
            _ => None,
        }
    }
}

fn edits(console: &mut Console) -> Option<&mut Editor> {
    console.state::<State>().edit.as_deref_mut()
}

fn edits_ref(console: &Console) -> Option<&Editor> {
    console.state_ref::<State>()?.edit.as_deref()
}

/// The mimic is in edit mode.
pub(crate) fn editing(console: &Console) -> bool {
    edits_ref(console).is_some()
}

/// `--edit` with `--page mimic`: edit once the screen is in.
pub(crate) fn launch(console: &mut Console, edit: bool) {
    console.state::<Launch>().edit = edit;
}

/// Enter the edit mode asked for, once the active screen's mimic is in
/// (called by the viewer's render; the entering itself runs after it).
pub(super) fn launch_ready(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let wanted = console
        .state_ref::<Launch>()
        .is_some_and(|launch| launch.edit);
    if !wanted || console.page != Page::Mimic || console.phase != Phase::Console {
        return;
    }
    let screen = active(console);
    let ready = console
        .state_ref::<State>()
        .is_some_and(|state| matches!(state.layouts.get(&screen), Some(Loaded::Ready(_))));
    if ready {
        console.state::<Launch>().edit = false;
        cx.defer_in(window, |console, window, cx| {
            if console.is_operator() && !editing(console) {
                enter(console, window, cx);
            }
            let queued = std::mem::take(&mut console.state::<Launch>().queued);
            for cmd in queued {
                run(console, cmd, window, cx);
            }
        });
    }
}

/// Start editing the active screen's mimic as loaded.
fn enter(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let screen = active(console);
    let layout = match console.state::<State>().layouts.get(&screen) {
        Some(Loaded::Ready(layout)) => Layout::clone(layout),
        _ => return,
    };
    let query = cx.new(|cx| TextInput::new(cx).placeholder("Search symbols and widgets"));
    let tag_query = cx.new(|cx| TextInput::new(cx).placeholder("Find a tag by name or key"));
    let subscriptions = vec![
        cx.subscribe_in(
            &query,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => cx.notify(),
                InputEvent::Submitted => palette::add_first(console, window, cx),
                InputEvent::Cancelled => {
                    if let Some(editor) = edits(console) {
                        let query = editor.query.clone();
                        query.update(cx, |field, cx| field.set_text("", cx));
                    }
                    focus_stage(console, window, cx);
                }
                InputEvent::Blurred => {}
            },
        ),
        cx.subscribe_in(
            &tag_query,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => cx.notify(),
                InputEvent::Submitted => inspector::pick_first(console, window, cx),
                InputEvent::Cancelled => {
                    if let Some(editor) = edits(console) {
                        editor.picking = None;
                    }
                    focus_stage(console, window, cx);
                }
                InputEvent::Blurred => {}
            },
        ),
    ];
    let editor = Editor {
        screen,
        history: History::new(layout),
        selection: Vec::new(),
        line: None,
        tool: Tool::Select,
        line_type: edit::LINE_TYPES[0].0,
        from: None,
        pointer: None,
        hover: None,
        cursor: Pointer::Plain,
        grid: true,
        gesture: None,
        guides: Vec::new(),
        band: None,
        ghost: None,
        clip: Clip::default(),
        drags: 0,
        saving: None,
        next_save: 0,
        sent: None,
        after: None,
        asking: None,
        window: Some(window.window_handle()),
        query,
        tag_query,
        picking: None,
        fields: Vec::new(),
        fields_for: None,
        palette_view: CachedView::new(palette::render, cx),
        inspector_view: CachedView::new(inspector::render, cx),
        dialog_focus: cx.focus_handle(),
        _subscriptions: subscriptions,
        _field_subscriptions: Vec::new(),
    };
    let state = console.state::<State>();
    super::close_faceplate(state);
    state.cursor = None;
    state.hover = None;
    state.press = None;
    state.edit = Some(Box::new(editor));
    focus_stage(console, window, cx);
    cx.notify();
}

/// Back to the viewer.
fn exit(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.state::<State>().edit = None;
    focus_stage(console, window, cx);
    cx.notify();
}

fn focus_stage(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let focus = focus_handle(console, cx);
    window.focus(&focus);
}

/// E, the bar's Edit and Done: start editing, or leave (asking first when
/// something is unsaved).
fn toggle(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if editing(console) {
        if !holds(console, Leave::Done, window, cx) {
            exit(console, window, cx);
        }
        return;
    }
    if !console.is_operator() {
        console.show_toast(
            "Editing a screen needs an operator role (admin, demo or integration)",
            true,
            cx,
        );
        return;
    }
    if console.stress.is_some() {
        return;
    }
    console.state::<Launch>().edit = true;
    if console.page != Page::Mimic {
        console.show(Page::Mimic, window, cx);
    }
    cx.notify();
}

/// Whether leaving waits: with unsaved edits the dialog asks first (and
/// runs `leave` once answered); a save in flight holds it too.
pub(crate) fn holds(
    console: &mut Console,
    leave: Leave,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> bool {
    let Some(editor) = edits(console) else {
        return false;
    };
    if editor.saving.is_some() {
        console.show_toast("The screen is being saved", false, cx);
        return true;
    }
    if !editor.history.dirty() {
        return false;
    }
    editor.asking = Some(leave);
    let focus = editor.dialog_focus.clone();
    let overlay = Overlay::modal(DIALOG, px(460.0), dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
    true
}

/// Leave as asked: the edits are saved or discarded by now.
fn proceed(console: &mut Console, leave: Leave, window: &mut Window, cx: &mut Context<Console>) {
    match leave {
        Leave::Done => exit(console, window, cx),
        Leave::Screen(id) => super::switch_screen(console, &id, window, cx),
        Leave::Page(page) => {
            exit(console, window, cx);
            console.show(page, window, cx);
        }
        Leave::History(forward) => {
            exit(console, window, cx);
            console.history_step(forward, window, cx);
        }
        Leave::SignOut => {
            exit(console, window, cx);
            console.sign_out(window, cx);
        }
    }
}

/// [`proceed`] from a REST answer, which has no window.
fn proceed_later(console: &mut Console, leave: Leave, cx: &mut Context<Console>) {
    let Some(handle) = edits_ref(console).and_then(|editor| editor.window) else {
        return;
    };
    cx.spawn(async move |this, cx| {
        let _ = handle.update(cx, |_, window, cx| {
            let _ = this.update(cx, |console, cx| proceed(console, leave, window, cx));
        });
    })
    .detach();
}

#[derive(Clone, Copy)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

fn answer(console: &mut Console, answer: Answer, window: &mut Window, cx: &mut Context<Console>) {
    let leave = edits(console).and_then(|editor| editor.asking.take());
    console.close_overlay(window, cx);
    let Some(leave) = leave else {
        return;
    };
    match answer {
        Answer::Cancel => {}
        Answer::Discard => {
            if let Some(editor) = edits(console) {
                editor.history = History::new(editor.history.saved().clone());
                editor.prune();
            }
            proceed(console, leave, window, cx);
        }
        Answer::Save => save(console, Some(leave), cx),
    }
    cx.notify();
}

fn dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let doing = match editor.asking {
        Some(Leave::Screen(_)) => "switching screens",
        Some(Leave::Page(_) | Leave::History(_)) => "leaving the mimic",
        Some(Leave::SignOut) => "signing out",
        _ => "leaving edit mode",
    };
    let name = screen_name(console, &editor.screen);
    let focus = editor.dialog_focus.clone();
    div()
        .track_focus(&focus)
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "enter" {
                answer(console, Answer::Save, window, cx);
            }
        }))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .px(SPACE_8)
                .h(px(48.0))
                .border_b_1()
                .border_color(theme.border)
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child("Unsaved changes"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .p(SPACE_8)
                .child(format!("Save your changes to “{name}” before {doing}?"))
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child("Discard puts the screen back as it was last saved."),
                ),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap(SPACE_4)
                .px(SPACE_8)
                .py(SPACE_6)
                .border_t_1()
                .border_color(theme.border)
                .child(
                    Button::new("mimic-unsaved-cancel", "Cancel")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            answer(console, Answer::Cancel, window, cx);
                        })),
                )
                .child(
                    Button::new("mimic-unsaved-discard", "Discard")
                        .danger()
                        .on_click(cx.listener(|console, _, window, cx| {
                            answer(console, Answer::Discard, window, cx);
                        })),
                )
                .child(
                    Button::new("mimic-unsaved-save", "Save  ↵")
                        .primary()
                        .on_click(cx.listener(|console, _, window, cx| {
                            answer(console, Answer::Save, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

fn screen_name(console: &Console, id: &str) -> String {
    console
        .state_ref::<Screens>()
        .map_or_else(|| id.to_owned(), |screens| screens.name(id))
}

// ── Saving ────────────────────────────────────────────────────────────────

/// ⌘S: `PUT` the layout's rows; `after` runs once the server has them.
fn save(console: &mut Console, after: Option<Leave>, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast("Saving a screen needs an operator role", true, cx);
        return;
    }
    let Some(editor) = edits_ref(console) else {
        return;
    };
    if editor.saving.is_some() {
        return;
    }
    let screen = editor.screen.clone();
    let snapshot = editor.layout().clone();
    let named = edit::named(&snapshot, |tag| {
        console.store.get(tag).map(|tag| tag.info.name.clone())
    });
    let (rows, lines) = mimic::to_rows(&named);
    let Some(editor) = edits(console) else {
        return;
    };
    editor.next_save += 1;
    let seq = editor.next_save;
    editor.saving = Some(seq);
    editor.after = after;
    editor.sent = Some((screen.clone(), snapshot, named));
    console.call(
        CallKey::new(SAVE).with(seq),
        Call::new(move |api| async move {
            api.send_json::<_, IgnoredAny>(
                Method::PUT,
                &equipment_path(&screen),
                &Items { items: rows },
            )
            .await?;
            api.send_json::<_, IgnoredAny>(
                Method::PUT,
                &connections_path(&screen),
                &Items { items: lines },
            )
            .await?;
            Ok(())
        }),
    );
    cx.notify();
}

/// The save's answer: marked saved (the viewer shows the saved layout),
/// or a toast and the edits kept.
pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    if key.name != SAVE {
        return;
    }
    let result = reply.take::<()>();
    let state = console.state::<State>();
    let Some(editor) = state.edit.as_deref_mut() else {
        return;
    };
    if editor.saving != Some(key.arg) {
        return;
    }
    editor.saving = None;
    let after = editor.after.take();
    let sent = editor.sent.take();
    match result {
        Ok(()) => {
            if let Some((screen, snapshot, named)) = sent {
                if editor.screen == screen {
                    editor.history.mark_saved(snapshot);
                }
                state.layouts.insert(screen, Loaded::Ready(Rc::new(named)));
            }
            console.show_toast("Screen saved", false, cx);
            if let Some(leave) = after {
                proceed_later(console, leave, cx);
            }
        }
        Err(error) => {
            console.show_toast(
                format!("Screen not saved: {error}. The edits are kept; save again."),
                true,
                cx,
            );
        }
    }
    cx.notify();
}

// ── Commands ──────────────────────────────────────────────────────────────

/// The editor's actions, on the window's root so the palette and
/// `--overlay` reach them from anywhere.
pub(crate) fn actions(root: Stateful<Div>, cx: &Context<Console>) -> Stateful<Div> {
    root.on_action(cx.listener(|console, _: &MimicEdit, window, cx| {
        run(console, Cmd::Toggle, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditSave, window, cx| {
        run(console, Cmd::Save, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditUndo, window, cx| {
        run(console, Cmd::Undo, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditRedo, window, cx| {
        run(console, Cmd::Redo, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditCopy, window, cx| {
        run(console, Cmd::Copy, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditPaste, window, cx| {
        run(console, Cmd::Paste, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditDuplicate, window, cx| {
        run(console, Cmd::Duplicate, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditSelectAll, window, cx| {
        run(console, Cmd::SelectAll, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditDelete, window, cx| {
        run(console, Cmd::Delete, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditFront, window, cx| {
        run(console, Cmd::Front, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditBack, window, cx| {
        run(console, Cmd::Back, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditSelectTool, window, cx| {
        run(console, Cmd::Tool(Tool::Select), window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditConnectTool, window, cx| {
        run(console, Cmd::Tool(Tool::Connect), window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditRotate, window, cx| {
        run(console, Cmd::Rotate(1), window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditRotateBack, window, cx| {
        run(console, Cmd::Rotate(-1), window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditMirror, window, cx| {
        run(console, Cmd::Mirror, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditGrid, window, cx| {
        run(console, Cmd::Grid, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditFind, window, cx| {
        run(console, Cmd::Find, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditNext, window, cx| {
        run(console, Cmd::Step(true), window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditPrevious, window, cx| {
        run(console, Cmd::Step(false), window, cx);
    }))
    .on_action(cx.listener(|console, nudge: &MimicEditNudge, window, cx| {
        run(console, Cmd::Nudge(nudge.0, nudge.1), window, cx);
    }))
}

fn run(console: &mut Console, cmd: Cmd, window: &mut Window, cx: &mut Context<Console>) {
    if console.phase != Phase::Console {
        return;
    }
    if cmd == Cmd::Toggle {
        toggle(console, window, cx);
        return;
    }
    if !editing(console) {
        let launch = console.state::<Launch>();
        if launch.edit {
            launch.queued.push(cmd);
        } else {
            console.show_toast("Edit the mimic first (E on the mimic)", true, cx);
        }
        return;
    }
    match cmd {
        Cmd::Save => save(console, None, cx),
        Cmd::Find => {
            if let Some(editor) = edits(console) {
                let query = editor.query.clone();
                query.read(cx).focus(window);
            }
        }
        Cmd::Grid => {
            if let Some(editor) = edits(console) {
                editor.grid = !editor.grid;
            }
            cx.notify();
        }
        Cmd::Tool(tool) => {
            if let Some(editor) = edits(console) {
                editor.tool = tool;
                editor.from = None;
                if tool == Tool::Connect {
                    editor.selection.clear();
                    editor.line = None;
                }
            }
            changed(console, window, cx);
        }
        Cmd::Undo | Cmd::Redo => {
            if let Some(editor) = edits(console) {
                if cmd == Cmd::Undo {
                    editor.history.undo();
                } else {
                    editor.history.redo();
                }
                editor.prune();
            }
            changed(console, window, cx);
        }
        Cmd::Copy => {
            if let Some(editor) = edits(console)
                && !editor.selection.is_empty()
            {
                editor.clip = edit::copy(editor.history.present(), &editor.selection);
            }
        }
        Cmd::SelectAll => {
            if let Some(editor) = edits(console) {
                editor.selection = editor
                    .history
                    .present()
                    .widgets
                    .iter()
                    .map(|widget| widget.id.clone())
                    .collect();
                editor.line = None;
                editor.tool = Tool::Select;
            }
            changed(console, window, cx);
        }
        Cmd::Step(forward) => {
            if let Some(editor) = edits(console) {
                let next = edit::step(
                    editor.history.present(),
                    editor.selection.last().map(String::as_str),
                    forward,
                );
                editor.selection = next.into_iter().collect();
                editor.line = None;
            }
            changed(console, window, cx);
        }
        Cmd::Paste | Cmd::Duplicate => {
            let pasted = edits(console).and_then(|editor| {
                let clip = if cmd == Cmd::Paste {
                    editor.clip.clone()
                } else {
                    edit::copy(editor.history.present(), &editor.selection)
                };
                let pasted = edit::paste(
                    editor.history.present(),
                    &clip,
                    PASTE_OFFSET,
                    edit::fresh_id,
                )?;
                if cmd == Cmd::Paste {
                    editor.clip.shift(PASTE_OFFSET);
                }
                Some(pasted)
            });
            if let Some(pasted) = pasted {
                let ids: Vec<String> = match &pasted {
                    Edit::Add { widgets, .. } => widgets.iter().map(|w| w.id.clone()).collect(),
                    _ => Vec::new(),
                };
                apply(console, &pasted, None, window, cx);
                if let Some(editor) = edits(console) {
                    editor.selection = ids;
                    editor.line = None;
                }
                changed(console, window, cx);
            }
        }
        Cmd::Delete => {
            let gone = edits(console).and_then(|editor| {
                if let Some(line) = editor.line.take() {
                    return Some(Edit::Remove {
                        widgets: Vec::new(),
                        connections: vec![line],
                    });
                }
                if editor.selection.is_empty() {
                    return None;
                }
                Some(Edit::Remove {
                    widgets: std::mem::take(&mut editor.selection),
                    connections: Vec::new(),
                })
            });
            if let Some(gone) = gone {
                apply(console, &gone, None, window, cx);
            }
        }
        Cmd::Nudge(dx, dy) => {
            let nudged = edits_ref(console).and_then(|editor| {
                let moved = edit::nudge(
                    editor.layout(),
                    &editor.selection,
                    f64::from(dx),
                    f64::from(dy),
                )?;
                Some((moved, format!("nudge:{}", editor.selection.join(","))))
            });
            if let Some((moved, key)) = nudged {
                apply(console, &moved, Some(key.as_str()), window, cx);
            }
        }
        Cmd::Front | Cmd::Back => {
            let reorder = edits_ref(console)
                .filter(|editor| !editor.selection.is_empty())
                .map(|editor| Edit::Reorder {
                    ids: editor.selection.clone(),
                    front: cmd == Cmd::Front,
                });
            if let Some(reorder) = reorder {
                apply(console, &reorder, None, window, cx);
            }
        }
        Cmd::Rotate(quarters) => {
            let turned = edits_ref(console)
                .and_then(|editor| edit::rotate(editor.layout(), &editor.selection, quarters));
            if let Some(turned) = turned {
                apply(console, &turned, None, window, cx);
            }
        }
        Cmd::Mirror => {
            let mirrored = edits_ref(console)
                .and_then(|editor| edit::mirror(editor.layout(), &editor.selection));
            if let Some(mirrored) = mirrored {
                apply(console, &mirrored, None, window, cx);
            }
        }
        Cmd::Toggle => {}
    }
}

/// Apply an edit (joining the step with key `key`), then redraw.
fn apply(
    console: &mut Console,
    edit: &Edit,
    key: Option<&str>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(editor) = edits(console)
        && editor.history.apply(edit, key)
    {
        editor.prune();
    }
    changed(console, window, cx);
}

/// After a change: the inspector's fields follow, everything redraws.
fn changed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    inspector::sync(console, window, cx);
    cx.notify();
}

/// Add from the palette: at a stage point (a drop), or at the centre.
fn add(
    console: &mut Console,
    placing: Placing,
    at: Option<(f64, f64)>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(editor) = edits(console) else {
        return;
    };
    let (width, height) = placing.size();
    let (x, y) = match at {
        Some((x, y)) => edit::drop_at(x, y, width, height),
        None => edit::place_at(editor.layout().widgets.len(), width, height),
    };
    let widget = placing.widget(edit::fresh_id("w"), x, y);
    let id = widget.id.clone();
    let added = Edit::Add {
        widgets: vec![widget],
        connections: Vec::new(),
    };
    editor.tool = Tool::Select;
    editor.from = None;
    apply(console, &added, None, window, cx);
    if let Some(editor) = edits(console) {
        editor.selection = vec![id];
        editor.line = None;
    }
    changed(console, window, cx);
    focus_stage(console, window, cx);
}

// ── The pointer ───────────────────────────────────────────────────────────

/// A window point on the stage, and the zoom it was drawn at.
fn stage_at(console: &mut Console, at: Point<Pixels>) -> Option<((f64, f64), f64)> {
    let drawn: Drawn = console.state::<State>().drawn.get()?;
    let x = f64::from(f32::from(at.x - drawn.bounds.origin.x));
    let y = f64::from(f32::from(at.y - drawn.bounds.origin.y));
    Some((drawn.camera.to_stage(x, y), drawn.camera.zoom))
}

fn knob_at(rect: Rect, zoom: f64) -> (f64, f64) {
    (rect.center_x(), rect.y - KNOB_GAP / zoom)
}

fn near(a: (f64, f64), b: (f64, f64), radius: f64) -> bool {
    (a.0 - b.0).abs() <= radius && (a.1 - b.1).abs() <= radius
}

fn on_press(
    console: &mut Console,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    focus_stage(console, window, cx);
    let Some(((x, y), zoom)) = stage_at(console, event.position) else {
        return;
    };
    let shift = event.modifiers.shift;
    let alt = event.modifiers.alt;
    let state = console.state::<State>();
    state.glide = None;
    let Some(editor) = state.edit.as_deref_mut() else {
        return;
    };
    if editor.tool == Tool::Connect {
        let layout = editor.history.present();
        let hit = edit::hit_widget(layout, x, y).map(|index| layout.widgets[index].id.clone());
        match hit {
            Some(id) => match editor.from.take() {
                Some(from) if from != id => {
                    let line =
                        edit::connection(edit::fresh_id("c"), &from, &id, editor.line_type);
                    let line_id = line.id.clone();
                    let added = Edit::Add {
                        widgets: Vec::new(),
                        connections: vec![line],
                    };
                    editor.history.apply(&added, None);
                    editor.selection.clear();
                    editor.line = Some(line_id);
                }
                _ => editor.from = Some(id),
            },
            None => editor.from = None,
        }
        changed(console, window, cx);
        return;
    }
    // The single selection's handles and knob.
    if let Some(widget) = editor.single() {
        let rect = Rect::of(widget);
        let id = widget.id.clone();
        let knob = edit::turns(widget) && near((x, y), knob_at(rect, zoom), HANDLE_HIT / zoom);
        if knob {
            editor.gesture = Some(Gesture::Knob);
            return;
        }
        if let Some(handle) = edit::hit_handle(rect, x, y, HANDLE_HIT / zoom) {
            editor.drags += 1;
            editor.gesture = Some(Gesture::Resize {
                at: event.position,
                key: format!("resize:{}", editor.drags),
                id,
                start: rect,
                handle,
            });
            return;
        }
    }
    let layout = editor.history.present();
    let widget = if alt {
        None
    } else {
        edit::hit_widget(layout, x, y).map(|index| layout.widgets[index].id.clone())
    };
    let line = if alt || widget.is_some() {
        None
    } else {
        edit::hit_connection(layout, x, y, LINE_HIT / zoom)
            .map(|index| layout.connections[index].id.clone())
    };
    if let Some(id) = widget {
        let selected = editor.selection.contains(&id);
        let (mut click, mut toggle) = (None, None);
        if shift {
            if selected {
                toggle = Some(id);
            } else {
                editor.selection.push(id);
            }
        } else if selected {
            click = Some(id);
        } else {
            editor.selection = vec![id];
        }
        editor.line = None;
        let layout = editor.history.present();
        let picked: Vec<&mimic::Widget> = edit::picked(layout, &editor.selection).collect();
        let ids: Vec<String> = picked.iter().map(|w| w.id.clone()).collect();
        let starts: Vec<(f64, f64)> = picked.iter().map(|w| (w.x, w.y)).collect();
        let area = edit::bounds(picked.iter().copied()).unwrap_or_default();
        editor.drags += 1;
        editor.gesture = Some(Gesture::Move {
            at: event.position,
            moved: false,
            key: format!("move:{}", editor.drags),
            ids,
            starts,
            area,
            click,
            toggle,
        });
    } else if let Some(line) = line {
        editor.selection.clear();
        editor.line = Some(line);
    } else {
        if !shift {
            editor.selection.clear();
        }
        editor.line = None;
        editor.gesture = Some(Gesture::Band {
            from: (x, y),
            base: editor.selection.clone(),
        });
    }
    changed(console, window, cx);
}

fn on_drag(console: &mut Console, event: &MouseMoveEvent, cx: &mut App) {
    let Some(((x, y), zoom)) = stage_at(console, event.position) else {
        return;
    };
    let free = event.modifiers.platform;
    let Some(editor) = edits(console) else {
        return;
    };
    let snapping = Snapping {
        grid: editor.grid && !free,
        threshold: if free { 0.0 } else { GUIDE_SNAP / zoom },
    };
    let pixels = |at: Point<Pixels>| {
        (
            f64::from(f32::from(event.position.x - at.x)) / zoom,
            f64::from(f32::from(event.position.y - at.y)) / zoom,
        )
    };
    match editor.gesture.as_mut() {
        Some(Gesture::Move {
            at,
            moved,
            ids,
            starts,
            area,
            key,
            ..
        }) => {
            let (dx, dy) = pixels(*at);
            if !*moved && (dx * zoom).hypot(dy * zoom) < f64::from(super::DRAG_SLOP) {
                return;
            }
            *moved = true;
            let snapped = edit::snap_move(editor.history.present(), ids, *area, dx, dy, snapping);
            let moves = ids
                .iter()
                .zip(starts.iter())
                .map(|(id, (sx, sy))| (id.clone(), sx + snapped.dx, sy + snapped.dy))
                .collect();
            editor.history.apply(&Edit::Move(moves), Some(key.as_str()));
            editor.guides = snapped.guides;
        }
        Some(Gesture::Resize {
            at,
            id,
            start,
            handle,
            key,
        }) => {
            let (dx, dy) = pixels(*at);
            let (rect, guides) = edit::snap_resize(
                editor.history.present(),
                id,
                *start,
                *handle,
                dx,
                dy,
                snapping,
            );
            let resized = Edit::Resize {
                id: id.clone(),
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            };
            editor.history.apply(&resized, Some(key.as_str()));
            editor.guides = guides;
        }
        Some(Gesture::Band { from, base }) => {
            let band = Rect::between(*from, (x, y));
            let mut selection = base.clone();
            for id in edit::enclosed(editor.history.present(), band) {
                if !selection.contains(&id) {
                    selection.push(id);
                }
            }
            editor.selection = selection;
            editor.band = Some(band);
        }
        Some(Gesture::Knob) | None => return,
    }
    redraw(console, cx);
}

fn on_release(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = edits(console) else {
        return;
    };
    let Some(gesture) = editor.gesture.take() else {
        return;
    };
    editor.guides.clear();
    editor.band = None;
    match gesture {
        Gesture::Move {
            moved: false,
            click,
            toggle,
            ..
        } => {
            if let Some(id) = click {
                editor.selection = vec![id];
            }
            if let Some(id) = toggle {
                editor.selection.retain(|selected| *selected != id);
            }
        }
        Gesture::Knob => {
            if let Some(turned) = edit::rotate(editor.history.present(), &editor.selection, 1) {
                editor.history.apply(&turned, None);
            }
        }
        Gesture::Move { .. } | Gesture::Resize { .. } | Gesture::Band { .. } => {}
    }
    changed(console, window, cx);
}

/// The pointer over the stage (nothing pressed): what it would grab, and
/// the connect tool's route.
fn on_hover(console: &mut Console, event: &MouseMoveEvent, cx: &mut Context<Console>) {
    let Some(((x, y), zoom)) = stage_at(console, event.position) else {
        return;
    };
    let Some(editor) = edits(console) else {
        return;
    };
    if editor.gesture.is_some() {
        return;
    }
    let layout = editor.history.present();
    let hover = edit::hit_widget(layout, x, y).map(|index| layout.widgets[index].id.clone());
    let cursor = match (editor.tool, editor.single()) {
        (Tool::Connect, _) => Pointer::Plain,
        (Tool::Select, Some(widget)) => {
            let rect = Rect::of(widget);
            if edit::turns(widget) && near((x, y), knob_at(rect, zoom), HANDLE_HIT / zoom) {
                Pointer::Knob
            } else if let Some(handle) = edit::hit_handle(rect, x, y, HANDLE_HIT / zoom) {
                Pointer::Resize(handle)
            } else if hover.is_some() {
                Pointer::Grab
            } else {
                Pointer::Plain
            }
        }
        (Tool::Select, None) if hover.is_some() => Pointer::Grab,
        (Tool::Select, None) => Pointer::Plain,
    };
    let follow = editor.from.is_some();
    let different = editor.hover != hover || editor.cursor != cursor || follow;
    editor.hover = hover;
    editor.cursor = cursor;
    editor.pointer = Some((x, y));
    if different {
        redraw(console, cx);
    }
}

/// A palette item dragged over the stage: where it would land.
fn on_drag_over(
    console: &mut Console,
    event: &DragMoveEvent<Placing>,
    cx: &mut Context<Console>,
) {
    let placing = *event.drag(cx);
    let inside = event.bounds.contains(&event.event.position);
    let at = stage_at(console, event.event.position);
    let Some(editor) = edits(console) else {
        return;
    };
    let ghost = at.filter(|_| inside).map(|((x, y), _)| {
        let (width, height) = placing.size();
        let (x, y) = edit::drop_at(x, y, width, height);
        (placing, x, y)
    });
    if editor.ghost != ghost {
        editor.ghost = ghost;
        redraw(console, cx);
    }
}

fn on_drop(
    console: &mut Console,
    placing: Placing,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(editor) = edits(console) {
        editor.ghost = None;
    }
    let at = stage_at(console, window.mouse_position()).map(|(point, _)| point);
    add(console, placing, at, window, cx);
}

/// While a gesture runs, the window's pointer moves and release reach the
/// editor, wherever the pointer is.
fn press_listeners(console: WeakEntity<Console>, window: &mut Window) {
    let mover = console.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            let _ = mover.update(cx, |console, cx| on_drag(console, event, cx));
        }
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            let _ = console.update(cx, |console, cx| on_release(console, window, cx));
        }
    });
}

// ── Drawing ───────────────────────────────────────────────────────────────

/// The page in edit mode: the bar, the palette, the stage, the inspector.
pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let hmi = Hmi::new(&theme);
    let focus = focus_handle(console, cx);
    let screen = active(console);
    let window_height = f64::from(f32::from(window.viewport_size().height));
    let state = console.state::<State>();
    state.view = Some(window.current_view());
    state.window_height = window_height;
    let (camera, gliding) = state.camera_at(std::time::Instant::now());
    let drawn = state.drawn.clone();
    let cache: Rc<Tessellations> = state.cache.clone();
    cache.next_frame();
    let loaded = matches!(state.layouts.get(&screen), Some(Loaded::Ready(_)));
    // Another screen came on show: a clean editor follows it.
    let stale = state.edit.as_deref().is_some_and(|editor| {
        editor.screen != screen && !editor.history.dirty() && editor.saving.is_none()
    });
    if stale && loaded {
        cx.defer_in(window, |console, window, cx| {
            follow_screen(console, window, cx);
        });
    }
    if gliding {
        anim::request_frame(window, &console.meter);
    }

    let view_camera = camera.or_else(|| drawn.get().map(|d| super::fit_of(&d)));
    let visible = drawn.get().zip(view_camera).map(|(d, camera)| {
        let (x0, y0) = camera.to_stage(0.0, 0.0);
        let (x1, y1) = camera.to_stage(
            f64::from(f32::from(d.bounds.size.width)),
            f64::from(f32::from(d.bounds.size.height)),
        );
        let pad = super::OVERSCAN;
        (x0 - pad, y0 - pad, x1 + pad, y1 + pad)
    });
    let in_view = |x: f64, y: f64, w: f64, h: f64| {
        visible.is_none_or(|(x0, y0, x1, y1)| x + w >= x0 && x <= x1 && y + h >= y0 && y <= y1)
    };

    let Some(state) = console.state_ref::<State>() else {
        return div().into_any_element();
    };
    let Some(editor) = state.edit.as_deref() else {
        return div().into_any_element();
    };
    let layout = editor.layout();
    let items: Vec<_> = layout
        .widgets
        .iter()
        .filter(|w| in_view(w.x, w.y, w.width, w.height))
        .map(|w| super::prepare(console, w, state))
        .collect();
    let values = StoreValues(&console.store);
    let wires: Vec<Wire> = layout
        .connections
        .iter()
        .filter_map(|connection| {
            let from = layout.widget(&connection.from)?;
            let to = layout.widget(&connection.to)?;
            Some(Wire {
                flowing: mimic::flowing(connection, layout, &values),
                line: connection.line(),
                points: mimic::route(from, to, &connection.waypoints),
            })
        })
        .collect();
    let marks = stage::marks(editor, hmi, cache.clone(), drawn.clone());
    let bar = Toolbar {
        dirty: editor.history.dirty(),
        saving: editor.saving.is_some(),
        can_undo: editor.history.can_undo(),
        can_redo: editor.history.can_redo(),
        tool: editor.tool,
        line_type: editor.line_type,
        grid: editor.grid,
        zoom: view_camera.map(|camera| camera.zoom),
    };
    let tool = editor.tool;
    let grid = editor.grid;
    let pressing = editor.gesture.is_some();
    let moving = matches!(editor.gesture, Some(Gesture::Move { moved: true, .. }));
    let cursor = editor.cursor;
    let connect_hint = (tool == Tool::Connect).then(|| {
        if editor.from.is_some() {
            "Now click the target equipment · Esc to cancel"
        } else {
            "Click the source equipment"
        }
    });
    let empty = layout.widgets.is_empty();
    let palette_view = editor.palette_view.clone();
    let inspector_view = editor.inspector_view.clone();

    let frame = Frame {
        camera,
        items,
        wires,
        hmi,
        // The editor draws the plant still.
        t: 0.0,
        cache,
        drawn,
        focused: false,
        grid,
    };
    let weak = cx.entity().downgrade();
    let stage_canvas = canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            super::paint_frame(frame, bounds, window, cx);
            stage::paint(&marks, window, cx);
            if pressing {
                press_listeners(weak, window);
            }
        },
    )
    .size_full();

    let bar = toolbar(&theme, &screen, bar, console, cx);
    let stage_bg = anim::mix(theme.bg, hmi.stage, 0.35);
    let body = if loaded || !stale {
        div()
            .id("mimic-edit-stage")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(stage_bg)
            .cursor(match (tool, cursor) {
                (Tool::Connect, _) => CursorStyle::Crosshair,
                _ if moving => CursorStyle::ClosedHand,
                (_, Pointer::Grab) => CursorStyle::OpenHand,
                (_, Pointer::Knob) => CursorStyle::PointingHand,
                (_, Pointer::Resize(handle)) => resize_cursor(handle),
                (_, Pointer::Plain) => CursorStyle::Arrow,
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|console, event: &MouseDownEvent, window, cx| {
                    on_press(console, event, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|console, event: &MouseMoveEvent, _, cx| {
                on_hover(console, event, cx);
            }))
            .on_scroll_wheel(cx.listener(|console, event: &ScrollWheelEvent, _, cx| {
                super::on_scroll(console, event, cx);
            }))
            .on_drag_move(
                cx.listener(|console, event: &DragMoveEvent<Placing>, _, cx| {
                    on_drag_over(console, event, cx);
                }),
            )
            .on_drop(cx.listener(|console, placing: &Placing, window, cx| {
                on_drop(console, *placing, window, cx);
            }))
            .child(stage_canvas)
            .when_some(connect_hint, |this, hint| {
                this.child(
                    div()
                        .absolute()
                        .top(SPACE_5)
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px(SPACE_6)
                                .py(SPACE_2)
                                .rounded(RADIUS_XL)
                                .bg(theme.elevated)
                                .border_1()
                                .border_color(theme.border)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(hint),
                        ),
                )
            })
            .when(empty, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(FONT_BODY)
                        .text_color(theme.fg_dim)
                        .child(
                            "Drag equipment from the palette to start this screen; bind tags in the inspector.",
                        ),
                )
            })
            .into_any_element()
    } else {
        empty_state(
            Icon::Factory,
            "Loading the screen",
            "Its equipment and connections are on the way.",
            Vec::new(),
            cx,
        )
        .into_any_element()
    };

    div()
        .id("mimic-edit")
        .track_focus(&focus)
        .key_context(EDIT)
        .on_action(cx.listener(|console, _: &Dismiss, window, cx| {
            if !escape(console, window, cx) {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|console, _: &super::MimicZoomIn, _, cx| {
            super::zoom_key(console, Some(true), cx);
        }))
        .on_action(cx.listener(|console, _: &super::MimicZoomOut, _, cx| {
            super::zoom_key(console, Some(false), cx);
        }))
        .on_action(cx.listener(|console, _: &super::MimicFit, _, cx| {
            super::zoom_key(console, None, cx);
        }))
        .flex()
        .flex_col()
        .size_full()
        .child(bar)
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    AnyView::from(palette_view).cached(
                        div()
                            .flex_none()
                            .w(PALETTE_WIDTH)
                            .h_full()
                            .style()
                            .clone(),
                    ),
                )
                .child(body)
                .child(
                    AnyView::from(inspector_view).cached(
                        div()
                            .flex_none()
                            .w(INSPECTOR_WIDTH)
                            .h_full()
                            .style()
                            .clone(),
                    ),
                ),
        )
        .into_any_element()
}

fn resize_cursor(handle: Handle) -> CursorStyle {
    match handle {
        Handle::NorthWest | Handle::SouthEast => CursorStyle::ResizeUpLeftDownRight,
        Handle::NorthEast | Handle::SouthWest => CursorStyle::ResizeUpRightDownLeft,
        Handle::North | Handle::South => CursorStyle::ResizeUpDown,
        Handle::East | Handle::West => CursorStyle::ResizeLeftRight,
    }
}

/// Esc: cancel the connect tool's source, then the tool, then the
/// selection; `false` when there was nothing to cancel.
fn escape(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    let Some(editor) = edits(console) else {
        return false;
    };
    if editor.from.is_some() {
        editor.from = None;
    } else if editor.tool == Tool::Connect {
        editor.tool = Tool::Select;
    } else if !editor.selection.is_empty() || editor.line.is_some() {
        editor.selection.clear();
        editor.line = None;
    } else {
        return false;
    }
    changed(console, window, cx);
    true
}

/// A clean editor follows the screen now on show.
fn follow_screen(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let screen = active(console);
    let state = console.state::<State>();
    let layout = match state.layouts.get(&screen) {
        Some(Loaded::Ready(layout)) => Layout::clone(layout),
        _ => return,
    };
    let Some(editor) = state.edit.as_deref_mut() else {
        return;
    };
    if editor.screen == screen || editor.history.dirty() {
        return;
    }
    editor.screen = screen;
    editor.history = History::new(layout);
    editor.selection.clear();
    editor.line = None;
    editor.from = None;
    editor.gesture = None;
    changed(console, window, cx);
}

/// What the bar shows.
struct Toolbar {
    dirty: bool,
    saving: bool,
    can_undo: bool,
    can_redo: bool,
    tool: Tool,
    line_type: &'static str,
    grid: bool,
    zoom: Option<f64>,
}

fn toolbar(
    theme: &Theme,
    screen: &str,
    bar: Toolbar,
    console: &Console,
    cx: &mut Context<Console>,
) -> AnyElement {
    let ids = console
        .state_ref::<Screens>()
        .map(Screens::ids)
        .unwrap_or_default();
    let hover = theme.hover;
    let tabs: Vec<AnyElement> = ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| {
            let selected = id == screen;
            let name = screen_name(console, &id);
            div()
                .id(ElementId::NamedInteger(
                    "mimic-edit-tab".into(),
                    u64::try_from(index).unwrap_or(0),
                ))
                .flex_none()
                .h(CONTROL_SM)
                .px(SPACE_4)
                .flex()
                .items_center()
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .text_size(FONT_SMALL)
                .text_color(if selected { theme.fg } else { theme.fg_muted })
                .when(selected, |this| this.bg(theme.selected))
                .when(!selected, |this| this.hover(move |style| style.bg(hover)))
                .on_click(cx.listener(move |console, _, window, cx| {
                    super::switch_screen(console, &id, window, cx);
                }))
                .child(name)
                .into_any_element()
        })
        .collect();
    let percent = bar
        .zoom
        .map_or_else(|| "—".to_owned(), |zoom| format!("{:.0}%", zoom * 100.0));
    let separator = || div().flex_none().w(px(1.0)).h(px(16.0)).bg(theme.border);
    let status = if bar.saving {
        "Saving…"
    } else if bar.dirty {
        "Unsaved changes"
    } else {
        "Saved"
    };
    let tool_segments = vec![
        Segment::new(
            "Select",
            bar.tool == Tool::Select,
            cx.listener(|console, _, window, cx| {
                run(console, Cmd::Tool(Tool::Select), window, cx);
            }),
        )
        .icon(Icon::Square),
        Segment::new(
            "Connect",
            bar.tool == Tool::Connect,
            cx.listener(|console, _, window, cx| {
                run(console, Cmd::Tool(Tool::Connect), window, cx);
            }),
        )
        .icon(Icon::Waypoints),
    ];
    let line_segments: Vec<Segment> = edit::LINE_TYPES
        .iter()
        .map(|&(kind, label)| {
            Segment::new(
                label,
                bar.line_type == kind,
                cx.listener(move |console, _, _, cx| {
                    if let Some(editor) = edits(console) {
                        editor.line_type = kind;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();
    let save = Button::new("mimic-edit-save", "Save")
        .icon(Icon::Download)
        .compact()
        .tooltip("Save  ⌘S")
        .disabled(!bar.dirty || bar.saving)
        .on_click(cx.listener(|console, _, window, cx| run(console, Cmd::Save, window, cx)));
    let save = if bar.dirty { save.primary() } else { save };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(44.0))
        .px(SPACE_7)
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.chrome)
        .child(
            div()
                .flex_none()
                .px(SPACE_4)
                .py(SPACE_1)
                .rounded(RADIUS_SM)
                .bg(theme.accent_soft)
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.accent)
                .child("EDITING"),
        )
        .child(
            div()
                .id("mimic-edit-tabs")
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_1)
                .overflow_x_scroll()
                .children(tabs),
        )
        .child(
            IconButton::new("mimic-edit-undo", Icon::ChevronLeft)
                .tooltip("Undo  ⌘Z")
                .disabled(!bar.can_undo)
                .on_click(cx.listener(|console, _, window, cx| run(console, Cmd::Undo, window, cx))),
        )
        .child(
            IconButton::new("mimic-edit-redo", Icon::ChevronRight)
                .tooltip("Redo  ⇧⌘Z")
                .disabled(!bar.can_redo)
                .on_click(cx.listener(|console, _, window, cx| run(console, Cmd::Redo, window, cx))),
        )
        .child(separator())
        .child(segmented("mimic-edit-tool", tool_segments, cx))
        .when(bar.tool == Tool::Connect, |this| {
            this.child(segmented("mimic-edit-line", line_segments, cx))
        })
        .child(separator())
        .child(
            IconButton::new("mimic-edit-grid", Icon::Columns)
                .tooltip("Grid  G")
                .active(bar.grid)
                .on_click(cx.listener(|console, _, window, cx| run(console, Cmd::Grid, window, cx))),
        )
        .child(
            Button::new("mimic-edit-zoom-out", "−")
                .compact()
                .tooltip("Zoom out  ⌘−")
                .on_click(cx.listener(|console, _, _, cx| super::zoom_key(console, Some(false), cx))),
        )
        .child(
            Button::new("mimic-edit-fit", percent)
                .compact()
                .tooltip("Fit the stage  ⌘0")
                .on_click(cx.listener(|console, _, _, cx| super::zoom_key(console, None, cx))),
        )
        .child(
            Button::new("mimic-edit-zoom-in", "+")
                .compact()
                .tooltip("Zoom in  ⌘+")
                .on_click(cx.listener(|console, _, _, cx| super::zoom_key(console, Some(true), cx))),
        )
        .child(separator())
        .child(
            div()
                .flex_none()
                .text_size(FONT_CAPTION)
                .text_color(if bar.dirty { theme.warning } else { theme.fg_dim })
                .whitespace_nowrap()
                .child(status),
        )
        .child(save)
        .child(
            Button::new("mimic-edit-done", "Done")
                .icon(Icon::Check)
                .compact()
                .tooltip("Leave edit mode  E")
                .on_click(cx.listener(|console, _, window, cx| run(console, Cmd::Toggle, window, cx))),
        )
        .into_any_element()
}

/// The bar's Edit button in the viewer (operators).
pub(super) fn edit_button(cx: &mut Context<Console>) -> AnyElement {
    Button::new("mimic-edit", "Edit")
        .icon(Icon::Pencil)
        .compact()
        .tooltip("Edit the screen  E")
        .on_click(cx.listener(|console, _, window, cx| run(console, Cmd::Toggle, window, cx)))
        .into_any_element()
}

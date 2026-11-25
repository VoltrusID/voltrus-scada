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
//! - **Live sessions** ([`live`]): on a server with collaboration the
//!   editor joins the screen's session; edits, undo and ⌘S go through it
//!   and the other editors' cursors and selections show on the stage.

mod binding;
mod inspector;
mod live;
mod palette;
mod panes;
mod stage;

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, Context, CursorStyle, DispatchPhase, Div,
    DragMoveEvent, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, Stateful,
    Subscription, WeakEntity, Window, actions, canvas, div, px,
};
use voltrus_scada_native::designer::arrange::{self, Align, Axis};
use voltrus_scada_native::designer::{self, Saved};
use voltrus_scada_native::mimic::edit::{
    self, Clip, Edit, Guide, Handle, History, PASTE_OFFSET, Placing, Rect, Snapping,
};
use voltrus_scada_native::mimic::{self, Layout};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, empty_state, segmented};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

pub(crate) use self::live::{on_link as on_design_link, on_server as on_design};
pub(super) use self::stage::paint_grid;
use super::paint::{Hmi, Tessellations};
use super::scene::StoreValues;
use super::{Drawn, Frame, Loaded, State, Wire, active, focus_handle, redraw, remember_revision};
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
        MimicEditCut,
        MimicEditForward,
        MimicEditBackward,
        DesignerGroup,
        DesignerUngroup,
        DesignerLock,
        DesignerAlignLeft,
        DesignerAlignCenter,
        DesignerAlignRight,
        DesignerAlignTop,
        DesignerAlignMiddle,
        DesignerAlignBottom,
        DesignerDistributeHorizontally,
        DesignerDistributeVertically,
        DesignerEditBinding,
        DesignerProject,
        DesignerTags,
        DesignerComponents,
    ]
);

/// Move the selection by `(dx, dy)` stage pixels (the arrows: 1, with ⇧ 10).
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = voltrus_console, no_json)]
pub struct MimicEditNudge(pub i32, pub i32);

/// The palette's width (`PANEL_WIDTH`).
const PALETTE_WIDTH: Pixels = px(288.0);
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
/// Loading the server's copy after a conflict ("Load theirs").
const RELOAD: &str = "mimic.reload";
/// The server's copy read again when the window comes back to the front,
/// to learn of a save made elsewhere.
pub(super) const REMOTE: &str = "mimic.remote";
/// The unsaved-changes dialog.
const DIALOG: &str = "mimic-unsaved";
/// The dialog when a save found the screen changed elsewhere.
const CONFLICT: &str = "mimic-conflict";

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
    /// `config.label`, for a type whose settings have none (the Designer
    /// lists a label for every component).
    Label,
    /// `config.opacity`, a property every component has.
    Opacity,
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
    /// The tag of the binding being edited.
    BindTag,
    /// A `{ref}` inserted into the binding's expression.
    Insert,
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
        /// Select only this widget (and its group) on a click.
        click: Option<String>,
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
    /// ⌘Q or the window closing.
    Quit,
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
    Pane(panes::Pane),
    Cut,
    Order(edit::Order),
    Group,
    Ungroup,
    Lock,
    Align(Align),
    Distribute(Axis),
    /// Open the binding editor on the selection's first bound property
    /// (or its label).
    EditBinding,
}

/// The placement menu of a tag dropped on the empty stage (`DropMenu`):
/// what it can become, the highlighted choice, where it was dropped.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DropMenu {
    tag: i64,
    /// The drop point on the stage.
    at: (f64, f64),
    /// The drop point in the window, where the menu opens.
    shown_at: Point<Pixels>,
    choices: Vec<arrange::Choice>,
    index: usize,
}

/// The edit mode asked for before the screen was in (`--edit`, E, the
/// palette), and the commands that came meanwhile (`--overlay`).
#[derive(Default)]
pub(crate) struct Launch {
    edit: bool,
    /// `--screen` with `--page mimic`: its id or position (1 is the
    /// overview), shown before editing.
    screen: Option<String>,
    queued: Vec<Cmd>,
}

/// A save the server refused because the screen changed since it was
/// loaded (409): its current revision, the server's words, and the leave
/// that waited on the save.
#[derive(Debug, Clone, PartialEq)]
struct Conflict {
    revision: String,
    message: String,
    after: Option<Leave>,
}

/// Unsaved edits kept when the session ended under them (the link
/// failed, the token expired): restored when the same user signs in to
/// the same server and opens the screen again.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    /// The server and user they belong to.
    session: (String, String),
    screen: String,
    /// The server's copy the edits started from, and the edits.
    saved: Layout,
    present: Layout,
    /// The revision they were based on: a save after the restore still
    /// conflicts when the screen changed meanwhile.
    base: Option<String>,
    /// The Designer was asked to switch to its screen after the sign-in.
    launched: bool,
}

/// The kept draft; one of the states that outlive a session
/// (`kept_across_sessions`).
#[derive(Debug, Default)]
pub(crate) struct Drafts {
    draft: Option<Draft>,
}

/// The session is ending: keep the editor's unsaved edits.
pub(crate) fn stash(console: &mut Console) {
    let Some(state) = console.state_ref::<State>() else {
        return;
    };
    let (Some(session), Some(editor)) = (state.session.clone(), state.edit.as_deref()) else {
        return;
    };
    if !editor.history.dirty() {
        return;
    }
    let draft = Draft {
        session,
        screen: editor.screen.clone(),
        saved: editor.history.saved().clone(),
        present: editor.layout().clone(),
        base: editor.base.clone(),
        launched: false,
    };
    console.state::<Drafts>().draft = Some(draft);
}

/// The signed-in server and user.
fn session_of(console: &Console) -> Option<(String, String)> {
    console
        .user
        .as_ref()
        .map(|(user, _)| (console.server.clone(), user.clone()))
}

/// A draft of this session waits: open the Designer on its screen once.
fn launch_draft(console: &mut Console) {
    if editing(console) || !console.may_configure() {
        return;
    }
    let session = session_of(console);
    let Some(draft) = console
        .state_ref::<Drafts>()
        .and_then(|drafts| drafts.draft.as_ref())
        .filter(|draft| !draft.launched && Some(&draft.session) == session.as_ref())
    else {
        return;
    };
    let screen = draft.screen.clone();
    // Another screen: wait for the list, so the launch can switch to it.
    let here = active(console) == screen;
    let listed = console
        .state_ref::<Screens>()
        .is_some_and(|screens| screens.listed);
    if !here && !listed {
        return;
    }
    if console
        .state_ref::<Launch>()
        .is_some_and(|launch| launch.edit)
    {
        return;
    }
    // Switching screens is asked once; on its own screen the draft waits
    // for the layout (the editor takes it when it opens).
    if !here && let Some(draft) = console.state::<Drafts>().draft.as_mut() {
        draft.launched = true;
    }
    let launch = console.state::<Launch>();
    launch.edit = true;
    if !here {
        launch.screen = Some(screen);
    }
}

/// The draft for `screen` in this session, taken.
fn take_draft(console: &mut Console, screen: &str) -> Option<Draft> {
    let session = session_of(console)?;
    let drafts = console.state::<Drafts>();
    if drafts
        .draft
        .as_ref()
        .is_some_and(|draft| draft.session == session && draft.screen == screen)
    {
        drafts.draft.take()
    } else {
        None
    }
}

/// The editor of the active screen.
pub(crate) struct Editor {
    screen: String,
    history: History,
    /// The revision the edits are based on (the save sends it, the server
    /// refuses the save when the screen changed since); `None` from a
    /// server without revisions.
    base: Option<String>,
    /// A refused save waiting for the user's answer.
    conflict: Option<Conflict>,
    /// The server's copy being loaded after "Load theirs", and the leave
    /// to run once it is in.
    reloading: Option<Option<Leave>>,
    /// The screen was saved elsewhere while this one has unsaved edits
    /// (the web's `remoteChanged`): the banner offers their copy.
    remote_changed: bool,
    /// The check for a save made elsewhere, in flight.
    checking: Option<i64>,
    /// The placement menu, open.
    menu: Option<DropMenu>,
    /// The inspector's binding editor, open on one property.
    bind: Option<binding::BindEditor>,
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
    /// The Designer's left column: its tab and the tag browser's search.
    pane: panes::Pane,
    tag_filter: Entity<TextInput>,
    picking: Option<Pick>,
    /// The inspector's fields and the widget they were made for.
    fields: Vec<(Slot, Entity<TextInput>)>,
    fields_for: Option<String>,
    palette_view: Entity<CachedView>,
    inspector_view: Entity<CachedView>,
    dialog_focus: FocusHandle,
    /// The live session (docs/collaboration.md), when the server has one.
    live: Option<live::Live>,
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

/// The editor is open or about to be: the mimic must be the draft.
pub(crate) fn wants_draft(console: &Console) -> bool {
    editing(console)
        || console
            .state_ref::<Launch>()
            .is_some_and(|launch| launch.edit)
}

/// `--edit` with `--page mimic`: edit once the screen is in.
pub(crate) fn launch(console: &mut Console, edit: bool, screen: Option<String>) {
    let launch = console.state::<Launch>();
    launch.edit = edit;
    launch.screen = screen;
}

/// Show the `--screen` asked for, once the screen list is in.
fn launch_screen(console: &mut Console) {
    let Some(screens) = console
        .state_ref::<Screens>()
        .filter(|screens| screens.listed)
    else {
        return;
    };
    let ids = screens.ids();
    let Some(wanted) = console.state::<Launch>().screen.take() else {
        return;
    };
    let target = wanted
        .parse::<usize>()
        .ok()
        .and_then(|position| ids.get(position.checked_sub(1)?).cloned())
        .or_else(|| ids.contains(&wanted).then_some(wanted));
    if let Some(id) = target {
        crate::console::screens::select(console, &id);
        super::sync(console);
    }
}

/// Enter the edit mode asked for, once the active screen's mimic is in
/// (called by the viewer's render; the entering itself runs after it).
pub(super) fn launch_ready(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.page == Page::Mimic && console.phase == Phase::Console {
        launch_draft(console);
    }
    let wanted = console
        .state_ref::<Launch>()
        .is_some_and(|launch| launch.edit);
    if !wanted || console.page != Page::Mimic || console.phase != Phase::Console {
        return;
    }
    if console
        .state_ref::<Launch>()
        .is_some_and(|launch| launch.screen.is_some())
    {
        // No REST from render: switch (and fetch) after this frame.
        cx.defer_in(window, |console, _, cx| {
            launch_screen(console);
            cx.notify();
        });
        return;
    }
    let screen = active(console);
    let ready = console
        .state_ref::<State>()
        .is_some_and(|state| matches!(state.layouts.get(&screen), Some(Loaded::Ready(_))));
    // Run mode showed the published copy: read the draft before editing.
    if ready && super::shown(console).is_some_and(|shown| shown.published) {
        cx.defer_in(window, move |console, _, cx| {
            super::forget(console, &screen);
            super::sync(console);
            cx.notify();
        });
        return;
    }
    if ready {
        console.state::<Launch>().edit = false;
        cx.defer_in(window, |console, window, cx| {
            if console.may_configure() && !editing(console) {
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
    let ready = matches!(
        console.state::<State>().layouts.get(&screen),
        Some(Loaded::Ready(_))
    );
    let draft = if ready {
        take_draft(console, &screen)
    } else {
        None
    };
    let state = console.state::<State>();
    let layout = match state.layouts.get(&screen) {
        Some(Loaded::Ready(layout)) => Layout::clone(layout),
        _ => return,
    };
    let mut base = state.revisions.get(&screen).cloned().flatten();
    let history = match draft {
        Some(draft) => {
            base = draft.base.or(base);
            History::restored(draft.saved, draft.present)
        }
        None => History::new(layout),
    };
    let restored = history.dirty();
    let query = cx.new(|cx| TextInput::new(cx).placeholder("Search symbols and widgets"));
    let tag_query = cx.new(|cx| TextInput::new(cx).placeholder("Find a tag by name or key"));
    let tag_filter = cx.new(|cx| TextInput::new(cx).placeholder("Search tags"));
    let subscriptions = vec![
        // Back to the front: has the screen been saved elsewhere?
        cx.observe_window_activation(window, |console, window, _| {
            if window.is_window_active() {
                check_remote(console);
            }
        }),
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
        cx.subscribe_in(
            &tag_filter,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Cancelled => focus_stage(console, window, cx),
                // Enter: the first match binds the selected component, or
                // opens the placement menu (a drop from the keyboard).
                InputEvent::Submitted => panes::submit_tag(console, window, cx),
                _ => cx.notify(),
            },
        ),
    ];
    let editor = Editor {
        screen,
        history,
        base,
        conflict: None,
        reloading: None,
        remote_changed: false,
        checking: None,
        menu: None,
        bind: None,
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
        pane: panes::Pane::default(),
        tag_filter,
        picking: None,
        fields: Vec::new(),
        fields_for: None,
        palette_view: CachedView::new(palette::render, cx),
        inspector_view: CachedView::new(inspector::render, cx),
        dialog_focus: cx.focus_handle(),
        live: None,
        _subscriptions: subscriptions,
        _field_subscriptions: Vec::new(),
    };
    let state = console.state::<State>();
    super::close_faceplate(state);
    state.cursor = None;
    state.hover = None;
    state.press = None;
    state.edit = Some(Box::new(editor));
    live::start(console, window, cx);
    focus_stage(console, window, cx);
    if restored {
        console.show_toast(
            "Your unsaved edits from before the sign-in are back; save to keep them",
            false,
            cx,
        );
    }
    cx.notify();
}

/// Back to the viewer: a screen with a published copy is read again as
/// operators see it (the draft just edited is not what they see).
fn exit(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    live::leave(console);
    let state = console.state::<State>();
    let screen = state.edit.as_ref().map(|editor| editor.screen.clone());
    // Others edited it too: the viewer reads it again.
    let shared = state.edit.as_deref().is_some_and(live::active);
    state.edit = None;
    if let Some(screen) = screen
        && (shared || state.published.contains(&screen))
    {
        super::forget(console, &screen);
    }
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
    if !console.may_configure() {
        console.show_toast(
            "Editing a screen needs the admin, demo or integration role",
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
        Leave::Quit => {
            exit(console, window, cx);
            console.shutdown();
            cx.quit();
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
        Some(Leave::Quit) => "quitting",
        _ => "leaving edit mode",
    };
    let name = screen_name(console, &editor.screen);
    let focus = editor.dialog_focus.clone();
    let own = focus.clone();
    div()
        .track_focus(&focus)
        .on_key_down(
            cx.listener(move |console, event: &KeyDownEvent, window, cx| {
                // Enter on a focused button (Cancel, Discard) is that button's.
                if event.keystroke.key == "enter" && own.is_focused(window) {
                    answer(console, Answer::Save, window, cx);
                    cx.stop_propagation();
                }
            }),
        )
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

/// ⌘S: `PUT` the layout's rows, based on the revision the edits started
/// from; `after` runs once the server has them.
fn save(console: &mut Console, after: Option<Leave>, cx: &mut Context<Console>) {
    let base = edits_ref(console).and_then(|editor| editor.base.clone());
    save_on(console, after, base, cx);
}

/// The save, based on revision `base` (the loaded one, or after a
/// conflict the one the user chose to overwrite).
fn save_on(
    console: &mut Console,
    after: Option<Leave>,
    base: Option<String>,
    cx: &mut Context<Console>,
) {
    if !console.may_configure() {
        console.show_toast(
            "Saving a screen needs the admin, demo or integration role",
            true,
            cx,
        );
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
    // Named as the web Designer saves (`saveLayoutToAPI`): the label, the
    // stored name, the primary tag's name, the type.
    let named = designer::named(&snapshot, &|tag| {
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
        Call::new(move |api| async move { designer::save(&api, &screen, rows, lines, base).await }),
    );
    cx.notify();
}

/// The save's answer: marked saved (the viewer shows the saved layout),
/// a conflict to resolve, or a toast and the edits kept. A leave waiting
/// on it runs only when nothing was edited while the save was on its way.
/// Also the server's copy loaded after "Load theirs".
pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    if key.name == RELOAD {
        reloaded(console, reply, cx);
        return;
    }
    if key.name == REMOTE {
        remote_checked(console, key.arg, reply, cx);
        return;
    }
    if key.name != SAVE {
        return;
    }
    let result = reply.take::<Saved>();
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
        Ok(Saved::Done { revision }) => {
            if let Some((screen, snapshot, named)) = sent {
                if editor.screen == screen {
                    editor.history.mark_saved(snapshot);
                    // A revision once known is never dropped: a save
                    // whose answer named none stays based on the last.
                    if revision.is_some() {
                        editor.base.clone_from(&revision);
                    }
                    editor.remote_changed = false;
                }
                remember_revision(state, &screen, revision);
                state.layouts.insert(screen, Loaded::Ready(Rc::new(named)));
            }
            let clean = edits_ref(console).is_none_or(|editor| !editor.history.dirty());
            match after {
                Some(_) if !clean => {
                    console.show_toast("Edited while saving; save again", true, cx);
                }
                Some(leave) => {
                    console.show_toast("Screen saved", false, cx);
                    proceed_later(console, leave, cx);
                }
                None => console.show_toast("Screen saved", false, cx),
            }
        }
        Ok(Saved::Conflict { revision, message }) => {
            editor.conflict = Some(Conflict {
                revision,
                message,
                after,
            });
            ask_conflict_later(console, cx);
        }
        Ok(Saved::Half(error)) => {
            console.show_toast(
                format!(
                    "Screen half saved: the equipment is in, the connections are not \
                     ({error}). The edits are kept; save again."
                ),
                true,
                cx,
            );
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

/// Open the conflict dialog.
fn ask_conflict(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = edits_ref(console) else {
        return;
    };
    let focus = editor.dialog_focus.clone();
    let overlay = Overlay::modal(CONFLICT, px(480.0), conflict_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

/// [`ask_conflict`] from a REST answer, which has no window.
fn ask_conflict_later(console: &mut Console, cx: &mut Context<Console>) {
    let Some(handle) = edits_ref(console).and_then(|editor| editor.window) else {
        return;
    };
    cx.spawn(async move |this, cx| {
        let _ = handle.update(cx, |_, window, cx| {
            let _ = this.update(cx, |console, cx| ask_conflict(console, window, cx));
        });
    })
    .detach();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resolve {
    /// Load the server's copy; the edits here are lost.
    Theirs,
    /// Save again over the revision the conflict named.
    Mine,
    Cancel,
}

fn resolve(console: &mut Console, answer: Resolve, window: &mut Window, cx: &mut Context<Console>) {
    let conflict = edits(console).and_then(|editor| editor.conflict.take());
    console.close_overlay(window, cx);
    let Some(conflict) = conflict else {
        return;
    };
    match answer {
        Resolve::Cancel => focus_stage(console, window, cx),
        Resolve::Mine => save_on(console, conflict.after, Some(conflict.revision), cx),
        Resolve::Theirs => load_theirs(console, conflict.after),
    }
    cx.notify();
}

/// Load the server's copy over the edits ("Load theirs"); `after` runs
/// once it is in.
fn load_theirs(console: &mut Console, after: Option<Leave>) {
    let Some(editor) = edits(console) else {
        return;
    };
    if editor.reloading.is_some() {
        return;
    }
    editor.reloading = Some(after);
    let screen = editor.screen.clone();
    console.call(
        CallKey::new(RELOAD),
        Call::new(move |api| async move {
            let (layout, revision) = designer::load(&api, &screen).await?;
            Ok((screen, layout, revision))
        }),
    );
}

/// Read the screen's revision again (the window came to the front), to
/// learn of a save made elsewhere, as the web Designer refetches on focus.
fn check_remote(console: &mut Console) {
    let Some(editor) = edits(console) else {
        return;
    };
    // A session shows others' edits as they come.
    if live::active(editor)
        || editor.base.is_none()
        || editor.saving.is_some()
        || editor.reloading.is_some()
        || editor.checking.is_some()
        || editor.conflict.is_some()
    {
        return;
    }
    editor.next_save += 1;
    let seq = editor.next_save;
    editor.checking = Some(seq);
    let screen = editor.screen.clone();
    console.call(
        CallKey::new(REMOTE).with(seq),
        Call::new(move |api| async move {
            let (layout, revision) = designer::load(&api, &screen).await?;
            Ok((screen, layout, revision))
        }),
    );
}

/// The server's copy, read on focus: a clean editor takes it; one with
/// unsaved edits shows the banner (the save would conflict anyway).
fn remote_checked(console: &mut Console, seq: i64, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some(editor) = state.edit.as_deref_mut() else {
        return;
    };
    if editor.checking != Some(seq) {
        return;
    }
    editor.checking = None;
    // A save or a reload since: their answer wins.
    if editor.saving.is_some() || editor.reloading.is_some() {
        return;
    }
    let Ok((screen, layout, Some(revision))) = reply.take::<(String, Layout, Option<String>)>()
    else {
        return;
    };
    if screen != editor.screen || editor.base.as_deref() == Some(revision.as_str()) {
        return;
    }
    if editor.history.dirty() {
        editor.remote_changed = true;
    } else {
        editor.history = History::new(layout.clone());
        editor.base = Some(revision.clone());
        editor.remote_changed = false;
        editor.prune();
        remember_revision(state, &screen, Some(revision));
        state.layouts.insert(screen, Loaded::Ready(Rc::new(layout)));
    }
    cx.notify();
}

/// The banner over the stage: the screen was saved elsewhere.
fn remote_banner(theme: &Theme, cx: &mut Context<Console>) -> AnyElement {
    div()
        .absolute()
        .top(SPACE_5)
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .id("designer-remote-changed")
                .flex()
                .items_center()
                .gap(SPACE_3)
                .pl(SPACE_5)
                .pr(SPACE_2)
                .py(SPACE_2)
                .rounded(RADIUS_XL)
                .bg(theme.elevated)
                .border_1()
                .border_color(theme.warning)
                .text_size(FONT_SMALL)
                .text_color(theme.fg)
                .child("This screen was saved elsewhere while you were editing.")
                .child(
                    Button::new("designer-remote-reload", "Load theirs")
                        .compact()
                        .on_click(cx.listener(|console, _, _, cx| {
                            load_theirs(console, None);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("designer-remote-keep", "Keep editing")
                        .compact()
                        .on_click(cx.listener(|console, _, _, cx| {
                            if let Some(editor) = edits(console) {
                                editor.remote_changed = false;
                            }
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

/// The server's copy is in: the editor starts over from it, and the leave
/// that waited runs.
fn reloaded(console: &mut Console, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some(editor) = state.edit.as_deref_mut() else {
        return;
    };
    let Some(after) = editor.reloading.take() else {
        return;
    };
    match reply.take::<(String, Layout, Option<String>)>() {
        Ok((screen, layout, revision)) if screen == editor.screen => {
            editor.history = History::new(layout.clone());
            if revision.is_some() {
                editor.base.clone_from(&revision);
            }
            editor.remote_changed = false;
            editor.prune();
            remember_revision(state, &screen, revision);
            state.layouts.insert(screen, Loaded::Ready(Rc::new(layout)));
            console.show_toast("Loaded the screen as saved elsewhere", false, cx);
            if let Some(leave) = after {
                proceed_later(console, leave, cx);
            }
        }
        Ok(_) => {}
        Err(error) => {
            console.show_toast(
                format!("The screen did not load: {error}. Your edits are kept."),
                true,
                cx,
            );
        }
    }
    cx.notify();
}

fn conflict_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let message = editor
        .conflict
        .as_ref()
        .map(|conflict| conflict.message.clone())
        .unwrap_or_default();
    let name = screen_name(console, &editor.screen);
    let focus = editor.dialog_focus.clone();
    div()
        .track_focus(&focus)
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
                .child("Screen changed elsewhere"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .p(SPACE_8)
                .child(format!("{message}.").replace("..", "."))
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(format!(
                            "Load their version of “{name}” (your changes here are lost), \
                             or overwrite it with yours?"
                        )),
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
                    Button::new("mimic-conflict-cancel", "Cancel")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            resolve(console, Resolve::Cancel, window, cx);
                        })),
                )
                .child(
                    Button::new("mimic-conflict-theirs", "Load theirs").on_click(cx.listener(
                        |console, _, window, cx| {
                            resolve(console, Resolve::Theirs, window, cx);
                        },
                    )),
                )
                .child(
                    Button::new("mimic-conflict-mine", "Overwrite with mine")
                        .danger()
                        .on_click(cx.listener(|console, _, window, cx| {
                            resolve(console, Resolve::Mine, window, cx);
                        })),
                ),
        )
        .into_any_element()
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
    .on_action(
        cx.listener(|console, _: &MimicEditConnectTool, window, cx| {
            run(console, Cmd::Tool(Tool::Connect), window, cx);
        }),
    )
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
    .on_action(cx.listener(|console, _: &MimicEditCut, window, cx| {
        run(console, Cmd::Cut, window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditForward, window, cx| {
        run(console, Cmd::Order(edit::Order::Forward), window, cx);
    }))
    .on_action(cx.listener(|console, _: &MimicEditBackward, window, cx| {
        run(console, Cmd::Order(edit::Order::Backward), window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerGroup, window, cx| {
        run(console, Cmd::Group, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerUngroup, window, cx| {
        run(console, Cmd::Ungroup, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerLock, window, cx| {
        run(console, Cmd::Lock, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerAlignLeft, window, cx| {
        run(console, Cmd::Align(Align::Left), window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerAlignCenter, window, cx| {
        run(console, Cmd::Align(Align::Center), window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerAlignRight, window, cx| {
        run(console, Cmd::Align(Align::Right), window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerAlignTop, window, cx| {
        run(console, Cmd::Align(Align::Top), window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerAlignMiddle, window, cx| {
        run(console, Cmd::Align(Align::Middle), window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerAlignBottom, window, cx| {
        run(console, Cmd::Align(Align::Bottom), window, cx);
    }))
    .on_action(
        cx.listener(|console, _: &DesignerDistributeHorizontally, window, cx| {
            run(console, Cmd::Distribute(Axis::Horizontal), window, cx);
        }),
    )
    .on_action(
        cx.listener(|console, _: &DesignerDistributeVertically, window, cx| {
            run(console, Cmd::Distribute(Axis::Vertical), window, cx);
        }),
    )
    .on_action(cx.listener(|console, _: &DesignerEditBinding, window, cx| {
        run(console, Cmd::EditBinding, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerProject, window, cx| {
        run(console, Cmd::Pane(panes::Pane::Project), window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerTags, window, cx| {
        run(console, Cmd::Pane(panes::Pane::Tags), window, cx);
    }))
    .on_action(cx.listener(|console, _: &DesignerComponents, window, cx| {
        run(console, Cmd::Pane(panes::Pane::Components), window, cx);
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
        Cmd::Save => {
            if !live::save(console, cx) {
                save(console, None, cx);
            }
        }
        Cmd::Pane(pane) => {
            if let Some(editor) = edits(console) {
                editor.pane = pane;
                // The tags pane is for searching: its field takes the keys.
                if pane == panes::Pane::Tags {
                    let filter = editor.tag_filter.clone();
                    filter.read(cx).focus(window);
                }
            }
            cx.notify();
        }
        Cmd::EditBinding => {
            let prop = edits_ref(console)
                .and_then(Editor::single)
                .and_then(|widget| {
                    let props = designer::properties(widget);
                    let bound = designer::bindings::bindings_of(&widget.config);
                    props
                        .iter()
                        .find(|prop| {
                            prop.bindable() && bound.iter().any(|(key, _)| key == prop.key)
                        })
                        .or_else(|| props.iter().find(|prop| prop.key == "label"))
                        .cloned()
                });
            match prop {
                Some(prop) => binding::open(console, &prop, window, cx),
                None => {
                    console.show_toast("Select one component to bind its properties", false, cx)
                }
            }
        }
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
            if live::undo(console, cmd == Cmd::Redo) {
                changed(console, window, cx);
                return;
            }
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
                // Unlocked widgets, groups whole (as the web's ⌘A).
                editor.selection = arrange::select_all(editor.history.present());
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
                // A group member brings its group (`expandToGroups`); the
                // stepped one stays last, so the next Tab goes on from it.
                let picked: Vec<String> = next.into_iter().collect();
                let mut selection: Vec<String> =
                    arrange::expand_to_groups(editor.history.present(), &picked)
                        .into_iter()
                        .filter(|id| !picked.contains(id))
                        .collect();
                selection.extend(picked);
                editor.selection = selection;
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
                    let gone = Edit::Remove {
                        widgets: Vec::new(),
                        connections: vec![line],
                    };
                    return Some((gone, false));
                }
                if editor.selection.is_empty() {
                    return None;
                }
                let widgets = arrange::deletable(editor.history.present(), &editor.selection);
                let kept = widgets.len() < editor.selection.len();
                editor.selection.retain(|id| !widgets.contains(id));
                let gone = Edit::Remove {
                    widgets,
                    connections: Vec::new(),
                };
                Some((gone, kept))
            });
            if let Some((gone, kept)) = gone {
                apply(console, &gone, None, window, cx);
                if kept {
                    console.show_toast(
                        "Locked components were kept; unlock them to delete",
                        false,
                        cx,
                    );
                }
            }
        }
        Cmd::Cut => {
            // Only what the cut removes is copied: locked components stay
            // on the stage and out of the clipboard, as Delete keeps them.
            if let Some(editor) = edits(console)
                && editor.line.is_none()
                && !editor.selection.is_empty()
            {
                let cut = arrange::deletable(editor.history.present(), &editor.selection);
                if cut.is_empty() {
                    console.show_toast(
                        "Locked components were kept; unlock them to cut",
                        false,
                        cx,
                    );
                    return;
                }
                editor.clip = edit::copy(editor.history.present(), &cut);
            }
            run(console, Cmd::Delete, window, cx);
        }
        Cmd::Nudge(_, dy) if menu_open(console) => {
            // The placement menu's cursor.
            if let Some(menu) = edits(console).and_then(|editor| editor.menu.as_mut()) {
                if dy > 0 {
                    menu.index = (menu.index + 1).min(menu.choices.len().saturating_sub(1));
                } else if dy < 0 {
                    menu.index = menu.index.saturating_sub(1);
                }
            }
            cx.notify();
        }
        Cmd::Nudge(dx, dy) => {
            let nudged = edits_ref(console).and_then(|editor| {
                let moved = arrange::nudge(
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
        Cmd::Front => run(console, Cmd::Order(edit::Order::Front), window, cx),
        Cmd::Back => run(console, Cmd::Order(edit::Order::Back), window, cx),
        Cmd::Order(to) => {
            let reorder = edits_ref(console)
                .filter(|editor| !editor.selection.is_empty())
                .map(|editor| Edit::Reorder {
                    ids: editor.selection.clone(),
                    to,
                });
            if let Some(reorder) = reorder {
                apply(console, &reorder, None, window, cx);
            }
        }
        Cmd::Group | Cmd::Ungroup | Cmd::Lock | Cmd::Align(_) | Cmd::Distribute(_) => {
            arrange_selection(console, cmd, window, cx);
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

/// Whether the placement menu is open.
fn menu_open(console: &Console) -> bool {
    edits_ref(console).is_some_and(|editor| editor.menu.is_some())
}

/// Group, ungroup, lock, align and distribute the selection: one undo
/// step each, nothing when there is nothing to change.
fn arrange_selection(
    console: &mut Console,
    cmd: Cmd,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(editor) = edits_ref(console) else {
        return;
    };
    let layout = editor.layout();
    let ids = &editor.selection;
    let changed = match cmd {
        Cmd::Group => arrange::group(layout, ids, &edit::fresh_id("g")),
        Cmd::Ungroup => arrange::ungroup(layout, ids),
        Cmd::Lock => arrange::toggle_lock(layout, ids),
        Cmd::Align(mode) => arrange::align(layout, ids, mode),
        Cmd::Distribute(axis) => arrange::distribute(layout, ids, axis),
        _ => None,
    };
    match changed {
        Some(changed) => apply(console, &changed, None, window, cx),
        None => {
            let why = match cmd {
                Cmd::Group => "Select two or more components to group them",
                Cmd::Align(_) => "Select two or more components to align them",
                Cmd::Distribute(_) => "Select three or more components to distribute them",
                _ => return,
            };
            if editor.selection.len() < 3 {
                console.show_toast(why, false, cx);
            }
        }
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
        && editor
            .history
            .apply_at(edit, key, Some(std::time::Instant::now()))
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
    // A number typed in the inspector belongs to the widget it was typed
    // for: commit it before the press moves focus and the selection.
    inspector::commit_typing(console, window, cx);
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
    // A press elsewhere closes the placement menu.
    editor.menu = None;
    if editor.tool == Tool::Connect {
        let layout = editor.history.present();
        let hit = edit::hit_widget(layout, x, y).map(|index| layout.widgets[index].id.clone());
        match hit {
            Some(id) => match editor.from.take() {
                Some(from) if from != id => {
                    let line = edit::connection(edit::fresh_id("c"), &from, &id, editor.line_type);
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
    // The single selection's handles and knob (a locked one has none).
    if let Some(widget) = editor.single().filter(|widget| !edit::is_locked(widget)) {
        let rect = Rect::of(widget);
        let id = widget.id.clone();
        let knob = edit::turns(widget) && near((x, y), knob_at(rect, zoom), HANDLE_HIT / zoom);
        if knob {
            editor.gesture = Some(Gesture::Knob);
            // The paint with `pressing` adds the window's move and release.
            changed(console, window, cx);
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
            changed(console, window, cx);
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
        let layout = editor.history.present();
        let selected = editor.selection.contains(&id);
        // A member of a group picks the whole group (`expandToGroups`);
        // ⇧ adds the group, or takes it out when it is all selected.
        editor.selection = arrange::press_selection(layout, &editor.selection, &id, shift);
        editor.line = None;
        if shift {
            changed(console, window, cx);
            return;
        }
        let click = selected.then(|| id.clone());
        // Locked widgets stay where they are: pressing one only selects.
        if edit::is_locked(layout.widget(&id).expect("hit")) {
            changed(console, window, cx);
            return;
        }
        let movable = arrange::movable(layout, &editor.selection);
        let picked: Vec<&mimic::Widget> = edit::picked(layout, &movable).collect();
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
            // What the band touches, groups whole, locked ones not
            // (`marqueeSelect`).
            for id in arrange::marquee(editor.history.present(), band) {
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
    // A gesture is one undo step; the next edit starts another.
    editor.history.seal();
    match gesture {
        Gesture::Move {
            moved: false,
            click,
            ..
        } => {
            if let Some(id) = click {
                editor.selection = arrange::expand_to_groups(editor.history.present(), &[id]);
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

// ── The placement menu ────────────────────────────────────────────────────

/// Open the placement menu for `tag` dropped at stage point `at`.
pub(super) fn open_menu(
    console: &mut Console,
    tag: i64,
    at: (f64, f64),
    shown_at: Point<Pixels>,
    cx: &mut Context<Console>,
) {
    let shape = console.store.get(tag).map(|tag| arrange::TagShape {
        key: &tag.info.key,
        name: &tag.info.name,
        unit: &tag.info.unit,
        min: tag.info.min_val,
        max: tag.info.max_val,
    });
    let choices = arrange::drop_choices(shape.as_ref());
    if let Some(editor) = edits(console) {
        editor.menu = Some(DropMenu {
            tag,
            at,
            shown_at,
            choices,
            index: 0,
        });
    }
    cx.notify();
}

/// A choice of the placement menu picked: the component, bound to the
/// tag, placed where the tag was dropped (one undo step) and selected.
fn pick_menu(console: &mut Console, index: usize, window: &mut Window, cx: &mut Context<Console>) {
    let Some(menu) = edits(console).and_then(|editor| editor.menu.take()) else {
        return;
    };
    let Some(choice) = menu.choices.get(index).copied() else {
        return;
    };
    let name = console.store.get(menu.tag).map(|tag| tag.info.name.clone());
    let widget = arrange::place_tag(
        choice,
        menu.tag,
        name.as_deref(),
        menu.at,
        edit::fresh_id("w"),
    );
    let id = widget.id.clone();
    let added = Edit::Add {
        widgets: vec![widget],
        connections: Vec::new(),
    };
    if let Some(editor) = edits(console) {
        editor.tool = Tool::Select;
        editor.from = None;
    }
    apply(console, &added, None, window, cx);
    if let Some(editor) = edits(console) {
        editor.selection = vec![id];
        editor.line = None;
    }
    changed(console, window, cx);
    focus_stage(console, window, cx);
}

/// The placement menu over the stage, at the drop point.
fn menu_view(
    menu: &DropMenu,
    tag_name: String,
    origin: Point<Pixels>,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let hover = theme.hover;
    let rows: Vec<AnyElement> = menu
        .choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let active = index == menu.index;
            div()
                .id(ElementId::NamedInteger(
                    "designer-drop-choice".into(),
                    u64::try_from(index).unwrap_or(0),
                ))
                .flex()
                .items_center()
                .h(px(26.0))
                .px(SPACE_4)
                .rounded(RADIUS_SM)
                .cursor_pointer()
                .text_size(FONT_SMALL)
                .text_color(if active { theme.fg } else { theme.fg_muted })
                .when(active, |this| this.bg(theme.selected))
                .when(!active, |this| this.hover(move |style| style.bg(hover)))
                .on_click(cx.listener(move |console, _, window, cx| {
                    pick_menu(console, index, window, cx);
                }))
                .child(choice.label)
                .into_any_element()
        })
        .collect();
    div()
        .id("designer-drop-menu")
        .absolute()
        .left(menu.shown_at.x - origin.x)
        .top(menu.shown_at.y - origin.y)
        .w(px(220.0))
        .flex()
        .flex_col()
        .p(SPACE_2)
        .gap(px(1.0))
        .rounded(RADIUS_MD)
        .bg(theme.elevated)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .px(SPACE_4)
                .py(SPACE_2)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(format!("Show {tag_name} as")),
        )
        .children(rows)
        .into_any_element()
}

/// A palette item dragged over the stage: where it would land.
fn on_drag_over(console: &mut Console, event: &DragMoveEvent<Placing>, cx: &mut Context<Console>) {
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
    state.bound.borrow_mut().frame();
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
    let live_bar = live::bar(editor);
    let peers = live::peer_marks(editor);
    let labels = live::cursor_labels(&peers, view_camera);
    let bar = Toolbar {
        dirty: editor.history.dirty(),
        saving: editor.saving.is_some(),
        can_undo: live_bar
            .as_ref()
            .map_or(editor.history.can_undo(), |b| b.can_undo),
        can_redo: live_bar
            .as_ref()
            .map_or(editor.history.can_redo(), |b| b.can_redo),
        live: live_bar,
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
    let remote_changed = editor.remote_changed && editor.conflict.is_none();
    let menu = editor.menu.clone().map(|menu| {
        let name = console
            .store
            .get(menu.tag)
            .map_or_else(|| format!("tag #{}", menu.tag), |tag| tag.info.name.clone());
        (menu, name)
    });
    let menu_origin = drawn.get().map(|drawn| drawn.bounds.origin);
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
        designing: true,
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
    let left = panes::render(console, AnyView::from(palette_view), window, cx);
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
            .on_drop(cx.listener(|console, tag: &panes::TagDrag, window, cx| {
                panes::drop_tag(console, tag, window, cx);
            }))
            .child(stage_canvas)
            .children(labels)
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
            .when(remote_changed, |this| this.child(remote_banner(&theme, cx)))
            .when_some(menu.zip(menu_origin), |this, ((menu, name), origin)| {
                this.child(menu_view(&menu, name, origin, &theme, cx))
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
        // Enter picks the placement menu's highlighted choice.
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "enter"
                && let Some(index) = edits_ref(console)
                    .and_then(|editor| editor.menu.as_ref())
                    .map(|menu| menu.index)
            {
                pick_menu(console, index, window, cx);
                cx.stop_propagation();
            }
        }))
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
                .child(div().flex_none().w(PALETTE_WIDTH).h_full().child(left))
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
    if editor.menu.is_some() {
        editor.menu = None;
    } else if editor.bind.is_some() {
        editor.bind = None;
    } else if editor.from.is_some() {
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
    editor.base = state.revisions.get(&screen).cloned().flatten();
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
    /// In a live session: its status and the other people.
    live: Option<live::Bar>,
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
    let status: String = if let Some(live) = &bar.live {
        live.status.clone()
    } else if bar.saving {
        "Saving…".into()
    } else if bar.dirty {
        "Unsaved changes".into()
    } else {
        "Saved".into()
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
    let in_session = bar.live.is_some();
    let save = Button::new("mimic-edit-save", "Save")
        .icon(Icon::Download)
        .compact()
        .tooltip(if in_session {
            "Write the session's edits to the draft now  ⌘S"
        } else {
            "Save  ⌘S"
        })
        .disabled(!in_session && (!bar.dirty || bar.saving))
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
                .child("DESIGNER"),
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
                .on_click(
                    cx.listener(|console, _, window, cx| run(console, Cmd::Undo, window, cx)),
                ),
        )
        .child(
            IconButton::new("mimic-edit-redo", Icon::ChevronRight)
                .tooltip("Redo  ⇧⌘Z")
                .disabled(!bar.can_redo)
                .on_click(
                    cx.listener(|console, _, window, cx| run(console, Cmd::Redo, window, cx)),
                ),
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
                .on_click(
                    cx.listener(|console, _, window, cx| run(console, Cmd::Grid, window, cx)),
                ),
        )
        .child(
            Button::new("mimic-edit-zoom-out", "−")
                .compact()
                .tooltip("Zoom out  ⌘−")
                .on_click(
                    cx.listener(|console, _, _, cx| super::zoom_key(console, Some(false), cx)),
                ),
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
                .on_click(
                    cx.listener(|console, _, _, cx| super::zoom_key(console, Some(true), cx)),
                ),
        )
        .child(separator())
        .child(
            div()
                .flex_none()
                .text_size(FONT_CAPTION)
                .text_color(if bar.dirty {
                    theme.warning
                } else {
                    theme.fg_dim
                })
                .whitespace_nowrap()
                .child(status),
        )
        .when_some(
            bar.live
                .as_ref()
                .map(|live| live::avatars(&live.peers, theme)),
            |this, avatars| this.child(avatars),
        )
        .child(save)
        .child(
            Button::new("mimic-edit-done", "Done")
                .icon(Icon::Check)
                .compact()
                .tooltip("Close the Designer  E")
                .on_click(
                    cx.listener(|console, _, window, cx| run(console, Cmd::Toggle, window, cx)),
                ),
        )
        .into_any_element()
}

/// The bar's Edit button in the viewer (operators).
pub(super) fn edit_button(cx: &mut Context<Console>) -> AnyElement {
    Button::new("mimic-edit", "Designer")
        .icon(Icon::Pencil)
        .compact()
        .tooltip("Open the screen in the Designer  E")
        .on_click(cx.listener(|console, _, window, cx| run(console, Cmd::Toggle, window, cx)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{Focusable, KeyUpEvent, Keystroke, Modifiers, TestAppContext, point};
    use voltrus_scada_native::mimic::{Widget, WidgetKind};

    use super::*;
    use crate::console::testing::Harness;

    fn tank(id: &str, x: f64) -> Widget {
        Widget {
            id: id.into(),
            kind: WidgetKind::parse("tank_level"),
            name: String::new(),
            tags: Vec::new(),
            signals: Vec::new(),
            x,
            y: 200.0,
            width: 100.0,
            height: 100.0,
            config: serde_json::Map::new(),
        }
    }

    /// Editing a screen with tanks `a` (x 200) and `b` (x 600), `a`
    /// selected, the stage focused and drawn.
    fn editing_a(cx: &mut TestAppContext) -> Harness {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| console.show(Page::Mimic, window, cx));
        h.draw();
        h.update(|console, window, cx| {
            let screen = active(console);
            let layout = Layout {
                widgets: vec![tank("a", 200.0), tank("b", 600.0)],
                connections: Vec::new(),
            };
            console
                .state::<State>()
                .layouts
                .insert(screen, Loaded::Ready(Rc::new(layout)));
            enter(console, window, cx);
            if let Some(editor) = edits(console) {
                editor.selection = vec!["a".into()];
            }
            changed(console, window, cx);
        });
        h.draw();
        h.calls();
        h
    }

    fn widget(h: &mut Harness, id: &str) -> Widget {
        h.update(|console, _, _| {
            edits_ref(console)
                .and_then(|editor| editor.layout().widget(id).cloned())
                .expect("the widget")
        })
    }

    /// A stage point in window pixels, as last drawn.
    fn on_window(h: &mut Harness, (x, y): (f64, f64)) -> Point<Pixels> {
        let drawn = h
            .update(|console, _, _| console.state::<State>().drawn.get())
            .expect("the stage is drawn");
        let camera = drawn.camera;
        #[allow(clippy::cast_possible_truncation)]
        let at = |origin: Pixels, offset: f64, value: f64| {
            origin + px((offset + value * camera.zoom) as f32)
        };
        point(
            at(drawn.bounds.origin.x, camera.x, x),
            at(drawn.bounds.origin.y, camera.y, y),
        )
    }

    fn click(h: &mut Harness, at: Point<Pixels>) {
        h.cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
        h.cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    }

    fn dirty(h: &mut Harness) -> bool {
        h.update(|console, _, _| edits_ref(console).is_some_and(|e| e.history.dirty()))
    }

    fn move_a(h: &mut Harness, x: f64) {
        h.update(|console, window, cx| {
            let moved = Edit::Move(vec![("a".into(), x, 200.0)]);
            apply(console, &moved, None, window, cx);
        });
    }

    fn saves(h: &mut Harness) -> Vec<CallKey> {
        h.calls()
            .into_iter()
            .filter(|key| key.name == SAVE)
            .collect()
    }

    fn last_toast(h: &mut Harness) -> String {
        h.update(|console, _, _| {
            console
                .toasts
                .last()
                .map(|toast| toast.message.to_string())
                .unwrap_or_default()
        })
    }

    /// Done with an unsaved edit: the dialog, focused.
    fn asking(h: &mut Harness) {
        move_a(h, 300.0);
        h.update(|console, window, cx| run(console, Cmd::Toggle, window, cx));
        h.draw();
    }

    /// Enter pressed and released: a focused button clicks on the release
    /// (`simulate_keystrokes` sends only the press).
    fn press_enter(h: &mut Harness) {
        h.keys("enter");
        h.cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("enter").expect("a keystroke"),
        });
    }

    /// Tab `times` from the dialog: focus is on one of its buttons.
    fn tab_to_button(h: &mut Harness, times: usize) {
        for _ in 0..times {
            h.keys("tab");
        }
        let (on_dialog, in_overlay) = h.update(|console, window, cx| {
            let on_dialog =
                edits_ref(console).is_some_and(|editor| editor.dialog_focus.is_focused(window));
            let in_overlay = console
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.focus.contains_focused(window, cx));
            (on_dialog, in_overlay)
        });
        assert!(!on_dialog && in_overlay, "Tab moved to a dialog button");
    }

    #[gpui::test]
    fn enter_on_the_unsaved_dialog_saves(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        asking(&mut h);
        h.keys("enter");
        assert_eq!(saves(&mut h).len(), 1);
    }

    #[gpui::test]
    fn enter_on_the_focused_cancel_saves_nothing(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        asking(&mut h);
        tab_to_button(&mut h, 1);
        press_enter(&mut h);
        assert!(saves(&mut h).is_empty(), "Cancel is not Save");
        assert!(h.update(|console, _, _| editing(console)), "still editing");
        assert!(dirty(&mut h), "the edit is kept");
    }

    #[gpui::test]
    fn enter_on_the_focused_discard_discards(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        asking(&mut h);
        tab_to_button(&mut h, 2);
        press_enter(&mut h);
        assert!(saves(&mut h).is_empty(), "the discarded edit is not PUT");
        assert!(
            !h.update(|console, _, _| editing(console)),
            "left the editor"
        );
    }

    #[gpui::test]
    fn an_edit_while_saving_to_leave_stays_in_the_editor(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        asking(&mut h);
        h.keys("enter");
        let save = saves(&mut h).pop().expect("the save");
        move_a(&mut h, 400.0);
        h.reply(save, Reply::ok(Saved::Done { revision: None }));
        h.draw();
        assert!(h.update(|console, _, _| editing(console)), "not left");
        assert!(dirty(&mut h), "the later edit is unsaved");
        assert_eq!(last_toast(&mut h), "Edited while saving; save again");
    }

    #[gpui::test]
    fn a_half_written_save_says_so(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        move_a(&mut h, 300.0);
        h.update(|console, _, cx| save(console, None, cx));
        let save = saves(&mut h).pop().expect("the save");
        h.reply(save, Reply::ok(Saved::Half("500 connections".to_owned())));
        assert!(dirty(&mut h), "not marked saved");
        let toast = last_toast(&mut h);
        assert!(toast.contains("half saved"), "{toast}");
    }

    #[gpui::test]
    fn pressing_a_resize_handle_starts_the_drag(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        let rect = Rect::of(&widget(&mut h, "a"));
        let from = on_window(&mut h, Handle::SouthEast.point(rect));
        let to = point(from.x + px(40.0), from.y + px(40.0));
        h.cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
        h.cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
        h.cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        assert!(widget(&mut h, "a").width > 100.0, "the drag resized");
    }

    #[gpui::test]
    fn separate_nudge_runs_are_separate_undo_steps(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        let nudge = |h: &mut Harness| {
            h.update(|console, window, cx| run(console, Cmd::Nudge(1, 0), window, cx));
        };
        nudge(&mut h);
        nudge(&mut h);
        // A click on the selected widget ends the run.
        let centre = on_window(&mut h, (250.0, 250.0));
        click(&mut h, centre);
        nudge(&mut h);
        h.update(|console, window, cx| run(console, Cmd::Undo, window, cx));
        assert_eq!(widget(&mut h, "a").x, 202.0, "one undo takes back one run");
    }

    #[gpui::test]
    fn a_typed_number_survives_a_click_on_another_widget(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        h.update(|console, window, cx| {
            let field = edits_ref(console)
                .and_then(|editor| editor.fields.iter().find(|(slot, _)| *slot == Slot::X))
                .map(|(_, field)| field.clone())
                .expect("the X field");
            field.read(cx).focus(window);
            field.update(cx, |field, cx| field.set_text("50", cx));
        });
        let on_b = on_window(&mut h, (650.0, 250.0));
        click(&mut h, on_b);
        assert_eq!(widget(&mut h, "a").x, 50.0, "the number went to a");
        assert_eq!(widget(&mut h, "b").x, 600.0);
        let selected =
            h.update(|console, _, _| edits_ref(console).map(|editor| editor.selection.clone()));
        assert_eq!(selected, Some(vec!["b".to_owned()]));
    }

    fn selection(h: &mut Harness) -> Vec<String> {
        h.update(|console, _, _| {
            edits_ref(console)
                .map(|editor| editor.selection.clone())
                .unwrap_or_default()
        })
    }

    fn undo_depth(h: &mut Harness) -> usize {
        h.update(|console, window, cx| {
            let mut depth = 0;
            let start = edits_ref(console).map(|editor| editor.layout().clone());
            while edits(console).is_some_and(|editor| editor.history.undo()) {
                depth += 1;
            }
            while edits(console).is_some_and(|editor| editor.history.redo()) {}
            changed(console, window, cx);
            assert_eq!(start, edits_ref(console).map(|e| e.layout().clone()));
            depth
        })
    }

    #[gpui::test]
    fn a_tag_dropped_on_the_empty_stage_offers_components_and_places_one(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        let mut level = Harness::tag(42, "raw_tank_level");
        level.device_id = "wtp".into();
        level.unit = "%".into();
        h.event(crate::console::Event::Tags(vec![level]));
        let at = on_window(&mut h, (1000.0, 600.0));
        h.update(|console, _, cx| open_menu(console, 42, (1000.0, 600.0), at, cx));
        h.draw();
        let first = h.update(|console, _, _| {
            edits_ref(console)
                .and_then(|editor| editor.menu.as_ref())
                .map(|menu| menu.choices[0].label)
        });
        assert_eq!(first, Some("Tank (ISA-101)"), "a level tag: the tank first");
        // Down, then Enter: the second choice.
        h.update(|console, window, cx| run(console, Cmd::Nudge(0, 1), window, cx));
        h.keys("enter");
        let placed = selection(&mut h);
        assert_eq!(placed.len(), 1);
        let widget = widget(&mut h, &placed[0]);
        assert_eq!(widget.kind.as_str(), "big_number");
        assert_eq!(widget.tags, [42]);
        assert_eq!(widget.x, 1000.0 - widget.width / 2.0);
        assert!(h.update(|console, _, _| edits_ref(console).is_some_and(|e| e.menu.is_none())));
        assert_eq!(undo_depth(&mut h), 1, "one undo step");
        // Esc closes the menu without placing anything.
        h.update(|console, _, cx| open_menu(console, 42, (100.0, 100.0), at, cx));
        h.keys("escape");
        assert!(h.update(|console, _, _| edits_ref(console).is_some_and(|e| e.menu.is_none())));
        assert_eq!(undo_depth(&mut h), 1);
    }

    #[gpui::test]
    fn a_group_is_picked_and_dragged_whole_and_a_locked_widget_stays(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        h.update(|console, window, cx| {
            if let Some(editor) = edits(console) {
                editor.selection = vec!["a".into(), "b".into()];
            }
            run(console, Cmd::Group, window, cx);
            if let Some(editor) = edits(console) {
                editor.selection.clear();
            }
            changed(console, window, cx);
        });
        h.draw();
        // A click on b picks a too.
        let on_b = on_window(&mut h, (650.0, 250.0));
        click(&mut h, on_b);
        assert_eq!(selection(&mut h), ["a", "b"]);
        // A drag on b moves both.
        let to = point(on_b.x + px(60.0), on_b.y);
        h.cx.simulate_mouse_down(on_b, MouseButton::Left, Modifiers::none());
        h.cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
        h.cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        let (ax, bx) = (widget(&mut h, "a").x, widget(&mut h, "b").x);
        assert!(ax > 200.0 && bx > 600.0, "both moved: {ax} {bx}");
        assert_eq!(bx - ax, 400.0, "together");
        // ⌘L locks both: a drag no longer moves them, ⌫ keeps them.
        h.update(|console, window, cx| run(console, Cmd::Lock, window, cx));
        assert!(edit::is_locked(&widget(&mut h, "a")));
        h.draw();
        let on_a = on_window(&mut h, (ax + 50.0, 250.0));
        let to = point(on_a.x + px(80.0), on_a.y + px(40.0));
        h.cx.simulate_mouse_down(on_a, MouseButton::Left, Modifiers::none());
        h.cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
        h.cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        assert_eq!(widget(&mut h, "a").x, ax, "locked: not moved");
        h.update(|console, window, cx| run(console, Cmd::Delete, window, cx));
        assert_eq!(widget(&mut h, "b").x, bx, "locked: not deleted");
        assert_eq!(
            last_toast(&mut h),
            "Locked components were kept; unlock them to delete"
        );
    }

    #[gpui::test]
    fn cut_leaves_locked_components_out_of_the_clipboard(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        h.update(|console, window, cx| {
            if let Some(editor) = edits(console) {
                editor.selection = vec!["b".into()];
            }
            run(console, Cmd::Lock, window, cx);
            if let Some(editor) = edits(console) {
                editor.selection = vec!["a".into(), "b".into()];
            }
            run(console, Cmd::Cut, window, cx);
        });
        let ids = |h: &mut Harness| {
            h.update(|console, _, _| {
                edits_ref(console)
                    .map(|editor| {
                        editor
                            .layout()
                            .widgets
                            .iter()
                            .map(|w| w.id.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
        };
        assert_eq!(ids(&mut h), ["b"], "a cut, the locked b kept");
        assert_eq!(
            last_toast(&mut h),
            "Locked components were kept; unlock them to delete"
        );
        h.update(|console, window, cx| run(console, Cmd::Paste, window, cx));
        assert_eq!(ids(&mut h).len(), 2, "only a was copied");
        // Only locked ones selected: nothing is cut or copied.
        h.update(|console, window, cx| {
            if let Some(editor) = edits(console) {
                editor.selection = vec!["b".into()];
                editor.clip = edit::Clip::default();
            }
            run(console, Cmd::Cut, window, cx);
            run(console, Cmd::Paste, window, cx);
        });
        assert_eq!(ids(&mut h).len(), 2);
        assert_eq!(
            last_toast(&mut h),
            "Locked components were kept; unlock them to cut"
        );
    }

    #[gpui::test]
    fn align_and_distribute_are_one_undo_step_each(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        h.update(|console, window, cx| {
            let added = Edit::Add {
                widgets: vec![Widget {
                    y: 500.0,
                    ..tank("c", 1500.0)
                }],
                connections: Vec::new(),
            };
            apply(console, &added, None, window, cx);
            if let Some(editor) = edits(console) {
                editor.selection = vec!["a".into(), "b".into(), "c".into()];
            }
            run(console, Cmd::Align(Align::Top), window, cx);
        });
        assert_eq!(widget(&mut h, "c").y, 200.0);
        h.update(|console, window, cx| {
            run(console, Cmd::Distribute(Axis::Horizontal), window, cx);
        });
        assert_eq!(widget(&mut h, "b").x, 850.0);
        h.update(|console, window, cx| run(console, Cmd::Undo, window, cx));
        assert_eq!(widget(&mut h, "b").x, 600.0, "distribute undone");
        assert_eq!(widget(&mut h, "c").y, 200.0, "align kept");
        h.update(|console, window, cx| run(console, Cmd::Undo, window, cx));
        assert_eq!(widget(&mut h, "c").y, 500.0, "align undone in one step");
    }

    #[gpui::test]
    fn any_property_binds_to_an_expression_with_a_live_preview(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        let mut run_tag = Harness::tag(5, "p101a_run");
        run_tag.device_id = "wtp".into();
        h.event(crate::console::Event::Tags(vec![run_tag]));
        h.update(|console, window, cx| {
            let prop = designer::properties(edits_ref(console).unwrap().single().unwrap())
                .into_iter()
                .find(|prop| prop.key == "visible")
                .expect("visible");
            binding::open(console, &prop, window, cx);
            let bind = edits(console)
                .and_then(|editor| editor.bind.as_mut())
                .unwrap();
            bind.mode = binding::Mode::Expr;
            let expr = bind.expr.clone();
            expr.update(cx, |field, cx| field.set_text("{wtp/p101a_run} > 0", cx));
        });
        h.draw();
        let draft = h.update(|console, _, cx| {
            edits_ref(console)
                .and_then(|editor| editor.bind.as_ref())
                .map(|bind| bind.draft(cx))
        });
        assert_eq!(
            draft,
            Some(designer::Binding::Expr {
                expr: "{wtp/p101a_run} > 0".into()
            })
        );
        // Apply is a button: press it through its handler.
        h.update(|console, window, cx| {
            let a = edits_ref(console).unwrap().single().unwrap().clone();
            let draft = edits_ref(console).unwrap().bind.as_ref().unwrap().draft(cx);
            let edit = designer::set_binding(&a, "visible", Some(&draft));
            apply(console, &edit, None, window, cx);
            binding::close(console, window, cx);
        });
        let bound = designer::bindings::binding_of(&widget(&mut h, "a").config, "visible");
        assert!(matches!(bound, Some(designer::Binding::Expr { .. })));
        // A tag picker field offers no binding.
        let props = designer::properties(&Widget {
            config: serde_json::json!({ "symbol": "pipe" })
                .as_object()
                .unwrap()
                .clone(),
            kind: WidgetKind::Symbol,
            ..tank("p", 0.0)
        });
        assert!(
            props
                .iter()
                .any(|p| p.key == "runSensorId" && !p.bindable())
        );
    }

    #[gpui::test]
    fn quitting_with_unsaved_edits_asks_first(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        assert!(!h.update(|console, window, cx| console.holds_quit(window, cx)));
        move_a(&mut h, 300.0);
        assert!(h.update(|console, window, cx| console.holds_quit(window, cx)));
        let asking = h.update(|console, _, _| edits_ref(console).and_then(|e| e.asking.clone()));
        assert_eq!(asking, Some(Leave::Quit));
    }

    #[gpui::test]
    fn a_conflicting_save_asks_and_can_overwrite(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        h.update(|console, _, _| {
            if let Some(editor) = edits(console) {
                editor.base = Some("rev-1".into());
            }
        });
        move_a(&mut h, 300.0);
        h.update(|console, _, cx| save(console, None, cx));
        let first = saves(&mut h).pop().expect("the save");
        h.reply(
            first,
            Reply::ok(Saved::Conflict {
                revision: "rev-2".into(),
                message: "This screen was changed by someone else since you opened it".into(),
            }),
        );
        h.draw();
        let conflict = h.update(|console, _, _| {
            edits_ref(console)
                .and_then(|editor| editor.conflict.as_ref().map(|c| c.revision.clone()))
        });
        assert_eq!(conflict.as_deref(), Some("rev-2"));
        assert!(dirty(&mut h), "nothing saved");
        h.update(|console, window, cx| resolve(console, Resolve::Mine, window, cx));
        let again = saves(&mut h).pop().expect("saved again over rev-2");
        h.reply(
            again,
            Reply::ok(Saved::Done {
                revision: Some("rev-3".into()),
            }),
        );
        assert!(!dirty(&mut h));
        let base = h.update(|console, _, _| edits_ref(console).and_then(|e| e.base.clone()));
        assert_eq!(base.as_deref(), Some("rev-3"), "the next save builds on it");
    }

    /// Answer the pending mimic read with `view`.
    fn answer_layout(h: &mut Harness, view: designer::View) {
        let key = h
            .calls()
            .into_iter()
            .find(|key| key.name == "mimic.layout")
            .expect("the mimic read");
        h.reply(key, Reply::ok(view));
        h.draw();
    }

    fn layout_with_a(x: f64) -> Layout {
        Layout {
            widgets: vec![tank("a", x)],
            connections: Vec::new(),
        }
    }

    #[gpui::test]
    fn run_mode_shows_the_published_screen_and_the_editor_the_draft(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| console.show(Page::Mimic, window, cx));
        h.draw();
        answer_layout(
            &mut h,
            designer::View {
                layout: layout_with_a(200.0),
                revision: Some("r1".into()),
                published: true,
                draft_differs: true,
            },
        );
        assert!(h.update(|console, _, _| crate::console::views::mimic::draft_note_shown(console)));
        // Edit: the published rows are not what the editor changes; the
        // draft is read first.
        h.update(toggle);
        h.draw();
        assert!(!h.update(|console, _, _| editing(console)));
        answer_layout(
            &mut h,
            designer::View {
                layout: layout_with_a(300.0),
                revision: Some("r1".into()),
                published: false,
                draft_differs: false,
            },
        );
        h.draw();
        let x = h.update(|console, _, _| {
            edits_ref(console).and_then(|e| e.layout().widget("a").map(|w| w.x))
        });
        assert_eq!(x, Some(300.0), "the editor holds the draft");
        assert!(!h.update(|console, _, _| crate::console::views::mimic::draft_note_shown(console)));
        // Done: run mode reads what operators see again.
        h.update(exit);
        h.draw();
        assert!(
            h.calls().iter().any(|key| key.name == "mimic.layout"),
            "the published copy is read again"
        );
    }

    #[gpui::test]
    fn operators_get_no_draft_note(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("operator");
        h.update(|console, window, cx| console.show(Page::Mimic, window, cx));
        h.draw();
        answer_layout(
            &mut h,
            designer::View {
                layout: layout_with_a(200.0),
                revision: Some("r1".into()),
                published: true,
                draft_differs: true,
            },
        );
        assert!(!h.update(|console, _, _| crate::console::views::mimic::draft_note_shown(console)));
    }

    /// The screen's layout read on focus, answered with revision `rev`.
    fn answer_remote(h: &mut Harness, layout: Layout, rev: &str) {
        let key = h
            .calls()
            .into_iter()
            .find(|key| key.name == REMOTE)
            .expect("the check");
        let screen = h.update(|console, _, _| active(console));
        h.reply(key, Reply::ok((screen, layout, Some(rev.to_owned()))));
        h.draw();
    }

    #[gpui::test]
    fn a_save_elsewhere_shows_the_banner_or_loads_into_a_clean_editor(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        let theirs = Layout {
            widgets: vec![tank("t", 100.0)],
            connections: Vec::new(),
        };
        h.update(|console, _, _| {
            if let Some(editor) = edits(console) {
                editor.base = Some("rev-1".into());
            }
            check_remote(console);
        });
        // Clean: their copy comes in.
        answer_remote(&mut h, theirs.clone(), "rev-2");
        let (ids, base) = h.update(|console, _, _| {
            let editor = edits_ref(console).expect("editing");
            (
                editor
                    .layout()
                    .widgets
                    .iter()
                    .map(|w| w.id.clone())
                    .collect::<Vec<_>>(),
                editor.base.clone(),
            )
        });
        assert_eq!(ids, ["t"]);
        assert_eq!(base.as_deref(), Some("rev-2"));
        // Unsaved edits: the banner, the edits kept.
        h.update(|console, window, cx| {
            let moved = Edit::Move(vec![("t".into(), 300.0, 200.0)]);
            apply(console, &moved, None, window, cx);
            check_remote(console);
        });
        answer_remote(&mut h, theirs.clone(), "rev-3");
        let banner = h.update(|console, _, _| edits_ref(console).is_some_and(|e| e.remote_changed));
        assert!(banner);
        assert!(dirty(&mut h));
        // Same revision: nothing to say.
        h.update(|console, _, _| {
            if let Some(editor) = edits(console) {
                editor.remote_changed = false;
                editor.base = Some("rev-3".into());
            }
            check_remote(console);
        });
        answer_remote(&mut h, theirs, "rev-3");
        let banner = h.update(|console, _, _| edits_ref(console).is_some_and(|e| e.remote_changed));
        assert!(!banner);
    }

    #[gpui::test]
    fn unsaved_edits_outlive_a_failed_link_and_come_back_after_sign_in(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        h.update(|console, _, _| {
            if let Some(editor) = edits(console) {
                editor.base = Some("rev-1".into());
            }
        });
        move_a(&mut h, 320.0);
        h.event(crate::console::Event::Link(
            voltrus_scada_native::runtime::Link::Failed("Session expired".into()),
        ));
        assert!(
            h.update(|console, _, _| !editing(console)),
            "the session ended"
        );
        let kept = h.update(|console, _, _| {
            console
                .state_ref::<Drafts>()
                .and_then(|drafts| drafts.draft.as_ref().map(|d| d.present.widgets[0].x))
        });
        assert_eq!(kept, Some(320.0));
        h.sign_in("admin");
        h.update(|console, window, cx| {
            console.show(Page::Mimic, window, cx);
            let screen = active(console);
            let layout = Layout {
                widgets: vec![tank("a", 200.0), tank("b", 600.0)],
                connections: Vec::new(),
            };
            console
                .state::<State>()
                .layouts
                .insert(screen, Loaded::Ready(Rc::new(layout)));
        });
        // The viewer opens the Designer on the draft's screen by itself.
        h.draw();
        h.draw();
        assert!(h.update(|console, _, _| editing(console)));
        assert_eq!(widget(&mut h, "a").x, 320.0, "the edit is back");
        assert!(dirty(&mut h));
        let base = h.update(|console, _, _| edits_ref(console).and_then(|e| e.base.clone()));
        assert_eq!(
            base.as_deref(),
            Some("rev-1"),
            "still based on what it started from"
        );
        assert_eq!(
            last_toast(&mut h),
            "Your unsaved edits from before the sign-in are back; save to keep them"
        );
        // Undo goes back to the server's copy.
        h.update(|console, window, cx| run(console, Cmd::Undo, window, cx));
        assert_eq!(widget(&mut h, "a").x, 200.0);
    }

    #[gpui::test]
    fn enter_in_the_tag_search_places_or_binds_from_the_keyboard(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        let mut flow = Harness::tag(9, "intake_flow");
        flow.device_id = "wtp".into();
        h.event(crate::console::Event::Tags(vec![flow]));
        // One component selected: Enter binds it.
        h.update(|console, window, cx| {
            run(console, Cmd::Pane(panes::Pane::Tags), window, cx);
            let filter = edits_ref(console).unwrap().tag_filter.clone();
            assert!(
                filter.read(cx).focus_handle(cx).is_focused(window),
                "the search has the keys"
            );
            filter.update(cx, |field, cx| field.set_text("intake", cx));
            panes::submit_tag(console, window, cx);
        });
        assert_eq!(widget(&mut h, "a").tags, [9]);
        // Nothing selected: the placement menu, in the middle of the stage.
        h.update(|console, window, cx| {
            if let Some(editor) = edits(console) {
                editor.selection.clear();
            }
            panes::submit_tag(console, window, cx);
        });
        let menu = h.update(|console, _, _| edits_ref(console).and_then(|e| e.menu.clone()));
        let menu = menu.expect("the placement menu");
        assert_eq!(menu.tag, 9);
        assert!(
            (menu.at.0 - 960.0).abs() < 1.0 && (menu.at.1 - 540.0).abs() < 1.0,
            "{:?}",
            menu.at
        );
        // ⌘B opens the binding editor on the label of the selected component.
        h.update(|console, window, cx| {
            if let Some(editor) = edits(console) {
                editor.menu = None;
                editor.selection = vec!["a".into()];
            }
            run(console, Cmd::EditBinding, window, cx);
        });
        let open = h.update(|console, _, _| {
            edits_ref(console).and_then(|e| e.bind.as_ref().map(|b| b.prop))
        });
        assert_eq!(open, Some("label"));
    }

    // ── Live sessions ──

    use voltrus_collab::wire::{self, Handled};
    use voltrus_collab::yrs::{Doc, Transact};
    use voltrus_scada_native::designer::collab;
    use voltrus_scada_native::runtime::{Command, Link};
    use voltrus_scada_native::ws_types::{ClientMessage, DesignLimits, ServerMessage};

    /// The server's part of a session: answers the console's frames from
    /// `doc`; the design frames the console sent.
    fn serve(h: &mut Harness, doc: &Doc) -> Vec<ClientMessage> {
        let mut seen = Vec::new();
        for _ in 0..10 {
            let frames: Vec<ClientMessage> = h
                .commands()
                .into_iter()
                .filter_map(|c| match c {
                    Command::Design(m) => Some(m),
                    _ => None,
                })
                .collect();
            if frames.is_empty() {
                break;
            }
            for frame in frames {
                let answers = match &frame {
                    ClientMessage::DesignJoin { screen, .. } => vec![
                        ServerMessage::DesignJoined {
                            screen: screen.clone(),
                            epoch: "e1".into(),
                            resync: false,
                            user: "admin".into(),
                            color: "#3366ff".into(),
                            protocol: 1,
                            limits: DesignLimits {
                                max_update: 262_144,
                                max_rate: 50,
                                max_doc: 5 << 20,
                                max_awareness: 4096,
                                flush_ms: 2000,
                            },
                        },
                        ServerMessage::DesignSync {
                            screen: screen.clone(),
                            data: wire::step1(doc),
                        },
                    ],
                    ClientMessage::DesignSync { screen, data } => {
                        match wire::handle(doc, "console", wire::decode(data).unwrap()).unwrap() {
                            Handled::Reply(reply) => vec![ServerMessage::DesignSync {
                                screen: screen.clone(),
                                data: reply,
                            }],
                            Handled::Applied => Vec::new(),
                        }
                    }
                    _ => Vec::new(),
                };
                seen.push(frame);
                for answer in answers {
                    h.event(crate::console::Event::Design(answer));
                }
            }
        }
        seen
    }

    fn server_doc() -> Doc {
        let layout = Layout {
            widgets: vec![tank("a", 200.0), tank("b", 600.0)],
            connections: Vec::new(),
        };
        let (eq, cn) = mimic::to_rows(&layout);
        let doc = Doc::new();
        voltrus_collab::document::load(&mut doc.transact_mut(), &collab::collab_rows(&eq, &cn));
        doc
    }

    fn join(h: &mut Harness, doc: &Doc) {
        h.update(|console, _, _| live::on_link(console, &Link::Live));
        serve(h, doc);
    }

    fn tick(h: &mut Harness) {
        std::thread::sleep(collab::BATCH);
        h.update(|console, _, cx| {
            live::tick(console, cx);
        });
    }

    #[gpui::test]
    fn a_live_session_shares_edits_and_saves_through_the_server(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        let doc = server_doc();
        join(&mut h, &doc);
        assert!(h.update(|console, _, _| edits_ref(console).is_some_and(live::active)));
        // An edit goes into the server's document; nothing is unsaved.
        move_a(&mut h, 333.0);
        tick(&mut h);
        serve(&mut h, &doc);
        let server = voltrus_collab::document::read(&doc.transact());
        assert_eq!(server.widgets["a"].0.x, 333.0);
        assert!(!dirty(&mut h));
        // A remote edit shows; the selection stays.
        {
            use voltrus_collab::yrs::ReadTxn;
            let sv = doc.transact().state_vector();
            let mut txn = doc.transact_mut();
            let mut b = voltrus_collab::document::read(&txn).widgets["b"].0.clone();
            b.y = 444.0;
            voltrus_collab::document::put_widget(&mut txn, "b", Some(&b), None);
            drop(txn);
            let update = doc.transact().encode_diff_v1(&sv);
            let screen = h.update(|console, _, _| active(console));
            h.event(crate::console::Event::Design(ServerMessage::DesignSync {
                screen,
                data: wire::update(update),
            }));
        }
        assert_eq!(widget(&mut h, "b").y, 444.0);
        assert_eq!(
            h.update(|console, _, _| edits_ref(console).map(|e| e.selection.clone())),
            Some(vec!["a".to_string()])
        );
        // Undo takes back the console's move only.
        h.update(|console, window, cx| run(console, Cmd::Undo, window, cx));
        assert_eq!(widget(&mut h, "a").x, 200.0);
        assert_eq!(widget(&mut h, "b").y, 444.0);
        // ⌘S asks the server to write the draft: no PUT.
        h.update(|console, window, cx| run(console, Cmd::Save, window, cx));
        let sent = serve(&mut h, &doc);
        assert!(
            sent.iter()
                .any(|m| matches!(m, ClientMessage::DesignFlush { .. }))
        );
        assert!(saves(&mut h).is_empty());
    }

    #[gpui::test]
    fn without_collaboration_the_editor_saves_layouts(cx: &mut TestAppContext) {
        let mut h = editing_a(cx);
        h.update(|console, _, _| live::on_link(console, &Link::Live));
        let screen = h.update(|console, _, _| active(console));
        assert!(
            h.commands()
                .iter()
                .any(|c| matches!(c, Command::Design(ClientMessage::DesignJoin { .. })))
        );
        h.event(crate::console::Event::Design(ServerMessage::DesignError {
            screen,
            code: "disabled".into(),
            message: "Collaboration is off".into(),
            reload: false,
        }));
        move_a(&mut h, 300.0);
        tick(&mut h);
        assert!(dirty(&mut h), "edits stay local until saved");
        h.update(|console, window, cx| run(console, Cmd::Save, window, cx));
        assert_eq!(saves(&mut h).len(), 1, "a whole-layout save");
    }
}

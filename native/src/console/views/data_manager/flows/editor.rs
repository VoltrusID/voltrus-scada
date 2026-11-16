//! The native flow editor: a branching graph on a GPUI canvas, Node-RED
//! style, laid out like the console's other editors.
//!
//! - Palette (left): the server's node types (`GET …/flows/node-types`,
//!   or the legacy engine's steps from an older server) grouped by
//!   category, with a filter box, and the trigger types. Enter or a click
//!   adds a node, wired from the selected node's first output; drag one
//!   onto the canvas to place it there. A trigger type replaces the
//!   trigger with its defaults.
//! - Canvas (centre, [`super::canvas`]): drag the background or scroll to
//!   pan, ⌘-scroll or + − 0 to zoom about the pointer, drag a card to move
//!   it, drag from an output port onto another card to wire them, click a
//!   card or a wire to select it, ⌫ removes it, ⌘D duplicates a node,
//!   ⌘Z / ⇧⌘Z undo and redo.
//! - Right panel: Settings (the selection's fields, from the catalogue's
//!   field definitions, required ones checked) or Test (a dry run of the
//!   canvas as it is, its trace listed and its path lit on the canvas).
//!
//! The trigger is the graph's virtual source node `trigger`; its place on
//! the canvas is this editor's own (the server stores none). ⌘S saves
//! (`PUT`): a v2 graph, or, for a server without the graph engine, the
//! legacy array when the graph is a straight line. Users who may not edit
//! (below operator) see the flow read-only.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, Context, Entity, EntityInputHandler, FocusHandle, Focusable, FontWeight,
    Hsla, KeyDownEvent, Pixels, SharedString, Subscription, Window, div, px,
};
use serde_json::Value;
use voltrus_scada_native::api::eng_flows::{
    Catalogue, Category, End, FieldDef, FieldKind, Flow, FlowGraph, Path, RESERVED, TEST_INPUT,
    TestRun, config_id, config_type, field_text, parse_field, parse_input, pretty, preview,
    test_path, value_kind,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, kbd, pill, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::canvas::{self, Colors, Drawn, DrawnWire, Mark, Scene};
use super::model::{self, Geometry, Hit, NODE_H, NODE_W, Vec2, View, snap};
use super::{LIST, State, forms, now_millis, now_secs, refresh, tone_color};
use crate::console::Console;
use crate::console::keys::{
    Dismiss, EngFlowsAddStep, EngFlowsArrange, EngFlowsDeleteSelection, EngFlowsDuplicate,
    EngFlowsFit, EngFlowsInspect, EngFlowsNudgeDown, EngFlowsNudgeLeft, EngFlowsNudgeRight,
    EngFlowsNudgeUp, EngFlowsRedo, EngFlowsRuns, EngFlowsSave, EngFlowsTest, EngFlowsUndo,
    EngFlowsZoomIn, EngFlowsZoomOut, FocusNext, FocusPrevious,
};
use crate::console::overlay::Overlay;
use crate::console::text_area::TextArea;
use crate::console::views::mimic::{listen_for_pinches, pinch::Gesture};
use crate::console::views::{ListKeys, focus_ring, keyboard_list};

/// The palette's keyboard list.
const PALETTE_LIST: &str = "data.flows.palette";
/// Key contexts: the whole editor (⌘S works in its fields too) and the
/// canvas (`keys::ENG_FLOW_EDITOR`, `keys::ENG_FLOW_CANVAS`).
const EDITOR_CONTEXT: &str = "EngFlowEditor";
const CANVAS_CONTEXT: &str = "EngFlowCanvas";
const DISCARD: &str = "data-flow-discard";
const PALETTE_W: f32 = 210.0;
const INSPECTOR_W: f32 = 320.0;
/// What the editor's chrome takes from the window's height, for the
/// canvas's size before it was first laid out.
const CHROME_H: f32 = 200.0;
/// Arrow keys move a card this far (⇧: further), or pan the view.
const NUDGE: f32 = 10.0;
const NUDGE_FAR: f32 = 50.0;
const PAN_STEP: f32 = 60.0;
const UNDO_DEPTH: usize = 100;
/// Suggestions shown under a tag or device field.
const SUGGESTIONS: usize = 6;

/// What is selected on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    None,
    Node(End),
    /// Edge `n` of the graph.
    Wire(usize),
}

/// What the pointer is doing.
#[derive(Debug, Clone, Copy)]
enum Drag {
    /// Moving the view: where the press was (canvas point) and the pan
    /// then.
    Pan { from: Vec2, pan: Vec2 },
    /// Moving a card: where on it it was taken (world).
    Move { end: End, grab: Vec2, moved: bool },
    /// A new wire from `from`'s output `port` to the pointer (world), and
    /// the node it would land on.
    Wire {
        from: End,
        port: usize,
        to: Vec2,
        target: Option<usize>,
    },
}

/// One palette entry: a catalogue trigger or node type, by position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Item {
    Trigger(usize),
    Node(usize),
}

/// A palette entry on its way to the canvas.
#[derive(Clone)]
pub(crate) struct PaletteDrag {
    item: Item,
    label: SharedString,
    color: Hsla,
}

impl Render for PaletteDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        div()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .px(SPACE_6)
            .h(px(28.0))
            .rounded(RADIUS_MD)
            .bg(theme.elevated)
            .border_1()
            .border_color(self.color)
            .shadow(theme.shadow(Elevation::Popover))
            .text_size(FONT_SMALL)
            .text_color(theme.fg)
            .child(div().size(px(8.0)).rounded(px(4.0)).bg(self.color))
            .child(self.label.clone())
    }
}

/// The right panel's tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Settings,
    Test,
}

/// The settings panel's fields for one selection.
#[derive(Default)]
struct Inspector {
    /// What the fields were made for; `None` makes them again.
    shows: Option<Selection>,
    fields: Vec<Field>,
    _subscriptions: Vec<Subscription>,
}

struct Field {
    def: FieldDef,
    input: Input,
    /// The text does not read as the field's kind.
    error: Option<String>,
}

enum Input {
    Line(Entity<TextInput>),
    Area(Entity<TextArea>),
    /// A true/false setting: Off/On, read from the graph.
    Switch,
    /// One of the field's choices, read from the graph.
    Choice,
    /// Read-only: the value as text.
    Fixed,
}

/// The graph, the trigger's place and the selection before an edit.
#[derive(Clone)]
struct Snapshot {
    graph: FlowGraph,
    trigger_at: Vec2,
    selection: Selection,
}

/// The test panel.
pub(crate) struct TestPanel {
    input: Entity<TextArea>,
    /// The last run (and whether it was the legacy, stored-flow test), or
    /// why it failed.
    result: Option<Result<(TestRun, bool), String>>,
    running: bool,
    seq: i64,
    /// The trace row whose input and output show.
    picked: Option<usize>,
    /// A legacy test runs the stored outputs for real: the first press
    /// only warns.
    armed: bool,
    _subscription: Subscription,
}

/// The editor while a flow is open.
pub(crate) struct Editor {
    /// The flow as the server has it.
    flow: Flow,
    /// Its graph as stored, to tell unsaved changes.
    saved: FlowGraph,
    /// Why the stored texts could not be read (the canvas starts empty
    /// and saving replaces them).
    broken: Option<String>,
    /// The server stored the legacy array: shown converted.
    legacy: bool,
    graph: FlowGraph,
    trigger_at: Vec2,
    view: View,
    selection: Selection,
    drag: Option<Drag>,
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    /// Consecutive edits with this key (typing in one field) are one undo
    /// step.
    last_key: Option<String>,
    name: Entity<TextInput>,
    enabled: bool,
    canvas_focus: FocusHandle,
    /// Where the canvas was laid out last.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    inspector: Inspector,
    /// Enter or a double click: focus the first field once it is made.
    focus_inspector: bool,
    /// The outcome of the last edit or save; errors in red.
    message: Option<(String, bool)>,
    saving: bool,
    /// The save the editor waits for.
    save_seq: i64,
    filter: Entity<TextInput>,
    side: Side,
    test: TestPanel,
    /// Run now was pressed once: the next press runs.
    run_armed: bool,
    running_now: bool,
    /// The user may look but not change (below operator).
    read_only: bool,
    catalogue: Rc<Catalogue>,
    /// Whether the server has the graph engine; `None` until it answered.
    graph_server: Option<bool>,
    discard_focus: Option<FocusHandle>,
    /// The window's height, to place trackpad gestures.
    window_height: f32,
    _subscriptions: Vec<Subscription>,
}

fn editor_mut(console: &mut Console) -> Option<&mut Editor> {
    console.state::<State>().editor.as_mut()
}

/// The catalogue the section knows: the server's, else the built-in.
pub(super) fn catalogue_of(state: &mut State) -> Rc<Catalogue> {
    state
        .catalogue
        .get_or_insert_with(|| Rc::new(Catalogue::builtin()))
        .clone()
}

/// Open `flow` on the canvas.
pub(super) fn open(
    console: &mut Console,
    flow: Flow,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let read_only = !console.is_operator();
    let state = console.state::<State>();
    let catalogue = catalogue_of(state);
    let graph_server = state.graph_server;
    let input_text = state
        .test_payload
        .clone()
        .unwrap_or_else(|| TEST_INPUT.to_owned());
    let (graph, legacy, broken) = match FlowGraph::of(&flow) {
        Ok((graph, legacy)) => (graph, legacy, None),
        Err(error) => {
            let mut graph = FlowGraph::default();
            if let Some(trigger) = catalogue.triggers.first() {
                graph.set_trigger_kind(trigger);
            }
            (graph, false, Some(error))
        }
    };
    let (x, y) = graph.trigger_spot();
    #[allow(clippy::cast_possible_truncation)]
    let trigger_at = Vec2::new(x as f32, y as f32);
    let name_text = flow.name.clone();
    let name = cx.new(|cx| TextInput::new(cx).text(name_text).placeholder("Flow name"));
    let filter = cx.new(|cx| TextInput::new(cx).placeholder("Filter nodes"));
    let input = cx.new(|cx| TextArea::new(6, cx).placeholder("{\"payload\": …}   ⌘↵ run"));
    input.update(cx, |area, cx| {
        area.replace_text_in_range(None, &input_text, window, cx);
    });
    let subscriptions = vec![
        cx.subscribe_in(
            &name,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted | InputEvent::Cancelled => focus_canvas(console, window),
                InputEvent::Changed => cx.notify(),
                InputEvent::Blurred => {}
            },
        ),
        cx.subscribe_in(
            &filter,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => {
                    console.list(PALETTE_LIST, cx).cursor = None;
                    cx.notify();
                }
                // Enter adds the first match.
                InputEvent::Submitted => palette_activate(console, 0, false, window, cx),
                InputEvent::Cancelled => focus_canvas(console, window),
                InputEvent::Blurred => {}
            },
        ),
    ];
    let test_subscription = cx.subscribe_in(
        &input,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => run_test(console, cx),
            InputEvent::Cancelled => focus_canvas(console, window),
            InputEvent::Changed => {
                if let Some(editor) = editor_mut(console)
                    && editor.test.armed
                {
                    editor.test.armed = false;
                    cx.notify();
                }
            }
            InputEvent::Blurred => {}
        },
    );
    let mut editor = Editor {
        flow,
        saved: graph.clone(),
        broken,
        legacy,
        graph,
        trigger_at,
        view: View::default(),
        selection: Selection::None,
        drag: None,
        past: Vec::new(),
        future: Vec::new(),
        last_key: None,
        name,
        enabled: false,
        canvas_focus: cx.focus_handle(),
        bounds: Rc::new(Cell::new(None)),
        inspector: Inspector::default(),
        focus_inspector: false,
        message: None,
        saving: false,
        save_seq: 0,
        filter,
        side: Side::Settings,
        test: TestPanel {
            input,
            result: None,
            running: false,
            seq: 0,
            picked: None,
            armed: false,
            _subscription: test_subscription,
        },
        run_armed: false,
        running_now: false,
        read_only,
        catalogue,
        graph_server,
        discard_focus: None,
        window_height: f32::from(window.viewport_size().height),
        _subscriptions: subscriptions,
    };
    editor.enabled = editor.flow.enabled;
    editor.view = View::fit(geometry(&editor).bounds(), estimated_size(window));
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.editor = Some(editor);
    console.list(PALETTE_LIST, cx).cursor = None;
    listen_for_pinches(console, cx);
    focus_canvas(console, window);
    cx.notify();
}

/// Open `flow` with the test panel showing (T in the list).
pub(super) fn open_testing(
    console: &mut Console,
    flow: Flow,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    open(console, flow, window, cx);
    if let Some(editor) = editor_mut(console) {
        editor.side = Side::Test;
        let input = editor.test.input.clone();
        input.read(cx).focus(window);
    }
}

pub(super) fn focus_canvas(console: &mut Console, window: &mut Window) {
    if let Some(editor) = editor_mut(console) {
        window.focus(&editor.canvas_focus);
    }
}

/// The server's catalogue arrived (or it has none): an open editor uses it.
pub(super) fn catalogue_changed(console: &mut Console) {
    let state = console.state::<State>();
    let catalogue = catalogue_of(state);
    let graph_server = state.graph_server;
    if let Some(editor) = state.editor.as_mut() {
        editor.catalogue = catalogue;
        editor.graph_server = graph_server;
        editor.inspector.shows = None;
    }
}

/// The canvas's size before it is laid out: the window less the sidebar,
/// the palette, the panel and the bars.
fn estimated_size(window: &Window) -> Vec2 {
    let viewport = window.viewport_size();
    Vec2::new(
        (f32::from(viewport.width - super::super::sidebar_width()) - PALETTE_W - INSPECTOR_W)
            .max(200.0),
        (f32::from(viewport.height) - CHROME_H).max(200.0),
    )
}

fn canvas_size(editor: &Editor, window: &Window) -> Vec2 {
    editor.bounds.get().map_or_else(
        || estimated_size(window),
        |bounds| Vec2::new(f32::from(bounds.size.width), f32::from(bounds.size.height)),
    )
}

fn geometry(editor: &Editor) -> Geometry {
    Geometry::build(&editor.graph, editor.trigger_at, &editor.catalogue)
}

/// Focus the panel's first text field; false when it has none.
fn focus_first_field(editor: &Editor, window: &mut Window, cx: &gpui::App) -> bool {
    for field in &editor.inspector.fields {
        match &field.input {
            Input::Line(line) => {
                line.read(cx).focus(window);
                return true;
            }
            Input::Area(area) => {
                area.read(cx).focus(window);
                return true;
            }
            Input::Switch | Input::Choice | Input::Fixed => {}
        }
    }
    false
}

fn dirty(editor: &Editor, cx: &gpui::App) -> bool {
    editor.broken.is_some()
        || editor.graph != editor.saved
        || editor.enabled != editor.flow.enabled
        || editor.name.read(cx).value().trim() != editor.flow.name
}

// ── Edits ────────────────────────────────────────────────────────────────

fn select(editor: &mut Editor, selection: Selection) {
    if editor.selection != selection {
        editor.selection = selection;
        editor.inspector.shows = None;
        editor.last_key = None;
    }
}

fn snapshot(editor: &Editor) -> Snapshot {
    Snapshot {
        graph: editor.graph.clone(),
        trigger_at: editor.trigger_at,
        selection: editor.selection,
    }
}

/// Keep the state before an edit for ⌘Z. Edits with the same `key` in a
/// row (typing in one field) are one step.
fn remember(editor: &mut Editor, key: Option<String>) {
    if key.is_some() && key == editor.last_key {
        return;
    }
    if editor.past.len() == UNDO_DEPTH {
        editor.past.remove(0);
    }
    editor.past.push(snapshot(editor));
    editor.future.clear();
    editor.last_key = key;
}

fn restore(editor: &mut Editor, to: Snapshot) {
    editor.graph = to.graph;
    editor.trigger_at = to.trigger_at;
    editor.selection = to.selection;
    editor.inspector.shows = None;
    editor.drag = None;
    editor.last_key = None;
}

fn undo(editor: &mut Editor) {
    if editor.read_only {
        return;
    }
    match editor.past.pop() {
        Some(before) => {
            editor.future.push(snapshot(editor));
            restore(editor, before);
            say(editor, "Undone", false);
        }
        None => say(editor, "Nothing to undo", true),
    }
}

fn redo(editor: &mut Editor) {
    if editor.read_only {
        return;
    }
    match editor.future.pop() {
        Some(after) => {
            editor.past.push(snapshot(editor));
            restore(editor, after);
            say(editor, "Redone", false);
        }
        None => say(editor, "Nothing to redo", true),
    }
}

fn say(editor: &mut Editor, text: impl Into<String>, error: bool) {
    editor.message = Some((text.into(), error));
}

/// Refuse an edit in a read-only editor, saying why.
fn locked(editor: &mut Editor) -> bool {
    if editor.read_only {
        say(
            editor,
            "Read-only: changing flows needs the operator role",
            true,
        );
    }
    editor.read_only
}

/// Wire `from`'s output `port` into node `to`.
fn connect(editor: &mut Editor, from: End, port: usize, to: usize) {
    let catalogue = Rc::clone(&editor.catalogue);
    let mut graph = editor.graph.clone();
    match graph.connect(from, port, to, &catalogue) {
        Ok(true) => {
            remember(editor, None);
            editor.graph = graph;
            let edge = editor.graph.edges.len() - 1;
            select(editor, Selection::Wire(edge));
            say(
                editor,
                format!(
                    "{} now feeds {}",
                    port_label(&editor.graph, from, port, &catalogue),
                    editor.graph.label(End::Node(to), &catalogue)
                ),
                false,
            );
        }
        Ok(false) => say(editor, "That wire is there already", false),
        Err(error) => say(editor, error, true),
    }
}

/// `Switch · hot`, `Filter`.
fn port_label(graph: &FlowGraph, end: End, port: usize, catalogue: &Catalogue) -> String {
    let label = graph.label(end, catalogue);
    let outputs = graph.outputs(end, catalogue);
    match outputs.get(port) {
        Some(name) if outputs.len() > 1 && !name.is_empty() => format!("{label} · {name}"),
        _ => label,
    }
}

/// Add a palette entry: a node at `at` (or near the selection, wired from
/// its first output), or the trigger's type.
fn add_item(editor: &mut Editor, item: Item, at: Option<Vec2>, size: Vec2) {
    if locked(editor) {
        return;
    }
    let catalogue = Rc::clone(&editor.catalogue);
    match item {
        Item::Trigger(index) => {
            let Some(kind) = catalogue.triggers.get(index) else {
                return;
            };
            remember(editor, None);
            editor.graph.set_trigger_kind(kind);
            editor.graph.prune_ports(&catalogue);
            editor.inspector.shows = None;
            select(editor, Selection::Node(End::Trigger));
            say(
                editor,
                format!("The trigger is now {} (its defaults)", kind.label()),
                false,
            );
        }
        Item::Node(index) => {
            let Some(kind) = catalogue.nodes.get(index) else {
                return;
            };
            let geometry = geometry(editor);
            let after = match editor.selection {
                Selection::Node(end) => Some(end),
                _ => None,
            };
            let place = match (at, after.and_then(|end| geometry.card(end))) {
                (Some(at), _) => snap(at),
                (None, Some(card)) => {
                    geometry.free_spot(snap(card.at + Vec2::new(NODE_W + 70.0, 0.0)))
                }
                (None, None) => geometry.free_spot(snap(
                    editor.view.visible(size).center() - Vec2::new(NODE_W / 2.0, NODE_H / 2.0),
                )),
            };
            let id = editor.graph.fresh_id(now_millis());
            remember(editor, None);
            let index = editor
                .graph
                .add(kind, id, f64::from(place.x), f64::from(place.y));
            let mut fed = String::new();
            if at.is_none()
                && let Some(from) = after
                && !editor.graph.outputs(from, &catalogue).is_empty()
                && editor.graph.connect(from, 0, index, &catalogue) == Ok(true)
            {
                fed = format!(
                    ", fed by {}",
                    port_label(&editor.graph, from, 0, &catalogue)
                );
            }
            editor.inspector.shows = None;
            editor.selection = Selection::Node(End::Node(index));
            if let Some(card) = self::geometry(editor).card(End::Node(index)) {
                editor.view = editor.view.reveal(card.rect(), size);
            }
            say(editor, format!("Added {}{fed}", kind.label()), false);
        }
    }
}

fn delete_selection(editor: &mut Editor) {
    if locked(editor) {
        return;
    }
    let catalogue = Rc::clone(&editor.catalogue);
    match editor.selection {
        Selection::Node(End::Trigger) => say(
            editor,
            "A flow keeps its trigger: change its type in the settings on the right",
            true,
        ),
        Selection::Node(End::Node(index)) => {
            let label = editor.graph.label(End::Node(index), &catalogue);
            remember(editor, None);
            if editor.graph.remove(index) {
                select(editor, Selection::None);
                say(editor, format!("Removed {label}"), false);
            }
        }
        Selection::Wire(edge) if edge < editor.graph.edges.len() => {
            remember(editor, None);
            editor.graph.edges.remove(edge);
            select(editor, Selection::None);
            say(editor, "Removed the wire", false);
        }
        Selection::Wire(_) | Selection::None => {}
    }
}

/// ⌘D: a copy of the selected node beside it, unwired.
fn duplicate(editor: &mut Editor, size: Vec2) {
    if locked(editor) {
        return;
    }
    let Selection::Node(End::Node(index)) = editor.selection else {
        say(editor, "Select a node to duplicate it", true);
        return;
    };
    let id = editor.graph.fresh_id(now_millis());
    let mut graph = editor.graph.clone();
    let Some(copy) = graph.duplicate(index, id) else {
        return;
    };
    remember(editor, None);
    editor.graph = graph;
    select(editor, Selection::Node(End::Node(copy)));
    if let Some(card) = geometry(editor).card(End::Node(copy)) {
        editor.view = editor.view.reveal(card.rect(), size);
    }
    let label = editor.graph.label(End::Node(copy), &editor.catalogue);
    say(editor, format!("Duplicated {label}"), false);
}

fn arrange(editor: &mut Editor, size: Vec2) {
    if locked(editor) {
        return;
    }
    remember(editor, None);
    let (x, y) = editor.graph.arrange();
    #[allow(clippy::cast_possible_truncation)]
    {
        editor.trigger_at = Vec2::new(x as f32, y as f32);
    }
    editor.view = View::fit(geometry(editor).bounds(), size);
}

fn set_trigger_kind(console: &mut Console, index: usize, cx: &mut Context<Console>) {
    if let Some(editor) = editor_mut(console) {
        let current = editor.graph.trigger_type().to_owned();
        if editor
            .catalogue
            .triggers
            .get(index)
            .is_some_and(|kind| kind.kind != current)
        {
            add_item(editor, Item::Trigger(index), None, Vec2::default());
        }
    }
    cx.notify();
}

// ── Pointer (from the canvas) ────────────────────────────────────────────

/// A press at `at` (canvas point): a port starts a wire, a card is
/// selected and taken, a wire selected, the background panned.
pub(super) fn press(console: &mut Console, at: Vec2, clicks: usize) -> bool {
    let Some(editor) = editor_mut(console) else {
        return false;
    };
    editor.message = None;
    let world = editor.view.to_world(at);
    let geometry = geometry(editor);
    match model::hit(&geometry, world, editor.view.zoom) {
        Hit::Output(from, port) if !editor.read_only => {
            editor.drag = Some(Drag::Wire {
                from,
                port,
                to: world,
                target: None,
            });
        }
        Hit::Output(end, _) | Hit::Node(end) => {
            select(editor, Selection::Node(end));
            if clicks >= 2 {
                editor.side = Side::Settings;
                editor.focus_inspector = true;
            }
            if !editor.read_only
                && let Some(card) = geometry.card(end)
            {
                editor.drag = Some(Drag::Move {
                    end,
                    grab: world - card.at,
                    moved: false,
                });
            }
        }
        Hit::Wire(edge) => select(editor, Selection::Wire(edge)),
        Hit::Empty => {
            select(editor, Selection::None);
            editor.drag = Some(Drag::Pan {
                from: at,
                pan: editor.view.pan,
            });
        }
    }
    true
}

/// The pointer moved to `at` while pressed.
pub(super) fn drag_to(console: &mut Console, at: Vec2) -> bool {
    let Some(editor) = editor_mut(console) else {
        return false;
    };
    let Some(drag) = editor.drag else {
        return false;
    };
    let world = editor.view.to_world(at);
    let next = match drag {
        Drag::Pan { from, pan } => {
            editor.view.pan = pan + (at - from);
            drag
        }
        Drag::Move { end, grab, moved } => {
            if !moved {
                remember(editor, None);
            }
            let place = snap(world - grab);
            match end {
                End::Trigger => editor.trigger_at = place,
                End::Node(index) => {
                    editor
                        .graph
                        .set_position(index, f64::from(place.x), f64::from(place.y));
                }
            }
            Drag::Move {
                end,
                grab,
                moved: true,
            }
        }
        Drag::Wire { from, port, .. } => {
            let target = model::drop_target(&geometry(editor), world, editor.view.zoom)
                .filter(|target| End::Node(*target) != from);
            Drag::Wire {
                from,
                port,
                to: world,
                target,
            }
        }
    };
    editor.drag = Some(next);
    true
}

/// The button came up at `at`: a wire lands, a move or a pan ends.
pub(super) fn release(console: &mut Console, at: Vec2) {
    let Some(editor) = editor_mut(console) else {
        return;
    };
    let Some(Drag::Wire { from, port, .. }) = editor.drag.take() else {
        return;
    };
    let world = editor.view.to_world(at);
    if let Some(target) = model::drop_target(&geometry(editor), world, editor.view.zoom)
        && End::Node(target) != from
    {
        connect(editor, from, port, target);
    }
}

/// The wheel: pan by `delta`, or zoom about `at`.
pub(super) fn scroll(console: &mut Console, at: Vec2, delta: Vec2, zoom: bool) {
    let Some(editor) = editor_mut(console) else {
        return;
    };
    editor.view = if zoom {
        editor
            .view
            .zoom_about(at, (delta.y * 0.01).exp().clamp(0.5, 2.0))
    } else {
        editor.view.panned(delta)
    };
}

/// A trackpad pinch (macOS, through the mimic's gesture listener): zoom
/// about the fingers; a two-finger double tap fits the flow.
pub(crate) fn pinch(console: &mut Console, gesture: Gesture, cx: &mut Context<Console>) {
    let Some(editor) = editor_mut(console) else {
        return;
    };
    let Some(bounds) = editor.bounds.get() else {
        return;
    };
    let (x, y) = match gesture {
        Gesture::Magnify { x, y, .. } | Gesture::SmartMagnify { x, y } => (x, y),
    };
    // AppKit counts from the window's bottom-left corner, GPUI from its
    // top-left.
    #[allow(clippy::cast_possible_truncation)]
    let local = Vec2::new(
        x as f32 - f32::from(bounds.origin.x),
        editor.window_height - y as f32 - f32::from(bounds.origin.y),
    );
    let size = Vec2::new(f32::from(bounds.size.width), f32::from(bounds.size.height));
    if local.x < 0.0 || local.y < 0.0 || local.x > size.x || local.y > size.y {
        return;
    }
    match gesture {
        Gesture::Magnify { amount, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let factor = (1.0 + amount as f32).clamp(0.5, 2.0);
            editor.view = editor.view.zoom_about(local, factor);
        }
        Gesture::SmartMagnify { .. } => fit(editor, size),
    }
    cx.notify();
}

/// A palette entry dropped on the canvas, where the pointer is.
fn drop_item(console: &mut Console, item: Item, window: &mut Window, cx: &mut Context<Console>) {
    let pointer = window.mouse_position();
    let Some(editor) = editor_mut(console) else {
        return;
    };
    let size = canvas_size(editor, window);
    let at = editor.bounds.get().map(|bounds| {
        let local = Vec2::new(
            f32::from(pointer.x - bounds.origin.x),
            f32::from(pointer.y - bounds.origin.y),
        );
        editor.view.to_world(local) - Vec2::new(NODE_W / 2.0, NODE_H / 2.0)
    });
    add_item(editor, item, at, size);
    window.focus(&editor.canvas_focus);
    cx.notify();
}

// ── Keys ─────────────────────────────────────────────────────────────────

fn with_editor(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
    act: impl FnOnce(&mut Editor, Vec2),
) {
    if let Some(editor) = editor_mut(console) {
        let size = canvas_size(editor, window);
        editor.message = None;
        act(editor, size);
    }
    cx.notify();
}

fn nudge(console: &mut Console, dx: f32, dy: f32, window: &mut Window, cx: &mut Context<Console>) {
    let far = window.modifiers().shift;
    with_editor(console, window, cx, |editor, size| match editor.selection {
        Selection::Node(end) if !editor.read_only => {
            let step = if far { NUDGE_FAR } else { NUDGE };
            remember(editor, Some(format!("nudge {end:?}")));
            let Some(card) = geometry(editor).card(end).copied() else {
                return;
            };
            let at = card.at + Vec2::new(dx * step, dy * step);
            match end {
                End::Trigger => editor.trigger_at = at,
                End::Node(index) => {
                    editor
                        .graph
                        .set_position(index, f64::from(at.x), f64::from(at.y));
                }
            }
            if let Some(card) = geometry(editor).card(end) {
                editor.view = editor.view.reveal(card.rect(), size);
            }
        }
        _ => {
            editor.view = editor
                .view
                .panned(Vec2::new(-dx * PAN_STEP, -dy * PAN_STEP));
        }
    });
}

fn cycle(console: &mut Console, forward: bool, window: &mut Window, cx: &mut Context<Console>) {
    with_editor(console, window, cx, |editor, size| {
        let from = match editor.selection {
            Selection::Node(end) => Some(end),
            Selection::Wire(edge) => editor
                .graph
                .edges
                .get(edge)
                .and_then(|edge| editor.graph.index_of(&edge.to))
                .map(End::Node),
            Selection::None => None,
        };
        let next = model::cycle(editor.graph.nodes.len(), from, forward);
        select(editor, Selection::Node(next));
        if let Some(card) = geometry(editor).card(next) {
            editor.view = editor.view.reveal(card.rect(), size);
        }
    });
}

/// Esc on the canvas: drop a drag, then the selection, then leave.
fn escape(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = editor_mut(console) else {
        return;
    };
    if editor.drag.take().is_some() {
        cx.notify();
    } else if editor.selection != Selection::None {
        select(editor, Selection::None);
        cx.notify();
    } else {
        close(console, window, cx);
    }
}

/// Back to the list, asking first when there are unsaved changes.
fn close(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = console.state::<State>().editor.as_ref() else {
        return;
    };
    if editor.read_only || !dirty(editor, cx) {
        close_now(console, window, cx);
        return;
    }
    let focus = cx.focus_handle();
    if let Some(editor) = editor_mut(console) {
        editor.discard_focus = Some(focus.clone());
    }
    let overlay = Overlay::modal(DISCARD, px(420.0), discard_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn close_now(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let id = state.editor.take().map(|editor| editor.flow.id);
    state.focus_id = id;
    refresh(console);
    console.focus_list(LIST, window, cx);
    cx.notify();
}

/// The node an error from the server names (by its id), if any.
fn named_node(graph: &FlowGraph, error: &str) -> Option<usize> {
    graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| config_id(node).len() >= 2 && error.contains(config_id(node)))
        .max_by_key(|(_, node)| config_id(node).len())
        .map(|(index, _)| index)
}

/// The texts to save: v2, or the legacy array for a server without the
/// graph engine.
fn save_texts(editor: &Editor) -> Result<(String, String), String> {
    let nodes = match editor.graph_server {
        Some(false) => editor.graph.legacy_json(&editor.catalogue)?,
        _ => editor.graph.nodes_json(),
    };
    Ok((editor.graph.trigger_json(), nodes))
}

/// ⌘S: `PUT /api/v1/flows/{id}` with the canvas's graph.
fn save(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    if editor.saving || locked(editor) {
        cx.notify();
        return;
    }
    let name = editor.name.read(cx).value().trim().to_owned();
    if name.is_empty() {
        say(editor, "ID and name required", true);
        cx.notify();
        return;
    }
    if let Some((label, error)) = editor
        .inspector
        .fields
        .iter()
        .find_map(|field| Some((field.def.label().to_owned(), field.error.clone()?)))
    {
        say(editor, format!("{label}: {error}"), true);
        cx.notify();
        return;
    }
    if let Err(problem) = editor.graph.validate(&editor.catalogue) {
        if let Some(end) = problem.at {
            select(editor, Selection::Node(end));
            editor.side = Side::Settings;
        }
        say(editor, problem.message, true);
        cx.notify();
        return;
    }
    let (trigger_config, nodes) = match save_texts(editor) {
        Ok(texts) => texts,
        Err(error) => {
            say(editor, error, true);
            cx.notify();
            return;
        }
    };
    let body = editor
        .flow
        .with_texts(&name, editor.enabled, trigger_config, nodes, now_secs());
    let graph = editor.graph.clone();
    editor.saving = true;
    editor.save_seq = seq;
    editor.message = None;
    console.call(
        CallKey::new("data.flows.save").with(seq),
        Call::new(move |api| async move {
            api.update_flow(&body).await?;
            anyhow::Ok((body, graph))
        }),
    );
    cx.notify();
}

/// The editor, if save `seq` is the one it waits for.
fn saving_editor(console: &mut Console, seq: i64) -> Option<&mut Editor> {
    editor_mut(console).filter(|editor| editor.save_seq == seq)
}

/// The answer to a save; an editor saving again since waits for that one.
/// A refusal naming a node selects it.
pub(super) fn on_saved(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    match reply.take::<(Flow, FlowGraph)>() {
        Ok((flow, graph)) => {
            let name = flow.name.clone();
            if let Some(editor) = saving_editor(console, key.arg) {
                editor.legacy = editor.graph_server == Some(false);
                editor.flow = flow;
                editor.saved = graph;
                editor.broken = None;
                editor.saving = false;
                say(editor, format!("Saved {name}"), false);
            }
            console.show_toast(format!("Saved {name}"), false, cx);
            refresh(console);
        }
        Err(error) => match saving_editor(console, key.arg) {
            Some(editor) => {
                editor.saving = false;
                if let Some(index) = named_node(&editor.graph, &error) {
                    select(editor, Selection::Node(End::Node(index)));
                    editor.side = Side::Settings;
                }
                say(editor, format!("Saving failed: {error}"), true);
            }
            None => console.show_toast(format!("Saving the flow failed: {error}"), true, cx),
        },
    }
}

// ── Test and run ─────────────────────────────────────────────────────────

/// Run test: a dry run of the canvas (`POST /api/v1/flows/test`). A server
/// without it tests the stored flow (`POST …/{id}/test`), outputs for
/// real: saved first, and only on a second press.
fn run_test(console: &mut Console, cx: &mut Context<Console>) {
    let unsaved = console
        .state::<State>()
        .editor
        .as_ref()
        .is_some_and(|editor| dirty(editor, cx));
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    editor.side = Side::Test;
    if editor.test.running || editor.read_only {
        cx.notify();
        return;
    }
    let text = editor.test.input.read(cx).value().to_owned();
    let input = match parse_input(&text) {
        Ok(input) => input,
        Err(error) => {
            editor.test.result = Some(Err(format!("The input is {error}")));
            cx.notify();
            return;
        }
    };
    let legacy = editor.graph_server == Some(false);
    if legacy {
        if unsaved {
            editor.test.result = Some(Err(
                "This server tests the saved flow only: save with ⌘S first".into(),
            ));
            cx.notify();
            return;
        }
        let acts = editor.graph.nodes.iter().any(|node| {
            editor
                .catalogue
                .node(config_type(node))
                .is_some_and(|kind| kind.side_effect)
        });
        if acts && !editor.test.armed {
            editor.test.armed = true;
            cx.notify();
            return;
        }
    }
    editor.test.armed = false;
    editor.test.running = true;
    editor.test.seq = seq;
    editor.test.picked = None;
    let (trigger_config, nodes) = (editor.graph.trigger_json(), editor.graph.nodes_json());
    let id = editor.flow.id.clone();
    let payload = input.get("payload").cloned().unwrap_or(Value::Null);
    state.test_payload = Some(text);
    console.call(
        CallKey::new("data.flows.draft_test").with(seq),
        Call::new(move |api| async move {
            if !legacy
                && let Some(run) = api.test_flow_draft(&trigger_config, &nodes, input).await?
            {
                return anyhow::Ok((run, false));
            }
            let result = api.test_flow(&id, payload).await?;
            anyhow::Ok((TestRun::from_legacy(&result), true))
        }),
    );
    cx.notify();
}

/// Run now: one real run of the stored flow (`POST …/{id}/run`), on the
/// second press.
fn run_now(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    if editor.running_now || locked(editor) {
        cx.notify();
        return;
    }
    if !editor.run_armed {
        editor.run_armed = true;
        say(
            editor,
            "Run now runs the saved flow once, for real: writes, alarms and webhooks act. Press Run now again to go.",
            true,
        );
        cx.notify();
        return;
    }
    editor.run_armed = false;
    editor.running_now = true;
    let input = parse_input(editor.test.input.read(cx).value()).ok();
    let (id, name) = (editor.flow.id.clone(), editor.flow.name.clone());
    console.call(
        CallKey::new("data.flows.run_now").with(seq),
        Call::new(move |api| async move {
            api.run_flow_now(&id, input).await?;
            anyhow::Ok(name)
        }),
    );
    cx.notify();
}

/// Answers to the test and run calls.
pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    match key.name {
        "data.flows.draft_test" => {
            if let Some(editor) = editor_mut(console)
                && editor.test.seq == key.arg
            {
                editor.test.running = false;
                editor.test.result = Some(reply.take::<(TestRun, bool)>());
            }
        }
        "data.flows.run_now" => {
            if let Some(editor) = editor_mut(console) {
                editor.running_now = false;
            }
            match reply.take::<String>() {
                Ok(name) => console.show_toast(format!("Ran {name} once"), false, cx),
                Err(error) => {
                    let error = if error.contains("404") || error.contains("405") {
                        "this server cannot run a flow on demand".to_owned()
                    } else {
                        error
                    };
                    if let Some(editor) = editor_mut(console) {
                        say(editor, format!("Run now failed: {error}"), true);
                    }
                }
            }
        }
        _ => {}
    }
}

/// The path the shown test run took.
fn shown_path(editor: &Editor) -> Option<Path> {
    match &editor.test.result {
        Some(Ok((run, _))) => Some(test_path(&editor.graph, run)),
        _ => None,
    }
}

// ── Settings fields ──────────────────────────────────────────────────────

/// A node's optional label, when its type has no `name` setting itself.
fn name_def() -> FieldDef {
    FieldDef {
        key: "name".into(),
        label: "Name".into(),
        kind: FieldKind::String,
        help: Some("A label on the canvas (optional)".into()),
        ..FieldDef::default()
    }
}

/// The fields of `selection`: its type's, then the settings it has that
/// the type does not declare (as JSON, so nothing is lost).
fn field_defs(graph: &FlowGraph, selection: Selection, catalogue: &Catalogue) -> Vec<FieldDef> {
    let (config, declared, skip): (_, Vec<FieldDef>, &[&str]) = match selection {
        Selection::Node(End::Trigger) => (
            &graph.trigger,
            catalogue
                .trigger(graph.trigger_type())
                .map(|kind| kind.fields.clone())
                .unwrap_or_default(),
            &["type"],
        ),
        Selection::Node(End::Node(index)) => {
            let Some(node) = graph.nodes.get(index) else {
                return Vec::new();
            };
            let declared = catalogue
                .node(config_type(node))
                .map(|kind| kind.fields.clone())
                .unwrap_or_default();
            let mut defs = Vec::new();
            if !declared.iter().any(|field| field.key == "name") {
                defs.push(name_def());
            }
            defs.extend(declared);
            (node, defs, &RESERVED)
        }
        Selection::Wire(_) | Selection::None => return Vec::new(),
    };
    let mut defs = declared;
    for (key, value) in config {
        if skip.contains(&key.as_str()) || defs.iter().any(|field| field.key == *key) {
            continue;
        }
        let kind = value_kind(value);
        defs.push(FieldDef {
            key: key.clone(),
            label: key.replace('_', " "),
            kind: if kind == FieldKind::String {
                kind
            } else {
                FieldKind::Json
            },
            ..FieldDef::default()
        });
    }
    defs
}

/// The settings map the panel edits.
fn config_of(editor: &Editor, selection: Selection) -> Option<&serde_json::Map<String, Value>> {
    match selection {
        Selection::Node(End::Trigger) => Some(&editor.graph.trigger),
        Selection::Node(End::Node(index)) => editor.graph.nodes.get(index),
        _ => None,
    }
}

/// Make the panel's fields for the selection, once per selection.
fn ensure_inspector(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = editor_mut(console) else {
        return;
    };
    if editor.inspector.shows == Some(editor.selection) {
        return;
    }
    let selection = editor.selection;
    let defs = field_defs(&editor.graph, selection, &editor.catalogue);
    let config = config_of(editor, selection).cloned().unwrap_or_default();
    let read_only = editor.read_only;
    let mut fields = Vec::with_capacity(defs.len());
    let mut subscriptions = Vec::with_capacity(defs.len());
    for (index, def) in defs.into_iter().enumerate() {
        let on_event = move |console: &mut Console,
                             event: &InputEvent,
                             window: &mut Window,
                             cx: &mut Context<Console>| match event {
            InputEvent::Changed => field_changed(console, index, cx),
            InputEvent::Submitted | InputEvent::Cancelled => focus_canvas(console, window),
            InputEvent::Blurred => {}
        };
        let text = config.get(&def.key).map(field_text).unwrap_or_default();
        let placeholder = def.default.as_ref().map(field_text).unwrap_or_default();
        let input = if read_only {
            Input::Fixed
        } else if def.kind == FieldKind::Bool {
            Input::Switch
        } else if matches!(def.kind, FieldKind::Select | FieldKind::Severity)
            && !def.choices().is_empty()
        {
            Input::Choice
        } else if def.kind.is_json() || def.kind == FieldKind::Code {
            let rows = if def.kind == FieldKind::Code { 8 } else { 4 };
            let area = cx.new(|cx| TextArea::new(rows, cx).placeholder(placeholder));
            area.update(cx, |area, cx| {
                area.replace_text_in_range(None, &text, window, cx);
            });
            subscriptions.push(cx.subscribe_in(
                &area,
                window,
                move |console, _, event: &InputEvent, window, cx| {
                    on_event(console, event, window, cx);
                },
            ));
            Input::Area(area)
        } else {
            let kind = def.kind;
            let line = cx.new(|cx| {
                let line = TextInput::new(cx).text(text).placeholder(placeholder);
                let line = if matches!(
                    kind,
                    FieldKind::Expression
                        | FieldKind::Cron
                        | FieldKind::Tag
                        | FieldKind::Device
                        | FieldKind::Number
                        | FieldKind::DurationMs
                ) {
                    line.mono()
                } else {
                    line
                };
                if kind.is_number() {
                    line.kind(InputKind::Number)
                } else {
                    line
                }
            });
            subscriptions.push(cx.subscribe_in(
                &line,
                window,
                move |console, _, event: &InputEvent, window, cx| {
                    on_event(console, event, window, cx);
                },
            ));
            Input::Line(line)
        };
        fields.push(Field {
            def,
            input,
            error: None,
        });
    }
    editor.inspector = Inspector {
        shows: Some(selection),
        fields,
        _subscriptions: subscriptions,
    };
}

/// Write `value` (or remove the setting) to `key` of what the panel shows.
fn set_value(editor: &mut Editor, key: &str, value: Option<Value>) {
    let Some(selection) = editor.inspector.shows else {
        return;
    };
    let catalogue = Rc::clone(&editor.catalogue);
    let config = match selection {
        Selection::Node(End::Trigger) => &mut editor.graph.trigger,
        Selection::Node(End::Node(index)) => match editor.graph.nodes.get_mut(index) {
            Some(node) => node,
            None => return,
        },
        _ => return,
    };
    // An empty label is no label.
    let value = value.filter(|value| !(key == "name" && value.as_str() == Some("")));
    if config.get(key) == value.as_ref() {
        return;
    }
    match value {
        Some(value) => {
            config.insert(key.to_owned(), value);
        }
        None => {
            config.shift_remove(key);
        }
    }
    // A switch's rules decide its ports.
    if key == "rules" {
        editor.graph.prune_ports(&catalogue);
    }
}

/// A field's text changed: store it if it reads as its kind.
fn field_changed(console: &mut Console, index: usize, cx: &mut Context<Console>) {
    let Some(editor) = editor_mut(console) else {
        return;
    };
    let Some(field) = editor.inspector.fields.get(index) else {
        return;
    };
    let text = match &field.input {
        Input::Line(line) => line.read(cx).value().to_owned(),
        Input::Area(area) => area.read(cx).value().to_owned(),
        Input::Switch | Input::Choice | Input::Fixed => return,
    };
    let key = field.def.key.clone();
    let error = match parse_field(field.def.kind, &text) {
        Ok(value) => {
            let key_tag = format!("{:?}/{key}", editor.inspector.shows);
            let before = config_of(editor, editor.selection).and_then(|config| config.get(&key));
            let same =
                before == value.as_ref() || (key == "name" && before.is_none() && text.is_empty());
            if !same {
                remember(editor, Some(key_tag));
                set_value(editor, &key, value);
            }
            None
        }
        Err(error) => Some(error),
    };
    if let Some(field) = editor.inspector.fields.get_mut(index) {
        field.error = error;
    }
    cx.notify();
}

/// A switch or a choice was clicked.
fn set_field(console: &mut Console, key: &str, value: Value, cx: &mut Context<Console>) {
    if let Some(editor) = editor_mut(console)
        && !locked(editor)
    {
        let current = config_of(editor, editor.selection).and_then(|config| config.get(key));
        if current != Some(&value) {
            remember(editor, None);
            set_value(editor, key, Some(value));
        }
    }
    cx.notify();
}

/// A suggestion was picked for the line field `index`.
fn pick_suggestion(
    console: &mut Console,
    index: usize,
    text: String,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let line = editor_mut(console)
        .and_then(|editor| editor.inspector.fields.get(index))
        .and_then(|field| match &field.input {
            Input::Line(line) => Some(line.clone()),
            _ => None,
        });
    if let Some(line) = line {
        line.update(cx, |line, cx| line.set_text(text, cx));
        field_changed(console, index, cx);
        line.read(cx).focus(window);
    }
}

/// Up to [`SUGGESTIONS`] of `candidates` holding `typed`, the exact one
/// left out.
pub(crate) fn suggestions(candidates: &BTreeSet<String>, typed: &str) -> Vec<String> {
    let typed_lower = typed.trim().to_lowercase();
    if candidates.contains(typed.trim()) {
        return Vec::new();
    }
    candidates
        .iter()
        .filter(|candidate| candidate.to_lowercase().contains(&typed_lower))
        .take(SUGGESTIONS)
        .cloned()
        .collect()
}

// ── Palette ──────────────────────────────────────────────────────────────

/// The palette's entries matching `filter`: the triggers, then the node
/// types by category.
pub(crate) fn palette_items(catalogue: &Catalogue, filter: &str) -> Vec<Item> {
    let mut items: Vec<Item> = catalogue
        .trigger_palette(filter)
        .into_iter()
        .filter_map(|trigger| {
            catalogue
                .triggers
                .iter()
                .position(|other| std::ptr::eq(other, trigger))
                .map(Item::Trigger)
        })
        .collect();
    for (_, nodes) in catalogue.palette(filter) {
        items.extend(nodes.into_iter().filter_map(|node| {
            catalogue
                .nodes
                .iter()
                .position(|other| std::ptr::eq(other, node))
                .map(Item::Node)
        }));
    }
    items
}

fn palette_activate(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(editor) = editor_mut(console) else {
        return;
    };
    let filter = editor.filter.read(cx).value().to_owned();
    let Some(item) = palette_items(&editor.catalogue, &filter).get(row).copied() else {
        return;
    };
    let size = canvas_size(editor, window);
    add_item(editor, item, None, size);
    window.focus(&editor.canvas_focus);
    cx.notify();
}

fn item_look(catalogue: &Catalogue, item: Item, theme: &Theme) -> (SharedString, Hsla) {
    match item {
        Item::Trigger(index) => (
            catalogue
                .triggers
                .get(index)
                .map_or_else(String::new, |kind| kind.label().to_owned())
                .into(),
            theme.accent,
        ),
        Item::Node(index) => catalogue.nodes.get(index).map_or_else(
            || (SharedString::default(), theme.fg_dim),
            |kind| {
                (
                    kind.label().to_owned().into(),
                    tone_color(kind.category().tone(), theme),
                )
            },
        ),
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// The line on the trigger's card: its first settings.
fn trigger_summary(graph: &FlowGraph) -> String {
    let shown = preview(&graph.trigger);
    if !shown.is_empty() {
        return shown;
    }
    graph
        .trigger
        .iter()
        .filter(|(key, _)| key.as_str() != "type")
        .take(2)
        .map(|(key, value)| format!("{key} {}", field_text(value)))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A node's colour: its category's.
fn node_color(graph: &FlowGraph, end: End, catalogue: &Catalogue, theme: &Theme) -> Hsla {
    match end {
        End::Trigger => theme.accent,
        End::Node(index) => graph
            .nodes
            .get(index)
            .and_then(|node| catalogue.node(config_type(node)))
            .map_or(theme.fg_dim, |kind| {
                tone_color(kind.category().tone(), theme)
            }),
    }
}

/// What the canvas draws this frame: only what is in view.
fn scene(editor: &Editor, size: Vec2, theme: &Theme) -> Scene {
    let graph = &editor.graph;
    let catalogue = &editor.catalogue;
    let geometry = geometry(editor);
    let visible = editor.view.visible(size).inflate(32.0);
    let path = shown_path(editor);
    let target = match editor.drag {
        Some(Drag::Wire { target, .. }) => target,
        _ => None,
    };
    let mut cards = Vec::new();
    for card in &geometry.cards {
        if !card.rect().inflate(130.0).intersects(&visible) {
            continue;
        }
        let end = card.end;
        let (kicker, title, preview_text) = match end {
            End::Trigger => (
                "TRIGGER".to_owned(),
                graph.label(End::Trigger, catalogue),
                trigger_summary(graph),
            ),
            End::Node(index) => {
                let config = &graph.nodes[index];
                let kind = config_type(config);
                let category = catalogue
                    .node(kind)
                    .map_or(Category::Other, |known| known.category());
                (
                    format!("{} · {kind}", category.label().to_uppercase()),
                    graph.label(end, catalogue),
                    preview(config),
                )
            }
        };
        let mark = match (&path, end) {
            (None, _) => Mark::Plain,
            (Some(_), End::Trigger) => Mark::Taken,
            (Some(path), End::Node(index)) if path.errors.contains(&index) => Mark::Failed,
            (Some(path), End::Node(index)) if path.nodes.contains(&index) => Mark::Taken,
            (Some(_), End::Node(_)) => Mark::Dim,
        };
        cards.push(Drawn {
            at: card.at,
            height: card.height,
            color: node_color(graph, end, catalogue, theme),
            kicker: kicker.into(),
            title: title.into(),
            preview: preview_text.into(),
            selected: editor.selection == Selection::Node(end),
            target: matches!(end, End::Node(index) if Some(index) == target),
            input: card.input,
            outputs: graph
                .outputs(end, catalogue)
                .into_iter()
                .map(SharedString::from)
                .collect(),
            mark,
            invalid: !graph.missing(end, catalogue).is_empty(),
        });
    }
    let wires = (0..geometry.edges.len())
        .filter_map(|edge| {
            let curve = geometry.curve(edge)?;
            curve.bounds().intersects(&visible).then(|| DrawnWire {
                curve,
                selected: editor.selection == Selection::Wire(edge),
                mark: match &path {
                    None => Mark::Plain,
                    Some(path) if path.edges.contains(&edge) => Mark::Taken,
                    Some(_) => Mark::Dim,
                },
            })
        })
        .collect();
    let pending = match editor.drag {
        Some(Drag::Wire { from, port, to, .. }) => {
            geometry.card(from).map(|card| (card.output_port(port), to))
        }
        _ => None,
    };
    let dragging = match editor.drag {
        Some(Drag::Pan { .. } | Drag::Move { .. }) => Some(gpui::CursorStyle::ClosedHand),
        Some(Drag::Wire { .. }) => Some(gpui::CursorStyle::Crosshair),
        None => None,
    };
    Scene {
        view: editor.view,
        cards,
        wires,
        pending,
        dragging,
        colors: Colors {
            bg: theme.bg,
            grid: theme.border.alpha(0.35),
            surface: theme.surface,
            border: theme.border_strong,
            fg: theme.fg,
            fg_dim: theme.fg_dim,
            wire: theme.fg_dim,
            accent: theme.accent,
            success: theme.success,
            danger: theme.danger,
        },
    }
}

/// The tag keys and device ids the console knows, for suggestions.
fn candidates(console: &Console) -> (BTreeSet<String>, BTreeSet<String>) {
    let tags = console
        .store
        .tags
        .iter()
        .map(|tag| tag.info.key.clone())
        .collect();
    let mut devices: BTreeSet<String> = console.devices.keys().cloned().collect();
    devices.extend(
        console
            .store
            .tags
            .iter()
            .map(|tag| tag.info.device_id.clone())
            .filter(|id| !id.is_empty()),
    );
    (tags, devices)
}

#[allow(clippy::too_many_lines)]
pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    ensure_inspector(console, window, cx);
    let theme = theme(cx).clone();
    let page_view = window.current_view();
    let weak = cx.entity().downgrade();
    let palette_focused = console.list(PALETTE_LIST, cx).focus.is_focused(window);
    let palette_cursor = console.list(PALETTE_LIST, cx).cursor;
    let lists = candidates(console);
    let Some(editor) = editor_mut(console) else {
        return div().into_any_element();
    };

    // Enter or a double click asked for the first field.
    if editor.focus_inspector {
        editor.focus_inspector = false;
        if !focus_first_field(editor, window, cx) {
            say(editor, "This has no settings to edit", false);
        }
    }

    editor.window_height = f32::from(window.viewport_size().height);
    let size = canvas_size(editor, window);
    let scene = scene(editor, size, &theme);
    let is_dirty = dirty(editor, cx);
    let canvas_focus = editor.canvas_focus.clone();
    let canvas_focused = canvas_focus.is_focused(window);
    let bounds = Rc::clone(&editor.bounds);
    let name = editor.name.clone();
    let filter = editor.filter.clone();
    let flow = editor.flow.clone();
    let catalogue = Rc::clone(&editor.catalogue);
    let (enabled, saving, message, broken, zoom, read_only, legacy, graph_server) = (
        editor.enabled,
        editor.saving,
        editor.message.clone(),
        editor.broken.clone(),
        editor.view.zoom,
        editor.read_only,
        editor.legacy,
        editor.graph_server,
    );
    let (side, run_armed, running_now) = (editor.side, editor.run_armed, editor.running_now);
    let (can_undo, can_redo) = (!editor.past.is_empty(), !editor.future.is_empty());
    let filter_text = filter.read(cx).value().to_owned();
    let items = palette_items(&catalogue, &filter_text);

    // ── Toolbar ──
    fn set_enabled(
        on: bool,
        cx: &mut Context<Console>,
    ) -> impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
        cx.listener(move |console: &mut Console, _: &gpui::ClickEvent, _, cx| {
            if let Some(editor) = editor_mut(console)
                && !locked(editor)
            {
                editor.enabled = on;
            }
            cx.notify();
        })
    }
    let toolbar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(56.0))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border)
        .child(
            IconButton::new("data-flow-back", Icon::ChevronLeft)
                .tooltip("Back to the flows  Esc")
                .on_click(cx.listener(|console, _, window, cx| close(console, window, cx))),
        )
        .map(|this| {
            if read_only {
                this.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(flow.name.clone()),
                )
            } else {
                this.child(div().w(px(240.0)).child(name))
            }
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(flow.id.clone()),
        )
        .when(read_only, |this| {
            this.child(pill("Read-only", theme.fg_dim))
        })
        .when(!read_only && is_dirty, |this| {
            this.child(pill("Unsaved", theme.warning))
        })
        .child(div().flex_1())
        .when(!read_only, |this| {
            this.child(
                IconButton::new("data-flow-undo", Icon::ChevronLeft)
                    .tooltip("Undo  ⌘Z")
                    .disabled(!can_undo)
                    .on_click(cx.listener(|console, _, window, cx| {
                        with_editor(console, window, cx, |editor, _| undo(editor));
                    })),
            )
            .child(
                IconButton::new("data-flow-redo", Icon::ChevronRight)
                    .tooltip("Redo  ⇧⌘Z")
                    .disabled(!can_redo)
                    .on_click(cx.listener(|console, _, window, cx| {
                        with_editor(console, window, cx, |editor, _| redo(editor));
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child("Enabled")
                    .child(segmented(
                        "data-flow-enabled",
                        vec![
                            Segment::new("Off", !enabled, set_enabled(false, cx)),
                            Segment::new("On", enabled, set_enabled(true, cx)),
                        ],
                        cx,
                    )),
            )
            .child(
                Button::new(
                    "data-flow-run-now",
                    if running_now {
                        "Running…"
                    } else if run_armed {
                        "Run now?"
                    } else {
                        "Run now"
                    },
                )
                .ghost()
                .icon(Icon::Play)
                .disabled(running_now)
                .tooltip("Run the saved flow once, for real")
                .on_click(cx.listener(|console, _, _, cx| run_now(console, cx))),
            )
        })
        .child(
            Button::new("data-flow-editor-runs", "Runs")
                .ghost()
                .icon(Icon::History)
                .tooltip("The engine's latest runs  H")
                .on_click(cx.listener(|console, _, window, cx| open_runs(console, window, cx))),
        )
        .when(!read_only, |this| {
            this.child(
                Button::new("data-flow-save", if saving { "Saving…" } else { "Save" })
                    .primary()
                    .icon(Icon::Check)
                    .tooltip("⌘S")
                    .disabled(saving || !is_dirty)
                    .on_click(cx.listener(|console, _, _, cx| save(console, cx))),
            )
        });

    let bar = |text: String, color: Hsla, glyph: Icon| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_3)
            .px(SPACE_8)
            .py(SPACE_3)
            .bg(color.opacity(0.08))
            .border_b_1()
            .border_color(theme.border)
            .text_size(FONT_SMALL)
            .text_color(color)
            .child(vt_ui::icon(glyph, ICON_SM, color))
            .child(div().flex_1().min_w_0().truncate().child(text))
    };

    // ── Palette ──
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut group: Option<String> = None;
    for (at, item) in items.iter().copied().enumerate() {
        let heading = match item {
            Item::Trigger(_) => "TRIGGERS".to_owned(),
            Item::Node(index) => catalogue.nodes[index].category().label().to_uppercase(),
        };
        if group.as_deref() != Some(heading.as_str()) {
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .h(px(28.0))
                    .px(SPACE_6)
                    .text_size(FONT_CAPTION)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg_dim)
                    .child(heading.clone())
                    .into_any_element(),
            );
            group = Some(heading);
        }
        let (label, color) = item_look(&catalogue, item, &theme);
        let tip = match item {
            Item::Trigger(index) => catalogue.triggers[index].description.clone(),
            Item::Node(index) => catalogue.nodes[index].description.clone(),
        };
        let current = palette_cursor == Some(at);
        let hover = theme.hover;
        let dragged = PaletteDrag {
            item,
            label: label.clone(),
            color,
        };
        rows.push(
            div()
                .id(("data-flow-palette-item", at))
                .relative()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(28.0))
                .mx(SPACE_3)
                .px(SPACE_4)
                .rounded(RADIUS_MD)
                .cursor_grab()
                .when(current, |this| this.bg(theme.selected))
                .when(!current, |this| this.hover(move |style| style.bg(hover)))
                .when(palette_focused && current, |this| {
                    this.child(focus_ring(&theme))
                })
                .when(!tip.is_empty(), |this| {
                    this.tooltip(move |_, cx| {
                        vt_ui::components::Tooltip::view(tip.clone().into(), cx)
                    })
                })
                .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                .on_click(cx.listener(move |console, _, window, cx| {
                    console.list(PALETTE_LIST, cx).select(at);
                    palette_activate(console, at, false, window, cx);
                }))
                .child(div().flex_none().size(px(8.0)).rounded(px(4.0)).bg(color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                )
                .into_any_element(),
        );
    }
    if items.is_empty() {
        rows.push(
            div()
                .px(SPACE_6)
                .py(SPACE_5)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child("No node type matches")
                .into_any_element(),
        );
    }
    let palette_list = keyboard_list(
        ListKeys {
            id: PALETTE_LIST,
            on_move: None,
            on_activate: palette_activate,
        },
        items.len(),
        items.len(),
        console,
        cx,
    )
    .flex()
    .flex_col()
    .flex_none()
    .w(px(PALETTE_W))
    .h_full()
    .overflow_y_scroll()
    .bg(theme.chrome)
    .border_r_1()
    .border_color(theme.border)
    .child(
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(SPACE_3)
            .px(SPACE_6)
            .py(SPACE_5)
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg_dim)
                    .child("PALETTE"),
            )
            .child(filter)
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child("↵ add · drag onto the canvas"),
            ),
    )
    .children(rows);

    // ── Canvas ──
    let accent_soft = theme.accent_soft;
    let canvas_area = div()
        .id("data-flow-canvas")
        .key_context(CANVAS_CONTEXT)
        .track_focus(&canvas_focus)
        .relative()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .on_action(cx.listener(|console, _: &FocusNext, window, cx| {
            cycle(console, true, window, cx);
        }))
        .on_action(cx.listener(|console, _: &FocusPrevious, window, cx| {
            cycle(console, false, window, cx);
        }))
        .on_action(cx.listener(|console, _: &Dismiss, window, cx| escape(console, window, cx)))
        .on_action(
            cx.listener(|console, _: &EngFlowsDeleteSelection, window, cx| {
                with_editor(console, window, cx, |editor, _| delete_selection(editor));
            }),
        )
        .on_action(cx.listener(|console, _: &EngFlowsNudgeLeft, window, cx| {
            nudge(console, -1.0, 0.0, window, cx);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsNudgeRight, window, cx| {
            nudge(console, 1.0, 0.0, window, cx);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsNudgeUp, window, cx| {
            nudge(console, 0.0, -1.0, window, cx);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsNudgeDown, window, cx| {
            nudge(console, 0.0, 1.0, window, cx);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsZoomIn, window, cx| {
            with_editor(console, window, cx, |editor, size| {
                editor.view = editor.view.zoom_about(size.scale(0.5), model::ZOOM_STEP);
            });
        }))
        .on_action(cx.listener(|console, _: &EngFlowsZoomOut, window, cx| {
            with_editor(console, window, cx, |editor, size| {
                editor.view = editor
                    .view
                    .zoom_about(size.scale(0.5), 1.0 / model::ZOOM_STEP);
            });
        }))
        .on_action(cx.listener(|console, _: &EngFlowsFit, window, cx| {
            with_editor(console, window, cx, fit);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsArrange, window, cx| {
            with_editor(console, window, cx, arrange);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsDuplicate, window, cx| {
            with_editor(console, window, cx, duplicate);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsUndo, window, cx| {
            with_editor(console, window, cx, |editor, _| undo(editor));
        }))
        .on_action(cx.listener(|console, _: &EngFlowsRedo, window, cx| {
            with_editor(console, window, cx, |editor, _| redo(editor));
        }))
        .on_action(cx.listener(|console, _: &EngFlowsAddStep, window, cx| {
            let filter = editor_mut(console).map(|editor| editor.filter.clone());
            match filter {
                Some(filter) => filter.read(cx).focus(window),
                None => console.focus_list(PALETTE_LIST, window, cx),
            }
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &EngFlowsInspect, _, cx| {
            if let Some(editor) = editor_mut(console) {
                editor.side = Side::Settings;
                editor.focus_inspector = true;
            }
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &EngFlowsTest, window, cx| {
            if let Some(editor) = editor_mut(console) {
                editor.side = Side::Test;
                let input = editor.test.input.clone();
                input.read(cx).focus(window);
            }
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &EngFlowsRuns, window, cx| {
            open_runs(console, window, cx);
        }))
        .on_drop(cx.listener(|console, dragged: &PaletteDrag, window, cx| {
            drop_item(console, dragged.item, window, cx);
        }))
        .drag_over::<PaletteDrag>(move |style, _, _, _| style.bg(accent_soft))
        .child(canvas::element(
            scene,
            weak,
            page_view,
            bounds,
            canvas_focus,
        ))
        .when(canvas_focused, |this| {
            this.child(
                div()
                    .absolute()
                    .inset_0()
                    .border_1()
                    .border_color(theme.accent.opacity(0.6)),
            )
        });
    let hints = if read_only {
        "Read-only · drag the background to pan · ⌘-scroll or + − 0 zoom · Tab next node · Esc back"
    } else {
        "Drag a port onto a node to wire · ⌫ remove · ⌘D duplicate · arrows move · Tab next · + − 0 zoom · A arrange · N add · ↵ settings · T test · ⌘Z ⇧⌘Z undo, redo · ⌘S save · Esc back"
    };
    let strip = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_3)
        .h(px(32.0))
        .px(SPACE_6)
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.chrome)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(hints),
        )
        .child(
            div()
                .w(px(44.0))
                .text_right()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(format!("{:.0}%", zoom * 100.0)),
        )
        .child(strip_button(
            "data-flow-zoom-out",
            "−",
            "Zoom out  −",
            cx,
            |editor, size| {
                editor.view = editor
                    .view
                    .zoom_about(size.scale(0.5), 1.0 / model::ZOOM_STEP);
            },
        ))
        .child(strip_button(
            "data-flow-zoom-in",
            "+",
            "Zoom in  +",
            cx,
            |editor, size| {
                editor.view = editor.view.zoom_about(size.scale(0.5), model::ZOOM_STEP);
            },
        ))
        .child(strip_button(
            "data-flow-fit",
            "Fit",
            "Fit the flow in view  0",
            cx,
            fit,
        ))
        .when(!read_only, |this| {
            this.child(strip_button(
                "data-flow-arrange",
                "Arrange",
                "Lay the nodes out by distance from the trigger  A",
                cx,
                arrange,
            ))
        });

    // ── Right panel ──
    let tabs = segmented(
        "data-flow-side",
        vec![
            Segment::new(
                "Settings",
                side == Side::Settings,
                cx.listener(|console, _, _, cx| {
                    if let Some(editor) = editor_mut(console) {
                        editor.side = Side::Settings;
                    }
                    cx.notify();
                }),
            ),
            Segment::new(
                "Test",
                side == Side::Test,
                cx.listener(|console, _, _, cx| {
                    if let Some(editor) = editor_mut(console) {
                        editor.side = Side::Test;
                    }
                    cx.notify();
                }),
            ),
        ],
        cx,
    );
    let panel_body = match side {
        Side::Settings => inspector_panel(console, &theme, &lists, window, cx),
        Side::Test => test_panel(console, &theme, cx),
    };
    let panel = div()
        .id("data-flow-inspector")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(INSPECTOR_W))
        .h_full()
        .overflow_y_scroll()
        .bg(theme.chrome)
        .border_l_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_none()
                .px(SPACE_7)
                .py(SPACE_4)
                .border_b_1()
                .border_color(theme.border)
                .child(tabs),
        )
        .child(panel_body);

    let notice = match (broken, legacy, graph_server) {
        (Some(error), _, _) => Some((
            format!("The stored flow could not be read ({error}); saving replaces it with this canvas"),
            theme.warning,
        )),
        (None, _, Some(false)) => Some((
            "This server runs flows in a straight line (no node-type catalogue): the built-in steps are shown, and branches cannot be saved".to_owned(),
            theme.fg_muted,
        )),
        (None, true, _) if !read_only => Some((
            "Stored in the older straight-line format: shown as a graph, saved as one".to_owned(),
            theme.fg_muted,
        )),
        _ => None,
    };

    div()
        .key_context(EDITOR_CONTEXT)
        .flex()
        .flex_col()
        .size_full()
        .on_action(cx.listener(|console, _: &EngFlowsSave, _, cx| save(console, cx)))
        // Esc in the palette, a field or the name: back to the canvas.
        .on_action(cx.listener(|console, _: &Dismiss, window, cx| {
            focus_canvas(console, window);
            cx.notify();
        }))
        .child(toolbar)
        .when_some(notice, |this, (text, color)| {
            this.child(bar(text, color, Icon::Info))
        })
        .when_some(message, |this, (text, error)| {
            this.child(bar(
                text,
                if error { theme.danger } else { theme.fg_muted },
                if error { Icon::CircleAlert } else { Icon::Info },
            ))
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .when(!read_only, |this| this.child(palette_list))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(canvas_area)
                        .child(strip),
                )
                .child(panel),
        )
        .into_any_element()
}

fn fit(editor: &mut Editor, size: Vec2) {
    editor.view = View::fit(geometry(editor).bounds(), size);
}

fn strip_button(
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    cx: &mut Context<Console>,
    act: fn(&mut Editor, Vec2),
) -> AnyElement {
    Button::new(id, label)
        .ghost()
        .compact()
        .tooltip(tooltip)
        .on_click(cx.listener(move |console, _, window, cx| {
            with_editor(console, window, cx, act);
        }))
        .into_any_element()
}

fn open_runs(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(flow) = console
        .state::<State>()
        .editor
        .as_ref()
        .map(|editor| editor.flow.clone())
    else {
        return;
    };
    forms::open_runs(console, &flow, window, cx);
}

fn caption(text: String, theme: &Theme) -> gpui::Div {
    div()
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(text)
}

fn dim(text: String, theme: &Theme) -> gpui::Div {
    div()
        .text_size(FONT_SMALL)
        .text_color(theme.fg_muted)
        .child(text)
}

fn heading(color: Hsla, title: String) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .child(div().flex_none().size(px(8.0)).rounded(px(4.0)).bg(color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
}

/// One field as the panel draws it.
struct Shown {
    at: usize,
    def: FieldDef,
    control: Option<AnyElement>,
    line: Option<Entity<TextInput>>,
    fixed: bool,
    error: Option<String>,
}

/// The settings panel: the selection's settings, or what is selected.
#[allow(clippy::too_many_lines)]
fn inspector_panel(
    console: &mut Console,
    theme: &Theme,
    (tags, devices): &(BTreeSet<String>, BTreeSet<String>),
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let Some(editor) = editor_mut(console) else {
        return div().into_any_element();
    };
    let graph = editor.graph.clone();
    let catalogue = Rc::clone(&editor.catalogue);
    let selection = editor.selection;
    let read_only = editor.read_only;
    let fields: Vec<Shown> = editor
        .inspector
        .fields
        .iter()
        .enumerate()
        .map(|(at, field)| Shown {
            at,
            def: field.def.clone(),
            control: match &field.input {
                Input::Line(line) => Some(line.clone().into_any_element()),
                Input::Area(area) => Some(area.clone().into_any_element()),
                Input::Switch | Input::Choice | Input::Fixed => None,
            },
            line: match &field.input {
                Input::Line(line) => Some(line.clone()),
                _ => None,
            },
            fixed: matches!(field.input, Input::Fixed),
            error: field.error.clone(),
        })
        .collect();

    let mut body: Vec<AnyElement> = Vec::new();
    match selection {
        Selection::None => {
            body.push(heading(theme.accent, "Flow".to_owned()).into_any_element());
            body.push(
                dim(
                    format!(
                        "Trigger {} · {} nodes · {} wires",
                        graph.label(End::Trigger, &catalogue),
                        graph.nodes.len(),
                        graph.edges.len()
                    ),
                    theme,
                )
                .into_any_element(),
            );
            body.push(
                dim("Select a node, the trigger or a wire. A message leaves a node by one of its outputs and goes on along every wire from that output.".to_owned(), theme)
                    .into_any_element(),
            );
            let keys: &[(&str, &str)] = if read_only {
                &[("Tab", "next node"), ("+ − 0", "zoom, fit"), ("H", "runs")]
            } else {
                &[
                    ("Tab", "next node"),
                    ("↵", "edit its settings"),
                    ("N", "add from the palette"),
                    ("⌫", "remove the node or wire"),
                    ("⌘D", "duplicate the node"),
                    ("← ↑ ↓ →", "move it (⇧ further)"),
                    ("+ − 0", "zoom, fit"),
                    ("T", "test"),
                    ("⌘Z ⇧⌘Z", "undo, redo"),
                    ("⌘S", "save"),
                ]
            };
            for (keys, what) in keys {
                body.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(div().w(px(72.0)).flex_none().child(kbd(*keys, cx)))
                        .child(*what)
                        .into_any_element(),
                );
            }
        }
        Selection::Wire(edge) => {
            let Some(wire) = graph.edges.get(edge) else {
                return div().into_any_element();
            };
            let from = graph.end_of(&wire.from).map_or_else(
                || wire.from.clone(),
                |end| port_label(&graph, end, wire.port, &catalogue),
            );
            let to = graph.index_of(&wire.to).map_or_else(
                || wire.to.clone(),
                |index| graph.label(End::Node(index), &catalogue),
            );
            body.push(heading(theme.fg_dim, "Wire".to_owned()).into_any_element());
            body.push(dim(format!("{from} → {to}"), theme).into_any_element());
            if !read_only {
                body.push(
                    Button::new("data-flow-remove-wire", "Remove")
                        .danger()
                        .compact()
                        .icon(Icon::Trash)
                        .tooltip("⌫")
                        .on_click(cx.listener(|console, _, window, cx| {
                            with_editor(console, window, cx, |editor, _| {
                                delete_selection(editor);
                            });
                        }))
                        .into_any_element(),
                );
            }
        }
        Selection::Node(end) => {
            let color = node_color(&graph, end, &catalogue, theme);
            match end {
                End::Trigger => {
                    body.push(
                        heading(
                            color,
                            format!("Trigger · {}", graph.label(End::Trigger, &catalogue)),
                        )
                        .into_any_element(),
                    );
                    if let Some(kind) = catalogue.trigger(graph.trigger_type())
                        && !kind.description.is_empty()
                    {
                        body.push(dim(kind.description.clone(), theme).into_any_element());
                    }
                    if !read_only {
                        let current = graph.trigger_type().to_owned();
                        let choices = catalogue.triggers.iter().enumerate().map(|(index, kind)| {
                            let chosen = kind.kind == current;
                            Button::new(("data-flow-trigger-type", index), kind.label().to_owned())
                                .compact()
                                .map(|button| {
                                    if chosen {
                                        button.primary()
                                    } else {
                                        button.ghost()
                                    }
                                })
                                .on_click(cx.listener(move |console, _, _, cx| {
                                    set_trigger_kind(console, index, cx);
                                }))
                        });
                        body.push(
                            div()
                                .flex()
                                .flex_col()
                                .gap(SPACE_2)
                                .child(caption("TRIGGER TYPE".to_owned(), theme))
                                .child(div().flex().flex_wrap().gap(SPACE_2).children(choices))
                                .into_any_element(),
                        );
                    }
                }
                End::Node(index) => {
                    let Some(config) = graph.nodes.get(index) else {
                        return div().into_any_element();
                    };
                    let kind = catalogue.node(config_type(config));
                    body.push(heading(color, graph.label(end, &catalogue)).into_any_element());
                    body.push(
                        div()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child(format!("{} · {}", config_type(config), config_id(config)))
                            .into_any_element(),
                    );
                    match kind {
                        Some(kind) => {
                            let mut about = format!("{} · {}", kind.category().label(), kind.description);
                            if kind.side_effect {
                                about.push_str(" · acts for real when the flow runs");
                            }
                            body.push(dim(about, theme).into_any_element());
                        }
                        None => body.push(
                            dim(
                                format!("This server has no node type “{}”: its settings show as stored", config_type(config)),
                                theme,
                            )
                            .into_any_element(),
                        ),
                    }
                    let outputs = graph.outputs(end, &catalogue);
                    if outputs.len() > 1 {
                        body.push(
                            dim(format!("Outputs: {}", outputs.join(" · ")), theme)
                                .into_any_element(),
                        );
                    }
                }
            }
            let config = match end {
                End::Trigger => Some(&graph.trigger),
                End::Node(index) => graph.nodes.get(index),
            };
            if fields.is_empty() {
                body.push(dim("No settings.".to_owned(), theme).into_any_element());
            }
            for shown in fields {
                let Shown {
                    at,
                    def,
                    control,
                    line,
                    fixed,
                    error,
                } = shown;
                let value = config.and_then(|config| config.get(&def.key)).cloned();
                let control = match control {
                    Some(control) => control,
                    None if fixed => div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg)
                        .child(value.as_ref().map_or_else(|| "—".to_owned(), field_text))
                        .into_any_element(),
                    None if def.kind == FieldKind::Bool => {
                        let on = value.as_ref().and_then(Value::as_bool).unwrap_or(false);
                        let (off_key, on_key) = (def.key.clone(), def.key.clone());
                        segmented(
                            ("data-flow-switch", at),
                            vec![
                                Segment::new(
                                    "Off",
                                    !on,
                                    cx.listener(move |console, _, _, cx| {
                                        set_field(console, &off_key, Value::Bool(false), cx);
                                    }),
                                ),
                                Segment::new(
                                    "On",
                                    on,
                                    cx.listener(move |console, _, _, cx| {
                                        set_field(console, &on_key, Value::Bool(true), cx);
                                    }),
                                ),
                            ],
                            cx,
                        )
                    }
                    None => {
                        let choices =
                            def.choices()
                                .into_iter()
                                .enumerate()
                                .map(|(choice, option)| {
                                    let chosen = value.as_ref() == Some(&option.value);
                                    let key = def.key.clone();
                                    let picked = option.value.clone();
                                    Button::new(
                                        SharedString::from(format!(
                                            "data-flow-choice-{at}-{choice}"
                                        )),
                                        option.label(),
                                    )
                                    .compact()
                                    .map(|button| {
                                        if chosen {
                                            button.primary()
                                        } else {
                                            button.ghost()
                                        }
                                    })
                                    .on_click(cx.listener(
                                        move |console, _, _, cx| {
                                            set_field(console, &key, picked.clone(), cx);
                                        },
                                    ))
                                });
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(SPACE_2)
                            .children(choices)
                            .into_any_element()
                    }
                };
                // Tags and devices the console knows, under a focused field.
                let picks = match (&line, def.kind) {
                    (Some(line), FieldKind::Tag | FieldKind::Device)
                        if line.read(cx).focus_handle(cx).is_focused(window) =>
                    {
                        let list = if def.kind == FieldKind::Tag {
                            tags
                        } else {
                            devices
                        };
                        suggestions(list, line.read(cx).value())
                    }
                    _ => Vec::new(),
                };
                let hint = match def.kind {
                    FieldKind::DurationMs => " (ms)",
                    FieldKind::Json | FieldKind::Other => " (JSON)",
                    FieldKind::Cron => " (cron)",
                    _ => "",
                };
                let star = if def.required { " *" } else { "" };
                let problem = error.or_else(|| {
                    def.is_missing(value.as_ref())
                        .then(|| "Required".to_owned())
                });
                body.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_2)
                        .child(caption(
                            format!("{}{hint}{star}", def.label().to_uppercase()),
                            theme,
                        ))
                        .child(control)
                        .when(!picks.is_empty(), |this| {
                            this.child(div().flex().flex_wrap().gap(SPACE_1).children(
                                picks.into_iter().enumerate().map(|(pick, text)| {
                                    let chosen = text.clone();
                                    Button::new(
                                        SharedString::from(format!("data-flow-pick-{at}-{pick}")),
                                        text,
                                    )
                                    .compact()
                                    .ghost()
                                    .on_click(cx.listener(
                                        move |console, _, window, cx| {
                                            pick_suggestion(
                                                console,
                                                at,
                                                chosen.clone(),
                                                window,
                                                cx,
                                            );
                                        },
                                    ))
                                }),
                            ))
                        })
                        .when_some(def.help.clone().filter(|_| !fixed), |this, help| {
                            this.child(
                                div()
                                    .text_size(FONT_CAPTION)
                                    .text_color(theme.fg_dim)
                                    .child(help),
                            )
                        })
                        .when_some(problem.filter(|_| !fixed), |this, problem| {
                            this.child(
                                div()
                                    .text_size(FONT_CAPTION)
                                    .text_color(theme.danger)
                                    .child(problem),
                            )
                        })
                        .into_any_element(),
                );
            }
            if let End::Node(index) = end
                && !read_only
            {
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(SPACE_2)
                        .pt(SPACE_4)
                        .child(
                            Button::new(("data-flow-duplicate", index), "Duplicate")
                                .ghost()
                                .compact()
                                .icon(Icon::Copy)
                                .tooltip("⌘D")
                                .on_click(cx.listener(|console, _, window, cx| {
                                    with_editor(console, window, cx, duplicate);
                                })),
                        )
                        .child(
                            Button::new(("data-flow-remove", index), "Remove")
                                .danger()
                                .compact()
                                .icon(Icon::Trash)
                                .tooltip("⌫")
                                .on_click(cx.listener(|console, _, window, cx| {
                                    with_editor(console, window, cx, |editor, _| {
                                        delete_selection(editor);
                                    });
                                })),
                        )
                        .into_any_element(),
                );
            }
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .px(SPACE_7)
        .py(SPACE_6)
        .pb(SPACE_10)
        .children(body)
        .into_any_element()
}

/// Select the node of trace row `row` and show its input and output.
fn pick_trace(console: &mut Console, row: usize, window: &mut Window, cx: &mut Context<Console>) {
    if let Some(editor) = editor_mut(console) {
        let size = canvas_size(editor, window);
        editor.test.picked = Some(row);
        let node = match &editor.test.result {
            Some(Ok((run, _))) => run
                .trace
                .get(row)
                .and_then(|step| editor.graph.index_of(&step.node_id)),
            _ => None,
        };
        if let Some(index) = node {
            select(editor, Selection::Node(End::Node(index)));
            if let Some(card) = geometry(editor).card(End::Node(index)) {
                editor.view = editor.view.reveal(card.rect(), size);
            }
        }
    }
    cx.notify();
}

/// The test panel: the input, Run test, the trace.
#[allow(clippy::too_many_lines)]
fn test_panel(console: &mut Console, theme: &Theme, cx: &mut Context<Console>) -> AnyElement {
    let Some(editor) = editor_mut(console) else {
        return div().into_any_element();
    };
    let graph = editor.graph.clone();
    let catalogue = Rc::clone(&editor.catalogue);
    let input = editor.test.input.clone();
    let (result, running, picked, armed, read_only) = (
        editor.test.result.clone(),
        editor.test.running,
        editor.test.picked,
        editor.test.armed,
        editor.read_only,
    );
    let legacy_server = editor.graph_server == Some(false);

    let mut body: Vec<AnyElement> = Vec::new();
    body.push(
        dim(
            if legacy_server {
                "This server tests the saved flow, and its outputs act for real.".to_owned()
            } else {
                "A dry run of the canvas as it is, saved or not: nodes that act (writes, alarms, webhooks) are skipped.".to_owned()
            },
            theme,
        )
        .into_any_element(),
    );
    body.push(
        div()
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .child(caption("INPUT (JSON: PAYLOAD, METADATA)".to_owned(), theme))
            .child(input)
            .into_any_element(),
    );
    if armed {
        body.push(
            div()
                .p(SPACE_4)
                .rounded(RADIUS_MD)
                .bg(theme.warning.opacity(0.08))
                .text_size(FONT_SMALL)
                .text_color(theme.warning)
                .child("Run anyway? The saved flow's outputs will write, post and raise alarms for real. Press Run test again.")
                .into_any_element(),
        );
    }
    if !read_only {
        body.push(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(
                    Button::new(
                        "data-flow-test-run",
                        if running {
                            "Running…"
                        } else if armed {
                            "Run anyway"
                        } else {
                            "Run test"
                        },
                    )
                    .primary()
                    .compact()
                    .icon(Icon::Play)
                    .disabled(running)
                    .tooltip("⌘↵")
                    .on_click(cx.listener(|console, _, _, cx| run_test(console, cx))),
                )
                .when(result.is_some(), |this| {
                    this.child(
                        Button::new("data-flow-test-clear", "Clear")
                            .ghost()
                            .compact()
                            .tooltip("Clear the result and the lit path")
                            .on_click(cx.listener(|console, _, _, cx| {
                                if let Some(editor) = editor_mut(console) {
                                    editor.test.result = None;
                                    editor.test.picked = None;
                                }
                                cx.notify();
                            })),
                    )
                })
                .into_any_element(),
        );
    }
    match result {
        None => {}
        Some(Err(error)) => body.push(
            div()
                .p(SPACE_4)
                .rounded(RADIUS_MD)
                .bg(theme.danger.opacity(0.08))
                .text_size(FONT_SMALL)
                .text_color(theme.danger)
                .child(error)
                .into_any_element(),
        ),
        Some(Ok((run, legacy))) => {
            let (glyph, color, verdict) = if run.ok {
                (Icon::CircleCheck, theme.success, "Ran through")
            } else {
                (Icon::CircleX, theme.danger, "Failed")
            };
            body.push(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .child(vt_ui::icon(glyph, ICON_SM, color))
                    .child(div().text_color(color).child(verdict))
                    .child(div().text_color(theme.fg_muted).child(format!(
                        "· {} nodes · outputs reached {}{}",
                        run.trace.len(),
                        run.outputs_text(),
                        if legacy { " · saved flow" } else { "" }
                    )))
                    .into_any_element(),
            );
            for (row, step) in run.trace.iter().enumerate() {
                let index = graph.index_of(&step.node_id);
                let label = index.map_or_else(
                    || step.node_id.clone(),
                    |index| graph.label(End::Node(index), &catalogue),
                );
                let port = match (index, step.port) {
                    (Some(index), Some(port)) => {
                        let outputs = graph.outputs(End::Node(index), &catalogue);
                        match outputs.get(port) {
                            Some(name) if outputs.len() > 1 => format!("→ {name}"),
                            _ => "→".to_owned(),
                        }
                    }
                    (_, Some(port)) => format!("→ {port}"),
                    (_, None) => "stop".to_owned(),
                };
                let (tint, outcome) = match (&step.error, step.side_effect_skipped) {
                    (Some(_), _) => (theme.danger, "error"),
                    (None, true) => (theme.warning, "skipped"),
                    (None, false) => (theme.success, "ok"),
                };
                let current = picked == Some(row);
                let hover = theme.hover;
                body.push(
                    div()
                        .id(("data-flow-trace", row))
                        .flex()
                        .flex_col()
                        .gap(SPACE_1)
                        .px(SPACE_3)
                        .py(SPACE_2)
                        .rounded(RADIUS_MD)
                        .cursor_pointer()
                        .when(current, |this| this.bg(theme.selected))
                        .when(!current, |this| this.hover(move |style| style.bg(hover)))
                        .on_click(cx.listener(move |console, _, window, cx| {
                            pick_trace(console, row, window, cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_3)
                                .text_size(FONT_SMALL)
                                .child(
                                    div()
                                        .w(px(18.0))
                                        .flex_none()
                                        .font_family(MONO)
                                        .text_color(theme.fg_dim)
                                        .child(format!("{}", row + 1)),
                                )
                                .child(div().flex_1().min_w_0().truncate().child(label))
                                .child(
                                    div()
                                        .flex_none()
                                        .font_family(MONO)
                                        .text_color(theme.fg_muted)
                                        .child(port),
                                )
                                .child(pill(outcome, tint))
                                .when(step.duration_us > 0, |this| {
                                    this.child(
                                        div()
                                            .flex_none()
                                            .font_family(MONO)
                                            .text_color(theme.fg_dim)
                                            .child(format!("{} µs", step.duration_us)),
                                    )
                                }),
                        )
                        .when_some(step.error.clone(), |this, error| {
                            this.child(
                                div()
                                    .pl(px(26.0))
                                    .text_size(FONT_CAPTION)
                                    .text_color(theme.danger)
                                    .child(error),
                            )
                        })
                        .when(current, |this| {
                            this.child(json_block("INPUT", &step.input, theme))
                                .child(json_block("OUTPUT", &step.output, theme))
                        })
                        .into_any_element(),
                );
            }
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .px(SPACE_7)
        .py(SPACE_6)
        .pb(SPACE_10)
        .children(body)
        .into_any_element()
}

fn json_block(title: &str, value: &Value, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_1)
        .pl(px(26.0))
        .child(caption(title.to_owned(), theme))
        .child(
            div()
                .p(SPACE_3)
                .rounded(RADIUS_MD)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .font_family(MONO)
                .text_size(FONT_CAPTION)
                .child(if value.is_null() {
                    "—".to_owned()
                } else {
                    pretty(value)
                }),
        )
}

/// Leaving with unsaved changes: Enter or the button discards, Esc keeps.
fn discard_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(editor) = console.state::<State>().editor.as_ref() else {
        return div().into_any_element();
    };
    let focus = editor.discard_focus.clone();
    let name = editor.flow.name.clone();
    let discard = |console: &mut Console, window: &mut Window, cx: &mut Context<Console>| {
        console.close_overlay(window, cx);
        close_now(console, window, cx);
    };
    div()
        .when_some(focus, |this, focus| this.track_focus(&focus))
        .on_key_down(
            cx.listener(move |console, event: &KeyDownEvent, window, cx| {
                // Enter only while the dialog itself has focus (not a button).
                let own = console
                    .state::<State>()
                    .editor
                    .as_ref()
                    .and_then(|editor| editor.discard_focus.as_ref())
                    .is_some_and(|focus| focus.is_focused(window));
                if own && event.keystroke.key == "enter" {
                    discard(console, window, cx);
                    cx.stop_propagation();
                }
            }),
        )
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(vt_ui::icon(Icon::CircleAlert, ICON_LG, theme.warning))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Discard the changes to {name}?")),
                ),
        )
        .child(
            div()
                .text_color(theme.fg_muted)
                .child("The server keeps the flow as it was last saved."),
        )
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
                        .child("discard")
                        .child(kbd("Esc", cx))
                        .child("keep editing"),
                )
                .child(
                    Button::new("data-flow-discard-keep", "Keep editing")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new("data-flow-discard", "Discard")
                        .danger()
                        .icon(Icon::Trash)
                        .on_click(cx.listener(move |console, _, window, cx| {
                            discard(console, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::console::testing::Harness;

    const CATALOGUE: &str = r#"{
        "triggers": [
            { "type": "timer", "label": "Timer",
              "fields": [ { "key": "interval_ms", "label": "Interval", "kind": "duration_ms",
                            "required": true, "default": 5000 } ] },
            { "type": "tag_change", "label": "Tag change",
              "fields": [ { "key": "tag", "label": "Tag", "kind": "tag", "required": true } ] }
        ],
        "nodes": [
            { "type": "delay", "label": "Delay", "category": "timing", "description": "Wait",
              "outputs": ["out"],
              "fields": [ { "key": "duration_ms", "label": "Duration", "kind": "duration_ms",
                            "required": true, "default": 1000 } ] },
            { "type": "switch", "label": "Switch", "category": "logic", "description": "Route",
              "outputs": ["otherwise"],
              "fields": [ { "key": "rules", "label": "Rules", "kind": "json", "required": true,
                            "default": [] } ] },
            { "type": "alarm_out", "label": "Alarm", "category": "output",
              "description": "Raise an alarm", "outputs": [], "side_effect": true,
              "fields": [ { "key": "message", "label": "Message", "kind": "string",
                            "required": true } ] }
        ]
    }"#;

    /// trigger → switch; rule "hot" → alarm, otherwise → delay.
    const GRAPH: &str = r#"{"version": 2,
        "nodes": [
            {"id": "s", "type": "switch", "x": 260, "y": 0,
             "rules": [{"name": "hot", "expression": "payload.temp > 80"}]},
            {"id": "a", "type": "alarm_out", "x": 520, "y": 0, "message": "Hot", "colour": "red"},
            {"id": "d", "type": "delay", "x": 520, "y": 110, "duration_ms": 1000}
        ],
        "edges": [
            {"from": "trigger", "port": 0, "to": "s"},
            {"from": "s", "port": 0, "to": "a"},
            {"from": "s", "port": 1, "to": "d"}
        ]}"#;

    const SECONDARY: &str = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };

    fn catalogue() -> Catalogue {
        serde_json::from_str(CATALOGUE).unwrap()
    }

    fn flow() -> Flow {
        Flow {
            id: "f".into(),
            name: "Boiler".into(),
            enabled: true,
            trigger_config: r#"{"type": "timer", "interval_ms": 5000}"#.into(),
            nodes: GRAPH.into(),
            created_at: 1,
            updated_at: 1,
        }
    }

    /// The editor on [`GRAPH`] with the test catalogue, signed in as `role`.
    fn opened(cx: &mut TestAppContext, role: &str) -> Harness {
        let mut h = Harness::new(cx);
        h.sign_in(role);
        h.update(|console, window, cx| {
            let state = console.state::<State>();
            state.catalogue = Some(Rc::new(catalogue()));
            state.graph_server = Some(true);
            state.catalogue_asked = true;
            super::super::super::launch(console, "flows");
            console.show(crate::console::pages::Page::DataManager, window, cx);
            open(console, flow(), window, cx);
        });
        h.draw();
        h.update(|console, window, _| focus_canvas(console, window));
        h.draw();
        h
    }

    fn with<R>(h: &mut Harness, f: impl FnOnce(&mut Editor) -> R) -> R {
        h.update(|console, _, _| f(editor_mut(console).expect("the editor is open")))
    }

    fn ids(editor: &Editor) -> Vec<String> {
        editor
            .graph
            .nodes
            .iter()
            .map(|node| config_id(node).to_owned())
            .collect()
    }

    #[test]
    fn the_palette_lists_triggers_then_categories_and_filters() {
        let catalogue = catalogue();
        assert_eq!(
            palette_items(&catalogue, ""),
            [
                Item::Trigger(0),
                Item::Trigger(1),
                Item::Node(1),
                Item::Node(2),
                Item::Node(0)
            ],
            "logic, output, timing"
        );
        assert_eq!(palette_items(&catalogue, "wait"), [Item::Node(0)]);
        assert_eq!(palette_items(&catalogue, "tag"), [Item::Trigger(1)]);
        assert!(palette_items(&catalogue, "zzz").is_empty());
        // The built-in catalogue when the server has none.
        let builtin = Catalogue::builtin();
        assert_eq!(palette_items(&builtin, "").len(), 3 + 11);
    }

    #[test]
    fn fields_come_from_the_catalogue_then_what_the_node_has() {
        let catalogue = catalogue();
        let (graph, _) = FlowGraph::parse(r#"{"type": "timer"}"#, GRAPH).unwrap();
        let alarm: Vec<(String, FieldKind, bool)> =
            field_defs(&graph, Selection::Node(End::Node(1)), &catalogue)
                .into_iter()
                .map(|def| (def.key, def.kind, def.required))
                .collect();
        assert_eq!(
            alarm,
            [
                ("name".to_owned(), FieldKind::String, false),
                ("message".to_owned(), FieldKind::String, true),
                ("colour".to_owned(), FieldKind::String, false),
            ],
            "the label, the type's field, then an undeclared setting"
        );
        let switch = field_defs(&graph, Selection::Node(End::Node(0)), &catalogue);
        assert_eq!(switch[1].kind, FieldKind::Json);
        let trigger = field_defs(&graph, Selection::Node(End::Trigger), &catalogue);
        assert_eq!(trigger.len(), 1);
        assert_eq!(trigger[0].kind, FieldKind::DurationMs);
        assert!(field_defs(&graph, Selection::Wire(0), &catalogue).is_empty());
    }

    #[test]
    fn suggestions_and_server_errors_point_somewhere() {
        let tags: BTreeSet<String> = ["inv1.power", "inv1.temp", "inv2.power"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(suggestions(&tags, "power"), ["inv1.power", "inv2.power"]);
        assert!(suggestions(&tags, "inv1.temp").is_empty(), "already exact");
        assert_eq!(suggestions(&tags, "").len(), 3);
        let (graph, _) = FlowGraph::parse(r#"{"type": "timer"}"#, GRAPH).unwrap();
        assert_eq!(
            named_node(&graph, "node d: duration_ms must be positive"),
            None,
            "one-letter ids are too short to tell"
        );
        let mut long = graph.clone();
        long.nodes[2].insert("id".into(), Value::String("delay_1".into()));
        assert_eq!(
            named_node(&long, "node delay_1: duration_ms must be positive"),
            Some(2)
        );
    }

    #[gpui::test]
    fn duplicate_undo_redo_and_delete_on_the_canvas(cx: &mut TestAppContext) {
        let mut h = opened(cx, "admin");
        assert!(with(&mut h, |editor| !editor.read_only && !editor.legacy));
        with(&mut h, |editor| {
            select(editor, Selection::Node(End::Node(2)))
        });
        h.draw();
        h.keys(&format!("{SECONDARY}-d"));
        let after = with(&mut h, |editor| (ids(editor).len(), editor.selection));
        assert_eq!(
            after,
            (4, Selection::Node(End::Node(3))),
            "the copy is selected"
        );
        h.keys(&format!("{SECONDARY}-z"));
        assert_eq!(with(&mut h, |editor| ids(editor).len()), 3, "undone");
        h.keys(&format!("{SECONDARY}-shift-z"));
        assert_eq!(with(&mut h, |editor| ids(editor).len()), 4, "redone");
        // Delete the switch: its three wires go with it.
        with(&mut h, |editor| {
            select(editor, Selection::Node(End::Node(0)))
        });
        h.draw();
        h.keys("backspace");
        let (nodes, edges) = with(&mut h, |editor| (ids(editor), editor.graph.edges.len()));
        assert_eq!(nodes, ["a", "d", nodes[2].as_str()]);
        assert_eq!(edges, 0);
        // A wire: drawn from the trigger onto the delay, then removed.
        with(&mut h, |editor| connect(editor, End::Trigger, 0, 1));
        assert_eq!(with(&mut h, |editor| editor.selection), Selection::Wire(0));
        h.draw();
        h.keys("delete");
        assert!(with(&mut h, |editor| editor.graph.edges.is_empty()));
        // Saving sends the graph.
        h.commands();
        h.keys(&format!("{SECONDARY}-s"));
        let sent = h.calls();
        assert!(
            sent.iter().any(|key| key.name == "data.flows.save"),
            "{sent:?}"
        );
    }

    #[gpui::test]
    fn wiring_by_the_pointer_from_a_named_port(cx: &mut TestAppContext) {
        let mut h = opened(cx, "admin");
        h.update(|console, _, _| {
            let editor = editor_mut(console).unwrap();
            // Drop the existing "hot" wire, then draw it again by hand.
            editor.graph.edges.remove(1);
            let geometry = geometry(editor);
            let port = geometry.card(End::Node(0)).unwrap().output_port(0);
            let input = geometry.card(End::Node(1)).unwrap().input_port();
            let view = editor.view;
            assert!(press(console, view.to_screen(port), 1));
            assert!(drag_to(console, view.to_screen(input)));
            release(console, view.to_screen(input));
            let editor = editor_mut(console).unwrap();
            let last = editor.graph.edges.last().unwrap();
            assert_eq!(
                (last.from.as_str(), last.port, last.to.as_str()),
                ("s", 0, "a")
            );
            let message = editor.message.clone().unwrap().0;
            assert_eq!(message, "Switch · hot now feeds Alarm");
        });
    }

    #[gpui::test]
    fn a_required_field_blocks_saving_and_selects_the_node(cx: &mut TestAppContext) {
        let mut h = opened(cx, "admin");
        with(&mut h, |editor| {
            editor.graph.nodes[1].insert("message".into(), Value::String(String::new()));
        });
        h.update(|console, _, cx| save(console, cx));
        let (selection, message) = with(&mut h, |editor| {
            (editor.selection, editor.message.clone().unwrap())
        });
        assert_eq!(selection, Selection::Node(End::Node(1)));
        assert_eq!(message, ("Alarm (a): Message is required".to_owned(), true));
        assert!(!h.calls().iter().any(|key| key.name == "data.flows.save"));
    }

    #[gpui::test]
    fn a_test_run_lights_its_path_and_rows_select_nodes(cx: &mut TestAppContext) {
        let mut h = opened(cx, "admin");
        h.update(|console, _, cx| run_test(console, cx));
        let seq = with(&mut h, |editor| {
            assert!(editor.test.running);
            assert_eq!(editor.side, Side::Test);
            editor.test.seq
        });
        assert!(
            h.calls()
                .iter()
                .any(|key| key.name == "data.flows.draft_test"),
            "a dry run of the draft"
        );
        let run: TestRun = serde_json::from_str(
            r#"{"ok": true, "outputs_reached": 0, "trace": [
                {"node_id": "s", "port": 1}, {"node_id": "d", "port": 0}]}"#,
        )
        .unwrap();
        h.reply(
            CallKey::new("data.flows.draft_test").with(seq),
            Reply::ok((run, false)),
        );
        h.draw();
        let path = with(&mut h, |editor| shown_path(editor).unwrap());
        assert_eq!(path.edges.len(), 2, "trigger → switch → delay");
        let marks = h.update(|console, window, cx| {
            let theme = theme(cx).clone();
            let editor = editor_mut(console).unwrap();
            let size = canvas_size(editor, window);
            let scene = scene(editor, size, &theme);
            (
                scene.cards.iter().map(|card| card.mark).collect::<Vec<_>>(),
                scene.wires.iter().map(|wire| wire.mark).collect::<Vec<_>>(),
            )
        });
        assert_eq!(
            marks.0,
            [Mark::Taken, Mark::Taken, Mark::Dim, Mark::Taken],
            "the alarm was not reached"
        );
        assert_eq!(marks.1, [Mark::Taken, Mark::Dim, Mark::Taken]);
        h.update(|console, window, cx| pick_trace(console, 1, window, cx));
        assert_eq!(
            with(&mut h, |editor| editor.selection),
            Selection::Node(End::Node(2))
        );
    }

    #[gpui::test]
    fn below_operator_the_flow_is_read_only(cx: &mut TestAppContext) {
        let mut h = opened(cx, "viewer");
        assert!(with(&mut h, |editor| editor.read_only));
        with(&mut h, |editor| {
            select(editor, Selection::Node(End::Node(0)))
        });
        h.draw();
        h.keys("backspace");
        h.keys(&format!("{SECONDARY}-d"));
        assert_eq!(
            with(&mut h, |editor| ids(editor).len()),
            3,
            "nothing changes"
        );
        // The pointer selects and pans but neither moves nor wires.
        h.update(|console, _, _| {
            let editor = editor_mut(console).unwrap();
            let geometry = geometry(editor);
            let port = geometry.card(End::Node(0)).unwrap().output_port(0);
            let at = editor.view.to_screen(port);
            press(console, at, 1);
            assert!(editor_mut(console).unwrap().drag.is_none());
        });
        h.commands();
        h.update(|console, _, cx| save(console, cx));
        assert!(h.calls().is_empty(), "no save from a read-only editor");
    }

    #[gpui::test]
    fn a_legacy_server_saves_a_straight_line_and_refuses_branches(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| {
            let state = console.state::<State>();
            state.catalogue = Some(Rc::new(Catalogue::builtin()));
            state.graph_server = Some(false);
            state.catalogue_asked = true;
            let legacy = Flow {
                nodes: r#"[{"id": "f1", "type": "filter", "condition": "true"},
                           {"id": "o1", "type": "alarm_out", "severity": "warning",
                            "source_key": "k", "message": "m"}]"#
                    .into(),
                ..flow()
            };
            open(console, legacy, window, cx);
        });
        let texts = with(&mut h, |editor| {
            assert!(editor.legacy);
            assert_eq!(editor.graph.edges.len(), 2);
            save_texts(editor)
        });
        let (_, nodes) = texts.unwrap();
        assert!(nodes.trim_start().starts_with('['), "the legacy array");
        // A second wire from the trigger: a branch the old engine cannot run.
        with(&mut h, |editor| connect(editor, End::Trigger, 0, 1));
        h.update(|console, _, cx| save(console, cx));
        let message = with(&mut h, |editor| editor.message.clone().unwrap());
        assert!(
            message.1 && message.0.contains("straight line"),
            "{message:?}"
        );
        assert!(!h.calls().iter().any(|key| key.name == "data.flows.save"));
    }
}

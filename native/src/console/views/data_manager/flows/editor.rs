//! The native flow editor: the web client's `FlowEditor.tsx` (palette,
//! canvas, configuration panel, test) on a GPUI canvas.
//!
//! - Palette (left): the triggers and the step types of the server's
//!   engine, as the web client lists them. Enter or a click adds the
//!   type after the selected step (or into the selected wire); drag one
//!   onto the canvas to place it there. A trigger type replaces the
//!   trigger with its defaults.
//! - Canvas (centre, [`super::canvas`]): drag the background or scroll to
//!   pan, ⌘-scroll or + − 0 to zoom about the pointer, drag a card to
//!   move it, drag from a card's output port onto another card to wire
//!   them, click a card or a wire to select it.
//! - Configuration (right): the selected step's settings, one field per
//!   setting by its JSON type as the web client's `ConfigPanel`, the
//!   trigger's per type.
//!
//! The stored flow has no wires: the engine runs the process steps in
//! order, then the outputs from the last one. A wire drawn from A to B
//! makes B run right after A (the steps reorder); a wire into an output
//! can only come from the chain's end. Positions are this editor's own,
//! laid out as the web client lays them out, and A arranges them again.
//! ⌘S saves (`PUT`), ⌘Z undoes a structural edit or a move.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, Context, Entity, EntityInputHandler, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, Pixels, Subscription, Window, div, px,
};
use serde_json::Value;
use voltrus_scada_native::api::eng_flows::{
    Category, End, FieldKind, Flow, FlowGraph, NodeKind, Order, TRIGGER_KINDS, TriggerKind,
    category_of, config_id, config_type, field_text, label_of, node_fields, node_kind, palette,
    parse_json, parse_number, preview, trigger_fields, trigger_kind, trigger_value,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, kbd, pill, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::canvas::{self, Card, Colors, Scene};
use super::model::{self, Hit, Layout, NODE_H, NODE_W, Vec2, View, node_rect};
use super::{LIST, State, forms, now_millis, now_secs, refresh, tone_color};
use crate::console::Console;
use crate::console::keys::{
    Dismiss, EngFlowsAddStep, EngFlowsArrange, EngFlowsDeleteSelection, EngFlowsEarlier,
    EngFlowsFit, EngFlowsInspect, EngFlowsLater, EngFlowsNudgeDown, EngFlowsNudgeLeft,
    EngFlowsNudgeRight, EngFlowsNudgeUp, EngFlowsRuns, EngFlowsSave, EngFlowsTest, EngFlowsUndo,
    EngFlowsZoomIn, EngFlowsZoomOut, FocusNext, FocusPrevious,
};
use crate::console::overlay::Overlay;
use crate::console::text_area::TextArea;
use crate::console::views::{ListKeys, focus_ring, keyboard_list};

/// The palette's keyboard list.
const PALETTE_LIST: &str = "data.flows.palette";
/// Key contexts: the whole editor (⌘S works in its fields too) and the
/// canvas (`keys::ENG_FLOW_EDITOR`, `keys::ENG_FLOW_CANVAS`).
const EDITOR_CONTEXT: &str = "EngFlowEditor";
const CANVAS_CONTEXT: &str = "EngFlowCanvas";
const DISCARD: &str = "data-flow-discard";
const PALETTE_W: f32 = 200.0;
const INSPECTOR_W: f32 = 300.0;
/// What the editor's chrome takes from the window's height, for the
/// canvas's size before it was first laid out.
const CHROME_H: f32 = 200.0;
/// Arrow keys move a card this far (⇧: further), or pan the view.
const NUDGE: f32 = 10.0;
const NUDGE_FAR: f32 = 50.0;
const PAN_STEP: f32 = 60.0;
const UNDO_DEPTH: usize = 100;
/// The palette's groups, as the web client's: input, process, output.
const GROUPS: [(&str, Category); 3] = [
    ("INPUT", Category::Input),
    ("PROCESS", Category::Process),
    ("OUTPUT", Category::Output),
];

/// What is selected on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    None,
    Step(End),
    /// The wire into step `to`.
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
    /// A new wire from `from`'s output to the pointer (world), and the step
    /// it would land on.
    Wire {
        from: End,
        to: Vec2,
        target: Option<usize>,
    },
}

/// One palette entry.
#[derive(Clone, Copy)]
pub(crate) enum Item {
    Trigger(&'static TriggerKind),
    Step(&'static NodeKind),
}

impl Item {
    fn label(self) -> &'static str {
        match self {
            Self::Trigger(kind) => kind.label,
            Self::Step(kind) => kind.label,
        }
    }

    fn color(self, theme: &Theme) -> Hsla {
        match self {
            Self::Trigger(kind) => tone_color(kind.tone, theme),
            Self::Step(kind) => tone_color(kind.tone, theme),
        }
    }
}

/// A palette entry on its way to the canvas.
#[derive(Clone)]
pub(crate) struct PaletteDrag(Item);

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
            .border_color(self.0.color(theme))
            .shadow(theme.shadow(Elevation::Popover))
            .text_size(FONT_SMALL)
            .text_color(theme.fg)
            .child(div().size(px(8.0)).rounded(px(4.0)).bg(self.0.color(theme)))
            .child(self.0.label())
    }
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
    key: String,
    label: String,
    kind: FieldKind,
    input: Input,
    error: Option<String>,
}

enum Input {
    Line(Entity<TextInput>),
    Area(Entity<TextArea>),
    /// A true/false setting: Off/On, read from the graph.
    Switch,
}

/// The graph, the layout and the selection before an edit.
struct Snapshot {
    graph: FlowGraph,
    layout: Layout,
    selection: Selection,
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
    graph: FlowGraph,
    layout: Layout,
    view: View,
    selection: Selection,
    drag: Option<Drag>,
    undo: Vec<Snapshot>,
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
    /// Palette groups folded away.
    folded: [bool; 3],
    discard_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

fn editor_mut(console: &mut Console) -> Option<&mut Editor> {
    console.state::<State>().editor.as_mut()
}

/// Open `flow` on the canvas.
pub(super) fn open(
    console: &mut Console,
    flow: Flow,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let (graph, broken) = match FlowGraph::of(&flow) {
        Ok(graph) => (graph, None),
        Err(error) => {
            let mut graph = FlowGraph::default();
            graph.set_trigger_kind(&TRIGGER_KINDS[0]);
            (graph, Some(error))
        }
    };
    let layout = Layout::arrange(&graph);
    let name_text = flow.name.clone();
    let name = cx.new(|cx| TextInput::new(cx).text(name_text).placeholder("Flow name"));
    let subscription = cx.subscribe_in(
        &name,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted | InputEvent::Cancelled => focus_canvas(console, window),
            InputEvent::Changed => cx.notify(),
            InputEvent::Blurred => {}
        },
    );
    let size = estimated_size(window);
    let view = View::fit(layout.bounds(), size);
    let enabled = flow.enabled;
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.editor = Some(Editor {
        flow,
        saved: graph.clone(),
        broken,
        graph,
        layout,
        view,
        selection: Selection::None,
        drag: None,
        undo: Vec::new(),
        name,
        enabled,
        canvas_focus: cx.focus_handle(),
        bounds: Rc::new(Cell::new(None)),
        inspector: Inspector::default(),
        focus_inspector: false,
        message: None,
        saving: false,
        save_seq: 0,
        folded: [false; 3],
        discard_focus: None,
        _subscriptions: vec![subscription],
    });
    console.list(PALETTE_LIST, cx).cursor = None;
    focus_canvas(console, window);
    cx.notify();
}

pub(super) fn focus_canvas(console: &mut Console, window: &mut Window) {
    if let Some(editor) = editor_mut(console) {
        window.focus(&editor.canvas_focus);
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
            Input::Switch => {}
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
    }
}

/// Keep the graph, the layout and the selection for ⌘Z.
fn remember(editor: &mut Editor) {
    if editor.undo.len() == UNDO_DEPTH {
        editor.undo.remove(0);
    }
    editor.undo.push(Snapshot {
        graph: editor.graph.clone(),
        layout: editor.layout.clone(),
        selection: editor.selection,
    });
}

/// The selection after a structural edit moved the steps.
fn remap(selection: Selection, order: &[Option<usize>]) -> Selection {
    let find = |old: usize| order.iter().position(|before| *before == Some(old));
    match selection {
        Selection::Step(End::Node(old)) => {
            find(old).map_or(Selection::None, |new| Selection::Step(End::Node(new)))
        }
        Selection::Wire(old) => find(old).map_or(Selection::None, Selection::Wire),
        other => other,
    }
}

/// Run a structural edit; when it changed something, keep the state
/// before it for ⌘Z, move the positions and the selection along.
fn restructure(editor: &mut Editor, edit: impl FnOnce(&mut FlowGraph) -> Option<Order>) -> bool {
    let before = Snapshot {
        graph: editor.graph.clone(),
        layout: editor.layout.clone(),
        selection: editor.selection,
    };
    let Some(order) = edit(&mut editor.graph) else {
        return false;
    };
    if editor.undo.len() == UNDO_DEPTH {
        editor.undo.remove(0);
    }
    editor.undo.push(before);
    editor.layout.reorder(&order, Vec2::default());
    editor.selection = remap(editor.selection, &order);
    editor.inspector.shows = None;
    true
}

fn say(editor: &mut Editor, text: impl Into<String>, error: bool) {
    editor.message = Some((text.into(), error));
}

/// Wire `from` into step `to`.
fn connect(editor: &mut Editor, from: End, to: usize) {
    let (what, after) = (
        editor.graph.end_label(End::Node(to)),
        editor.graph.end_label(from),
    );
    let mut refused = None;
    let mut same = false;
    let changed = restructure(editor, |graph| match graph.connect(from, to) {
        Ok(Some(order)) => Some(order),
        Ok(None) => {
            same = true;
            None
        }
        Err(error) => {
            refused = Some(error);
            None
        }
    });
    match (changed, refused) {
        (true, _) => say(
            editor,
            format!("{what} now runs right after {after}"),
            false,
        ),
        (false, Some(error)) => say(editor, error, true),
        (false, None) if same => say(editor, format!("{after} already feeds {what}"), false),
        (false, None) => {}
    }
}

/// Add a palette entry: a step after the selected one (or into the
/// selected wire) at `at`, or the trigger's type.
fn add_item(editor: &mut Editor, item: Item, at: Option<Vec2>, size: Vec2) {
    match item {
        Item::Trigger(kind) => {
            remember(editor);
            editor.graph.set_trigger_kind(kind);
            editor.inspector.shows = None;
            select(editor, Selection::Step(End::Trigger));
            say(
                editor,
                format!("The trigger is now {} (its defaults)", kind.label),
                false,
            );
        }
        Item::Step(kind) => {
            let after = match editor.selection {
                Selection::Step(end) => Some(end),
                Selection::Wire(to) => editor.graph.source_of(to),
                Selection::None => None,
            };
            let near = match (at, after) {
                (Some(at), _) => at,
                (None, Some(end)) => editor.layout.at(end) + Vec2::new(NODE_W + 48.0, 0.0),
                (None, None) => {
                    editor.view.visible(size).center() - Vec2::new(NODE_W / 2.0, NODE_H / 2.0)
                }
            };
            let place = if at.is_some() {
                near
            } else {
                editor.layout.free_spot(near)
            };
            let id = editor.graph.fresh_id(now_millis());
            remember(editor);
            let (index, order) = editor.graph.add(kind, after, id);
            editor.layout.reorder(&order, place);
            editor.inspector.shows = None;
            editor.selection = Selection::Step(End::Node(index));
            editor.view = editor.view.reveal(node_rect(place), size);
            let source = editor
                .graph
                .source_of(index)
                .map(|from| editor.graph.end_label(from))
                .unwrap_or_default();
            say(
                editor,
                format!("Added {}, fed by {source}", kind.label),
                false,
            );
        }
    }
}

fn delete_selection(editor: &mut Editor) {
    match editor.selection {
        Selection::Step(End::Trigger) => say(
            editor,
            "A flow keeps its trigger: change its type in the settings on the right",
            true,
        ),
        Selection::Step(End::Node(index)) => {
            let label = editor.graph.end_label(End::Node(index));
            if restructure(editor, |graph| graph.remove(index)) {
                editor.selection = Selection::None;
                say(editor, format!("Removed {label}"), false);
            }
        }
        Selection::Wire(_) => say(
            editor,
            "Wires are the order the steps run in: drag from another step's output onto this step to rewire it, or remove a step",
            true,
        ),
        Selection::None => {}
    }
}

fn shift(editor: &mut Editor, earlier: bool) {
    let Selection::Step(End::Node(index)) = editor.selection else {
        say(editor, "Select a step to move it in the run order", true);
        return;
    };
    let label = editor.graph.end_label(End::Node(index));
    if restructure(editor, |graph| graph.shift(index, earlier)) {
        let when = if earlier { "earlier" } else { "later" };
        say(editor, format!("{label} runs one place {when}"), false);
    } else {
        let end = if earlier { "first" } else { "last" };
        let among = match editor.graph.category(index) {
            Category::Output => "outputs",
            _ => "process steps",
        };
        say(
            editor,
            format!("{label} already runs {end} among the {among}"),
            true,
        );
    }
}

fn undo(editor: &mut Editor) {
    match editor.undo.pop() {
        Some(snapshot) => {
            editor.graph = snapshot.graph;
            editor.layout = snapshot.layout;
            editor.selection = snapshot.selection;
            editor.inspector.shows = None;
            editor.drag = None;
            say(editor, "Undone", false);
        }
        None => say(editor, "Nothing to undo", true),
    }
}

fn set_trigger_kind(console: &mut Console, kind: &'static TriggerKind, cx: &mut Context<Console>) {
    if let Some(editor) = editor_mut(console)
        && editor.graph.trigger_type() != kind.id
    {
        add_item(editor, Item::Trigger(kind), None, Vec2::default());
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
    match model::hit(&editor.graph, &editor.layout, world, editor.view.zoom) {
        Hit::Output(from) => {
            editor.drag = Some(Drag::Wire {
                from,
                to: world,
                target: None,
            });
        }
        Hit::Node(end) => {
            select(editor, Selection::Step(end));
            if clicks >= 2 {
                editor.focus_inspector = true;
            }
            editor.drag = Some(Drag::Move {
                end,
                grab: world - editor.layout.at(end),
                moved: false,
            });
        }
        Hit::Wire(to) => select(editor, Selection::Wire(to)),
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
                remember(editor);
            }
            editor.layout.set(end, world - grab);
            Drag::Move {
                end,
                grab,
                moved: true,
            }
        }
        Drag::Wire { from, .. } => {
            let target = model::drop_target(&editor.graph, &editor.layout, world, editor.view.zoom)
                .filter(|target| End::Node(*target) != from);
            Drag::Wire {
                from,
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
    let Some(Drag::Wire { from, .. }) = editor.drag.take() else {
        return;
    };
    let world = editor.view.to_world(at);
    if let Some(target) = model::drop_target(&editor.graph, &editor.layout, world, editor.view.zoom)
        && End::Node(target) != from
    {
        connect(editor, from, target);
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
        Selection::Step(end) => {
            let step = if far { NUDGE_FAR } else { NUDGE };
            remember(editor);
            let at = editor.layout.at(end) + Vec2::new(dx * step, dy * step);
            editor.layout.set(end, at);
            editor.view = editor.view.reveal(node_rect(at), size);
        }
        _ => {
            editor.view = editor
                .view
                .panned(Vec2::new(-dx * PAN_STEP, -dy * PAN_STEP))
        }
    });
}

fn cycle(console: &mut Console, forward: bool, window: &mut Window, cx: &mut Context<Console>) {
    with_editor(console, window, cx, |editor, size| {
        let from = match editor.selection {
            Selection::Step(end) => Some(end),
            Selection::Wire(to) => Some(End::Node(to)),
            Selection::None => None,
        };
        let next = model::cycle(&editor.graph, from, forward);
        select(editor, Selection::Step(next));
        editor.view = editor.view.reveal(node_rect(editor.layout.at(next)), size);
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
    if !dirty(editor, cx) {
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

/// ⌘S: `PUT /api/v1/flows/{id}` with the canvas's graph.
fn save(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    if editor.saving {
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
        .find_map(|field| Some((field.label.clone(), field.error.clone()?)))
    {
        say(editor, format!("{label}: {error}"), true);
        cx.notify();
        return;
    }
    if let Err(error) = editor.graph.validate() {
        say(editor, error, true);
        cx.notify();
        return;
    }
    let body = editor
        .flow
        .with_graph(&name, editor.enabled, &editor.graph, now_secs());
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
                say(editor, format!("Saving failed: {error}"), true);
            }
            None => console.show_toast(format!("Saving the flow failed: {error}"), true, cx),
        },
    }
}

// ── Settings fields ──────────────────────────────────────────────────────

/// Make the panel's fields for the selection, once per selection.
fn ensure_inspector(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = editor_mut(console) else {
        return;
    };
    if editor.inspector.shows == Some(editor.selection) {
        return;
    }
    let selection = editor.selection;
    let specs: Vec<(String, String, FieldKind, Value)> = match selection {
        Selection::Step(End::Trigger) => trigger_fields(editor.graph.trigger_type())
            .iter()
            .map(|field| {
                (
                    field.key.to_owned(),
                    field.label.to_owned(),
                    field.kind,
                    trigger_value(&editor.graph.trigger, field.key),
                )
            })
            .collect(),
        Selection::Step(End::Node(index)) => editor
            .graph
            .nodes
            .get(index)
            .map(|config| {
                node_fields(config)
                    .into_iter()
                    .map(|(key, kind)| {
                        let value = config.get(&key).cloned().unwrap_or(Value::Null);
                        (key.clone(), key, kind, value)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Selection::Wire(_) | Selection::None => Vec::new(),
    };
    let mut fields = Vec::with_capacity(specs.len());
    let mut subscriptions = Vec::with_capacity(specs.len());
    for (index, (key, label, kind, value)) in specs.into_iter().enumerate() {
        let on_event = move |console: &mut Console,
                             event: &InputEvent,
                             window: &mut Window,
                             cx: &mut Context<Console>| match event {
            InputEvent::Changed => field_changed(console, index, cx),
            InputEvent::Submitted | InputEvent::Cancelled => focus_canvas(console, window),
            InputEvent::Blurred => {}
        };
        let text = field_text(&value);
        let input = match kind {
            FieldKind::Bool => Input::Switch,
            FieldKind::Json => {
                let area = cx.new(|cx| TextArea::new(4, cx));
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
            }
            FieldKind::Text | FieldKind::Number => {
                let line = cx.new(|cx| {
                    let line = TextInput::new(cx).text(text).mono();
                    if kind == FieldKind::Number {
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
            }
        };
        fields.push(Field {
            key,
            label,
            kind,
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

/// Write `value` to the setting `key` of what the panel shows.
fn set_value(editor: &mut Editor, key: &str, value: Value) {
    match editor.inspector.shows {
        Some(Selection::Step(End::Trigger)) => {
            editor.graph.trigger.insert(key.to_owned(), value);
        }
        Some(Selection::Step(End::Node(index))) => {
            if let Some(config) = editor.graph.nodes.get_mut(index) {
                config.insert(key.to_owned(), value);
            }
        }
        _ => {}
    }
}

/// A field's text changed: store it if it reads as its type.
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
        Input::Switch => return,
    };
    let parsed = match field.kind {
        FieldKind::Text => Ok(Value::String(text)),
        FieldKind::Number => parse_number(&text),
        FieldKind::Json => parse_json(&text),
        FieldKind::Bool => return,
    };
    let key = field.key.clone();
    let error = match parsed {
        Ok(value) => {
            set_value(editor, &key, value);
            None
        }
        Err(error) => Some(error),
    };
    if let Some(field) = editor.inspector.fields.get_mut(index) {
        field.error = error;
    }
    cx.notify();
}

fn set_switch(console: &mut Console, key: &str, on: bool, cx: &mut Context<Console>) {
    if let Some(editor) = editor_mut(console) {
        set_value(editor, key, Value::Bool(on));
    }
    cx.notify();
}

// ── Palette ──────────────────────────────────────────────────────────────

/// The entries the palette lists, its folded groups left out.
fn palette_items(folded: [bool; 3]) -> Vec<Item> {
    let mut items = Vec::new();
    for (group, (_, category)) in GROUPS.iter().enumerate() {
        if folded[group] {
            continue;
        }
        match category {
            Category::Input => items.extend(TRIGGER_KINDS.iter().map(Item::Trigger)),
            category => items.extend(palette(*category).map(Item::Step)),
        }
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
    let Some(item) = palette_items(editor.folded).get(row).copied() else {
        return;
    };
    let size = canvas_size(editor, window);
    add_item(editor, item, None, size);
    window.focus(&editor.canvas_focus);
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// The line on a trigger's card: what it listens to.
fn trigger_summary(graph: &FlowGraph) -> String {
    let text = |key: &str| {
        graph
            .trigger
            .get(key)
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    };
    match graph.trigger_type() {
        "modbus" => {
            let metric = text("metric");
            if metric.is_empty() {
                format!("device {}", text("device_id"))
            } else {
                format!("device {} · {metric}", text("device_id"))
            }
        }
        "timer" => format!("every {} ms", text("interval_ms")),
        "mqtt" => text("topic"),
        other => other.to_owned(),
    }
}

/// What the canvas draws this frame: only what is in view.
fn scene(editor: &Editor, size: Vec2, theme: &Theme) -> Scene {
    let graph = &editor.graph;
    let visible = editor.view.visible(size).inflate(32.0);
    let target = match editor.drag {
        Some(Drag::Wire { target, .. }) => target,
        _ => None,
    };
    let mut cards = Vec::new();
    let mut step = 0;
    for end in model::ends(graph) {
        let category = match end {
            End::Trigger => Category::Input,
            End::Node(index) => graph.category(index),
        };
        if category == Category::Process {
            step += 1;
        }
        let at = editor.layout.at(end);
        if !node_rect(at).intersects(&visible) {
            continue;
        }
        let (kicker, title, preview_text, color) = match end {
            End::Trigger => {
                let kind = trigger_kind(graph.trigger_type());
                (
                    "TRIGGER".to_owned(),
                    kind.map_or_else(
                        || graph.trigger_type().to_owned(),
                        |kind| kind.label.to_owned(),
                    ),
                    trigger_summary(graph),
                    kind.map_or(theme.fg_dim, |kind| tone_color(kind.tone, theme)),
                )
            }
            End::Node(index) => {
                let config = &graph.nodes[index];
                let kind = node_kind(config_type(config));
                (
                    match category {
                        Category::Output => "OUTPUT".to_owned(),
                        _ => format!("STEP {step}"),
                    },
                    label_of(config),
                    preview(config),
                    kind.map_or(theme.fg_dim, |kind| tone_color(kind.tone, theme)),
                )
            }
        };
        let (input, output) = model::ports(graph, end);
        cards.push(Card {
            at,
            color,
            kicker: kicker.into(),
            title: title.into(),
            preview: preview_text.into(),
            selected: editor.selection == Selection::Step(end),
            target: matches!(end, End::Node(index) if Some(index) == target),
            input,
            output,
        });
    }
    let wires = graph
        .wires()
        .into_iter()
        .map(|wire| {
            (
                editor.layout.curve(wire),
                editor.selection == Selection::Wire(wire.to),
            )
        })
        .filter(|(curve, _)| curve.bounds().intersects(&visible))
        .collect();
    let pending = match editor.drag {
        Some(Drag::Wire { from, to, .. }) => Some((model::output_port(editor.layout.at(from)), to)),
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
        },
    }
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

    let size = canvas_size(editor, window);
    let scene = scene(editor, size, &theme);
    let is_dirty = dirty(editor, cx);
    let canvas_focus = editor.canvas_focus.clone();
    let canvas_focused = canvas_focus.is_focused(window);
    let bounds = Rc::clone(&editor.bounds);
    let name = editor.name.clone();
    let flow = editor.flow.clone();
    let (enabled, saving, message, broken, folded, zoom) = (
        editor.enabled,
        editor.saving,
        editor.message.clone(),
        editor.broken.clone(),
        editor.folded,
        editor.view.zoom,
    );
    let items = palette_items(folded);

    // ── Toolbar ──
    let set_enabled = |on: bool, cx: &mut Context<Console>| {
        cx.listener(move |console: &mut Console, _: &gpui::ClickEvent, _, cx| {
            if let Some(editor) = editor_mut(console) {
                editor.enabled = on;
            }
            cx.notify();
        })
    };
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
        .child(div().w(px(240.0)).child(name))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(flow.id.clone()),
        )
        .when(is_dirty, |this| this.child(pill("Unsaved", theme.warning)))
        .child(div().flex_1())
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
            Button::new("data-flow-editor-test", "Test")
                .ghost()
                .icon(Icon::Send)
                .tooltip("Test the stored flow with a payload  T")
                .on_click(cx.listener(|console, _, window, cx| open_test(console, window, cx))),
        )
        .child(
            Button::new("data-flow-editor-runs", "Runs")
                .ghost()
                .icon(Icon::History)
                .tooltip("The engine's latest runs  H")
                .on_click(cx.listener(|console, _, window, cx| open_runs(console, window, cx))),
        )
        .child(
            Button::new("data-flow-save", if saving { "Saving…" } else { "Save" })
                .primary()
                .icon(Icon::Check)
                .tooltip("⌘S")
                .disabled(saving || !is_dirty)
                .on_click(cx.listener(|console, _, _, cx| save(console, cx))),
        );

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
    let mut groups: Vec<AnyElement> = Vec::new();
    let mut row = 0;
    for (group, (title, category)) in GROUPS.iter().enumerate() {
        let open = !folded[group];
        let group_items: Vec<Item> = match category {
            Category::Input => TRIGGER_KINDS.iter().map(Item::Trigger).collect(),
            category => palette(*category).map(Item::Step).collect(),
        };
        groups.push(
            div()
                .id(("data-flow-palette-group", group))
                .flex()
                .items_center()
                .justify_between()
                .h(px(30.0))
                .px(SPACE_6)
                .cursor_pointer()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .hover(|style| style.text_color(theme.fg))
                .on_click(cx.listener(move |console, _, _, cx| {
                    if let Some(editor) = editor_mut(console) {
                        editor.folded[group] = !editor.folded[group];
                    }
                    console.list(PALETTE_LIST, cx).cursor = None;
                    cx.notify();
                }))
                .child(*title)
                .child(if open { "▾" } else { "▸" })
                .into_any_element(),
        );
        if !open {
            continue;
        }
        for item in group_items {
            let at = row;
            row += 1;
            let color = item.color(&theme);
            let current = palette_cursor == Some(at);
            let hover = theme.hover;
            groups.push(
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
                    .on_drag(PaletteDrag(item), |dragged, _, _, cx| {
                        cx.new(|_| dragged.clone())
                    })
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
                            .child(item.label()),
                    )
                    .into_any_element(),
            );
        }
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
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child("↵ add · drag onto the canvas"),
            ),
    )
    .children(groups);

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
            with_editor(console, window, cx, |editor, size| {
                editor.view = View::fit(editor.layout.bounds(), size);
            });
        }))
        .on_action(cx.listener(|console, _: &EngFlowsArrange, window, cx| {
            with_editor(console, window, cx, arrange);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsEarlier, window, cx| {
            with_editor(console, window, cx, |editor, _| shift(editor, true));
        }))
        .on_action(cx.listener(|console, _: &EngFlowsLater, window, cx| {
            with_editor(console, window, cx, |editor, _| shift(editor, false));
        }))
        .on_action(cx.listener(|console, _: &EngFlowsUndo, window, cx| {
            with_editor(console, window, cx, |editor, _| undo(editor));
        }))
        .on_action(cx.listener(|console, _: &EngFlowsAddStep, window, cx| {
            console.focus_list(PALETTE_LIST, window, cx);
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &EngFlowsInspect, _, cx| {
            if let Some(editor) = editor_mut(console) {
                editor.focus_inspector = true;
            }
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &EngFlowsTest, window, cx| {
            open_test(console, window, cx);
        }))
        .on_action(cx.listener(|console, _: &EngFlowsRuns, window, cx| {
            open_runs(console, window, cx);
        }))
        .on_drop(cx.listener(|console, dragged: &PaletteDrag, window, cx| {
            drop_item(console, dragged.0, window, cx);
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
                .child("Drag a port onto a step to wire · ⌫ remove · arrows move · ⌥←→ run earlier/later · Tab next · + − 0 zoom · A arrange · N add · ↵ settings · ⌘Z undo · ⌘S save · Esc back"),
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
        .child(strip_button("data-flow-zoom-out", "−", "Zoom out  −", cx, |editor, size| {
            editor.view = editor.view.zoom_about(size.scale(0.5), 1.0 / model::ZOOM_STEP);
        }))
        .child(strip_button("data-flow-zoom-in", "+", "Zoom in  +", cx, |editor, size| {
            editor.view = editor.view.zoom_about(size.scale(0.5), model::ZOOM_STEP);
        }))
        .child(strip_button("data-flow-fit", "Fit", "Fit the flow in view  0", cx, |editor, size| {
            editor.view = View::fit(editor.layout.bounds(), size);
        }))
        .child(strip_button("data-flow-arrange", "Arrange", "Lay the steps out again  A", cx, arrange));

    // ── Settings panel ──
    let inspector = inspector_panel(console, &theme, cx);

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
        .when_some(broken, |this, error| {
            this.child(bar(
                format!("The stored flow could not be read ({error}); saving replaces it with this canvas"),
                theme.warning,
                Icon::CircleAlert,
            ))
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
                .child(palette_list)
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
                .child(inspector),
        )
        .into_any_element()
}

fn arrange(editor: &mut Editor, size: Vec2) {
    remember(editor);
    editor.layout = Layout::arrange(&editor.graph);
    editor.view = View::fit(editor.layout.bounds(), size);
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

fn open_test(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = console.state::<State>().editor.as_ref() else {
        return;
    };
    let (flow, unsaved) = (editor.flow.clone(), dirty(editor, cx));
    forms::open_test(console, &flow, unsaved, window, cx);
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

/// One field as the panel draws it: its control (none for a switch).
struct Shown {
    key: String,
    label: String,
    kind: FieldKind,
    control: Option<AnyElement>,
    error: Option<String>,
}

/// The settings panel: the selection's settings, or what is selected.
#[allow(clippy::too_many_lines)]
fn inspector_panel(console: &mut Console, theme: &Theme, cx: &mut Context<Console>) -> AnyElement {
    let Some(editor) = editor_mut(console) else {
        return div().into_any_element();
    };
    let graph = editor.graph.clone();
    let selection = editor.selection;
    let fields: Vec<Shown> = editor
        .inspector
        .fields
        .iter()
        .map(|field| Shown {
            key: field.key.clone(),
            label: field.label.clone(),
            kind: field.kind,
            control: match &field.input {
                Input::Line(line) => Some(line.clone().into_any_element()),
                Input::Area(area) => Some(area.clone().into_any_element()),
                Input::Switch => None,
            },
            error: field.error.clone(),
        })
        .collect();
    let caption = |text: String| {
        div()
            .text_size(FONT_CAPTION)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.fg_dim)
            .child(text)
    };
    let dim = |text: String| {
        div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_muted)
            .child(text)
    };
    let heading = |color: Hsla, title: String| {
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
    };

    let mut body: Vec<AnyElement> = Vec::new();
    match selection {
        Selection::None => {
            let steps = graph
                .nodes
                .iter()
                .filter(|node| category_of(node) != Category::Output)
                .count();
            let outputs = graph.nodes.len() - steps;
            body.push(heading(theme.accent, "Flow".to_owned()).into_any_element());
            body.push(
                dim(format!(
                    "Trigger {} · {steps} process steps · {outputs} outputs",
                    graph
                        .end_label(End::Trigger)
                        .trim_start_matches("Trigger · ")
                ))
                .into_any_element(),
            );
            body.push(
                dim("Select a step or the trigger to edit its settings. The engine runs the process steps in order, then every output.".to_owned())
                    .into_any_element(),
            );
            for (keys, what) in [
                ("Tab", "next step"),
                ("↵", "edit its settings"),
                ("N", "add from the palette"),
                ("⌫", "remove the step"),
                ("⌥← ⌥→", "run earlier, later"),
                ("← ↑ ↓ →", "move it (⇧ further)"),
                ("+ − 0", "zoom, fit"),
                ("⌘Z", "undo"),
                ("⌘S", "save"),
            ] {
                body.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(div().w(px(64.0)).flex_none().child(kbd(keys, cx)))
                        .child(what)
                        .into_any_element(),
                );
            }
        }
        Selection::Wire(to) => {
            let from = graph
                .source_of(to)
                .map(|from| graph.end_label(from))
                .unwrap_or_default();
            body.push(heading(theme.fg_dim, "Wire".to_owned()).into_any_element());
            body.push(
                dim(format!("{from} → {}", graph.end_label(End::Node(to)))).into_any_element(),
            );
            body.push(
                dim("The server stores no wires: the steps run in their stored order. Drag from another step's output port onto this step to change what feeds it; ⌫ on a step removes it.".to_owned())
                    .into_any_element(),
            );
        }
        Selection::Step(end) => {
            match end {
                End::Trigger => {
                    let kind = trigger_kind(graph.trigger_type());
                    body.push(
                        heading(
                            kind.map_or(theme.fg_dim, |kind| tone_color(kind.tone, theme)),
                            format!(
                                "Trigger · {}",
                                kind.map_or(graph.trigger_type(), |kind| kind.label)
                            ),
                        )
                        .into_any_element(),
                    );
                    let segments = TRIGGER_KINDS
                        .iter()
                        .map(|choice| {
                            Segment::new(
                                choice.label,
                                choice.id == graph.trigger_type(),
                                cx.listener(move |console, _, _, cx| {
                                    set_trigger_kind(console, choice, cx);
                                }),
                            )
                        })
                        .collect();
                    body.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SPACE_2)
                            .child(caption("TRIGGER TYPE".to_owned()))
                            .child(segmented("data-flow-trigger-type", segments, cx))
                            .into_any_element(),
                    );
                }
                End::Node(index) => {
                    let Some(config) = graph.nodes.get(index) else {
                        return div().into_any_element();
                    };
                    let kind = node_kind(config_type(config));
                    body.push(
                        heading(
                            kind.map_or(theme.fg_dim, |kind| tone_color(kind.tone, theme)),
                            label_of(config),
                        )
                        .into_any_element(),
                    );
                    body.push(
                        div()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child(config_id(config).to_owned())
                            .into_any_element(),
                    );
                    if let Some(kind) = kind {
                        body.push(
                            dim(format!("{} · {}", kind.category.label(), kind.desc))
                                .into_any_element(),
                        );
                    }
                    if let Some(from) = graph.source_of(index) {
                        body.push(
                            dim(format!("Fed by {}", graph.end_label(from))).into_any_element(),
                        );
                    }
                }
            }
            if fields.is_empty() && matches!(end, End::Node(_)) {
                body.push(dim("No settings.".to_owned()).into_any_element());
            }
            let config = match end {
                End::Trigger => Some(&graph.trigger),
                End::Node(index) => graph.nodes.get(index),
            };
            for (at, shown) in fields.into_iter().enumerate() {
                let Shown {
                    key,
                    label,
                    kind,
                    control,
                    error,
                } = shown;
                let control = match control {
                    Some(control) => control,
                    None => {
                        let on = config
                            .and_then(|config| config.get(&key))
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let off_key = key.clone();
                        segmented(
                            ("data-flow-switch", at),
                            vec![
                                Segment::new(
                                    "Off",
                                    !on,
                                    cx.listener(move |console, _, _, cx| {
                                        set_switch(console, &off_key, false, cx);
                                    }),
                                ),
                                Segment::new(
                                    "On",
                                    on,
                                    cx.listener(move |console, _, _, cx| {
                                        set_switch(console, &key, true, cx);
                                    }),
                                ),
                            ],
                            cx,
                        )
                    }
                };
                let hint = match kind {
                    FieldKind::Json => " (JSON)",
                    _ => "",
                };
                body.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_2)
                        .child(caption(format!("{}{hint}", label.to_uppercase())))
                        .child(control)
                        .when_some(error, |this, error| {
                            this.child(
                                div()
                                    .text_size(FONT_CAPTION)
                                    .text_color(theme.danger)
                                    .child(error),
                            )
                        })
                        .into_any_element(),
                );
            }
            if let End::Node(index) = end {
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(SPACE_2)
                        .pt(SPACE_4)
                        .child(inspector_button(
                            "data-flow-earlier",
                            "Earlier",
                            "Run one place earlier  ⌥←",
                            cx,
                            |editor| shift(editor, true),
                        ))
                        .child(inspector_button(
                            "data-flow-later",
                            "Later",
                            "Run one place later  ⌥→",
                            cx,
                            |editor| shift(editor, false),
                        ))
                        .child(
                            Button::new(("data-flow-remove", index), "Remove")
                                .danger()
                                .compact()
                                .icon(Icon::Trash)
                                .tooltip("⌫")
                                .on_click(cx.listener(|console, _, window, cx| {
                                    with_editor(console, window, cx, |editor, _| {
                                        delete_selection(editor)
                                    });
                                })),
                        )
                        .into_any_element(),
                );
            }
        }
    }

    div()
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
                .py(SPACE_5)
                .border_b_1()
                .border_color(theme.border)
                .child(caption("CONFIGURATION".to_owned())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .px(SPACE_7)
                .py(SPACE_6)
                .pb(SPACE_10)
                .children(body),
        )
        .into_any_element()
}

fn inspector_button(
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    cx: &mut Context<Console>,
    act: fn(&mut Editor),
) -> AnyElement {
    Button::new(id, label)
        .ghost()
        .compact()
        .tooltip(tooltip)
        .on_click(cx.listener(move |console, _, window, cx| {
            with_editor(console, window, cx, |editor, _| act(editor));
        }))
        .into_any_element()
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
    use super::*;

    #[test]
    fn the_selection_follows_the_steps() {
        let order = [Some(2), Some(0), None, Some(1)];
        assert_eq!(
            remap(Selection::Step(End::Node(0)), &order),
            Selection::Step(End::Node(1))
        );
        assert_eq!(remap(Selection::Wire(1), &order), Selection::Wire(3));
        assert_eq!(
            remap(Selection::Step(End::Node(5)), &order),
            Selection::None
        );
        assert_eq!(
            remap(Selection::Step(End::Trigger), &order),
            Selection::Step(End::Trigger)
        );
    }

    #[test]
    fn the_palette_lists_the_web_clients_groups() {
        let all = palette_items([false; 3]);
        assert_eq!(all.len(), 3 + 7 + 4);
        assert!(matches!(all[0], Item::Trigger(kind) if kind.id == "modbus"));
        assert!(matches!(all[3], Item::Step(kind) if kind.id == "filter"));
        assert!(matches!(all[13], Item::Step(kind) if kind.id == "write_out"));
        let folded = palette_items([true, false, true]);
        assert_eq!(folded.len(), 7);
    }

    #[test]
    fn trigger_cards_say_what_they_listen_to() {
        let mut graph = FlowGraph::default();
        graph.set_trigger_kind(trigger_kind("modbus").unwrap());
        assert_eq!(trigger_summary(&graph), "device *");
        graph
            .trigger
            .insert("metric".into(), Value::String("temp".into()));
        assert_eq!(trigger_summary(&graph), "device * · temp");
        graph.set_trigger_kind(trigger_kind("timer").unwrap());
        assert_eq!(trigger_summary(&graph), "every 5000 ms");
        graph.set_trigger_kind(trigger_kind("mqtt").unwrap());
        assert_eq!(trigger_summary(&graph), "sensors/+/temp");
    }
}

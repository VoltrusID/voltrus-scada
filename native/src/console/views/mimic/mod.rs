//! The HMI mimic (⌘7): the web client's SCADA view of a screen
//! (`client/src/scada2`), live. The screen's equipment and connections come
//! from `scada-equipment` and `scada-connections` and parse into the plain
//! model of [`voltrus_scada_native::mimic`]; each frame draws the visible
//! widgets on a canvas: library symbols ([`symbols`]), the other widgets
//! ([`widgets`]) and the connections, pipes flowing while their equipment
//! runs.
//!
//! - **Pan and zoom**: drag the stage or scroll to pan; pinch on a trackpad,
//!   ⌘-scroll, ⌘+ / ⌘− to zoom about the pointer or the centre, ⌘0 to fit.
//!   The stage follows the pointer at the display's rate; keyed zooms glide.
//! - **Keyboard**: Tab / ⇧Tab walk the equipment in reading order, the
//!   arrows move to the nearest one in their direction, Enter opens its
//!   faceplate, Esc closes it. `[` and `]` change screens, R reloads.
//! - **Faceplate** ([`faceplate`]): the selected equipment's state, value
//!   against its limits, trend and related tags, and its commands, armed
//!   then confirmed (1 arms on/start/open, 0 off/stop/close, Enter
//!   confirms): an admin's go over the WebSocket, an operator's through the
//!   policy-checked REST write.
//! - **Alarm banner** ([`banner`]): the newest active alarms from the alarm
//!   list the console already keeps.
//!
//! Frames: only while something moves (a pipe flows, an impeller turns, a
//! zoom glides, the stage is dragged); blinking (faults, critical alarms)
//! redraws at its half-second steps; an idle mimic draws nothing between
//! data batches. Only the widgets in view are prepared and drawn.

mod banner;
mod faceplate;
mod paint;
mod pinch;
mod scene;
mod symbols;
mod widgets;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Context, DispatchPhase, ElementId, EntityId, FocusHandle, FontWeight,
    Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollDelta,
    ScrollWheelEvent, SharedString, WeakEntity, Window, actions, canvas, div, px,
};
use voltrus_scada_native::mimic::{
    self, Armed, Camera, ConnectionRow, Direction, EquipmentRow, Items, Layout, LineKind, Widget,
    WidgetKind, connections_path, equipment_path,
};
use voltrus_scada_native::runtime::{Call, CallKey, Command, Reply};
use voltrus_scada_native::screens::OVERVIEW;
use voltrus_scada_native::tag_api::{self, WriteAccepted, WriteMode};
use vt_ui::Icon;
use vt_ui::components::{Button, empty_state, kbd};
use vt_ui::theme::*;

use self::paint::{Canvas, Hmi, Line, Shape, Tessellations, Text, Xf, fade};
use self::scene::{Item, StoreValues, TagView, motion};
use crate::console::keys::Dismiss;
use crate::console::screens::{self, Screens};
use crate::console::{Console, Page, Phase, anim};

/// The key context of the mimic: its Tab, arrows, Enter, zoom and command
/// keys.
pub const MIMIC: &str = "Mimic";

actions!(
    voltrus_console,
    [
        MimicNext,
        MimicPrevious,
        MimicUp,
        MimicDown,
        MimicLeft,
        MimicRight,
        MimicOpen,
        MimicZoomIn,
        MimicZoomOut,
        MimicFit,
        MimicArmOn,
        MimicArmOff,
        MimicStepUp,
        MimicStepDown,
        MimicNextScreen,
        MimicPreviousScreen,
        MimicReload,
    ]
);

/// A keyed zoom glides this long.
const GLIDE: Duration = Duration::from_millis(220);
/// Pixels kept clear around the stage when it is fitted.
const FIT_PAD: f64 = 24.0;
/// Pixels of the stage kept on screen however far it is panned.
const KEEP: f64 = 96.0;
/// A press that moves less than this is a click, not a drag.
const DRAG_SLOP: f32 = 3.0;
/// Stage pixels around the view in which widgets are still prepared
/// (labels and strokes overhang their boxes).
const OVERSCAN: f64 = 48.0;
/// The faceplate's width.
pub(crate) const FACEPLATE_WIDTH: Pixels = px(320.0);
/// The animation clock wraps at a common multiple of every period
/// (1.4 s, 0.9 s, 0.6 s, 0.5 s), so the phases stay exact.
const CLOCK_WRAP: f32 = 1260.0;

/// A screen's mimic as fetched.
#[derive(Clone)]
enum Loaded {
    Loading,
    Ready(Rc<Layout>),
    Failed(String),
}

/// What the last frame drew with: where the stage was in the window and
/// the camera, for the pointer and the keys.
#[derive(Clone, Copy)]
struct Drawn {
    bounds: Bounds<Pixels>,
    camera: Camera,
}

/// A press on the stage: a click, or a drag that pans.
#[derive(Clone, Copy)]
struct Press {
    at: gpui::Point<Pixels>,
    camera: Camera,
    moved: bool,
}

/// A keyed camera move.
#[derive(Clone, Copy)]
struct Glide {
    from: Camera,
    to: Camera,
    since: Instant,
}

pub(crate) struct State {
    layouts: HashMap<String, Loaded>,
    /// Calls in flight, by their key's argument: the screen they fetch.
    asks: HashMap<i64, String>,
    next_ask: i64,
    fetching: HashSet<String>,
    /// The server and user the layouts belong to.
    session: Option<(String, String)>,
    /// The camera when the operator moved it; `None` follows the fit.
    camera: Option<Camera>,
    glide: Option<Glide>,
    drawn: Rc<Cell<Option<Drawn>>>,
    press: Option<Press>,
    /// The keyboard cursor: a widget id.
    cursor: Option<String>,
    hover: Option<String>,
    /// The widget whose faceplate is open.
    faceplate: Option<String>,
    armed: Option<Armed>,
    /// A control's setpoint being adjusted: widget id and value.
    setpoint: Option<(String, f64)>,
    /// The REST write in flight: its tag.
    sending: Option<i64>,
    /// The WebSocket write sent from the faceplate: its widget and tag.
    sent: Option<(String, i64)>,
    /// The last REST write's answer, by widget.
    result: Option<(String, Result<String, String>)>,
    focus: Option<FocusHandle>,
    view: Option<EntityId>,
    /// When the animation clock started.
    epoch: Instant,
    /// A redraw is scheduled for the next blink step.
    blink_pending: bool,
    cache: Rc<Tessellations>,
    pinch_listening: bool,
    /// The window's height in points, for AppKit's bottom-up gesture
    /// coordinates.
    window_height: f64,
    /// What to do once the screen and its values are in (a key pressed
    /// before, e.g. `--overlay MimicOpen` right after sign-in).
    pending: Option<Pending>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            layouts: HashMap::new(),
            asks: HashMap::new(),
            next_ask: 0,
            fetching: HashSet::new(),
            session: None,
            camera: None,
            glide: None,
            drawn: Rc::new(Cell::new(None)),
            press: None,
            cursor: None,
            hover: None,
            faceplate: None,
            armed: None,
            setpoint: None,
            sending: None,
            sent: None,
            result: None,
            focus: None,
            view: None,
            epoch: Instant::now(),
            blink_pending: false,
            cache: Rc::new(Tessellations::default()),
            pinch_listening: false,
            window_height: 0.0,
            pending: None,
        }
    }
}

impl State {
    /// The camera now: a glide's, the operator's, or none (fit).
    fn camera_at(&mut self, now: Instant) -> (Option<Camera>, bool) {
        if let Some(glide) = self.glide {
            match anim::progress(glide.since, now, GLIDE) {
                Some(t) => {
                    return (
                        Some(glide.from.toward(&glide.to, f64::from(anim::ease_out(t)))),
                        true,
                    );
                }
                None => self.glide = None,
            }
        }
        (self.camera, false)
    }

    fn clock(&self, now: Instant) -> f32 {
        now.duration_since(self.epoch).as_secs_f32() % CLOCK_WRAP
    }
}

fn active(console: &Console) -> String {
    console
        .state_ref::<Screens>()
        .map_or_else(|| OVERVIEW.to_owned(), |screens| screens.active.clone())
}

/// The active screen's layout, when it has arrived.
fn layout(console: &mut Console) -> Option<Rc<Layout>> {
    let screen = active(console);
    match console.state::<State>().layouts.get(&screen) {
        Some(Loaded::Ready(layout)) => Some(layout.clone()),
        _ => None,
    }
}

/// Fetch what the active screen still needs: the screens, and its mimic.
fn sync(console: &mut Console) {
    screens::sync(console);
    let session = console
        .user
        .as_ref()
        .map(|(user, _)| (console.server.clone(), user.clone()));
    let stress = console.stress.is_some();
    let screen = active(console);
    let state = console.state::<State>();
    if state.session != session {
        *state = State {
            session: session.clone(),
            pinch_listening: state.pinch_listening,
            focus: state.focus.take(),
            ..State::default()
        };
    }
    if session.is_none() || stress || state.layouts.contains_key(&screen) {
        return;
    }
    if !state.fetching.insert(screen.clone()) {
        return;
    }
    state.next_ask += 1;
    let ask = state.next_ask;
    state.asks.insert(ask, screen.clone());
    state.layouts.insert(screen.clone(), Loaded::Loading);
    console.call(
        CallKey::new("mimic.layout").with(ask),
        Call::new(move |api| async move {
            let equipment: Items<EquipmentRow> = api.get_json(&equipment_path(&screen)).await?;
            let connections: Items<ConnectionRow> =
                api.get_json(&connections_path(&screen)).await?;
            Ok(mimic::parse(&equipment.items, &connections.items))
        }),
    );
}

/// Forget the active screen's mimic and fetch it again.
fn reload(console: &mut Console) {
    let screen = active(console);
    let state = console.state::<State>();
    state.layouts.remove(&screen);
    state.fetching.remove(&screen);
    sync(console);
}

fn focus_handle(console: &mut Console, cx: &mut Context<Console>) -> FocusHandle {
    console
        .state::<State>()
        .focus
        .get_or_insert_with(|| cx.focus_handle())
        .clone()
}

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    sync(console);
    let focus = focus_handle(console, cx);
    window.focus(&focus);
    listen_for_pinches(console, cx);
}

/// Start reading trackpad pinches (once per session).
fn listen_for_pinches(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.pinch_listening {
        return;
    }
    state.pinch_listening = true;
    let Some(gestures) = pinch::listen() else {
        return;
    };
    cx.spawn(async move |this: WeakEntity<Console>, cx| {
        while let Ok(gesture) = gestures.recv().await {
            let alive = this.update(cx, |console, cx| on_gesture(console, gesture, cx));
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

fn on_gesture(console: &mut Console, gesture: pinch::Gesture, cx: &mut Context<Console>) {
    if console.page != Page::Mimic || console.phase != Phase::Console {
        return;
    }
    let state = console.state::<State>();
    let Some(drawn) = state.drawn.get() else {
        return;
    };
    // AppKit gives the point in the window from its bottom-left corner,
    // GPUI from its top-left: flip it with the window's height, then make it
    // relative to the stage.
    let (x, y) = match gesture {
        pinch::Gesture::Magnify { x, y, .. } | pinch::Gesture::SmartMagnify { x, y } => (x, y),
    };
    let window_height = state.window_height;
    let local = (
        x - f64::from(f32::from(drawn.bounds.origin.x)),
        window_height - y - f64::from(f32::from(drawn.bounds.origin.y)),
    );
    let inside = local.0 >= 0.0
        && local.1 >= 0.0
        && local.0 <= f64::from(f32::from(drawn.bounds.size.width))
        && local.1 <= f64::from(f32::from(drawn.bounds.size.height));
    if !inside {
        return;
    }
    match gesture {
        pinch::Gesture::Magnify { amount, .. } => {
            let camera = drawn.camera;
            state.glide = None;
            state.camera = Some(camera.zoomed(camera.zoom * (1.0 + amount), local.0, local.1));
        }
        pinch::Gesture::SmartMagnify { .. } => {
            let fitted = fit_of(&drawn);
            let to = if (drawn.camera.zoom - fitted.zoom).abs() < 1e-3 {
                drawn.camera.zoomed(1.0, local.0, local.1)
            } else {
                fitted
            };
            glide_to(
                state,
                drawn.camera,
                to,
                (to.zoom - fitted.zoom).abs() < 1e-6,
            );
        }
    }
    redraw(console, cx);
}

/// Redraw only the page (pan, zoom, hover), not the whole console.
fn redraw(console: &mut Console, cx: &mut App) {
    if let Some(view) = console.state::<State>().view {
        cx.notify(view);
    }
}

fn fit_of(drawn: &Drawn) -> Camera {
    Camera::fit(
        f64::from(f32::from(drawn.bounds.size.width)),
        f64::from(f32::from(drawn.bounds.size.height)),
        FIT_PAD,
    )
}

/// Glide the camera from `from` to `to`; `fit` keeps following the fit
/// afterwards.
fn glide_to(state: &mut State, from: Camera, to: Camera, fit: bool) {
    state.glide = Some(Glide {
        from,
        to,
        since: Instant::now(),
    });
    state.camera = if fit { None } else { Some(to) };
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "mimic.layout" => {
            let state = console.state::<State>();
            let Some(screen) = state.asks.remove(&key.arg) else {
                return;
            };
            state.fetching.remove(&screen);
            let loaded = match reply.take::<Layout>() {
                Ok(layout) => Loaded::Ready(Rc::new(layout)),
                Err(error) => Loaded::Failed(error),
            };
            state.layouts.insert(screen, loaded);
        }
        "mimic.write" => {
            let state = console.state::<State>();
            state.sending = None;
            let widget = state.faceplate.clone().unwrap_or_default();
            let result = reply.take::<WriteAccepted>();
            let message = match &result {
                Ok(answer) => format!("Command sent: {}", answer.command_id),
                Err(error) => format!("Command refused: {error}"),
            };
            let failed = result.is_err();
            state.result = Some((widget, result.map(|answer| answer.command_id)));
            console.show_toast(message, failed, cx);
        }
        _ => {}
    }
    cx.notify();
}

// ── Selection and commands ────────────────────────────────────────────────

/// The widgets the keyboard visits, by index.
fn visit_order(layout: &Layout) -> Vec<usize> {
    mimic::tab_order(layout)
}

fn move_cursor(console: &mut Console, step: Step, cx: &mut Context<Console>) {
    let Some(layout) = layout(console) else {
        return;
    };
    let order = visit_order(&layout);
    if order.is_empty() {
        return;
    }
    let state = console.state::<State>();
    let current = state
        .cursor
        .as_ref()
        .and_then(|id| layout.widgets.iter().position(|w| &w.id == id))
        .filter(|index| order.contains(index));
    let next = match (current, step) {
        (None, Step::Previous) => order.last().copied(),
        (None, _) => order.first().copied(),
        (Some(at), Step::Next | Step::Previous) => {
            let place = order.iter().position(|index| *index == at).unwrap_or(0);
            let forward = matches!(step, Step::Next);
            let next = if forward {
                (place + 1) % order.len()
            } else {
                (place + order.len() - 1) % order.len()
            };
            Some(order[next])
        }
        (Some(at), Step::Toward(direction)) => {
            mimic::neighbor(&layout, at, direction, &order).or(Some(at))
        }
    };
    let Some(next) = next else {
        return;
    };
    let widget = &layout.widgets[next];
    state.cursor = Some(widget.id.clone());
    // A faceplate follows the cursor.
    if state.faceplate.is_some() && state.faceplate.as_deref() != Some(widget.id.as_str()) {
        open_faceplate(state, &widget.id);
    }
    reveal(state, widget);
    cx.notify();
}

#[derive(Clone, Copy)]
enum Step {
    Next,
    Previous,
    Toward(Direction),
}

/// Pan the cursor's widget into view, gliding.
fn reveal(state: &mut State, widget: &Widget) {
    let Some(drawn) = state.drawn.get() else {
        return;
    };
    let (camera, _) = state.camera_at(Instant::now());
    let camera = camera.unwrap_or(drawn.camera);
    let width = f64::from(f32::from(drawn.bounds.size.width));
    let height = f64::from(f32::from(drawn.bounds.size.height));
    if let Some(to) = camera.reveal(
        (widget.x, widget.y, widget.width, widget.height),
        width,
        height,
        32.0,
    ) {
        glide_to(state, camera, to, false);
    }
}

fn open_faceplate(state: &mut State, id: &str) {
    if state.faceplate.as_deref() != Some(id) {
        state.faceplate = Some(id.to_owned());
        state.armed = None;
        state.setpoint = None;
        state.result = None;
        state.sent = None;
    }
}

fn close_faceplate(state: &mut State) {
    state.faceplate = None;
    state.armed = None;
    state.setpoint = None;
}

/// Enter: confirm an armed command, or open the cursor's faceplate. Before
/// the screen has loaded (`--overlay MimicOpen` right after sign-in), the
/// first equipment's opens once it has.
fn activate(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().armed.is_some() {
        confirm(console, cx);
        return;
    }
    if layout(console).is_none() {
        console.state::<State>().pending = Some(Pending::Open);
        return;
    }
    open_cursor(console, cx);
    let focus = focus_handle(console, cx);
    window.focus(&focus);
}

/// Open the cursor's faceplate, the cursor on the first equipment if it is
/// nowhere.
fn open_cursor(console: &mut Console, cx: &mut Context<Console>) {
    if console.state::<State>().cursor.is_none() {
        move_cursor(console, Step::Next, cx);
    }
    let state = console.state::<State>();
    if let Some(cursor) = state.cursor.clone() {
        open_faceplate(state, &cursor);
    }
    cx.notify();
}

/// The widget with an open faceplate, and the commands it offers.
fn faceplate_commands(console: &mut Console) -> Option<(Widget, mimic::Commands)> {
    let layout = layout(console)?;
    let id = console.state::<State>().faceplate.clone()?;
    let widget = layout.widget(&id)?.clone();
    let commands = mimic::commands(&widget, &StoreValues(&console.store))?;
    Some((widget, commands))
}

/// A key that came before the screen did, and the hold of the arm it made.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Open,
    Arm(bool),
    /// An arm that waited for its screen (in practice `--overlay
    /// MimicArmOn`/`MimicArmOff` at launch) stays armed until then, so a
    /// launch's screenshot taken seconds later still shows it; it lapses
    /// `ARM_TIMEOUT` after. Any arm, confirm, dismiss or other faceplate
    /// ends the hold at once.
    Hold {
        until: Instant,
    },
}

/// How long an arm that waited for its screen is held armed.
const LAUNCH_HOLD: Duration = Duration::from_secs(20);

/// Run the pending key once it can act: the screen is in and, for a
/// command, some equipment takes it (its tag has a value and the choice is
/// enabled); then keep a held arm armed.
fn run_pending(console: &mut Console, cx: &mut Context<Console>) {
    let Some(pending) = console.state::<State>().pending else {
        return;
    };
    let Some(layout) = layout(console) else {
        return;
    };
    match pending {
        Pending::Open => {
            console.state::<State>().pending = None;
            open_cursor(console, cx);
        }
        Pending::Arm(on) => {
            // The first equipment whose asked-for choice is enabled, else
            // the first whose other one is; none until their values are in.
            let values = StoreValues(&console.store);
            let enabled = |wanted: bool| {
                layout.widgets.iter().find_map(|widget| {
                    let commands = mimic::commands(widget, &values)?;
                    let choice = if wanted { commands.on } else { commands.off };
                    choice.enabled.then(|| (widget.id.clone(), wanted))
                })
            };
            let Some((id, on)) = enabled(on).or_else(|| enabled(!on)) else {
                return;
            };
            let state = console.state::<State>();
            state.pending = None;
            state.cursor = Some(id.clone());
            open_faceplate(state, &id);
            arm(console, on, cx);
            let state = console.state::<State>();
            if state.armed.is_some() {
                state.pending = Some(Pending::Hold {
                    until: Instant::now() + LAUNCH_HOLD,
                });
            }
        }
        Pending::Hold { until } => {
            let now = Instant::now();
            let state = console.state::<State>();
            match state.armed.as_mut() {
                Some(armed) if now < until => armed.at = now,
                _ => state.pending = None,
            }
        }
    }
}

/// Arm the on (or off) command of the open faceplate; with none open, open
/// the cursor's (or the first equipment that takes commands) first.
fn arm(console: &mut Console, on: bool, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if matches!(state.pending, Some(Pending::Hold { .. })) {
        state.pending = None;
    }
    let Some(layout) = layout(console) else {
        console.state::<State>().pending = Some(Pending::Arm(on));
        return;
    };
    if console.state::<State>().faceplate.is_none() {
        let values = StoreValues(&console.store);
        let takes = |id: &String| {
            layout
                .widget(id)
                .is_some_and(|widget| mimic::commands(widget, &values).is_some())
        };
        // The cursor's equipment, or the first that takes commands.
        let wanted = console
            .state_ref::<State>()
            .and_then(|state| state.cursor.clone())
            .filter(|id| takes(id))
            .or_else(|| {
                layout
                    .widgets
                    .iter()
                    .find(|widget| mimic::commands(widget, &values).is_some())
                    .map(|widget| widget.id.clone())
            });
        let Some(id) = wanted else {
            console.show_toast("No equipment on this screen takes commands", true, cx);
            return;
        };
        let state = console.state::<State>();
        state.cursor = Some(id.clone());
        open_faceplate(state, &id);
    }
    let Some((widget, commands)) = faceplate_commands(console) else {
        console.show_toast("This equipment takes no commands", true, cx);
        return;
    };
    if mimic::write_route(console.is_admin(), console.is_operator()).is_none() {
        console.show_toast(
            "Commands need the operator role (admin, demo or integration)",
            true,
            cx,
        );
        return;
    }
    let Some(tag) = widget.tag() else {
        return;
    };
    let state = console.state::<State>();
    let (label, value, enabled) = match (commands.setpoint, on) {
        (Some(_), true) => {
            let value = state
                .setpoint
                .as_ref()
                .filter(|(id, _)| *id == widget.id)
                .map_or(commands.on.value, |(_, value)| *value);
            (format!("SET {}", trim_number(value)), value, true)
        }
        (Some(_), false) => return,
        (None, true) => (commands.on.label, commands.on.value, commands.on.enabled),
        (None, false) => (commands.off.label, commands.off.value, commands.off.enabled),
    };
    if !enabled || state.sending.is_some() {
        return;
    }
    state.armed = Some(Armed {
        widget: widget.id,
        tag,
        label,
        value,
        at: Instant::now(),
    });
    cx.notify();
}

/// A number without needless decimals.
fn trim_number(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Send the armed command: over the WebSocket for an admin, through the
/// policy-checked REST write for other operators.
fn confirm(console: &mut Console, cx: &mut Context<Console>) {
    let route = mimic::write_route(console.is_admin(), console.is_operator());
    let state = console.state::<State>();
    let Some(armed) = state.armed.take() else {
        return;
    };
    if armed.expired(Instant::now()) {
        cx.notify();
        return;
    }
    match route {
        Some(WriteMode::Direct) => {
            state.sent = Some((armed.widget.clone(), armed.tag));
            state.result = None;
            console.write_result = None;
            console.writing = Some(armed.tag);
            console.send(Command::Write {
                id: armed.tag,
                value: armed.value,
            });
        }
        Some(mode) => {
            let Some(body) = tag_api::write_request(armed.value, "", mode) else {
                return;
            };
            state.sending = Some(armed.tag);
            state.result = None;
            let path = format!("/api/v1/tags/{}/write", armed.tag);
            console.call(
                CallKey::new("mimic.write").with(armed.tag),
                Call::new(move |api| async move {
                    api.send_json::<_, WriteAccepted>(reqwest::Method::POST, &path, &body)
                        .await
                }),
            );
        }
        None => {}
    }
    cx.notify();
}

/// Step a control's setpoint by `steps` steps.
fn step_setpoint(console: &mut Console, steps: f64, cx: &mut Context<Console>) {
    let Some((widget, commands)) = faceplate_commands(console) else {
        return;
    };
    let Some(setpoint) = commands.setpoint else {
        return;
    };
    let state = console.state::<State>();
    let current = state
        .setpoint
        .as_ref()
        .filter(|(id, _)| *id == widget.id)
        .map_or(commands.on.value, |(_, value)| *value);
    state.setpoint = Some((widget.id, setpoint.stepped(current, steps)));
    state.armed = None;
    cx.notify();
}

fn switch_screen(console: &mut Console, id: &str, window: &mut Window, cx: &mut Context<Console>) {
    if active(console) != id {
        screens::select(console, id);
        let state = console.state::<State>();
        state.camera = None;
        state.glide = None;
        state.cursor = None;
        state.hover = None;
        close_faceplate(state);
        sync(console);
    }
    let focus = focus_handle(console, cx);
    window.focus(&focus);
    cx.notify();
}

fn step_screen(
    console: &mut Console,
    forward: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(ids) = console.state_ref::<Screens>().map(Screens::ids) else {
        return;
    };
    let current = active(console);
    let at = ids.iter().position(|id| *id == current).unwrap_or(0);
    let next = if forward {
        (at + 1) % ids.len()
    } else {
        (at + ids.len() - 1) % ids.len()
    };
    switch_screen(console, &ids[next].clone(), window, cx);
}

/// Zoom by a step (`Some(true)` in, `Some(false)` out) about the stage's
/// centre, or fit (`None`), gliding.
fn zoom_key(console: &mut Console, zoom_in: Option<bool>, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some(drawn) = state.drawn.get() else {
        return;
    };
    let (camera, _) = state.camera_at(Instant::now());
    let from = camera.unwrap_or(drawn.camera);
    match zoom_in {
        Some(zoom_in) => {
            let width = f64::from(f32::from(drawn.bounds.size.width));
            let height = f64::from(f32::from(drawn.bounds.size.height));
            let to = from.zoomed(from.step(zoom_in), width / 2.0, height / 2.0);
            glide_to(state, from, to, false);
        }
        None => glide_to(state, from, fit_of(&drawn), true),
    }
    redraw(console, cx);
}

/// The topmost widget under a window point that opens a faceplate.
fn hit(console: &mut Console, at: gpui::Point<Pixels>) -> Option<String> {
    let layout = layout(console)?;
    let drawn = console.state::<State>().drawn.get()?;
    let local_x = f64::from(f32::from(at.x - drawn.bounds.origin.x));
    let local_y = f64::from(f32::from(at.y - drawn.bounds.origin.y));
    let (x, y) = drawn.camera.to_stage(local_x, local_y);
    layout
        .widgets
        .iter()
        .rev()
        .find(|widget| widget.contains(x, y) && widget.kind != WidgetKind::Area)
        .filter(|widget| widget.kind.has_faceplate() && widget.tag().is_some())
        .map(|widget| widget.id.clone())
}

fn on_press(
    console: &mut Console,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    let Some(drawn) = state.drawn.get() else {
        return;
    };
    let (camera, _) = state.camera_at(Instant::now());
    state.glide = None;
    state.press = Some(Press {
        at: event.position,
        camera: camera.unwrap_or(drawn.camera),
        moved: false,
    });
    let focus = focus_handle(console, cx);
    window.focus(&focus);
    redraw(console, cx);
}

fn on_drag(console: &mut Console, at: gpui::Point<Pixels>, cx: &mut App) {
    let state = console.state::<State>();
    let (Some(mut press), Some(drawn)) = (state.press, state.drawn.get()) else {
        return;
    };
    let dx = f32::from(at.x - press.at.x);
    let dy = f32::from(at.y - press.at.y);
    if !press.moved && dx.hypot(dy) < DRAG_SLOP {
        return;
    }
    press.moved = true;
    state.press = Some(press);
    state.camera = Some(press.camera.panned(
        f64::from(dx),
        f64::from(dy),
        f64::from(f32::from(drawn.bounds.size.width)),
        f64::from(f32::from(drawn.bounds.size.height)),
        KEEP,
    ));
    redraw(console, cx);
}

fn on_release(console: &mut Console, at: gpui::Point<Pixels>, cx: &mut App) {
    let Some(press) = console.state::<State>().press.take() else {
        return;
    };
    if !press.moved {
        let target = hit(console, at);
        let state = console.state::<State>();
        match target {
            Some(id) if state.faceplate.as_deref() == Some(id.as_str()) => close_faceplate(state),
            Some(id) => {
                state.cursor = Some(id.clone());
                open_faceplate(state, &id);
            }
            None => close_faceplate(state),
        }
    }
    redraw(console, cx);
}

fn on_hover(console: &mut Console, event: &MouseMoveEvent, cx: &mut Context<Console>) {
    if console.state::<State>().press.is_some() {
        return;
    }
    let target = hit(console, event.position);
    let state = console.state::<State>();
    if state.hover != target {
        state.hover = target;
        redraw(console, cx);
    }
}

fn on_scroll(console: &mut Console, event: &ScrollWheelEvent, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some(drawn) = state.drawn.get() else {
        return;
    };
    let (camera, _) = state.camera_at(Instant::now());
    let camera = camera.unwrap_or(drawn.camera);
    state.glide = None;
    let width = f64::from(f32::from(drawn.bounds.size.width));
    let height = f64::from(f32::from(drawn.bounds.size.height));
    let zoom = event.modifiers.platform || event.modifiers.control;
    if zoom {
        let anchor = (
            f64::from(f32::from(event.position.x - drawn.bounds.origin.x)),
            f64::from(f32::from(event.position.y - drawn.bounds.origin.y)),
        );
        let factor = match event.delta {
            ScrollDelta::Pixels(delta) => (-f64::from(f32::from(delta.y)) * 0.01).exp(),
            ScrollDelta::Lines(delta) => 1.15_f64.powf(f64::from(-delta.y)),
        };
        state.camera = Some(camera.zoomed(camera.zoom * factor, anchor.0, anchor.1));
    } else {
        let delta = event.delta.pixel_delta(px(20.0));
        state.camera = Some(camera.panned(
            f64::from(f32::from(delta.x)),
            f64::from(f32::from(delta.y)),
            width,
            height,
            KEEP,
        ));
    }
    redraw(console, cx);
}

// ── Rendering ─────────────────────────────────────────────────────────────

/// Whether a symbol has a part that moves in this state.
fn animates(key: &str, live: &mimic::Live) -> bool {
    use mimic::EquipState::Running;
    match key {
        "pump" | "fan" | "conveyor" => live.state == Running,
        "mixer" | "clarifier" | "pipe" | "pipe_vertical" | "pipe_elbow" | "pipe_tee" => {
            live.run_state == Running
        }
        _ => false,
    }
}

/// How many samples a widget's line needs.
fn samples_for(widget: &Widget) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = |default: f64, max: f64| {
        widget
            .number("pointsCount")
            .unwrap_or(default)
            .clamp(10.0, max) as usize
    };
    match widget.kind {
        WidgetKind::Trend => count(180.0, 180.0),
        WidgetKind::MultiTrend => count(60.0, 300.0),
        _ => 0,
    }
}

/// A widget prepared for drawing.
fn prepare(console: &Console, widget: &Widget, state: &State) -> Item {
    let store = &console.store;
    let samples = samples_for(widget);
    let primary = widget
        .tag()
        .and_then(|tag| store.get(tag))
        .map(|tag| TagView::of(tag, samples));
    let multi = matches!(
        widget.kind,
        WidgetKind::DataTable
            | WidgetKind::Annunciator
            | WidgetKind::BarChart
            | WidgetKind::PieChart
            | WidgetKind::MultiTrend
    );
    let tags = if multi {
        // The plant's first tags stand in for none bound; bound ones go in
        // the plant's order: only the candidates, never every tag.
        let candidates: Vec<i64> = if widget.tags.is_empty() {
            store.tags.iter().take(24).map(|tag| tag.info.id).collect()
        } else {
            let mut known: Vec<(usize, i64)> = widget
                .tags
                .iter()
                .filter_map(|id| store.position(*id).map(|at| (at, *id)))
                .collect();
            known.sort_unstable();
            known.into_iter().map(|(_, id)| id).collect()
        };
        mimic::shown_tags(widget, &candidates)
            .into_iter()
            .filter_map(|id| store.get(id).map(|tag| TagView::of(tag, samples)))
            .collect()
    } else {
        primary.iter().cloned().collect()
    };
    let symbol = mimic::resolve(widget).map(|resolved| {
        (
            resolved,
            mimic::symbol_live(widget, resolved.def, &StoreValues(store)),
        )
    });
    Item {
        widget: widget.clone(),
        tags,
        primary,
        symbol,
        selected: state.faceplate.as_deref() == Some(widget.id.as_str()),
        cursor: state.cursor.as_deref() == Some(widget.id.as_str()),
        hovered: state.hover.as_deref() == Some(widget.id.as_str()),
    }
}

/// A connection to draw: its route on the stage and how.
struct Wire {
    points: Vec<(f64, f64)>,
    line: LineKind,
    flowing: bool,
}

/// Everything the canvas paints this frame.
struct Frame {
    camera: Option<Camera>,
    items: Vec<Item>,
    wires: Vec<Wire>,
    hmi: Hmi,
    t: f32,
    cache: Rc<Tessellations>,
    drawn: Rc<Cell<Option<Drawn>>>,
    focused: bool,
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    // No REST from render (pages are drawn hidden to warm them): on show,
    // and when the screen on show changed elsewhere (the dashboard's tabs,
    // `--screen`), fetch after this frame.
    if console.page == Page::Mimic
        && console.phase == Phase::Console
        && !console
            .state_ref::<State>()
            .is_some_and(|state| state.layouts.contains_key(&active(console)))
    {
        cx.defer_in(window, |console, _, _| sync(console));
    }
    let theme = theme(cx).clone();
    let hmi = Hmi::new(&theme);
    let now = Instant::now();
    let on_screen = console.page == Page::Mimic && console.phase == Phase::Console;
    let focus = focus_handle(console, cx);
    let focused = focus.contains_focused(window, cx);
    let screen = active(console);
    let stress = console.stress.is_some();
    let window_height = f64::from(f32::from(window.viewport_size().height));
    let state = console.state::<State>();
    state.view = Some(window.current_view());
    state.window_height = window_height;
    if state.armed.as_ref().is_some_and(|armed| armed.expired(now)) {
        state.armed = None;
    }
    let (camera, gliding) = state.camera_at(now);
    let t = state.clock(now);
    let loaded = state.layouts.get(&screen).cloned();
    let dragging = state.press.is_some_and(|press| press.moved);
    let pressing = state.press.is_some();
    let hovering = state.hover.is_some();
    let drawn = state.drawn.clone();
    let cache = state.cache.clone();
    cache.next_frame();

    // What is in view: the last frame's stage bounds under this camera.
    let view_camera = camera.or_else(|| drawn.get().map(|d| fit_of(&d)));
    let visible = drawn.get().zip(view_camera).map(|(d, camera)| {
        let (x0, y0) = camera.to_stage(0.0, 0.0);
        let (x1, y1) = camera.to_stage(
            f64::from(f32::from(d.bounds.size.width)),
            f64::from(f32::from(d.bounds.size.height)),
        );
        (x0 - OVERSCAN, y0 - OVERSCAN, x1 + OVERSCAN, y1 + OVERSCAN)
    });
    let in_view = |x: f64, y: f64, w: f64, h: f64| {
        visible.is_none_or(|(x0, y0, x1, y1)| x + w >= x0 && x <= x1 && y + h >= y0 && y <= y1)
    };

    if matches!(loaded, Some(Loaded::Ready(_))) {
        run_pending(console, cx);
    }
    let (items, wires, faceplate_item) = match &loaded {
        Some(Loaded::Ready(layout)) => {
            let state = console.state_ref::<State>().expect("made above");
            let items: Vec<Item> = layout
                .widgets
                .iter()
                .filter(|w| {
                    in_view(w.x, w.y, w.width, w.height)
                        || state.cursor.as_deref() == Some(w.id.as_str())
                })
                .map(|w| prepare(console, w, state))
                .collect();
            let values = StoreValues(&console.store);
            let wires: Vec<Wire> = layout
                .connections
                .iter()
                .filter_map(|connection| {
                    let from = layout.widget(&connection.from)?;
                    let to = layout.widget(&connection.to)?;
                    let points = mimic::route(from, to, &connection.waypoints);
                    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                    for (x, y) in &points {
                        x0 = x0.min(*x);
                        y0 = y0.min(*y);
                        x1 = x1.max(*x);
                        y1 = y1.max(*y);
                    }
                    in_view(x0, y0, x1 - x0, y1 - y0).then(|| Wire {
                        flowing: mimic::flowing(connection, layout, &values),
                        line: connection.line(),
                        points,
                    })
                })
                .collect();
            let faceplate_item = state
                .faceplate
                .as_ref()
                .and_then(|id| layout.widget(id))
                .map(|w| prepare(console, w, state));
            (items, wires, faceplate_item)
        }
        _ => (Vec::new(), Vec::new(), None),
    };
    if faceplate_item.is_none() {
        let state = console.state::<State>();
        if state.faceplate.is_some() && matches!(loaded, Some(Loaded::Ready(_))) {
            close_faceplate(state);
        }
    }

    // Motion: continuous while something turns or flows (or the camera
    // glides); half-second steps while something only blinks.
    let continuous = gliding
        || wires.iter().any(|wire| wire.flowing)
        || items.iter().any(|item| {
            item.symbol
                .as_ref()
                .is_some_and(|(resolved, live)| animates(resolved.def.key, live))
        });
    let banner_blinks = banner::blinking(&console.alarms);
    let blinking = banner_blinks
        || items.iter().any(|item| {
            item.symbol
                .as_ref()
                .is_some_and(|(_, live)| live.blinking())
                || widgets::blinking(item)
        });
    if on_screen {
        if continuous {
            anim::request_frame(window, &console.meter);
        } else if blinking {
            schedule_blink(console, t, cx);
        }
    }

    let can_command = mimic::write_route(console.is_admin(), console.is_operator()).is_some();
    let (tabs, names) = console.state_ref::<Screens>().map_or_else(
        || (vec![OVERVIEW.to_owned()], vec!["Overview".to_owned()]),
        |screens| {
            let tabs = screens.ids();
            let names = tabs.iter().map(|id| screens.name(id)).collect();
            (tabs, names)
        },
    );
    let zoom = view_camera.map(|camera| camera.zoom);
    let fitted = console.state::<State>().camera.is_none();
    let count = match &loaded {
        Some(Loaded::Ready(layout)) => layout.widgets.len(),
        _ => 0,
    };
    let bar = sub_bar(&tabs, &names, &screen, zoom, fitted, count, &theme, cx);

    let body: AnyElement = match loaded {
        _ if stress => empty_state(
            Icon::Factory,
            "No mimic in a stress run",
            "Mimics are the server's SCADA screens; the synthetic plant has none.",
            Vec::new(),
            cx,
        )
        .into_any_element(),
        None | Some(Loaded::Loading) => empty_state(
            Icon::Factory,
            "Loading the mimic",
            "The screen's equipment and connections are on the way.",
            Vec::new(),
            cx,
        )
        .into_any_element(),
        Some(Loaded::Failed(error)) => empty_state(
            Icon::CircleAlert,
            "The mimic did not load",
            error,
            vec![
                Button::new("mimic-retry", "Try again")
                    .icon(Icon::RefreshCw)
                    .on_click(cx.listener(|console, _, _, cx| {
                        reload(console);
                        cx.notify();
                    }))
                    .into_any_element(),
            ],
            cx,
        )
        .into_any_element(),
        Some(Loaded::Ready(layout)) if layout.widgets.is_empty() => empty_state(
            Icon::Factory,
            "No mimic on this screen",
            "Draw one in the web client's SCADA view (Edit); it shows here, live.",
            Vec::new(),
            cx,
        )
        .into_any_element(),
        Some(Loaded::Ready(_)) => {
            let frame = Frame {
                camera,
                items,
                wires,
                hmi,
                t,
                cache,
                drawn,
                focused,
            };
            stage(frame, pressing, cx)
        }
    };

    let stage_bg = anim::mix(theme.bg, hmi.stage, 0.6);
    let panel =
        faceplate_item.map(|item| faceplate::render(console, &item, &hmi, can_command, t, cx));
    let alarm_banner = banner::render(console, &hmi, t, cx);

    div()
        .id("mimic")
        .track_focus(&focus)
        .key_context(MIMIC)
        .on_action(cx.listener(|console, _: &Dismiss, _, cx| {
            let state = console.state::<State>();
            if state.armed.is_some() {
                state.armed = None;
                cx.notify();
            } else if state.faceplate.is_some() {
                close_faceplate(state);
                cx.notify();
            } else {
                cx.propagate();
            }
        }))
        .on_action(
            cx.listener(|console, _: &MimicNext, _, cx| move_cursor(console, Step::Next, cx)),
        )
        .on_action(cx.listener(|console, _: &MimicPrevious, _, cx| {
            move_cursor(console, Step::Previous, cx);
        }))
        .on_action(cx.listener(|console, _: &MimicUp, _, cx| {
            move_cursor(console, Step::Toward(Direction::Up), cx);
        }))
        .on_action(cx.listener(|console, _: &MimicDown, _, cx| {
            move_cursor(console, Step::Toward(Direction::Down), cx);
        }))
        .on_action(cx.listener(|console, _: &MimicLeft, _, cx| {
            move_cursor(console, Step::Toward(Direction::Left), cx);
        }))
        .on_action(cx.listener(|console, _: &MimicRight, _, cx| {
            move_cursor(console, Step::Toward(Direction::Right), cx);
        }))
        .on_action(cx.listener(|console, _: &MimicOpen, window, cx| activate(console, window, cx)))
        .on_action(cx.listener(|console, _: &MimicZoomIn, _, cx| zoom_key(console, Some(true), cx)))
        .on_action(cx.listener(|console, _: &MimicZoomOut, _, cx| {
            zoom_key(console, Some(false), cx);
        }))
        .on_action(cx.listener(|console, _: &MimicFit, _, cx| zoom_key(console, None, cx)))
        .on_action(cx.listener(|console, _: &MimicArmOn, _, cx| arm(console, true, cx)))
        .on_action(cx.listener(|console, _: &MimicArmOff, _, cx| arm(console, false, cx)))
        .on_action(cx.listener(|console, _: &MimicStepUp, _, cx| step_setpoint(console, 1.0, cx)))
        .on_action(cx.listener(|console, _: &MimicStepDown, _, cx| {
            step_setpoint(console, -1.0, cx);
        }))
        .on_action(cx.listener(|console, _: &MimicNextScreen, window, cx| {
            step_screen(console, true, window, cx);
        }))
        .on_action(cx.listener(|console, _: &MimicPreviousScreen, window, cx| {
            step_screen(console, false, window, cx);
        }))
        .on_action(cx.listener(|console, _: &MimicReload, _, cx| {
            reload(console);
            cx.notify();
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
                    div()
                        .id("mimic-stage")
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .bg(stage_bg)
                        .when(hovering && !dragging, |this| this.cursor_pointer())
                        .when(dragging, |this| this.cursor_grabbing())
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
                            on_scroll(console, event, cx);
                        }))
                        .child(body),
                )
                .children(panel),
        )
        .child(alarm_banner)
        .into_any_element()
}

/// Redraw at the next blink step (one pending at a time).
fn schedule_blink(console: &mut Console, t: f32, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.blink_pending {
        return;
    }
    state.blink_pending = true;
    let wait = Duration::from_secs_f32(motion::next_blink(t));
    cx.spawn(async move |this: WeakEntity<Console>, cx| {
        cx.background_executor().timer(wait).await;
        let _ = this.update(cx, |console, cx| {
            console.state::<State>().blink_pending = false;
            redraw(console, cx);
        });
    })
    .detach();
}

/// The stage: a canvas painting the frame, and while the stage is pressed,
/// window-wide listeners so a drag keeps panning outside it.
fn stage(frame: Frame, pressing: bool, cx: &mut Context<Console>) -> AnyElement {
    let console = cx.entity().downgrade();
    canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            paint_frame(frame, bounds, window, cx);
            if pressing {
                press_listeners(console, window);
            }
        },
    )
    .size_full()
    .into_any_element()
}

fn press_listeners(console: WeakEntity<Console>, window: &mut Window) {
    let mover = console.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            let _ = mover.update(cx, |console, cx| on_drag(console, event.position, cx));
        }
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            let _ = console.update(cx, |console, cx| on_release(console, event.position, cx));
        }
    });
}

fn paint_frame(frame: Frame, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let width = f64::from(f32::from(bounds.size.width));
    let height = f64::from(f32::from(bounds.size.height));
    let camera = frame
        .camera
        .unwrap_or_else(|| Camera::fit(width, height, FIT_PAD));
    frame.drawn.set(Some(Drawn { bounds, camera }));
    #[allow(clippy::cast_possible_truncation)]
    let zoom = camera.zoom as f32;
    #[allow(clippy::cast_possible_truncation)]
    let stage = Xf::translate(
        f32::from(bounds.origin.x) + camera.x as f32,
        f32::from(bounds.origin.y) + camera.y as f32,
    )
    .then(&Xf::scale(zoom, zoom));
    let hmi = frame.hmi;
    let mut canvas = Canvas {
        window,
        cx,
        hmi: &hmi,
        zoom,
        alpha: 1.0,
        cache: &frame.cache,
    };
    // The stage itself.
    #[allow(clippy::cast_possible_truncation)]
    let (stage_w, stage_h) = (mimic::STAGE_W as f32, mimic::STAGE_H as f32);
    canvas.rect(
        &stage,
        (0.0, 0.0, stage_w, stage_h),
        0.0,
        hmi.stage,
        Some((hmi.border, 1.0 / zoom)),
    );
    // The parent's overflow clips to the stage area.
    for wire in &frame.wires {
        paint_wire(&mut canvas, &stage, wire, frame.t);
    }
    for item in &frame.items {
        paint_item(&mut canvas, &stage, item, frame.t, frame.focused);
    }
}

/// A connection (`ConnectionLayer.tsx`): a pipe with its highlight and,
/// while it flows, moving dashes; a power line; a dashed signal line.
fn paint_wire(canvas: &mut Canvas<'_>, stage: &Xf, wire: &Wire, t: f32) {
    let Some((start, segments)) = mimic::rounded(&wire.points, 10.0) else {
        return;
    };
    // Routes are cached by their points: their strokes tessellate once
    // until the zoom changes.
    let mut hasher = DefaultHasher::new();
    for (x, y) in &wire.points {
        (x.to_bits(), y.to_bits()).hash(&mut hasher);
    }
    let shape = Shape::cached(hasher.finish(), || {
        let mut data = format!("M{} {}", start.0, start.1);
        for segment in &segments {
            match segment {
                mimic::Segment::Line((x, y)) => data.push_str(&format!(" L{x} {y}")),
                mimic::Segment::Quad { control, to } => {
                    data.push_str(&format!(" Q{} {} {} {}", control.0, control.1, to.0, to.1))
                }
            }
        }
        data
    });
    let hmi = *canvas.hmi;
    match wire.line {
        LineKind::Flow => {
            canvas.stroke(stage, &shape, hmi.pipe_lo, Line::scaled(11.0));
            canvas.stroke(stage, &shape, fade(hmi.pipe_hi, 0.7), Line::scaled(4.5));
            if wire.flowing {
                canvas.stroke(
                    stage,
                    &shape,
                    hmi.liquid_hi,
                    Line::scaled(3.0).round().dashed(8.0, 16.0, motion::flow(t)),
                );
            }
        }
        LineKind::Power => canvas.stroke(stage, &shape, hmi.power_line, Line::scaled(3.0)),
        LineKind::Data => canvas.stroke(
            stage,
            &shape,
            hmi.data_line,
            Line::scaled(1.75).dashed(6.0, 4.0, 0.0),
        ),
    }
}

fn paint_item(canvas: &mut Canvas<'_>, stage: &Xf, item: &Item, t: f32, focused: bool) {
    let widget = &item.widget;
    #[allow(clippy::cast_possible_truncation)]
    let (x, y, w, h) = (
        widget.x as f32,
        widget.y as f32,
        widget.width as f32,
        widget.height as f32,
    );
    let xf = stage.then(&Xf::translate(x, y));
    if item.symbol.is_some() {
        symbols::paint(canvas, item, &xf, t);
    } else if matches!(widget.kind, WidgetKind::Other(_)) {
        let hmi = *canvas.hmi;
        canvas.stroke(
            &xf,
            &Shape::rect(0.0, 0.0, w, h),
            hmi.fg_dim,
            Line::hairline(1.0).dashed(4.0, 3.0, 0.0),
        );
        canvas.text(
            &xf,
            (w / 2.0, h / 2.0),
            widget.kind.as_str(),
            Text::new(11.0, hmi.fg_dim).mono().max_width(w - 8.0),
        );
    } else if widget.kind.unclipped() {
        widgets::paint(canvas, item, &xf, t);
    } else {
        canvas.clip(&xf, (0.0, 0.0, w, h), |canvas| {
            widgets::paint(canvas, item, &xf, t)
        });
    }
    // The faceplate's equipment, the keyboard cursor, the pointer's.
    let hmi = *canvas.hmi;
    let ring = |pad: f32| Shape::round_rect(-pad, -pad, w + 2.0 * pad, h + 2.0 * pad, 4.0);
    if item.selected {
        canvas.stroke(&xf, &ring(2.0), hmi.accent, Line::hairline(1.5));
    } else if item.hovered {
        canvas.stroke(&xf, &ring(2.0), fade(hmi.accent, 0.45), Line::hairline(1.0));
    }
    if item.cursor && focused {
        canvas.stroke(
            &xf,
            &ring(5.0),
            hmi.accent,
            Line::hairline(2.0).dashed(5.0, 3.0, 0.0),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn sub_bar(
    tabs: &[String],
    names: &[String],
    active: &str,
    zoom: Option<f64>,
    fitted: bool,
    count: usize,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let hover = theme.hover;
    let accent = theme.accent;
    let tab_elements: Vec<AnyElement> = tabs
        .iter()
        .zip(names)
        .enumerate()
        .map(|(index, (id, name))| {
            let selected = id == active;
            let target = id.clone();
            div()
                .id(ElementId::NamedInteger(
                    "mimic-tab".into(),
                    u64::try_from(index).unwrap_or(0),
                ))
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_3)
                .h(CONTROL_MD)
                .px(SPACE_5)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .border_1()
                .border_color(gpui::transparent_black())
                .focus(move |style| style.border_color(accent))
                .text_size(FONT_BODY)
                .text_color(if selected { theme.fg } else { theme.fg_muted })
                .font_weight(if selected {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .when(selected, |this| this.bg(theme.selected))
                .when(!selected, |this| this.hover(move |style| style.bg(hover)))
                .on_click(cx.listener(move |console, _, window, cx| {
                    switch_screen(console, &target, window, cx);
                }))
                .when(index == 0, |this| {
                    this.child(vt_ui::icon(Icon::Gauge, ICON_SM, theme.fg_dim))
                })
                .child(name.clone())
                .into_any_element()
        })
        .collect();
    let percent: SharedString = zoom
        .map_or_else(|| "—".to_owned(), |zoom| format!("{:.0}%", zoom * 100.0))
        .into();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .h(px(44.0))
        .px(SPACE_9)
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.chrome)
        .child(
            div()
                .id("mimic-tabs")
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_2)
                .overflow_x_scroll()
                .children(tab_elements),
        )
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .whitespace_nowrap()
                .child(format!(
                    "{count} widget{}",
                    if count == 1 { "" } else { "s" }
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(
                    Button::new("mimic-zoom-out", "−")
                        .compact()
                        .tooltip("Zoom out  ⌘−")
                        .on_click(
                            cx.listener(|console, _, _, cx| zoom_key(console, Some(false), cx)),
                        ),
                )
                .child(
                    Button::new("mimic-fit", percent)
                        .compact()
                        .tooltip("Fit the stage  ⌘0")
                        .on_click(cx.listener(|console, _, _, cx| zoom_key(console, None, cx))),
                )
                .child(
                    Button::new("mimic-zoom-in", "+")
                        .compact()
                        .tooltip("Zoom in  ⌘+")
                        .on_click(
                            cx.listener(|console, _, _, cx| zoom_key(console, Some(true), cx)),
                        ),
                )
                .when(fitted, |this| {
                    this.child(
                        div()
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child("fit"),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .whitespace_nowrap()
                .child(kbd("Tab", cx))
                .child("equipment")
                .child(kbd("↵", cx))
                .child("faceplate"),
        )
        .into_any_element()
}

/// The colour of a level, for chips and pills.
pub(crate) fn level_color(hmi: &Hmi, level: mimic::Level) -> Hsla {
    match level {
        mimic::Level::Critical => hmi.fault,
        mimic::Level::Warning => hmi.warn,
        mimic::Level::Normal => hmi.fg_dim,
    }
}

/// The width of the faceplate along the page's right edge, which the
/// toasts keep clear of.
pub fn side_panel(console: &Console) -> Pixels {
    let open = console.page == Page::Mimic
        && console
            .state_ref::<State>()
            .is_some_and(|state| state.faceplate.is_some());
    if open { FACEPLATE_WIDTH } else { px(0.0) }
}

/// The REST write in flight and the last answer, for the faceplate.
pub(crate) fn write_status(
    console: &Console,
    widget: &str,
    tag: i64,
) -> Option<Result<String, String>> {
    let state = console.state_ref::<State>()?;
    if state.sending == Some(tag) {
        return Some(Ok("Sending…".to_owned()));
    }
    if let Some((id, result)) = &state.result
        && id == widget
    {
        return Some(result.clone());
    }
    if let Some((id, sent)) = &state.sent
        && id == widget
    {
        if console.writing == Some(*sent) {
            return Some(Ok("Sending…".to_owned()));
        }
        if let Some((tag, result)) = &console.write_result
            && tag == sent
        {
            return Some(result.clone().map(|_| "Written".to_owned()));
        }
    }
    None
}

/// The armed command of the faceplate on `widget`.
pub(crate) fn armed(console: &Console, widget: &str) -> Option<Armed> {
    console
        .state_ref::<State>()?
        .armed
        .clone()
        .filter(|armed| armed.widget == widget)
}

/// The setpoint being adjusted on `widget`.
pub(crate) fn setpoint(console: &Console, widget: &str) -> Option<f64> {
    console
        .state_ref::<State>()?
        .setpoint
        .as_ref()
        .filter(|(id, _)| id == widget)
        .map(|(_, value)| *value)
}

/// What the faceplate's buttons do.
pub(crate) mod act {
    use gpui::{Context, Window};

    use super::{State, close_faceplate};
    use crate::console::Console;

    pub fn arm(console: &mut Console, on: bool, cx: &mut Context<Console>) {
        super::arm(console, on, cx);
    }

    pub fn confirm(console: &mut Console, cx: &mut Context<Console>) {
        super::confirm(console, cx);
    }

    pub fn disarm(console: &mut Console, cx: &mut Context<Console>) {
        console.state::<State>().armed = None;
        cx.notify();
    }

    pub fn step(console: &mut Console, steps: f64, cx: &mut Context<Console>) {
        super::step_setpoint(console, steps, cx);
    }

    pub fn close(console: &mut Console, cx: &mut Context<Console>) {
        close_faceplate(console.state::<State>());
        cx.notify();
    }

    pub fn open_tag(
        console: &mut Console,
        tag: i64,
        window: &mut Window,
        cx: &mut Context<Console>,
    ) {
        console.open_tag(tag, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lose_needless_decimals() {
        assert_eq!(trim_number(50.0), "50");
        assert_eq!(trim_number(0.25), "0.25");
        assert_eq!(trim_number(-1.5), "-1.5");
    }

    #[test]
    fn only_moving_parts_ask_for_frames() {
        let live = |state, run_state| mimic::Live {
            state,
            level: None,
            run_state,
            running: false,
            alarm: None,
        };
        use mimic::EquipState::{Idle, Running, Stopped};
        assert!(animates("pump", &live(Running, Idle)));
        assert!(!animates("pump", &live(Stopped, Idle)));
        assert!(!animates("inverter", &live(Running, Idle)), "nothing turns");
        assert!(animates("pipe", &live(Idle, Running)));
        assert!(animates("mixer", &live(Idle, Running)));
    }
}

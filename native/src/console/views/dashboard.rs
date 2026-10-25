//! Screens as card dashboards (the web client's card view): a tab per
//! screen under the navbar, and the screen's tags as cards on the
//! 24-column grid of their saved layout. A click or Enter opens a card's
//! tag; the arrows move between cards.
//!
//! Operators edit (E): drag a card to move it and its corner to resize it,
//! with the grid reflowing under it at the display's rate; arrows move the
//! focused card and Shift-arrows resize it; A arranges the cards by device,
//! T adds tags, Backspace removes the card; screens are created, renamed and
//! deleted from the tabs or the palette. Leaving edit mode (E, Esc, Done)
//! saves the layout. Only the cards in view are drawn, so the overview of a
//! 10 000-tag plant scrolls as lightly as a small screen.
//!
//! The session's screens live in [`crate::console::screens`]; the grid's
//! rules in [`voltrus_scada_native::screens`].

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, DispatchPhase, Div, ElementId, Entity, FocusHandle, FontWeight, Hsla,
    Image, ImageFormat, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Point, SharedString, Stateful, Subscription, Window, actions, canvas, div, point, px,
    uniform_list,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use voltrus_scada_native::screens::{
    self as grid, Breakpoint, Card, CardKind, Direction, Grid, LayoutItem, OVERVIEW, Rect,
    ScreenLayouts,
};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, empty_state, field, kbd, pill};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::dashboard_cards::{self as cards, Picture};
use super::{LIST_TAIL, focus_ring, now_ms};
use crate::console::keys::{Dismiss, ListDown, ListUp, MENU};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::screens::{self, Rights, Screens, rights};
use crate::console::store::{Store, Tag};
use crate::console::{Console, Page, Phase, anim};

/// The key context of the card grid: its arrows, Enter and letters.
pub const DASHBOARD: &str = "Dashboard";

actions!(
    voltrus_console,
    [
        DashboardUp,
        DashboardDown,
        DashboardLeft,
        DashboardRight,
        DashboardTaller,
        DashboardShorter,
        DashboardWider,
        DashboardNarrower,
        DashboardOpen,
        DashboardEdit,
        DashboardArrange,
        DashboardAddTags,
        DashboardRemove,
        DashboardNewScreen,
        DashboardRenameScreen,
        DashboardDeleteScreen,
        DashboardNextScreen,
        DashboardPreviousScreen,
        DashboardReload,
    ]
);

/// Show the n-th screen: 1 is the overview, 2 the first of the server's.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = voltrus_console, no_json)]
pub struct DashboardScreen(pub usize);

const SCROLL: &str = "dashboard";
const PICK_LIST: &str = "dashboard-pick";
const PICK_ROW: f32 = 34.0;
/// Cards move to a new place in this long.
const MOVE: Duration = Duration::from_millis(180);
/// Cards this far outside the view are drawn too, so a scroll shows them.
const OVERSCAN: f32 = 320.0;
/// A press that moves less than this is a click, not a drag.
const DRAG_SLOP: f32 = 3.0;
/// The card header's height.
const HEADER: f32 = 34.0;
const PLACEHOLDER: &str = "\u{0}placeholder";

/// A card on its way from one place to another.
#[derive(Clone, Copy)]
struct Tween {
    from: Rect,
    to: Rect,
    since: Instant,
}

impl Tween {
    /// A tween already over: the card stands at `at` and asks for no
    /// frames.
    fn still(at: Rect, now: Instant) -> Self {
        Self {
            from: at,
            to: at,
            since: now.checked_sub(MOVE).unwrap_or(now),
        }
    }

    /// Where it is at `now`, and whether it is still moving.
    fn at(&self, now: Instant) -> (Rect, bool) {
        match anim::progress(self.since, now, MOVE) {
            Some(t) => {
                let t = anim::ease_out(t);
                let mix = |a: f32, b: f32| a + (b - a) * t;
                (
                    Rect {
                        x: mix(self.from.x, self.to.x),
                        y: mix(self.from.y, self.to.y),
                        w: mix(self.from.w, self.to.w),
                        h: mix(self.from.h, self.to.h),
                    },
                    true,
                )
            }
            None => (self.to, false),
        }
    }
}

/// A card being dragged (moved, or resized by its corner).
struct Drag {
    id: String,
    resize: bool,
    /// Where the pointer went down and where it is, in window pixels.
    grab: Point<gpui::Pixels>,
    pointer: Point<gpui::Pixels>,
    /// The card's place when the drag began, in grid pixels.
    start: Rect,
    /// The card's cells when the drag began.
    item: LayoutItem,
    /// The layout when the drag began: each step is placed on it afresh.
    base: Vec<LayoutItem>,
    /// The cell (or size) last placed.
    cell: (u32, u32),
    moved: bool,
    /// Where the card is drawn: under the pointer.
    drawn: Rect,
}

/// The layout the grid shows, cached until its inputs change: the screen,
/// and `key`: the breakpoint and the revisions of the screens, the tag list
/// and the draft.
struct Laid {
    screen: String,
    key: (Breakpoint, Option<u64>, u64, u64),
    /// The screen's tags (data source ids), in its order.
    ids: Vec<i64>,
    items: Vec<LayoutItem>,
    by_id: HashMap<String, usize>,
    rows: u32,
}

impl Laid {
    fn get(&self, id: &str) -> Option<&LayoutItem> {
        self.by_id.get(id).map(|index| &self.items[*index])
    }
}

enum Dialog {
    /// Name a new screen, or rename the one with this id.
    Name(Option<String>),
    Delete(String),
    AddTags,
}

#[derive(Default)]
struct State {
    focus: Option<FocusHandle>,
    editing: bool,
    /// Layout edits not saved yet: the screen and its layouts.
    draft: Option<(String, ScreenLayouts)>,
    /// Counts draft changes, for the layout cache.
    revision: u64,
    /// The card the keys act on.
    cursor: Option<String>,
    drag: Option<Drag>,
    laid: Option<Rc<Laid>>,
    tweens: HashMap<String, Tween>,
    /// The grid width and screen last drawn: a change places cards at once.
    drawn: (f32, String),
    images: HashMap<String, Picture>,
    image_asks: HashMap<i64, String>,
    next_image: i64,
    dialog: Option<Dialog>,
    name: Option<Entity<TextInput>>,
    query: Option<Entity<TextInput>>,
    dialog_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
    /// The command line's `--screen`, `--edit` and `--overlay add-tags`,
    /// applied once the screen list arrives.
    launch_screen: Option<String>,
    launch_edit: bool,
    launch_picker: bool,
}

/// What the grid shows right now.
struct Scene {
    screen: String,
    bp: Breakpoint,
    grid: Grid,
    laid: Rc<Laid>,
}

fn active(console: &Console) -> String {
    console
        .state_ref::<Screens>()
        .map_or_else(|| OVERVIEW.to_owned(), |screens| screens.active.clone())
}

fn ids(console: &Console, screen: &str) -> Vec<i64> {
    console
        .state_ref::<Screens>()
        .map_or_else(Vec::new, |screens| screens.tag_ids(&console.store, screen))
}

/// The tag with data source id `id`.
fn tag_of(store: &Store, id: i64) -> Option<&Tag> {
    store.position(id).map(|index| &store.tags[index])
}

fn cards_of<'a>(store: &'a Store, ids: &[i64]) -> Vec<Card<'a>> {
    ids.iter()
        .filter_map(|id| tag_of(store, *id))
        .map(|tag| Card {
            id: tag.info.id,
            device: &tag.info.device_id,
            kind: CardKind::of(&tag.info.chart_type),
        })
        .collect()
}

/// The grid's width: the window's, less the page's padding.
fn grid_width(window: &Window) -> f32 {
    (f32::from(window.viewport_size().width) - 2.0 * f32::from(SPACE_9)).max(240.0)
}

/// The saved (or drafted) layouts of `screen`.
fn layouts_of(console: &mut Console, screen: &str) -> ScreenLayouts {
    let draft = console
        .state::<State>()
        .draft
        .as_ref()
        .filter(|(drafted, _)| drafted == screen)
        .map(|(_, layouts)| layouts.clone());
    draft.unwrap_or_else(|| {
        console
            .state_ref::<Screens>()
            .and_then(|screens| screens.layouts.get(screen).cloned())
            .unwrap_or_default()
    })
}

fn laid(console: &mut Console, screen: &str, bp: Breakpoint) -> Rc<Laid> {
    let key = (
        bp,
        console
            .state_ref::<Screens>()
            .map(|screens| screens.generation),
        console.store.generation(),
        console.state::<State>().revision,
    );
    if let Some(laid) = console
        .state::<State>()
        .laid
        .as_ref()
        .filter(|laid| laid.key == key && laid.screen == screen)
    {
        return laid.clone();
    }
    let ids = ids(console, screen);
    let layouts = layouts_of(console, screen);
    let items = grid::effective(&layouts, bp, &cards_of(&console.store, &ids));
    let by_id = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.i.clone(), index))
        .collect();
    let laid = Rc::new(Laid {
        screen: screen.to_owned(),
        key,
        ids,
        rows: grid::bottom(&items),
        items,
        by_id,
    });
    console.state::<State>().laid = Some(laid.clone());
    laid
}

fn scene(console: &mut Console, window: &Window) -> Scene {
    let screen = active(console);
    let width = grid_width(window);
    let bp = Breakpoint::for_width(width);
    let laid = laid(console, &screen, bp);
    Scene {
        screen,
        bp,
        grid: Grid::new(bp, width),
        laid,
    }
}

/// Put `items` in the draft as `bp`'s layout; the first edit starts the
/// draft from every breakpoint as shown. Another screen's draft is saved,
/// never dropped.
fn edit(console: &mut Console, scene: &Scene, items: Vec<LayoutItem>) {
    let drafted = match console.state::<State>().draft.take() {
        Some((screen, layouts)) if screen == scene.screen => Some(layouts),
        Some((screen, layouts)) => {
            screens::save_layouts(console, &screen, layouts);
            None
        }
        None => None,
    };
    let mut layouts = match drafted {
        Some(layouts) => layouts,
        None => {
            let saved = layouts_of(console, &scene.screen);
            grid::materialize(&saved, &cards_of(&console.store, &scene.laid.ids))
        }
    };
    layouts.set(scene.bp, items);
    let state = console.state::<State>();
    state.draft = Some((scene.screen.clone(), layouts));
    state.revision += 1;
}

/// Save the draft, if there is one.
fn save_draft(console: &mut Console) {
    let state = console.state::<State>();
    if let Some((screen, layouts)) = state.draft.take() {
        state.revision += 1;
        screens::save_layouts(console, &screen, layouts);
    }
}

fn grid_focus(console: &mut Console, cx: &mut Context<Console>) -> FocusHandle {
    console
        .state::<State>()
        .focus
        .get_or_insert_with(|| cx.focus_handle())
        .clone()
}

fn focus_grid(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let focus = grid_focus(console, cx);
    window.focus(&focus);
}

fn rights_here(console: &Console) -> Rights {
    rights(console.role(), &active(console))
}

/// Whether the screen on show may be edited: its saved layout is in (see
/// [`Screens::layout_known`]).
fn layout_known(console: &Console) -> bool {
    console
        .state_ref::<Screens>()
        .is_some_and(|screens| screens.layout_known(&screens.active))
}

fn layout_unknown(console: &mut Console, cx: &mut Context<Console>) {
    console.show_toast(
        "The screen's layout did not load: reload it (R) before editing",
        true,
        cx,
    );
}

/// Remember the command line's dashboard options (`--screen`, `--edit`).
pub(crate) fn launch(console: &mut Console, screen: Option<String>, edit: bool) {
    let state = console.state::<State>();
    state.launch_screen = screen;
    state.launch_edit = edit;
}

/// `--overlay add-tags`: open the tag picker once the screen is on show.
pub(crate) fn launch_picker(console: &mut Console) {
    console.state::<State>().launch_picker = true;
}

/// Apply the launch options, once, when the screen list is in.
fn apply_launch(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let listed = console
        .state_ref::<Screens>()
        .is_some_and(|screens| screens.listed);
    let state = console.state::<State>();
    if !listed || (state.launch_screen.is_none() && !state.launch_edit && !state.launch_picker) {
        return;
    }
    let screen = state.launch_screen.take();
    let edit = std::mem::take(&mut state.launch_edit);
    let picker = std::mem::take(&mut state.launch_picker);
    cx.defer_in(window, move |console, window, cx| {
        if let Some(wanted) = screen {
            let ids = console
                .state_ref::<Screens>()
                .map(Screens::ids)
                .unwrap_or_default();
            let target = wanted
                .parse::<usize>()
                .ok()
                .and_then(|position| ids.get(position.checked_sub(1)?).cloned())
                .or_else(|| ids.contains(&wanted).then_some(wanted));
            if let Some(id) = target {
                switch_screen(console, &id, window, cx);
            }
        }
        if edit && rights_here(console).arrange {
            console.state::<State>().editing = true;
        }
        if picker {
            open_picker(console, window, cx);
        }
        cx.notify();
    });
}

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    screens::sync(console);
    fetch_images(console);
    focus_grid(console, window, cx);
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    if key.name == "dashboard.image" {
        let state = console.state::<State>();
        if let Some(url) = state.image_asks.remove(&key.arg) {
            let picture = match reply.take::<(Vec<u8>, &'static str)>() {
                Ok((bytes, mime)) => match ImageFormat::from_mime_type(mime) {
                    Some(format) => Picture::Ready(Arc::new(Image::from_bytes(format, bytes))),
                    None => Picture::Failed("Not an image the console can draw".into()),
                },
                Err(error) => Picture::Failed(format!("The image did not load: {error}").into()),
            };
            state.images.insert(url, picture);
        }
        cx.notify();
    } else {
        if key.name == "dashboard.created" && reply.error().is_none() {
            // The new screen comes on show: the layout being edited is
            // saved first, as a switch of screens does.
            save_draft(console);
        }
        screens::on_reply(console, key, reply, cx);
        fetch_images(console);
    }
}

/// Open the page if another one is on show; whether the console is ready
/// for a dashboard command.
fn here(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    if console.phase != Phase::Console {
        return false;
    }
    if console.page != Page::Dashboard {
        console.show(Page::Dashboard, window, cx);
    }
    console.page == Page::Dashboard
}

/// Show screen `id`; an unsaved layout of the screen left is saved first.
fn switch_screen(console: &mut Console, id: &str, window: &mut Window, cx: &mut Context<Console>) {
    if active(console) != id {
        save_draft(console);
        screens::select(console, id);
        fetch_images(console);
        let state = console.state::<State>();
        state.cursor = None;
        state.drag = None;
        state.tweens.clear();
        console.scroll(SCROLL).set_offset(point(px(0.0), px(0.0)));
    }
    focus_grid(console, window, cx);
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
    switch_screen(console, &ids[next], window, cx);
}

fn needs_operator(console: &mut Console, cx: &mut Context<Console>) {
    console.show_toast("Changing screens needs an operator role", true, cx);
}

fn toggle_edit(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !rights_here(console).arrange {
        needs_operator(console, cx);
        return;
    }
    if console.state::<State>().editing {
        finish_edit(console);
    } else if layout_known(console) {
        console.state::<State>().editing = true;
    } else {
        layout_unknown(console, cx);
    }
    focus_grid(console, window, cx);
    cx.notify();
}

fn finish_edit(console: &mut Console) {
    let state = console.state::<State>();
    state.editing = false;
    state.drag = None;
    save_draft(console);
}

fn arrange(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !rights_here(console).arrange {
        needs_operator(console, cx);
        return;
    }
    if !layout_known(console) {
        layout_unknown(console, cx);
        return;
    }
    let scene = scene(console, window);
    let layouts = grid::auto_arrange_all(&cards_of(&console.store, &scene.laid.ids));
    let state = console.state::<State>();
    state.editing = true;
    state.draft = Some((scene.screen, layouts));
    state.revision += 1;
    focus_grid(console, window, cx);
    cx.notify();
}

/// Arrows: the focus moves to the nearest card that way; while editing,
/// the focused card moves (or, with `resize`, grows and shrinks).
fn arrow(
    console: &mut Console,
    direction: Direction,
    resize: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if console.page != Page::Dashboard {
        return;
    }
    let scene = scene(console, window);
    let editing =
        console.state::<State>().editing && rights_here(console).arrange && layout_known(console);
    let cursor = console
        .state::<State>()
        .cursor
        .clone()
        .filter(|id| scene.laid.by_id.contains_key(id));
    let Some(id) = cursor else {
        let first = scene
            .laid
            .items
            .iter()
            .min_by_key(|item| (item.y, item.x))
            .map(|item| item.i.clone());
        console.state::<State>().cursor = first.clone();
        if let Some(first) = first.and_then(|id| scene.laid.get(&id).cloned()) {
            reveal(console, scene.grid.rect(&first));
        }
        cx.notify();
        return;
    };
    let cols = scene.bp.cols();
    let target = if editing {
        let items = if resize {
            grid::grow(&scene.laid.items, &id, direction, cols)
        } else {
            grid::nudge(&scene.laid.items, &id, direction, cols)
        };
        let moved = items.iter().find(|item| item.i == id).cloned();
        if items != scene.laid.items {
            edit(console, &scene, items);
        }
        moved
    } else if resize {
        None
    } else {
        let next = grid::neighbor(&scene.laid.items, &id, direction).cloned();
        if let Some(next) = &next {
            console.state::<State>().cursor = Some(next.i.clone());
        }
        next
    };
    if let Some(item) = target {
        reveal(console, scene.grid.rect(&item));
    }
    cx.notify();
}

/// Scroll so the card at `rect` is in view.
fn reveal(console: &mut Console, rect: Rect) {
    let scroll = console.scroll(SCROLL);
    let view = f32::from(scroll.bounds().size.height);
    if view <= 0.0 {
        return;
    }
    let pad = f32::from(SPACE_9);
    let offset = scroll.offset();
    let shown = -f32::from(offset.y);
    let (top, bottom) = (rect.y + pad, rect.bottom() + pad);
    let wanted = if top - pad < shown {
        top - pad
    } else if bottom + pad > shown + view {
        bottom + pad - view
    } else {
        return;
    };
    scroll.set_offset(point(offset.x, px(-wanted.max(0.0))));
}

fn open_card(console: &mut Console, id: &str, window: &mut Window, cx: &mut Context<Console>) {
    console.state::<State>().cursor = Some(id.to_owned());
    if let Ok(tag) = id.parse::<i64>() {
        console.open_tag(tag, window, cx);
    }
}

fn remove_card(console: &mut Console, id: &str, cx: &mut Context<Console>) {
    let screen = active(console);
    if !rights(console.role(), &screen).tags {
        console.show_toast(
            "Only a screen's own tags can be removed, by an operator",
            true,
            cx,
        );
        return;
    }
    let Ok(tag) = id.parse::<i64>() else {
        return;
    };
    let state = console.state::<State>();
    if let Some((drafted, layouts)) = &mut state.draft
        && *drafted == screen
    {
        for bp in Breakpoint::ALL {
            let kept: Vec<LayoutItem> = layouts
                .get(bp)
                .iter()
                .filter(|item| item.i != id)
                .cloned()
                .collect();
            layouts.set(bp, kept);
        }
        state.revision += 1;
    }
    if state.cursor.as_deref() == Some(id) {
        state.cursor = None;
    }
    screens::remove_tag(console, &screen, tag);
    cx.notify();
}

fn start_drag(
    console: &mut Console,
    id: &str,
    resize: bool,
    at: Point<gpui::Pixels>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let scene = scene(console, window);
    let Some(item) = scene.laid.get(id).cloned() else {
        return;
    };
    let start = scene.grid.rect(&item);
    let cell = if resize {
        (item.w, item.h)
    } else {
        (item.x, item.y)
    };
    let state = console.state::<State>();
    state.cursor = Some(id.to_owned());
    state.drag = Some(Drag {
        id: id.to_owned(),
        resize,
        grab: at,
        pointer: at,
        start,
        item,
        base: scene.laid.items.clone(),
        cell,
        moved: false,
        drawn: start,
    });
    focus_grid(console, window, cx);
    cx.notify();
}

fn end_drag(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some(drag) = state.drag.take() else {
        return;
    };
    if drag.moved {
        // The card glides from under the pointer to its cell.
        state
            .tweens
            .insert(drag.id.clone(), Tween::still(drag.drawn, Instant::now()));
    }
    state.cursor = Some(drag.id);
    cx.notify();
}

/// Follow the pointer: draw the card under it and lay the others out
/// around the cell it is over.
fn follow_drag(console: &mut Console, scene: &Scene) {
    let cols = scene.bp.cols();
    let grid = scene.grid;
    let Some(drag) = console.state::<State>().drag.as_mut() else {
        return;
    };
    let dx = f32::from(drag.pointer.x - drag.grab.x);
    let dy = f32::from(drag.pointer.y - drag.grab.y);
    if dx.abs() > DRAG_SLOP || dy.abs() > DRAG_SLOP {
        drag.moved = true;
    }
    let smallest = grid.rect(&LayoutItem {
        w: grid::MIN_W,
        h: grid::MIN_H,
        ..LayoutItem::default()
    });
    let cell = if drag.resize {
        drag.drawn = Rect {
            w: (drag.start.w + dx)
                .min(grid.width - drag.start.x)
                .max(smallest.w),
            h: (drag.start.h + dy).max(smallest.h),
            ..drag.start
        };
        grid.span(drag.item.x, drag.drawn.w, drag.drawn.h)
    } else {
        drag.drawn = Rect {
            x: (drag.start.x + dx).clamp(0.0, (grid.width - drag.start.w).max(0.0)),
            y: (drag.start.y + dy).max(0.0),
            ..drag.start
        };
        grid.cell(drag.drawn.x, drag.drawn.y, drag.item.w)
    };
    if !drag.moved || cell == drag.cell {
        return;
    }
    drag.cell = cell;
    let items = if drag.resize {
        grid::resize(&drag.base, &drag.id, cell.0, cell.1, cols)
    } else {
        grid::place(&drag.base, &drag.id, cell.0, cell.1, cols)
    };
    edit(console, scene, items);
}

/// Where a card is drawn now: gliding to `target` when it changed place.
fn glide(
    tweens: &mut HashMap<String, Tween>,
    id: &str,
    target: Rect,
    now: Instant,
) -> (Rect, bool) {
    match tweens.get_mut(id) {
        Some(tween) if tween.to == target => tween.at(now),
        Some(tween) => {
            let (from, _) = tween.at(now);
            *tween = Tween {
                from,
                to: target,
                since: now,
            };
            (from, true)
        }
        None => {
            tweens.insert(id.to_owned(), Tween::still(target, now));
            (target, false)
        }
    }
}

/// Forget the tweens of cards not drawn this frame (scrolled away, removed,
/// or on another screen): one drawn again starts where it is. Every drawn
/// card has one, so nothing is left to forget when the counts match.
fn prune_tweens(tweens: &mut HashMap<String, Tween>, shown: &[Shown]) {
    let placeholder = usize::from(tweens.contains_key(PLACEHOLDER));
    if tweens.len() <= shown.len() + placeholder {
        return;
    }
    let drawn: HashSet<&str> = shown.iter().map(|card| card.id.as_str()).collect();
    tweens.retain(|id, _| id == PLACEHOLDER || drawn.contains(id.as_str()));
}

/// One card to draw.
struct Shown {
    id: String,
    rect: Rect,
    ring: bool,
    lifted: bool,
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    // Launch options apply on the page itself, never in the warm-up's
    // hidden draw. Fetches happen on show and in the reply hooks, not here.
    if console.page == Page::Dashboard {
        apply_launch(console, window, cx);
    }
    let theme = theme(cx).clone();
    let view = window.current_view();
    let focus = grid_focus(console, cx);
    let focused = focus.is_focused(window);
    let scene = scene(console, window);
    follow_drag(console, &scene);
    // A drag step changed the draft: lay out again.
    let laid_now = laid(console, &scene.screen, scene.bp);
    let scene = Scene {
        laid: laid_now,
        ..scene
    };
    let can = rights(console.role(), &scene.screen);
    let (ready, error, tabs, names) = console.state_ref::<Screens>().map_or_else(
        || {
            (
                false,
                None,
                vec![OVERVIEW.to_owned()],
                vec!["Overview".to_owned()],
            )
        },
        |screens| {
            let tabs = screens.ids();
            let names = tabs.iter().map(|id| screens.name(id)).collect::<Vec<_>>();
            (
                screens.ready(&screens.active),
                screens.error.clone(),
                tabs,
                names,
            )
        },
    );
    let now = Instant::now();
    let wall = now_ms();
    let span = cards::spark_span(&console.store, wall);

    // Visible cards only: a scroll redraws the page.
    let scroll = console.scroll(SCROLL);
    let pad = f32::from(SPACE_9);
    let shown_from = -f32::from(scroll.offset().y) - pad - OVERSCAN;
    let view_height =
        f32::from(scroll.bounds().size.height).max(f32::from(window.viewport_size().height));
    let shown_to = shown_from + view_height + 2.0 * OVERSCAN;

    let known = layout_known(console);
    let state = console.state::<State>();
    let editing = state.editing && can.arrange && known;
    if state.drawn.0 != scene.grid.width || state.drawn.1 != scene.screen {
        state.drawn = (scene.grid.width, scene.screen.clone());
        state.tweens.clear();
    }
    let cursor = state.cursor.clone();
    let dragged = state
        .drag
        .as_ref()
        .map(|drag| (drag.id.clone(), drag.drawn));
    let mut moving = false;
    let mut shown: Vec<Shown> = Vec::new();
    let mut placeholder = None;
    for item in &scene.laid.items {
        let target = scene.grid.rect(item);
        let is_cursor = cursor.as_deref() == Some(item.i.as_str());
        if !is_cursor && (target.bottom() < shown_from || target.y > shown_to) {
            continue;
        }
        if let Some((id, drawn)) = &dragged
            && *id == item.i
        {
            let (rect, glides) = glide(&mut state.tweens, PLACEHOLDER, target, now);
            moving |= glides;
            placeholder = Some(rect);
            state.tweens.insert(id.clone(), Tween::still(*drawn, now));
            continue;
        }
        let (rect, glides) = glide(&mut state.tweens, &item.i, target, now);
        moving |= glides;
        shown.push(Shown {
            id: item.i.clone(),
            rect,
            ring: focused && is_cursor,
            lifted: false,
        });
    }
    if dragged.is_none() {
        state.tweens.remove(PLACEHOLDER);
    }
    if let Some((id, drawn)) = dragged {
        shown.push(Shown {
            id,
            rect: drawn,
            ring: false,
            lifted: true,
        });
    }
    prune_tweens(&mut state.tweens, &shown);

    // Image cards' pictures, fetched by `fetch_images`.
    let mut pictures: HashMap<String, Picture> = HashMap::new();
    for card in &shown {
        let Some(tag) = card
            .id
            .parse::<i64>()
            .ok()
            .and_then(|id| tag_of(&console.store, id))
        else {
            continue;
        };
        if CardKind::of(&tag.info.chart_type) != CardKind::Image {
            continue;
        }
        let url = grid::card_config(&tag.info.source_config, "image_url");
        if url.is_empty() {
            continue;
        }
        let picture = console
            .state::<State>()
            .images
            .get(&url)
            .cloned()
            .unwrap_or(Picture::Loading);
        pictures.insert(card.id.clone(), picture);
    }

    let mut elements: Vec<AnyElement> = Vec::with_capacity(shown.len() + 1);
    if let Some(rect) = placeholder {
        elements.push(
            div()
                .absolute()
                .left(px(rect.x))
                .top(px(rect.y))
                .w(px(rect.w))
                .h(px(rect.h))
                .rounded(RADIUS_XL)
                .border_2()
                .border_dashed()
                .border_color(theme.accent.opacity(0.7))
                .bg(theme.accent_soft)
                .into_any_element(),
        );
    }
    for card in &shown {
        let Some(tag) = card
            .id
            .parse::<i64>()
            .ok()
            .and_then(|id| tag_of(&console.store, id))
        else {
            continue;
        };
        let (element, animating) = card_element(
            card,
            tag,
            CardLook {
                editing,
                removable: editing && can.tags,
                picture: pictures.get(&card.id),
                now: (now, wall, span),
            },
            &theme,
            cx,
        );
        moving |= animating;
        elements.push(element);
    }
    let card_count = scene.laid.items.len();
    if moving {
        anim::request_frame(window, &console.meter);
    }

    let dragging = console.state::<State>().drag.is_some();
    let console_entity = cx.entity().downgrade();
    let body = if !ready && card_count == 0 {
        match error.clone() {
            Some(error) => empty_state(
                Icon::CircleAlert,
                "The screen did not load",
                error,
                vec![
                    Button::new("dashboard-retry", "Try again")
                        .icon(Icon::RefreshCw)
                        .on_click(cx.listener(|console, _, _, cx| {
                            screens::reload(console);
                            cx.notify();
                        }))
                        .into_any_element(),
                ],
                cx,
            )
            .into_any_element(),
            None => empty_state(
                Icon::Columns,
                "Loading the screen",
                "Its tags and card layout are on the way.",
                Vec::new(),
                cx,
            )
            .into_any_element(),
        }
    } else if card_count == 0 {
        let overview = scene.screen == OVERVIEW;
        empty_state(
            Icon::Columns,
            if overview {
                "No tags yet"
            } else {
                "No tags on this screen"
            },
            if overview {
                "The overview shows every tag once the server lists them."
            } else if can.tags {
                "Add tags to give the screen its cards (T)."
            } else {
                "An operator can add tags to this screen."
            },
            if can.tags {
                vec![
                    Button::new("dashboard-empty-add", "Add tags")
                        .primary()
                        .icon(Icon::Plus)
                        .on_click(cx.listener(|console, _, window, cx| {
                            open_picker(console, window, cx);
                        }))
                        .into_any_element(),
                ]
            } else {
                Vec::new()
            },
            cx,
        )
        .into_any_element()
    } else {
        let height = scene.grid.height(scene.laid.rows);
        div()
            .id("dashboard-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .child(
                div().p(SPACE_9).pb(LIST_TAIL).child(
                    div()
                        .relative()
                        .w(px(scene.grid.width))
                        .h(px(height))
                        .when(dragging, |this| this.cursor_grabbing())
                        .children(elements)
                        .when(dragging, |this| {
                            this.child(
                                canvas(
                                    |_, _, _| {},
                                    move |_, (), window, _| {
                                        drag_listeners(console_entity, view, window);
                                    },
                                )
                                .absolute()
                                .size_full(),
                            )
                        }),
                ),
            )
            .into_any_element()
    };

    div()
        .id("dashboard")
        .track_focus(&focus)
        .key_context(DASHBOARD)
        .on_action(cx.listener(|console, _: &Dismiss, _, cx| {
            if console.state::<State>().editing {
                finish_edit(console);
                cx.notify();
            } else {
                cx.propagate();
            }
        }))
        .flex()
        .flex_col()
        .size_full()
        .child(sub_bar(
            &tabs,
            &names,
            &scene.screen,
            editing,
            can,
            card_count,
            f32::from(window.viewport_size().width) >= WIDE,
            &theme,
            cx,
        ))
        .when_some(error.filter(|_| ready || card_count > 0), |this, error| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_4)
                    .px(SPACE_9)
                    .py(SPACE_3)
                    .bg(theme.danger.opacity(0.08))
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(FONT_SMALL)
                    .text_color(theme.danger)
                    .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
                    .child(div().flex_1().truncate().child(error)),
            )
        })
        .child(div().flex_1().min_h_0().child(body))
        .into_any_element()
}

/// While a card is dragged: every pointer move and the release, wherever
/// they happen, go to the drag; a move redraws only this page.
fn drag_listeners(console: gpui::WeakEntity<Console>, view: gpui::EntityId, window: &mut Window) {
    let mover = console.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let _ = mover.update(cx, |console, _| {
            if let Some(drag) = console.state::<State>().drag.as_mut() {
                drag.pointer = event.position;
            }
        });
        cx.notify(view);
    });
    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            let _ = console.update(cx, end_drag);
        }
    });
}

struct CardLook<'a> {
    editing: bool,
    removable: bool,
    picture: Option<&'a Picture>,
    now: (Instant, f64, f64),
}

fn card_element(
    card: &Shown,
    tag: &Tag,
    look: CardLook,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> (AnyElement, bool) {
    let kind = CardKind::of(&tag.info.chart_type);
    let rect = card.rect;
    let id = card.id.clone();
    let border = if card.lifted || card.ring {
        theme.accent
    } else if tag.status.needs_attention() && kind.shows_status() {
        super::status_color(tag.status, theme).alpha(0.45)
    } else {
        theme.border
    };
    let hover = theme.border_strong;
    let (body, animating) = cards::body(tag, kind, look.picture, look.now, rect.h - HEADER, theme);
    let header = (kind.shows_status() || look.editing).then(|| {
        let grip = look.editing.then(|| grip(theme.fg_dim));
        let remove = look.removable.then(|| {
            let id = id.clone();
            div()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    IconButton::new(SharedString::from(format!("card-remove-{id}")), Icon::X)
                        .color(theme.danger)
                        .tooltip("Remove from this screen  ⌫")
                        .on_click(cx.listener(move |console, _, _, cx| {
                            remove_card(console, &id, cx);
                        })),
                )
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_3)
            .h(px(HEADER))
            .children(grip)
            .child(if kind.shows_status() {
                cards::header(tag, theme).flex_1().into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(FONT_SMALL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg_muted)
                    .child(tag.info.name.clone())
                    .into_any_element()
            })
            .children(remove)
    });
    let element = div()
        .id(SharedString::from(format!("card-{id}")))
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.w))
        .h(px(rect.h))
        .flex()
        .flex_col()
        .gap(SPACE_2)
        .px(SPACE_6)
        .pb(SPACE_5)
        .when(header.is_none(), |this| this.pt(SPACE_5))
        .rounded(RADIUS_XL)
        .border_1()
        .border_color(border)
        .bg(if card.lifted {
            theme.elevated
        } else {
            theme.surface
        })
        .overflow_hidden()
        .when(card.lifted, |this| {
            this.shadow(theme.shadow(Elevation::Modal)).opacity(0.96)
        })
        .when(!card.lifted, |this| {
            this.hover(move |style| style.border_color(hover))
        })
        .map(|this| {
            if look.editing {
                let grab = id.clone();
                let release = id.clone();
                this.cursor_grab()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |console, event: &MouseDownEvent, window, cx| {
                            start_drag(console, &grab, false, event.position, window, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |console, _: &MouseUpEvent, _, cx| {
                            if console
                                .state::<State>()
                                .drag
                                .as_ref()
                                .is_some_and(|drag| drag.id == release)
                            {
                                end_drag(console, cx);
                            }
                        }),
                    )
            } else {
                let open = id.clone();
                this.cursor_pointer()
                    .on_click(cx.listener(move |console, _, window, cx| {
                        open_card(console, &open, window, cx);
                    }))
            }
        })
        .children(header)
        .child(body)
        .when(look.editing && !card.lifted, |this| {
            let corner = id.clone();
            this.child(
                div()
                    .id(SharedString::from(format!("card-resize-{id}")))
                    .absolute()
                    .right(px(2.0))
                    .bottom(px(2.0))
                    .size(px(14.0))
                    .cursor_nwse_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |console, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            start_drag(console, &corner, true, event.position, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(3.0))
                            .bottom(px(3.0))
                            .w(px(8.0))
                            .h(px(2.0))
                            .rounded_full()
                            .bg(theme.fg_dim),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(3.0))
                            .bottom(px(3.0))
                            .w(px(2.0))
                            .h(px(8.0))
                            .rounded_full()
                            .bg(theme.fg_dim),
                    ),
            )
        })
        .when(card.ring, |this| {
            this.child(focus_ring(theme).rounded(RADIUS_XL))
        })
        .into_any_element();
    (element, animating)
}

/// Fetch the pictures of the active screen's image cards not asked for
/// yet. Only while the page is on show, never for the warm-up's hidden
/// draw: on show, on a switch of screens, and when the screens or the tag
/// list change.
fn fetch_images(console: &mut Console) {
    if console.phase != Phase::Console || console.page != Page::Dashboard {
        return;
    }
    let screen = active(console);
    let wanted: Vec<String> = ids(console, &screen)
        .into_iter()
        .filter_map(|id| tag_of(&console.store, id))
        .filter(|tag| CardKind::of(&tag.info.chart_type) == CardKind::Image)
        .map(|tag| grid::card_config(&tag.info.source_config, "image_url"))
        .filter(|url| !url.is_empty())
        .collect();
    for url in wanted {
        fetch_image(console, url);
    }
}

/// The tag list changed (the runtime's `Event::Tags`): new image cards may
/// need their pictures.
pub(crate) fn tags_changed(console: &mut Console) {
    fetch_images(console);
}

fn fetch_image(console: &mut Console, url: String) {
    let state = console.state::<State>();
    if state.images.contains_key(&url) {
        return;
    }
    if url.starts_with("data:") {
        state.images.insert(
            url,
            Picture::Failed("Inline data images are not supported".into()),
        );
        return;
    }
    state.images.insert(url.clone(), Picture::Loading);
    state.next_image += 1;
    let ticket = state.next_image;
    state.image_asks.insert(ticket, url.clone());
    console.call(
        CallKey::new("dashboard.image").with(ticket),
        Call::new(move |api| async move { grid::fetch_image(&api, &url).await }),
    );
}

/// The drag handle: two columns of three dots.
fn grip(color: Hsla) -> Div {
    let dot = || div().size(px(3.0)).rounded_full().bg(color);
    div()
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(2.0))
        .children((0..3).map(|_| div().flex().gap(px(2.0)).child(dot()).child(dot())))
}

/// From this window width the edit bar has room for its key hints.
const WIDE: f32 = 1280.0;

#[allow(clippy::too_many_arguments)]
fn sub_bar(
    tabs: &[String],
    names: &[String],
    active: &str,
    editing: bool,
    can: Rights,
    count: usize,
    wide: bool,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> Div {
    let hover = theme.hover;
    let accent = theme.accent;
    let tab_elements: Vec<AnyElement> = tabs
        .iter()
        .zip(names)
        .enumerate()
        .flat_map(|(index, (id, name))| {
            let selected = id == active;
            let target = id.clone();
            let tab = div()
                .id(ElementId::NamedInteger(
                    "screen-tab".into(),
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
                .tab_index(0)
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
                .when(index < 9, |this| {
                    this.child(kbd((index + 1).to_string(), cx))
                })
                .into_any_element();
            let mut row = vec![tab];
            if selected && editing && can.screens && index > 0 {
                row.push(
                    IconButton::new("screen-rename", Icon::Pencil)
                        .tooltip("Rename the screen")
                        .on_click(cx.listener(|console, _, window, cx| {
                            open_name(console, true, window, cx);
                        }))
                        .into_any_element(),
                );
                row.push(
                    IconButton::new("screen-delete", Icon::Trash)
                        .color(theme.danger)
                        .tooltip("Delete the screen")
                        .on_click(cx.listener(|console, _, window, cx| {
                            open_delete(console, window, cx);
                        }))
                        .into_any_element(),
                );
            }
            row
        })
        .collect();
    let controls: Vec<AnyElement> = if editing {
        let mut controls: Vec<AnyElement> = Vec::new();
        if wide {
            controls.push(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .whitespace_nowrap()
                    .child("Drag to move · corner resizes · ←↑↓→ move · ⇧ resize")
                    .into_any_element(),
            );
        }
        if can.tags {
            controls.push(
                Button::new("dashboard-add", "Add tags")
                    .icon(Icon::Plus)
                    .compact()
                    .tooltip("Add tags to this screen  T")
                    .on_click(cx.listener(|console, _, window, cx| {
                        open_picker(console, window, cx);
                    }))
                    .into_any_element(),
            );
        }
        controls.push(
            Button::new("dashboard-arrange", "Arrange")
                .icon(Icon::Columns)
                .compact()
                .tooltip("Arrange the cards by device  A")
                .on_click(cx.listener(|console, _, window, cx| arrange(console, window, cx)))
                .into_any_element(),
        );
        controls.push(
            Button::new("dashboard-done", "Done")
                .primary()
                .icon(Icon::Check)
                .compact()
                .tooltip("Save the layout  E or Esc")
                .on_click(cx.listener(|console, _, window, cx| toggle_edit(console, window, cx)))
                .into_any_element(),
        );
        controls
    } else {
        let mut controls = vec![
            div()
                .font_family(MONO)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .whitespace_nowrap()
                .child(format!("{count} card{}", if count == 1 { "" } else { "s" }))
                .into_any_element(),
        ];
        if can.arrange {
            controls.push(
                Button::new("dashboard-edit", "Edit")
                    .icon(Icon::Pencil)
                    .compact()
                    .tooltip("Move, resize and add cards  E")
                    .on_click(
                        cx.listener(|console, _, window, cx| toggle_edit(console, window, cx)),
                    )
                    .into_any_element(),
            );
        }
        controls
    };
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
                .id("screen-tabs")
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_2)
                .overflow_x_scroll()
                .children(tab_elements)
                .when(editing && can.screens, |this| {
                    this.child(
                        IconButton::new("screen-new", Icon::Plus)
                            .tooltip("New screen  N")
                            .on_click(cx.listener(|console, _, window, cx| {
                                open_name(console, false, window, cx);
                            })),
                    )
                }),
        )
        .when(editing, |this| this.child(pill("Editing", theme.accent)))
        .children(controls)
}

// ── Dialogs ───────────────────────────────────────────────────────────────

fn open_name(console: &mut Console, rename: bool, window: &mut Window, cx: &mut Context<Console>) {
    if !here(console, window, cx) {
        return;
    }
    let screen = active(console);
    if !rights(console.role(), &screen).screens {
        needs_operator(console, cx);
        return;
    }
    if rename && screen == OVERVIEW {
        console.show_toast("The overview keeps its name", true, cx);
        return;
    }
    let initial = if rename {
        console
            .state_ref::<Screens>()
            .map(|screens| screens.name(&screen))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let field = cx.new(|cx| TextInput::new(cx).text(initial).placeholder("Screen name"));
    let subscription = cx.subscribe_in(
        &field,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => submit_name(console, window, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Changed | InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.dialog = Some(Dialog::Name(rename.then_some(screen)));
    state.name = Some(field.clone());
    state._subscriptions = vec![subscription];
    let overlay = Overlay::modal("dashboard-name", px(420.0), name_dialog, cx);
    console.open_overlay(overlay, window, cx);
    field.update(cx, |field, _| field.focus(window));
}

fn submit_name(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let name = state
        .name
        .as_ref()
        .map(|field| field.read(cx).value().trim().to_owned())
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let dialog = state.dialog.take();
    console.close_overlay(window, cx);
    match dialog {
        Some(Dialog::Name(Some(id))) => screens::rename(console, &id, name),
        Some(Dialog::Name(None)) => screens::create(console, name),
        _ => {}
    }
    cx.notify();
}

fn dialog_frame(
    title: &'static str,
    content: impl IntoElement,
    buttons: Vec<AnyElement>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .px(SPACE_8)
                .h(px(48.0))
                .border_b_1()
                .border_color(theme.border)
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().p(SPACE_8).child(content))
        .child(
            div()
                .flex()
                .flex_none()
                .justify_end()
                .gap(SPACE_4)
                .px(SPACE_8)
                .py(SPACE_6)
                .border_t_1()
                .border_color(theme.border)
                .children(buttons),
        )
}

fn cancel_button(cx: &mut Context<Console>) -> AnyElement {
    Button::new("dialog-cancel", "Cancel")
        .ghost()
        .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx)))
        .into_any_element()
}

fn name_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let renaming = matches!(state.dialog, Some(Dialog::Name(Some(_))));
    let Some(input) = state.name.clone() else {
        return div().into_any_element();
    };
    dialog_frame(
        if renaming {
            "Rename the screen"
        } else {
            "New screen"
        },
        div()
            .flex()
            .flex_col()
            .gap(SPACE_4)
            .child(field("Name", input, cx))
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(if renaming {
                        "Every operator sees the new name."
                    } else {
                        "A screen starts empty: add tags to it in edit mode."
                    }),
            ),
        vec![
            cancel_button(cx),
            Button::new("dialog-name-ok", if renaming { "Rename" } else { "Create" })
                .primary()
                .on_click(cx.listener(|console, _, window, cx| submit_name(console, window, cx)))
                .into_any_element(),
        ],
        &theme,
    )
    .into_any_element()
}

fn open_delete(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !here(console, window, cx) {
        return;
    }
    let screen = active(console);
    if !rights(console.role(), &screen).screens {
        needs_operator(console, cx);
        return;
    }
    if screen == OVERVIEW {
        console.show_toast("The overview cannot be deleted", true, cx);
        return;
    }
    let focus = cx.focus_handle();
    let state = console.state::<State>();
    state.dialog = Some(Dialog::Delete(screen));
    state.dialog_focus = Some(focus.clone());
    let overlay = Overlay::modal("dashboard-delete", px(440.0), delete_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn confirm_delete(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let dialog = console.state::<State>().dialog.take();
    console.close_overlay(window, cx);
    if let Some(Dialog::Delete(id)) = dialog {
        let state = console.state::<State>();
        state.draft = state.draft.take().filter(|(screen, _)| *screen != id);
        state.cursor = None;
        state.tweens.clear();
        screens::delete(console, &id);
    }
    cx.notify();
}

fn delete_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let name = console
        .state_ref::<Screens>()
        .and_then(Screens::active_screen)
        .map(|screen| screen.name.clone())
        .unwrap_or_default();
    let Some(focus) = console.state::<State>().dialog_focus.clone() else {
        return div().into_any_element();
    };
    div()
        .track_focus(&focus)
        .on_key_down(
            cx.listener(move |console, event: &KeyDownEvent, window, cx| {
                // Enter on a focused button (Cancel) is that button's.
                if event.keystroke.key == "enter" && focus.is_focused(window) {
                    confirm_delete(console, window, cx);
                }
            }),
        )
        .child(dialog_frame(
            "Delete the screen",
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(format!(
                    "Delete the screen “{name}”? This cannot be undone."
                ))
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child("Its tags stay on the server and on the overview."),
                ),
            vec![
                cancel_button(cx),
                Button::new("dialog-delete-ok", "Delete  ↵")
                    .danger()
                    .icon(Icon::Trash)
                    .on_click(cx.listener(|console, _, window, cx| {
                        confirm_delete(console, window, cx);
                    }))
                    .into_any_element(),
            ],
            &theme,
        ))
        .into_any_element()
}

fn open_picker(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !here(console, window, cx) {
        return;
    }
    let screen = active(console);
    if !rights(console.role(), &screen).tags {
        if screen == OVERVIEW {
            console.show_toast(
                "The overview has every tag: pick a screen to add tags to",
                true,
                cx,
            );
        } else {
            needs_operator(console, cx);
        }
        return;
    }
    let field = cx.new(|cx| TextInput::new(cx).placeholder("Find a tag by name or key"));
    let subscription = cx.subscribe_in(
        &field,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                console.list(PICK_LIST, cx).cursor = Some(0);
                cx.notify();
            }
            InputEvent::Submitted => {
                let row = console.list(PICK_LIST, cx).cursor;
                if let Some(id) = row.and_then(|row| picks(console, cx).get(row).copied()) {
                    add_pick(console, id, cx);
                }
            }
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.dialog = Some(Dialog::AddTags);
    state.query = Some(field.clone());
    state._subscriptions = vec![subscription];
    let overlay = Overlay::modal("dashboard-add", px(520.0), picker, cx);
    console.open_overlay(overlay, window, cx);
    console.list(PICK_LIST, cx).cursor = Some(0);
    field.update(cx, |field, _| field.focus(window));
}

/// The tags the picker offers: not on the screen yet, matching its query.
fn picks(console: &Console, cx: &App) -> Vec<i64> {
    let screen = active(console);
    let on: HashSet<i64> = console
        .state_ref::<Screens>()
        .and_then(|screens| screens.tags.get(&screen))
        .map(|ids| ids.iter().copied().collect())
        .unwrap_or_default();
    let query = console
        .state_ref::<State>()
        .and_then(|state| state.query.as_ref())
        .map(|field| field.read(cx).value())
        .unwrap_or_default();
    // The store keeps filter results per query: no tag is lowercased here.
    console
        .store
        .filter(query)
        .iter()
        .map(|index| console.store.tags[*index].info.id)
        .filter(|id| !on.contains(id))
        .collect()
}

fn add_pick(console: &mut Console, id: i64, cx: &mut Context<Console>) {
    let screen = active(console);
    screens::add_tag(console, &screen, id);
    cx.notify();
}

fn picker(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(input) = console.state::<State>().query.clone() else {
        return div().into_any_element();
    };
    let count = picks(console, cx).len();
    let screen_name = console
        .state_ref::<Screens>()
        .and_then(Screens::active_screen)
        .map(|screen| screen.name.clone())
        .unwrap_or_default();
    let nav = console.list(PICK_LIST, cx);
    nav.set_len(count);
    let scroll = nav.scroll.clone();
    let list = uniform_list(
        "dashboard-picks",
        count,
        cx.processor(move |console: &mut Console, range: Range<usize>, _, cx| {
            let theme = vt_ui::theme::theme(cx).clone();
            let rows = picks(console, cx);
            let cursor = console.list(PICK_LIST, cx).cursor;
            range
                .filter_map(|row| {
                    let id = *rows.get(row)?;
                    let tag = tag_of(&console.store, id)?;
                    let active = cursor == Some(row);
                    let hover = theme.hover;
                    let color = cards::tag_color(&tag.info.color, theme.accent);
                    Some(
                        div()
                            .id(("dashboard-pick", row))
                            .flex()
                            .items_center()
                            .gap(SPACE_5)
                            .h(px(PICK_ROW))
                            .w_full()
                            .px(SPACE_5)
                            .rounded(RADIUS_MD)
                            .cursor_pointer()
                            .when(active, |this| this.bg(theme.selected))
                            .when(!active, |this| this.hover(move |style| style.bg(hover)))
                            .on_click(cx.listener(move |console, _, _, cx| {
                                console.list(PICK_LIST, cx).cursor = Some(row);
                                add_pick(console, id, cx);
                            }))
                            .child(div().size(px(8.0)).rounded_full().bg(color))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(tag.info.name.clone()),
                            )
                            .child(
                                div()
                                    .max_w(px(160.0))
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(FONT_CAPTION)
                                    .text_color(theme.fg_dim)
                                    .child(tag.info.key.clone()),
                            )
                            .when(active, |this| {
                                this.child(
                                    div()
                                        .text_size(FONT_CAPTION)
                                        .text_color(theme.accent)
                                        .child("↵ Add"),
                                )
                            }),
                    )
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(scroll)
    .px(SPACE_3)
    .h(px(PICK_ROW * 9.0));

    div()
        .key_context(MENU)
        .on_action(cx.listener(|console, _: &ListUp, _, cx| {
            console.list_move(PICK_LIST, Move::Up, cx);
        }))
        .on_action(cx.listener(|console, _: &ListDown, _, cx| {
            console.list_move(PICK_LIST, Move::Down, cx);
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
                .child(vt_ui::icon(Icon::Plus, ICON_LG, theme.fg_dim))
                .child(div().flex_1().child(input))
                .child(pill(screen_name, theme.accent)),
        )
        .child(div().py(SPACE_3).child(if count == 0 {
            div()
                .h(px(PICK_ROW * 2.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_dim)
                .child("Every matching tag is on this screen")
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
                .child(kbd("↑↓", cx))
                .child("Move")
                .child(kbd("↵", cx))
                .child("Add")
                .child(kbd("Esc", cx))
                .child("Close")
                .child(div().flex_1())
                .child(format!("{count} to add")),
        )
        .into_any_element()
}

// ── Commands ──────────────────────────────────────────────────────────────

/// The dashboard's commands, on the window's root: the grid's keys reach it
/// from the focused grid, the palette's from anywhere (they open the page).
pub(crate) fn actions(root: Stateful<Div>, cx: &Context<Console>) -> Stateful<Div> {
    root.on_action(cx.listener(|console, _: &DashboardUp, window, cx| {
        arrow(console, Direction::Up, false, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardDown, window, cx| {
        arrow(console, Direction::Down, false, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardLeft, window, cx| {
        arrow(console, Direction::Left, false, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardRight, window, cx| {
        arrow(console, Direction::Right, false, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardShorter, window, cx| {
        arrow(console, Direction::Up, true, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardTaller, window, cx| {
        arrow(console, Direction::Down, true, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardNarrower, window, cx| {
        arrow(console, Direction::Left, true, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardWider, window, cx| {
        arrow(console, Direction::Right, true, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardOpen, window, cx| {
        // Enter on a focused button of the page (a screen tab, Edit) is
        // that button's click, not the card's.
        let on_grid = console
            .state_ref::<State>()
            .and_then(|state| state.focus.as_ref())
            .is_some_and(|focus| focus.is_focused(window));
        if console.page == Page::Dashboard
            && on_grid
            && let Some(id) = console.state::<State>().cursor.clone()
        {
            open_card(console, &id, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &DashboardEdit, window, cx| {
        if here(console, window, cx) {
            toggle_edit(console, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &DashboardArrange, window, cx| {
        if here(console, window, cx) {
            arrange(console, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &DashboardAddTags, window, cx| {
        open_picker(console, window, cx);
    }))
    .on_action(cx.listener(|console, _: &DashboardRemove, _, cx| {
        let on_page = console.page == Page::Dashboard;
        let state = console.state::<State>();
        if on_page
            && state.editing
            && let Some(id) = state.cursor.clone()
        {
            remove_card(console, &id, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &DashboardNewScreen, window, cx| {
        open_name(console, false, window, cx);
    }))
    .on_action(
        cx.listener(|console, _: &DashboardRenameScreen, window, cx| {
            open_name(console, true, window, cx);
        }),
    )
    .on_action(
        cx.listener(|console, _: &DashboardDeleteScreen, window, cx| {
            open_delete(console, window, cx);
        }),
    )
    .on_action(cx.listener(|console, _: &DashboardNextScreen, window, cx| {
        if here(console, window, cx) {
            step_screen(console, true, window, cx);
        }
    }))
    .on_action(
        cx.listener(|console, _: &DashboardPreviousScreen, window, cx| {
            if here(console, window, cx) {
                step_screen(console, false, window, cx);
            }
        }),
    )
    .on_action(
        cx.listener(|console, action: &DashboardScreen, window, cx| {
            if !here(console, window, cx) {
                return;
            }
            let target = console
                .state_ref::<Screens>()
                .and_then(|screens| screens.ids().get(action.0.saturating_sub(1)).cloned());
            if let Some(id) = target {
                switch_screen(console, &id, window, cx);
            }
        }),
    )
    .on_action(cx.listener(|console, _: &DashboardReload, window, cx| {
        if here(console, window, cx) {
            let state = console.state::<State>();
            state.images.clear();
            state.tweens.clear();
            screens::reload(console);
            cx.notify();
        }
    }))
}

#[cfg(test)]
mod tests {
    use voltrus_scada_native::api::DataSource;
    use voltrus_scada_native::runtime::{Event, Link};
    use voltrus_scada_native::screens::Screen;

    use super::*;
    use crate::console::testing::{Harness, SERVER};

    fn dashboard_calls(calls: &[CallKey]) -> Vec<&'static str> {
        calls
            .iter()
            .filter(|key| key.scope() == "dashboard")
            .map(|key| key.name)
            .collect()
    }

    fn find(calls: &[CallKey], name: &str) -> CallKey {
        *calls
            .iter()
            .find(|key| key.name == name)
            .unwrap_or_else(|| panic!("no {name} in {calls:?}"))
    }

    /// Signed in as an admin with the dashboard on show; the calls its
    /// showing sent come back.
    fn on_dashboard(cx: &mut gpui::TestAppContext) -> (Harness, Vec<CallKey>) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| console.show(Page::Dashboard, window, cx));
        let calls = h.calls();
        (h, calls)
    }

    /// On the dashboard with the server's screen "a" on show.
    fn with_a_screen(cx: &mut gpui::TestAppContext) -> Harness {
        let (mut h, calls) = on_dashboard(cx);
        h.reply(
            find(&calls, "dashboard.screens"),
            Reply::ok(vec![Screen {
                id: "a".into(),
                name: "A".into(),
                ..Screen::default()
            }]),
        );
        h.update(|console, window, cx| switch_screen(console, "a", window, cx));
        h.calls();
        h
    }

    fn drafted() -> ScreenLayouts {
        ScreenLayouts {
            lg: vec![LayoutItem::new(1, 0, 0, 6, 4)],
            ..ScreenLayouts::default()
        }
    }

    #[gpui::test]
    fn drawing_the_dashboard_fetches_nothing(cx: &mut gpui::TestAppContext) {
        let mut h = Harness::new(cx);
        h.event(Event::SignedIn {
            username: "op".into(),
            role: "admin".into(),
            server: SERVER.into(),
            must_change_password: false,
        });
        h.event(Event::Link(Link::Live));
        h.event(Event::Tags(vec![DataSource {
            chart_type: "image".into(),
            source_config: r#"{"image_url":"/plant.png"}"#.into(),
            ..Harness::tag(1, "plts_1.camera")
        }]));
        for _ in 0..64 {
            if h.update(|console, _, _| console.warm.is_empty()) {
                break;
            }
            h.draw();
        }
        assert_ne!(h.update(|console, _, _| console.page), Page::Dashboard);
        h.draw();
        assert_eq!(
            dashboard_calls(&h.calls()),
            Vec::<&str>::new(),
            "the warm-up's hidden draw and later draws ask nothing"
        );
        h.update(|console, window, cx| console.show(Page::Dashboard, window, cx));
        let shown = dashboard_calls(&h.calls());
        for name in ["dashboard.screens", "dashboard.layouts", "dashboard.image"] {
            assert!(shown.contains(&name), "showing the page asks {name}");
        }
    }

    #[gpui::test]
    fn a_failed_load_is_not_asked_again_until_a_reload(cx: &mut gpui::TestAppContext) {
        let (mut h, calls) = on_dashboard(cx);
        h.reply(find(&calls, "dashboard.screens"), Reply::failed("HTTP 500"));
        h.reply(find(&calls, "dashboard.layouts"), Reply::failed("HTTP 500"));
        h.draw();
        h.update(|console, window, cx| console.focus_page(window, cx));
        h.draw();
        assert_eq!(dashboard_calls(&h.calls()), Vec::<&str>::new());

        // The layout is unknown, not a default a save would make permanent.
        h.update(toggle_edit);
        h.update(arrange);
        let (editing, draft) = h.update(|console, _, _| {
            let state = console.state::<State>();
            (state.editing, state.draft.is_some())
        });
        assert!(!editing && !draft, "editing waits for the layout");

        h.update(|console, _, _| screens::reload(console));
        let again = dashboard_calls(&h.calls());
        assert!(again.contains(&"dashboard.screens") && again.contains(&"dashboard.layouts"));
    }

    #[gpui::test]
    fn enter_on_the_focused_cancel_deletes_nothing(cx: &mut gpui::TestAppContext) {
        let mut h = with_a_screen(cx);
        h.update(open_delete);
        h.draw();
        h.keys("tab");
        let on_dialog = h.update(|console, window, _| {
            console
                .state::<State>()
                .dialog_focus
                .as_ref()
                .is_some_and(|focus| focus.is_focused(window))
        });
        assert!(!on_dialog, "Tab moved to a button");
        h.keys("enter");
        assert!(!dashboard_calls(&h.calls()).contains(&"dashboard.deleted"));
    }

    #[gpui::test]
    fn enter_on_the_delete_dialog_deletes_the_screen(cx: &mut gpui::TestAppContext) {
        let mut h = with_a_screen(cx);
        h.update(open_delete);
        h.draw();
        h.keys("enter");
        assert!(dashboard_calls(&h.calls()).contains(&"dashboard.deleted"));
    }

    #[gpui::test]
    fn enter_on_a_focused_button_of_the_page_opens_no_card(cx: &mut gpui::TestAppContext) {
        let (mut h, calls) = on_dashboard(cx);
        h.event(Event::Tags(vec![Harness::tag(1, "plts_1.p")]));
        h.reply(
            find(&calls, "dashboard.layouts"),
            Reply::ok(ScreenLayouts::default()),
        );
        h.update(|console, _, _| console.state::<State>().cursor = Some("1".into()));
        h.draw();
        let mut on_button = false;
        for _ in 0..64 {
            h.keys("tab");
            on_button = h.update(|console, window, cx| {
                console
                    .state::<State>()
                    .focus
                    .as_ref()
                    .is_some_and(|focus| {
                        focus.contains_focused(window, cx) && !focus.is_focused(window)
                    })
            });
            if on_button {
                break;
            }
        }
        assert!(on_button, "Tab reached a button of the page");
        h.keys("enter");
        assert_eq!(h.update(|console, _, _| console.page), Page::Dashboard);

        h.update(focus_grid);
        h.keys("enter");
        assert_eq!(
            h.update(|console, _, _| console.page),
            Page::Tags,
            "on the grid, Enter opens the card"
        );
    }

    #[gpui::test]
    fn creating_a_screen_saves_the_layout_being_edited(cx: &mut gpui::TestAppContext) {
        let mut h = with_a_screen(cx);
        h.update(|console, _, _| {
            let state = console.state::<State>();
            state.editing = true;
            state.draft = Some(("a".into(), drafted()));
            screens::create(console, "B".into());
        });
        let created = find(&h.calls(), "dashboard.created");
        h.reply(
            created,
            Reply::ok(Screen {
                id: "b".into(),
                name: "B".into(),
                ..Screen::default()
            }),
        );
        assert!(dashboard_calls(&h.calls()).contains(&"dashboard.saved"));
        let (active, saved) = h.update(|console, _, _| {
            let screens = console.state::<Screens>();
            (screens.active.clone(), screens.layouts.get("a").cloned())
        });
        assert_eq!(active, "b");
        assert_eq!(saved, Some(drafted()), "a's edits are kept");
    }

    #[gpui::test]
    fn an_edit_saves_another_screens_draft(cx: &mut gpui::TestAppContext) {
        let mut h = with_a_screen(cx);
        h.update(|console, window, _| {
            console.state::<State>().draft = Some(("b".into(), drafted()));
            let scene = scene(console, window);
            edit(console, &scene, Vec::new());
        });
        assert!(dashboard_calls(&h.calls()).contains(&"dashboard.saved"));
        let saved = h.update(|console, _, _| console.state::<Screens>().layouts.get("b").cloned());
        assert_eq!(saved, Some(drafted()), "b's edits are kept");
    }

    #[test]
    fn a_card_standing_still_asks_for_no_frames() {
        let now = Instant::now();
        let at = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        assert_eq!(Tween::still(at, now).at(now), (at, false));
        let mut tweens = HashMap::new();
        assert_eq!(glide(&mut tweens, "1", at, now), (at, false), "a new card");
        assert_eq!(glide(&mut tweens, "1", at, now), (at, false), "staying put");
        let to = Rect { x: 100.0, ..at };
        assert!(glide(&mut tweens, "1", to, now).1, "a moved card glides");
        assert_eq!(glide(&mut tweens, "1", to, now + MOVE), (to, false));
    }

    #[test]
    fn tweens_of_cards_no_longer_drawn_are_forgotten() {
        let now = Instant::now();
        let at = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        let mut tweens = HashMap::new();
        for id in ["1", "2", PLACEHOLDER] {
            glide(&mut tweens, id, at, now);
        }
        let shown = [Shown {
            id: "1".into(),
            rect: at,
            ring: false,
            lifted: false,
        }];
        prune_tweens(&mut tweens, &shown);
        let mut kept: Vec<&str> = tweens.keys().map(String::as_str).collect();
        kept.sort_unstable();
        assert_eq!(kept, [PLACEHOLDER, "1"]);
    }
}

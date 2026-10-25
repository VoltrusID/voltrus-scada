//! The tag tree: every tag filed by its `path` (or its device when it has
//! none), from `GET /api/v1/tags/tree`, searched on the server as you type.
//! Operators move a branch or one tag to another path (`PUT
//! /api/v1/tags/{id}/path`, each tag keeping its place below the branch)
//! and set the naming convention new tag names must follow.

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, FontWeight, Pixels, SharedString, Subscription, Window,
    div, px, uniform_list,
};
use reqwest::Method;
use serde::de::IgnoredAny;
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use voltrus_scada_native::tag_api::{
    self, NamingConvention, NamingUpdate, TagTree, TreeNode, TreeRow,
};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, empty_state, field, pill, section_label, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::tag_detail::{note_line, text_field};
use super::{
    ListKeys, focus_ring, keyboard_list, page_header, rows_on_screen, status_color, tail_row,
    tail_rows, value_color,
};
use crate::console::Console;
use crate::console::nav::Move;

/// The tree's keyboard list.
const LIST: &str = "tagtree";
const ROW: f32 = 30.0;
const PANEL: f32 = 360.0;
const INDENT: f32 = 18.0;

/// What the cursor is on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Node(String),
    Tag { id: i64, path: String },
}

#[derive(Default)]
pub(crate) struct TreeState {
    search: Option<Entity<TextInput>>,
    path: Option<Entity<TextInput>>,
    regex: Option<Entity<TextInput>>,
    tree: Vec<TreeNode>,
    loaded: bool,
    error: Option<String>,
    /// The search in flight; older answers are dropped.
    seq: i64,
    /// Nodes the operator closed; the rest are open.
    closed: HashSet<String>,
    selection: Option<Selection>,
    /// Path changes in flight, and how the finished ones went.
    moving: usize,
    moved: usize,
    move_errors: Vec<String>,
    move_to: Option<String>,
    /// Each tag's new path while its change is in flight.
    pending: std::collections::HashMap<i64, Option<String>>,
    path_error: Option<String>,
    naming: Option<NamingConvention>,
    naming_error: Option<String>,
    /// The template picked, before it is saved.
    template: Option<String>,
    saving_naming: bool,
    subscriptions: Vec<Subscription>,
}

pub fn side_panel(_: &Console) -> Pixels {
    px(PANEL)
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console
        .state_ref::<TreeState>()
        .and_then(|state| state.search.clone())
}

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<TreeState>().search.is_none() {
        let search = text_field("Search path, name or device  /", InputKind::Text, false, cx);
        let path = text_field("Site/Area/Unit (empty clears)", InputKind::Text, true, cx);
        let regex = text_field("^[A-Z]{2}\\d{3}$", InputKind::Text, true, cx);
        let subscriptions = [
            cx.subscribe_in(
                &search,
                window,
                |console, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Changed => fetch_tree(console, cx),
                    InputEvent::Submitted | InputEvent::Cancelled => {
                        let nav = console.list(LIST, cx);
                        nav.step(Move::First);
                        window.focus(&nav.focus);
                        cx.notify();
                    }
                    InputEvent::Blurred => {}
                },
            ),
            cx.subscribe_in(
                &path,
                window,
                |console, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submitted => apply_path(console, cx),
                    InputEvent::Cancelled => console.focus_list(LIST, window, cx),
                    InputEvent::Changed => {
                        console.state::<TreeState>().path_error = None;
                        cx.notify();
                    }
                    InputEvent::Blurred => {}
                },
            ),
            cx.subscribe_in(
                &regex,
                window,
                |console, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submitted => save_naming(console, cx),
                    InputEvent::Cancelled => console.focus_list(LIST, window, cx),
                    InputEvent::Changed | InputEvent::Blurred => {}
                },
            ),
        ];
        let state = console.state::<TreeState>();
        state.subscriptions.extend(subscriptions);
        state.search = Some(search);
        state.path = Some(path);
        state.regex = Some(regex);
    }
    let state = console.state::<TreeState>();
    let (loaded, has_naming) = (state.loaded, state.naming.is_some());
    if console.stress.is_none() {
        if !loaded {
            fetch_tree(console, cx);
        }
        if !has_naming {
            console.call(
                CallKey::new("tagtree.naming"),
                Call::get::<NamingConvention>("/api/v1/settings/naming-convention"),
            );
        }
    }
    console.focus_list(LIST, window, cx);
}

fn fetch_tree(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<TreeState>();
    let search = state
        .search
        .as_ref()
        .map(|field| field.read(cx).value().trim().to_owned())
        .unwrap_or_default();
    state.seq += 1;
    let seq = state.seq;
    let path = if search.is_empty() {
        "/api/v1/tags/tree".to_owned()
    } else {
        format!("/api/v1/tags/tree?search={}", tag_api::query_value(&search))
    };
    console.call(
        CallKey::new("tagtree.tree").with(seq),
        Call::get::<TagTree>(path),
    );
    console.list(LIST, cx).cursor = None;
    cx.notify();
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "tagtree.tree" => {
            let state = console.state::<TreeState>();
            if key.arg == state.seq {
                state.loaded = true;
                match reply.take::<TagTree>() {
                    Ok(tree) => {
                        state.tree = tree.tree;
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
            }
        }
        "tagtree.path" => {
            let error = reply.error().map(str::to_owned);
            let failed = error.is_some();
            let state = console.state::<TreeState>();
            state.moving = state.moving.saturating_sub(1);
            let path = state.pending.remove(&key.arg);
            match error {
                Some(error) => state.move_errors.push(format!("Tag {}: {error}", key.arg)),
                None => state.moved += 1,
            }
            // The console's own copy of the data source follows.
            if !failed && let (Some(path), Some(index)) = (path, console.store.position(key.arg)) {
                console.store.tags[index].info.path = path;
            }
            let state = console.state::<TreeState>();
            if state.moving == 0 {
                let (moved, errors) = (state.moved, std::mem::take(&mut state.move_errors));
                let to = state.move_to.take().unwrap_or_else(|| "no path".to_owned());
                if errors.is_empty() {
                    console.show_toast(
                        format!("Moved {moved} tag{} to {to}", plural(moved)),
                        false,
                        cx,
                    );
                } else {
                    console.show_toast(
                        format!(
                            "{} of {} moves failed: {}",
                            errors.len(),
                            moved + errors.len(),
                            errors.join("; ")
                        ),
                        true,
                        cx,
                    );
                }
                fetch_tree(console, cx);
            }
        }
        "tagtree.naming" => {
            let state = console.state::<TreeState>();
            match reply.take::<NamingConvention>() {
                Ok(naming) => {
                    state.template = Some(naming.template.clone());
                    state.naming = Some(naming);
                    state.naming_error = None;
                }
                Err(error) => state.naming_error = Some(error),
            }
            let regex = state.regex.clone();
            let custom = state
                .naming
                .as_ref()
                .and_then(|naming| naming.custom_regex.clone())
                .unwrap_or_default();
            if let Some(regex) = regex {
                regex.update(cx, |field, cx| field.set_text(custom, cx));
            }
        }
        "tagtree.naming_save" => {
            let state = console.state::<TreeState>();
            state.saving_naming = false;
            match reply.error() {
                Some(error) => state.naming_error = Some(error.to_owned()),
                None => {
                    state.naming = None;
                    console.call(
                        CallKey::new("tagtree.naming"),
                        Call::get::<NamingConvention>("/api/v1/settings/naming-convention"),
                    );
                    console.show_toast("Naming convention saved", false, cx);
                }
            }
        }
        _ => {}
    }
    cx.notify();
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// The selection's path, for the path field.
fn selection_path(selection: &Selection) -> &str {
    match selection {
        Selection::Node(path) | Selection::Tag { path, .. } => path,
    }
}

/// Move the cursor's row: it becomes the selection, its path the field's.
fn follow(console: &mut Console, row: usize, _: &mut Window, cx: &mut Context<Console>) {
    let rows = rows(console);
    let Some(selection) = rows.get(row).map(|row| match row {
        TreeRow::Node { path, .. } => Selection::Node(path.clone()),
        TreeRow::Tag { id, path, .. } => Selection::Tag {
            id: *id,
            path: path.clone(),
        },
    }) else {
        return;
    };
    let state = console.state::<TreeState>();
    if state.selection.as_ref() == Some(&selection) {
        return;
    }
    let text = selection_path(&selection).to_owned();
    state.selection = Some(selection);
    state.path_error = None;
    if let Some(field) = state.path.clone() {
        field.update(cx, |field, cx| field.set_text(text, cx));
    }
    cx.notify();
}

/// Enter: open or close a node, open a tag on the Tags page. Space: open
/// or close a node, put a tag on Trends.
fn activate(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    follow(console, row, window, cx);
    match rows(console).get(row).cloned() {
        Some(TreeRow::Node { path, .. }) => {
            let closed = &mut console.state::<TreeState>().closed;
            if !closed.remove(&path) {
                closed.insert(path);
            }
            cx.notify();
        }
        Some(TreeRow::Tag { id, .. }) => {
            if toggle {
                console.toggle_trend(id, cx);
            } else {
                console.open_tag(id, window, cx);
            }
        }
        None => {}
    }
}

fn rows(console: &mut Console) -> Vec<TreeRow> {
    let state = console.state::<TreeState>();
    tag_api::tree_rows(&state.tree, &state.closed)
}

/// Move the selection's tags to the path field's path.
fn apply_path(console: &mut Console, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast(
            "Moving tags needs the operator role (admin, demo or integration)",
            true,
            cx,
        );
        return;
    }
    let state = console.state::<TreeState>();
    if state.moving > 0 {
        return;
    }
    let text = state
        .path
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    let to = match tag_api::check_path(&text) {
        Ok(to) => to,
        Err(error) => {
            state.path_error = Some(error);
            cx.notify();
            return;
        }
    };
    let moves: Vec<(i64, Option<String>)> = match &state.selection {
        Some(Selection::Node(path)) => tag_api::find(&state.tree, path)
            .map(|node| tag_api::moved_paths(node, to.as_deref()))
            .unwrap_or_default(),
        Some(Selection::Tag { id, .. }) => vec![(*id, to.clone())],
        None => Vec::new(),
    };
    if moves.is_empty() {
        state.path_error = Some("Pick a node or a tag with tags to move".into());
        cx.notify();
        return;
    }
    state.moving = moves.len();
    state.moved = 0;
    state.move_errors.clear();
    state.move_to = Some(to.clone().unwrap_or_else(|| "no path".to_owned()));
    state.path_error = None;
    // The new path is the selection's: the cursor follows the branch.
    state.selection = to.map(Selection::Node);
    state.pending = moves.iter().cloned().collect();
    for (id, path) in moves {
        console.call(
            CallKey::new("tagtree.path").with(id),
            Call::send::<IgnoredAny>(
                Method::PUT,
                format!("/api/v1/tags/{id}/path"),
                serde_json::json!({ "path": path }),
            ),
        );
    }
    cx.notify();
}

fn save_naming(console: &mut Console, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast(
            "The naming convention needs the operator role (admin, demo or integration)",
            true,
            cx,
        );
        return;
    }
    let state = console.state::<TreeState>();
    let Some(template) = state.template.clone() else {
        return;
    };
    let regex = state
        .regex
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    state.saving_naming = true;
    state.naming_error = None;
    let body = NamingUpdate::new(&template, &regex);
    console.call(
        CallKey::new("tagtree.naming_save"),
        Call::new(move |api| async move {
            api.send_json::<_, IgnoredAny>(Method::PUT, "/api/v1/settings/naming-convention", &body)
                .await
        }),
    );
    cx.notify();
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let rows = Rc::new(rows(console));
    let count = rows.len();
    let state = console.state::<TreeState>();
    let total: usize = state.tree.iter().map(TreeNode::total).sum();
    let (loaded, error) = (state.loaded, state.error.clone());
    let search = state.search.clone();
    let selection = state.selection.clone();
    let searching = search
        .as_ref()
        .is_some_and(|field| !field.read(cx).value().trim().is_empty());

    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: Some(follow),
            on_activate: activate,
        },
        count,
        rows_on_screen(window, ROW),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let body = if count == 0 {
        empty_state(
            Icon::Network,
            if !loaded {
                "Loading the tag tree"
            } else if searching {
                "Nothing matches"
            } else {
                "No tags yet"
            },
            error.clone().unwrap_or_else(|| {
                "Tags are filed by their path, or by their device when they have none.".to_owned()
            }),
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        let marked = selection.clone();
        uniform_list(
            "tag-tree",
            count + tail_rows(count, ROW),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    range
                        .map(|row| match rows.get(row) {
                            Some(line) => {
                                let ring = focused && cursor == Some(row);
                                tree_row(console, line, row, ring, marked.as_ref(), &theme, cx)
                            }
                            None => tail_row(ROW),
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .size_full()
        .into_any_element()
    };

    let filter = div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .w(px(280.0))
        .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
        .children(search.map(|field| div().flex_1().child(field)));

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header(
            "Tag tree",
            format!("{total} tags in {count} rows · Enter opens, Space folds, / searches"),
            vec![filter.into_any_element()],
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(list.flex_1().min_w_0().px(SPACE_5).py(SPACE_3).child(body))
                .child(side(console, cx)),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn tree_row(
    console: &Console,
    line: &TreeRow,
    row: usize,
    ring: bool,
    selection: Option<&Selection>,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let hover = theme.hover;
    let (depth, selected) = match line {
        TreeRow::Node { path, depth, .. } => (
            *depth,
            selection.is_some_and(
                |selection| matches!(selection, Selection::Node(picked) if picked == path),
            ),
        ),
        TreeRow::Tag { id, depth, .. } => (
            *depth,
            selection.is_some_and(
                |selection| matches!(selection, Selection::Tag { id: picked, .. } if picked == id),
            ),
        ),
    };
    #[allow(clippy::cast_precision_loss)]
    let indent = px(INDENT * depth as f32);
    let base = div()
        .id(ElementId::NamedInteger(
            "tree-row".into(),
            u64::try_from(row).unwrap_or(0),
        ))
        .relative()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .h(px(ROW))
        .pl(indent + SPACE_4)
        .pr(SPACE_5)
        .rounded(RADIUS_MD)
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.selected))
        .when(!selected, |this| this.hover(move |style| style.bg(hover)))
        .on_click(cx.listener(move |console, _, window, cx| {
            console.list(LIST, cx).select(row);
            follow(console, row, window, cx);
        }));
    let content = match line {
        TreeRow::Node {
            name, total, open, ..
        } => base
            .child(vt_ui::icon(
                if *open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                ICON_SM,
                theme.fg_dim,
            ))
            .child(vt_ui::icon(Icon::Folder, ICON_MD, theme.fg_muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(name.clone()),
            )
            .child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(format!("{total} tag{}", plural(*total))),
            ),
        TreeRow::Tag { id, .. } => match console.store.position(*id) {
            Some(index) => {
                let tag = &console.store.tags[index];
                let mut value = tag.display();
                if tag.value.is_finite() && !tag.info.unit.is_empty() {
                    value = format!("{value} {}", tag.info.unit);
                }
                base.child(div().w(px(12.0)))
                    .child(
                        div()
                            .size(px(7.0))
                            .rounded_full()
                            .bg(status_color(tag.status, theme)),
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
                            .max_w(px(180.0))
                            .truncate()
                            .font_family(MONO)
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(tag.info.key.clone()),
                    )
                    .child(
                        div()
                            .w(px(110.0))
                            .text_right()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(value_color(tag.status, theme))
                            .child(value),
                    )
            }
            None => base.child(div().w(px(12.0))).child(
                div()
                    .flex_1()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(format!("Tag {id}")),
            ),
        },
    };
    content
        .when(ring, |this| this.child(focus_ring(theme)))
        .into_any_element()
}

/// The panel on the right: the selection, its path, the naming convention.
fn side(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let state = console.state::<TreeState>();
    let selection = state.selection.clone();
    let node_tags: Option<usize> = match &selection {
        Some(Selection::Node(path)) => tag_api::find(&state.tree, path).map(TreeNode::total),
        _ => None,
    };
    let path_field = state.path.clone();
    let regex_field = state.regex.clone();
    let path_error = state.path_error.clone();
    let moving = state.moving;
    let naming = state.naming.clone();
    let naming_error = state.naming_error.clone();
    let picked = state.template.clone();
    let saving_naming = state.saving_naming;
    let tag_name = match &selection {
        Some(Selection::Tag { id, .. }) => console
            .store
            .position(*id)
            .map(|index| console.store.tags[index].info.name.clone()),
        _ => None,
    };

    let selected_card = match &selection {
        None => note_line(
            Icon::Info,
            "Move the cursor to a node or a tag to see and change its path.".to_owned(),
            theme.fg_dim,
        )
        .into_any_element(),
        Some(selection) => {
            let (title, caption) = match selection {
                Selection::Node(path) => (
                    path.rsplit('/').next().unwrap_or(path).to_owned(),
                    format!(
                        "Branch · {} tag{} at and below it",
                        node_tags.unwrap_or(0),
                        plural(node_tags.unwrap_or(0))
                    ),
                ),
                Selection::Tag { id, .. } => (
                    tag_name.clone().unwrap_or_else(|| format!("Tag {id}")),
                    "One tag".to_owned(),
                ),
            };
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .child(
                    div()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(caption),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(SharedString::from(selection_path(selection).to_owned())),
                )
                .into_any_element()
        }
    };

    let path_section = div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .child(section_label("Path", cx))
        .child(selected_card)
        .map(|this| {
            if !operator {
                return this.child(note_line(
                    Icon::Lock,
                    "Moving tags needs the operator role (admin, demo or integration).".to_owned(),
                    theme.fg_dim,
                ));
            }
            this.when_some(path_field.filter(|_| selection.is_some()), |this, field_input| {
                this.child(field("Move to", field_input, cx))
                    .child(note_line(
                        Icon::Info,
                        "Each tag keeps its place below the branch. Empty clears the path (the tag files under its device)."
                            .to_owned(),
                        theme.fg_dim,
                    ))
                    .when_some(path_error.clone(), |this, error| {
                        this.child(note_line(Icon::CircleAlert, error, theme.danger))
                    })
                    .child(
                        Button::new(
                            "apply-path",
                            if moving > 0 { "Moving…" } else { "Move" },
                        )
                        .primary()
                        .icon(Icon::Check)
                        .disabled(moving > 0)
                        .on_click(cx.listener(|console, _, _, cx| apply_path(console, cx))),
                    )
            })
        });

    let naming_section = div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .pt(SPACE_7)
        .border_t_1()
        .border_color(theme.border)
        .child(section_label("Naming convention", cx))
        .map(|this| match &naming {
            None => this.child(note_line(
                if naming_error.is_some() {
                    Icon::CircleAlert
                } else {
                    Icon::Clock
                },
                naming_error
                    .clone()
                    .unwrap_or_else(|| "Loading…".to_owned()),
                if naming_error.is_some() {
                    theme.danger
                } else {
                    theme.fg_dim
                },
            )),
            Some(naming) => {
                let current = picked.clone().unwrap_or_else(|| naming.template.clone());
                let label = naming
                    .templates
                    .iter()
                    .find(|template| template.id == current)
                    .map_or_else(|| current.clone(), |template| template.label.clone());
                let segments: Vec<Segment> = naming
                    .templates
                    .iter()
                    .map(|template| {
                        let id = template.id.clone();
                        let short = short_template(&template.id);
                        Segment::new(
                            short,
                            template.id == current,
                            cx.listener(move |console, _, _, cx| {
                                console.state::<TreeState>().template = Some(id.clone());
                                cx.notify();
                            }),
                        )
                    })
                    .collect();
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(div().text_size(FONT_SMALL).child("In force"))
                        .child(pill(short_template(&naming.template), theme.accent)),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(naming.regex.clone()),
                )
                .when(operator, |this| {
                    this.child(segmented("naming-template", segments, cx))
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(label),
                        )
                        .when(current == "custom", |this| {
                            this.children(
                                regex_field.clone().map(|regex| {
                                    field("Pattern (a regular expression)", regex, cx)
                                }),
                            )
                        })
                        .when_some(naming_error.clone(), |this, error| {
                            this.child(note_line(Icon::CircleAlert, error, theme.danger))
                        })
                        .child(
                            Button::new(
                                "save-naming",
                                if saving_naming {
                                    "Saving…"
                                } else {
                                    "Save convention"
                                },
                            )
                            .icon(Icon::Check)
                            .disabled(saving_naming)
                            .on_click(cx.listener(|console, _, _, cx| save_naming(console, cx))),
                        )
                })
            }
        });

    div()
        .id("tag-tree-side")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(PANEL))
        .h_full()
        .overflow_y_scroll()
        .gap(SPACE_7)
        .p(SPACE_8)
        .bg(theme.chrome)
        .border_l_1()
        .border_color(theme.border)
        .child(path_section)
        .child(naming_section)
        .into_any_element()
}

/// A template's name for a narrow control.
fn short_template(id: &str) -> SharedString {
    match id {
        "isa-5.1" => "ISA-5.1".into(),
        "iec-61346" => "IEC 61346".into(),
        "kks" => "KKS".into(),
        "custom" => "Custom".into(),
        "permissive" => "Any".into(),
        other => SharedString::from(other.to_owned()),
    }
}

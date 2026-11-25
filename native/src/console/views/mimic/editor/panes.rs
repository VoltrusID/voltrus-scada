//! The Designer's left column (`client/src/designer/panes.tsx`): three
//! tabs over one column.
//!
//! - **Project**: the screens, the one being edited opened into its
//!   components (top of the stack first); a click on a screen switches
//!   (asking first when something is unsaved), on a component selects it.
//!   Hidden (`visible` false) and locked components are marked.
//! - **Tags**: every tag with its live value, searched by name, key or
//!   ref. A click binds the selected component's primary tag (one more tag
//!   for a multi-tag widget); a drag onto a component on the stage binds
//!   that one.
//! - **Components**: the palette (symbols and widgets).

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyView, Context, ElementId, Render, SharedString, Window, div, px, uniform_list,
};
use voltrus_scada_native::designer::{self, bindings};
use voltrus_scada_native::mimic::edit::{self, Edit};
use vt_ui::Icon;
use vt_ui::components::{Segment, segmented};
use vt_ui::theme::*;

use super::{Editor, apply, edits, edits_ref, focus_stage, stage_at};
use crate::console::Console;
use crate::console::screens::Screens;

/// The column's tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Pane {
    #[default]
    Project,
    Tags,
    Components,
}

/// What a tag drag carries, drawn under the pointer.
#[derive(Clone)]
pub(super) struct TagDrag {
    pub id: i64,
    pub name: SharedString,
}

impl Render for TagDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        div()
            .px(SPACE_4)
            .py(SPACE_2)
            .rounded(RADIUS_MD)
            .bg(theme.elevated)
            .border_1()
            .border_color(theme.accent)
            .text_size(FONT_SMALL)
            .text_color(theme.fg)
            .child(self.name.clone())
    }
}

/// Bind `tag` to `id`: the primary tag, or one more of a multi-tag widget.
fn bind_edit(editor: &Editor, id: &str, tag: i64) -> Result<Edit, &'static str> {
    let widget = editor.layout().widget(id).ok_or("That component is gone")?;
    match edit::max_tags(widget) {
        0 => Err("This component shows no tag"),
        1 => Ok(designer::bind_primary(widget, Some(tag))),
        _ => {
            let mut tags = widget.tags.clone();
            if !tags.contains(&tag) {
                tags.push(tag);
            }
            Ok(Edit::Bind {
                id: widget.id.clone(),
                tags,
            })
        }
    }
}

fn bind(console: &mut Console, id: &str, tag: i64, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = edits_ref(console) else {
        return;
    };
    match bind_edit(editor, id, tag) {
        Ok(bound) => {
            apply(console, &bound, None, window, cx);
            if let Some(editor) = edits(console) {
                editor.selection = vec![id.to_owned()];
                editor.line = None;
            }
            super::changed(console, window, cx);
        }
        Err(why) => console.show_toast(why, true, cx),
    }
}

/// A tag row clicked: bind it to the one selected component.
fn bind_selected(console: &mut Console, tag: i64, window: &mut Window, cx: &mut Context<Console>) {
    let selected = edits_ref(console).and_then(|editor| editor.single().map(|w| w.id.clone()));
    match selected {
        Some(id) => bind(console, &id, tag, window, cx),
        None => console.show_toast(
            "Select one component, then click a tag to bind it (or drag the tag onto it)",
            false,
            cx,
        ),
    }
}

/// Enter in the tag search: the first match binds the one selected
/// component; with none selected it opens the placement menu at the
/// middle of the visible stage (a drop from the keyboard).
pub(super) fn submit_tag(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = edits_ref(console) else {
        return;
    };
    let query = editor.tag_filter.read(cx).value().to_owned();
    let Some(tag) = console
        .store
        .filter(&query)
        .first()
        .and_then(|at| console.store.tags.get(*at))
        .map(|tag| tag.info.id)
    else {
        return;
    };
    if editor.single().is_some() {
        bind_selected(console, tag, window, cx);
        return;
    }
    let Some(drawn) = console.state::<super::State>().drawn.get() else {
        return;
    };
    let centre = drawn.bounds.center();
    if let Some((at, _)) = stage_at(console, centre) {
        super::open_menu(console, tag, at, centre, cx);
    }
    focus_stage(console, window, cx);
}

/// A tag dropped on the stage: bind the component under the pointer
/// when it takes a tag and is not locked; else (the empty stage) the
/// placement menu of what the tag can become (the web's `onDrop`).
pub(super) fn drop_tag(
    console: &mut Console,
    tag: &TagDrag,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let pointer = window.mouse_position();
    let Some(((x, y), _)) = stage_at(console, pointer) else {
        return;
    };
    let hit = edits_ref(console).and_then(|editor| {
        let layout = editor.layout();
        edit::hit_widget(layout, x, y)
            .map(|at| &layout.widgets[at])
            .filter(|widget| edit::max_tags(widget) > 0 && !edit::is_locked(widget))
            .map(|widget| widget.id.clone())
    });
    match hit {
        Some(id) => bind(console, &id, tag.id, window, cx),
        None => super::open_menu(console, tag.id, (x, y), pointer, cx),
    }
    focus_stage(console, window, cx);
}

pub(super) fn render(
    console: &mut Console,
    palette: AnyView,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let pane = editor.pane;
    let tabs = [
        (Pane::Project, "Project"),
        (Pane::Tags, "Tags"),
        (Pane::Components, "Components"),
    ]
    .into_iter()
    .map(|(at, label)| {
        Segment::new(
            label,
            pane == at,
            cx.listener(move |console, _, _, cx| {
                if let Some(editor) = edits(console) {
                    editor.pane = at;
                }
                cx.notify();
            }),
        )
    })
    .collect();
    let body = match pane {
        Pane::Project => project(console, &theme, cx),
        Pane::Tags => tag_browser(console, &theme, window, cx),
        Pane::Components => palette
            .cached(div().flex_1().min_h_0().size_full().style().clone())
            .into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(theme.surface)
        .border_r_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_none()
                .px(SPACE_4)
                .py(SPACE_3)
                .border_b_1()
                .border_color(theme.border)
                .child(segmented("designer-pane", tabs, cx)),
        )
        .child(div().flex_1().min_h_0().flex().flex_col().child(body))
        .into_any_element()
}

fn row_base(id: ElementId, selected: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let hover = theme.hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(SPACE_3)
        .px(SPACE_4)
        .h(px(26.0))
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .text_size(FONT_SMALL)
        .when(selected, |this| this.bg(theme.selected))
        .when(!selected, |this| this.hover(move |style| style.bg(hover)))
}

/// A small icon button in a tree row that applies `edit` (and does not
/// select the row).
fn toggle(
    id: ElementId,
    icon: Icon,
    tooltip: &'static str,
    color: gpui::Hsla,
    edit: Edit,
    cx: &mut Context<Console>,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .p(px(2.0))
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .child(vt_ui::icon(icon, px(12.0), color))
        .tooltip(move |_, cx| vt_ui::components::Tooltip::view(tooltip.into(), cx))
        .on_click(cx.listener(move |console, _, window, cx| {
            cx.stop_propagation();
            apply(console, &edit, None, window, cx);
        }))
        .into_any_element()
}

/// A row of the project tree.
#[derive(Clone)]
enum ProjectRow {
    /// A screen: its position, id, name, and whether it is the edited one.
    Screen(usize, String, String, bool),
    /// A component of the edited screen, by its place in the stack.
    Component(usize),
}

/// The project tree's row height (`row_base`).
const TREE_ROW: f32 = 26.0;

/// The project tree: the screens, the edited one opened. A virtual list:
/// only the rows in view are built each frame, so a screen of hundreds of
/// components costs what its visible rows cost.
fn project(console: &Console, theme: &Theme, cx: &mut Context<Console>) -> AnyElement {
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let screens = console.state_ref::<Screens>();
    let ids = screens.map(Screens::ids).unwrap_or_default();
    let mut rows: Vec<ProjectRow> = Vec::new();
    for (index, id) in ids.into_iter().enumerate() {
        let open = id == editor.screen;
        let name = screens.map_or_else(|| id.clone(), |s| s.name(&id));
        rows.push(ProjectRow::Screen(index, id, name, open));
        if open {
            // Top of the stack first, as the web Designer lists them.
            rows.extend(
                (0..editor.layout().widgets.len())
                    .rev()
                    .map(ProjectRow::Component),
            );
        }
    }
    let count = rows.len();
    let rows = std::rc::Rc::new(rows);
    let theme = theme.clone();
    div()
        .id("designer-project")
        .flex_1()
        .min_h_0()
        .p(SPACE_3)
        .child(
            uniform_list(
                "designer-project-rows",
                count,
                cx.processor(
                    move |console: &mut Console,
                          range: std::ops::Range<usize>,
                          _,
                          cx: &mut Context<Console>| {
                        range
                            .filter_map(|at| rows.get(at))
                            .map(|row| project_row(console, row, &theme, cx))
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .size_full(),
        )
        .into_any_element()
}

fn project_row(
    console: &Console,
    row: &ProjectRow,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let Some(editor) = edits_ref(console) else {
        return div().h(px(TREE_ROW)).into_any_element();
    };
    match row {
        ProjectRow::Screen(index, id, name, open) => {
            let open = *open;
            let target = id.clone();
            row_base(
                ElementId::NamedInteger(
                    "designer-screen".into(),
                    u64::try_from(*index).unwrap_or(0),
                ),
                false,
                theme,
            )
            .text_color(if open { theme.fg } else { theme.fg_muted })
            .child(vt_ui::icon(
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                px(12.0),
                theme.fg_dim,
            ))
            .child(vt_ui::icon(Icon::Layers, px(13.0), theme.fg_dim))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name.clone()),
            )
            .when(open, |this| {
                this.child(
                    div()
                        .text_size(FONT_CAPTION)
                        .font_family(MONO)
                        .text_color(theme.fg_dim)
                        .child(editor.layout().widgets.len().to_string()),
                )
            })
            .on_click(cx.listener(move |console, _, window, cx| {
                super::super::switch_screen(console, &target, window, cx);
            }))
            .into_any_element()
        }
        ProjectRow::Component(at) => {
            let Some(widget) = editor.layout().widgets.get(*at) else {
                return div().h(px(TREE_ROW)).into_any_element();
            };
            component_row(editor, *at, widget, theme, cx)
        }
    }
}

fn component_row(
    editor: &Editor,
    at: usize,
    widget: &voltrus_scada_native::mimic::Widget,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let selected = editor.selection.contains(&widget.id);
    let hidden = !designer::visible(widget);
    let locked = designer::locked(widget);
    let grouped = edit::group_of(widget).is_some();
    let bound = bindings::has_bindings(&widget.config);
    let label = widget
        .text("label")
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .or_else(|| widget.text("title").map(str::to_owned))
        .or_else(|| widget.text("text").map(str::to_owned))
        .unwrap_or_else(|| edit::title(widget));
    let id = widget.id.clone();
    row_base(
        ElementId::NamedInteger("designer-component".into(), u64::try_from(at).unwrap_or(0)),
        selected,
        theme,
    )
    .pl(px(34.0))
    .text_color(if hidden { theme.fg_dim } else { theme.fg })
    .child(
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(label),
    )
    .when(bound, |this| {
        this.child(
            div()
                .text_size(FONT_CAPTION)
                .text_color(theme.accent)
                .child("ƒ"),
        )
    })
    .when(grouped, |this| {
        this.child(vt_ui::icon(Icon::Boxes, px(12.0), theme.fg_dim))
    })
    // Hide / show and lock / unlock from the tree (`IconToggle`):
    // shown when set, or on the selected row.
    .when(hidden || selected, |this| {
        this.child(toggle(
            ElementId::NamedInteger(
                "designer-toggle-visible".into(),
                u64::try_from(at).unwrap_or(0),
            ),
            Icon::Eye,
            if hidden {
                "Show it"
            } else {
                "Hide it at runtime"
            },
            if hidden { theme.fg_dim } else { theme.fg_muted },
            Edit::Config {
                id: widget.id.clone(),
                key: "visible".into(),
                value: serde_json::Value::Bool(hidden),
            },
            cx,
        ))
    })
    .when(locked || selected, |this| {
        this.child(toggle(
            ElementId::NamedInteger(
                "designer-toggle-lock".into(),
                u64::try_from(at).unwrap_or(0),
            ),
            if locked { Icon::Lock } else { Icon::LockOpen },
            if locked { "Unlock" } else { "Lock" },
            if locked { theme.fg_dim } else { theme.fg_muted },
            Edit::Config {
                id: widget.id.clone(),
                key: edit::LOCKED_KEY.into(),
                value: if locked {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::Bool(true)
                },
            },
            cx,
        ))
    })
    .child(
        div()
            .text_size(FONT_CAPTION)
            .font_family(MONO)
            .text_color(theme.fg_dim)
            .child(widget.kind.as_str().to_owned()),
    )
    .on_click(cx.listener(move |console, _, window, cx| {
        if let Some(editor) = edits(console) {
            editor.selection = vec![id.clone()];
            editor.line = None;
        }
        super::changed(console, window, cx);
    }))
    .into_any_element()
}

/// The tag browser: live values, searched; click or drag to bind.
fn tag_browser(
    console: &Console,
    theme: &Theme,
    _: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let input = editor.tag_filter.clone();
    let query = input.read(cx).value().to_owned();
    let bound: Vec<i64> = editor
        .single()
        .map(|widget| widget.tags.clone())
        .unwrap_or_default();
    let store = &console.store;
    let found = store.filter(&query);
    let total = found.len();
    let found = std::rc::Rc::new(found);
    let bound = std::rc::Rc::new(bound);
    let row_theme = theme.clone();
    let list = uniform_list(
        "designer-tag-rows",
        total,
        cx.processor(
            move |console: &mut Console,
                  range: std::ops::Range<usize>,
                  _,
                  cx: &mut Context<Console>| {
                range
                    .filter_map(|row| found.get(row))
                    .filter_map(|at| console.store.tags.get(*at))
                    .map(|tag| tag_row(console, tag, &bound, &row_theme, cx))
                    .collect::<Vec<_>>()
            },
        ),
    )
    .size_full();
    let caption = format!("{total} tags, live · click or drag onto a component to bind");

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .p(SPACE_4)
                .border_b_1()
                .border_color(theme.border)
                .child(input)
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(caption),
                ),
        )
        .child(
            div()
                .id("designer-tags")
                .flex_1()
                .min_h_0()
                .p(SPACE_3)
                .child(list),
        )
        .into_any_element()
}

fn tag_row(
    console: &Console,
    tag: &crate::console::store::Tag,
    bound: &[i64],
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let store = &console.store;

    let id = tag.info.id;
    // The numbered ref (`device/key#2`) when tags share a key, as
    // bindings name them.
    let reference = store.ref_of(id).map_or_else(
        || bindings::tag_ref(&tag.info.device_id, &tag.info.key),
        str::to_owned,
    );
    let value = if tag.at_secs == 0 && !tag.simulated {
        "—".to_owned()
    } else {
        tag.display()
    };
    let attention = tag.status.needs_attention();
    let drag = TagDrag {
        id,
        name: tag.label.clone(),
    };
    row_base(
        ElementId::NamedInteger("designer-tag".into(), u64::try_from(id).unwrap_or(0)),
        bound.contains(&id),
        theme,
    )
    .h(px(36.0))
    .cursor_grab()
    .child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .text_color(theme.fg)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(tag.label.clone()),
            )
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .font_family(MONO)
                    .text_color(theme.fg_dim)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(reference),
            ),
    )
    .child(
        div()
            .flex_none()
            .font_family(MONO)
            .text_color(if attention { theme.warning } else { theme.fg })
            .child(value),
    )
    .child(
        div()
            .flex_none()
            .w(px(34.0))
            .text_size(FONT_CAPTION)
            .text_color(theme.fg_dim)
            .overflow_hidden()
            .whitespace_nowrap()
            .child(tag.info.unit.clone()),
    )
    .on_click(cx.listener(move |console, _, window, cx| {
        bind_selected(console, id, window, cx);
    }))
    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
    .into_any_element()
}

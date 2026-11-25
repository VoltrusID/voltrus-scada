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
use gpui::{AnyElement, AnyView, Context, ElementId, Render, SharedString, Window, div, px};
use voltrus_scada_native::designer::{self, bindings};
use voltrus_scada_native::mimic::edit::{self, Edit};
use vt_ui::Icon;
use vt_ui::components::{Segment, segmented};
use vt_ui::theme::*;

use super::{Editor, apply, edits, edits_ref, focus_stage, stage_at};
use crate::console::Console;
use crate::console::screens::Screens;

/// Tags the browser lists at most (the search narrows them).
const TAG_ROWS: usize = 400;

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

/// A tag dropped on the stage: bind the component under the pointer.
pub(super) fn drop_tag(
    console: &mut Console,
    tag: &TagDrag,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(((x, y), _)) = stage_at(console, window.mouse_position()) else {
        return;
    };
    let hit = edits_ref(console).and_then(|editor| {
        edit::hit_widget(editor.layout(), x, y).map(|at| editor.layout().widgets[at].id.clone())
    });
    match hit {
        Some(id) => bind(console, &id, tag.id, window, cx),
        None => console.show_toast("Drop the tag on a component to bind it", false, cx),
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

/// The project tree: the screens, the edited one opened.
fn project(console: &Console, theme: &Theme, cx: &mut Context<Console>) -> AnyElement {
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let screens = console.state_ref::<Screens>();
    let ids = screens.map(Screens::ids).unwrap_or_default();
    let mut rows: Vec<AnyElement> = Vec::new();
    for (index, id) in ids.into_iter().enumerate() {
        let open = id == editor.screen;
        let name = screens.map_or_else(|| id.clone(), |s| s.name(&id));
        let target = id.clone();
        rows.push(
            row_base(
                ElementId::NamedInteger(
                    "designer-screen".into(),
                    u64::try_from(index).unwrap_or(0),
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
                    .child(name),
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
            .into_any_element(),
        );
        if !open {
            continue;
        }
        // Top of the stack first, as the web Designer lists them.
        for (at, widget) in editor.layout().widgets.iter().enumerate().rev() {
            let selected = editor.selection.contains(&widget.id);
            let hidden = !designer::visible(widget);
            let locked = designer::locked(widget);
            let bound = bindings::has_bindings(&widget.config);
            let label = widget
                .text("label")
                .filter(|label| !label.is_empty())
                .map(str::to_owned)
                .or_else(|| widget.text("title").map(str::to_owned))
                .or_else(|| widget.text("text").map(str::to_owned))
                .unwrap_or_else(|| edit::title(widget));
            let id = widget.id.clone();
            rows.push(
                row_base(
                    ElementId::NamedInteger(
                        "designer-component".into(),
                        u64::try_from(at).unwrap_or(0),
                    ),
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
                .when(hidden, |this| {
                    this.child(vt_ui::icon(Icon::Eye, px(12.0), theme.fg_dim))
                })
                .when(locked, |this| {
                    this.child(vt_ui::icon(Icon::Lock, px(12.0), theme.fg_dim))
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
                .into_any_element(),
            );
        }
    }
    div()
        .id("designer-project")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .p(SPACE_3)
        .children(rows)
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
    let rows: Vec<AnyElement> = found
        .iter()
        .take(TAG_ROWS)
        .filter_map(|at| store.tags.get(*at))
        .map(|tag| {
            let id = tag.info.id;
            let reference = bindings::tag_ref(&tag.info.device_id, &tag.info.key);
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
        })
        .collect();
    let caption = if total > TAG_ROWS {
        format!("{TAG_ROWS} of {total} tags, live · search to narrow")
    } else {
        format!("{total} tags, live · click or drag onto a component to bind")
    };
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
                .overflow_y_scroll()
                .p(SPACE_3)
                .children(rows),
        )
        .into_any_element()
}

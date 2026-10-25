//! The inspector (`ScadaInspector.tsx`): for one widget its tag binding
//! (searched in the store), position and size, its type's settings
//! (`configSchema`, `symbolSchema`) and arrange; for several, arrange; for
//! a connection its type; with nothing selected, the screen's summary and
//! the editor's keys. Text settings apply as typed (one undo step per
//! field), numbers on Enter or leaving the field.

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Focusable, FontWeight, Hsla, SharedString, Window, div, px,
};
use serde_json::Value;
use voltrus_scada_native::mimic::edit::{self, Edit, Field, FieldKind};
use voltrus_scada_native::mimic::{self, Widget, WidgetKind};
use vt_ui::Icon;
use vt_ui::components::{Button, kbd, section_label};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::{Cmd, Pick, Slot, Tool, apply, edits, edits_ref, focus_stage, run};
use crate::console::Console;
use crate::console::keys::display;

/// Tags the search lists at most.
const TAG_ROWS: usize = 40;

// ── The fields ────────────────────────────────────────────────────────────

/// The setting a field shows, as text.
fn text_of(widget: &Widget, slot: Slot) -> String {
    let number = |value: f64| edit::value_text(&Value::from(value));
    match slot {
        Slot::X => number(widget.x),
        Slot::Y => number(widget.y),
        Slot::Width => number(widget.width),
        Slot::Height => number(widget.height),
        Slot::Config(key) => field_of(widget, key)
            .map(|field| edit::value_text(&edit::field_value(widget, &field)))
            .unwrap_or_default(),
    }
}

fn field_of(widget: &Widget, key: &str) -> Option<Field> {
    edit::schema(widget)
        .into_iter()
        .find(|field| field.key == key)
}

/// Make the fields for the one selected widget, and show its settings in
/// the fields not being typed in.
pub(super) fn sync(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = edits_ref(console) else {
        return;
    };
    let widget = editor.single().cloned();
    let made_for = editor.fields_for.clone();
    match widget {
        None => {
            if made_for.is_some()
                && let Some(editor) = edits(console)
            {
                editor.fields.clear();
                editor._field_subscriptions.clear();
                editor.fields_for = None;
                editor.picking = None;
            }
        }
        Some(widget) if made_for.as_deref() != Some(widget.id.as_str()) => {
            make_fields(console, &widget, window, cx);
        }
        Some(widget) => {
            let fields = edits_ref(console)
                .map(|editor| editor.fields.clone())
                .unwrap_or_default();
            for (slot, field) in fields {
                let focused = field.read(cx).focus_handle(cx).is_focused(window);
                let text = text_of(&widget, slot);
                if !focused && field.read(cx).value() != text {
                    field.update(cx, |field, cx| field.set_text(text, cx));
                }
            }
        }
    }
}

fn make_fields(
    console: &mut Console,
    widget: &Widget,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let mut slots = vec![Slot::X, Slot::Y, Slot::Width, Slot::Height];
    slots.extend(
        edit::schema(widget)
            .into_iter()
            .filter(|field| {
                matches!(
                    field.kind,
                    FieldKind::Text | FieldKind::Color | FieldKind::Number { .. }
                )
            })
            .map(|field| Slot::Config(field.key)),
    );
    let mut fields = Vec::with_capacity(slots.len());
    let mut subscriptions = Vec::with_capacity(slots.len());
    for slot in slots {
        let numeric = match slot {
            Slot::Config(key) => field_of(widget, key)
                .is_some_and(|field| matches!(field.kind, FieldKind::Number { .. })),
            _ => true,
        };
        let text = text_of(widget, slot);
        let field = cx.new(|cx| {
            let field = TextInput::new(cx).text(text);
            if numeric {
                field.kind(InputKind::Number).mono()
            } else {
                field
            }
        });
        subscriptions.push(cx.subscribe_in(
            &field,
            window,
            move |console, _, event: &InputEvent, window, cx| {
                on_field(console, slot, numeric, event, window, cx);
            },
        ));
        fields.push((slot, field));
    }
    if let Some(editor) = edits(console) {
        editor.fields = fields;
        editor._field_subscriptions = subscriptions;
        editor.fields_for = Some(widget.id.clone());
        editor.picking = None;
    }
}

fn on_field(
    console: &mut Console,
    slot: Slot,
    numeric: bool,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(editor) = edits_ref(console) else {
        return;
    };
    let Some(widget) = editor.single().cloned() else {
        return;
    };
    let Some(field) = editor
        .fields
        .iter()
        .find(|(at, _)| *at == slot)
        .map(|(_, field)| field.clone())
    else {
        return;
    };
    let text = field.read(cx).value().to_owned();
    match event {
        InputEvent::Changed if !numeric => {
            if let Slot::Config(key) = slot {
                let typed = Edit::Config {
                    id: widget.id.clone(),
                    key: key.to_owned(),
                    value: Value::from(text),
                };
                let coalesce = format!("{}:{key}", widget.id);
                apply(console, &typed, Some(coalesce.as_str()), window, cx);
            }
        }
        InputEvent::Submitted | InputEvent::Blurred if numeric => {
            match commit_number(&widget, slot, &text) {
                Some(typed) => apply(console, &typed, None, window, cx),
                None => field.update(cx, |field, cx| field.set_text(text_of(&widget, slot), cx)),
            }
        }
        InputEvent::Cancelled => {
            field.update(cx, |field, cx| field.set_text(text_of(&widget, slot), cx));
            focus_stage(console, window, cx);
        }
        _ => {}
    }
}

/// A typed number as an edit: a box value (whole, x and y at least 0,
/// sizes at least 10, as the web's inspector), or a setting in its range.
fn commit_number(widget: &Widget, slot: Slot, text: &str) -> Option<Edit> {
    let kind = match slot {
        Slot::Config(key) => field_of(widget, key)?.kind,
        _ => FieldKind::Number {
            min: None,
            max: None,
        },
    };
    let value = edit::typed_number(text, kind)?;
    let boxed = |x: f64, y: f64, width: f64, height: f64| Edit::Resize {
        id: widget.id.clone(),
        x,
        y,
        width,
        height,
    };
    let (x, y, w, h) = (widget.x, widget.y, widget.width, widget.height);
    Some(match slot {
        Slot::X => boxed(value.round().max(0.0), y, w, h),
        Slot::Y => boxed(x, value.round().max(0.0), w, h),
        Slot::Width => boxed(x, y, value.round().max(10.0), h),
        Slot::Height => boxed(x, y, w, value.round().max(10.0)),
        Slot::Config(key) => Edit::Config {
            id: widget.id.clone(),
            key: key.to_owned(),
            value: edit::number_value(value),
        },
    })
}

// ── Tags ──────────────────────────────────────────────────────────────────

/// The tags the search offers: positions in the store.
fn found(console: &Console, cx: &Context<Console>) -> Vec<usize> {
    let Some(editor) = edits_ref(console) else {
        return Vec::new();
    };
    let query = editor.tag_query.read(cx).value().to_owned();
    console
        .store
        .filter(&query)
        .iter()
        .copied()
        .take(TAG_ROWS)
        .collect()
}

fn start_picking(
    console: &mut Console,
    pick: Pick,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(editor) = edits(console) else {
        return;
    };
    editor.picking = Some(pick);
    let query = editor.tag_query.clone();
    query.update(cx, |field, cx| field.set_text("", cx));
    query.read(cx).focus(window);
    cx.notify();
}

/// Bind `tag` where the search was opened for.
fn pick(console: &mut Console, tag: i64, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = edits(console) else {
        return;
    };
    let Some(pick) = editor.picking.take() else {
        return;
    };
    let Some(widget) = editor.single().cloned() else {
        return;
    };
    let bound = match pick {
        Pick::Primary => Edit::Bind {
            id: widget.id,
            tags: vec![tag],
        },
        Pick::Add => {
            let mut tags = widget.tags.clone();
            if !tags.contains(&tag) {
                tags.push(tag);
            }
            Edit::Bind {
                id: widget.id,
                tags,
            }
        }
        Pick::Config(key) => Edit::Config {
            id: widget.id,
            key: key.to_owned(),
            value: Value::from(tag),
        },
    };
    apply(console, &bound, None, window, cx);
    focus_stage(console, window, cx);
}

/// Enter in the tag search: the first match.
pub(super) fn pick_first(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let first = found(console, cx)
        .first()
        .and_then(|at| console.store.tags.get(*at))
        .map(|tag| tag.info.id);
    if let Some(tag) = first {
        pick(console, tag, window, cx);
    }
}

fn unbind(console: &mut Console, unbound: &Edit, window: &mut Window, cx: &mut Context<Console>) {
    if let Some(editor) = edits(console) {
        editor.picking = None;
    }
    apply(console, unbound, None, window, cx);
}

// ── Drawing ───────────────────────────────────────────────────────────────

pub(super) fn render(
    console: &mut Console,
    _: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let layout = editor.layout();
    let content: Vec<AnyElement> = if let Some(widget) = editor.single() {
        widget_panel(console, widget, &theme, cx)
    } else if editor.selection.len() > 1 {
        many_panel(editor.selection.len(), &theme, cx)
    } else if let Some(line) = editor
        .line
        .as_ref()
        .and_then(|id| layout.connections.iter().find(|line| line.id == *id))
    {
        line_panel(layout, line, &theme, cx)
    } else {
        screen_panel(layout, editor.tool, &theme, cx)
    };
    div()
        .id("mimic-inspector")
        .flex()
        .flex_col()
        .size_full()
        .overflow_y_scroll()
        .bg(theme.surface)
        .border_l_1()
        .border_color(theme.border)
        .children(content)
        .into_any_element()
}

fn header(title: String, subtitle: String, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .px(SPACE_6)
        .py(SPACE_5)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg)
                .child(title),
        )
        .child(
            div()
                .mt(SPACE_1)
                .text_size(FONT_CAPTION)
                .font_family(MONO)
                .text_color(theme.fg_dim)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(subtitle),
        )
        .into_any_element()
}

fn section(
    title: &'static str,
    body: Vec<AnyElement>,
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .px(SPACE_6)
        .pt(SPACE_5)
        .pb(SPACE_6)
        .border_b_1()
        .border_color(theme.border)
        .child(section_label(title, cx))
        .children(body)
        .into_any_element()
}

fn caption(text: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    div()
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_dim)
        .child(text.into())
        .into_any_element()
}

fn labelled(label: &'static str, control: impl IntoElement, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_1)
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(label),
        )
        .child(control)
        .into_any_element()
}

fn button(
    id: &'static str,
    label: &'static str,
    keys: &'static str,
    cmd: Cmd,
    cx: &mut Context<Console>,
) -> AnyElement {
    let button = Button::new(id, label)
        .compact()
        .tooltip(display(keys))
        .on_click(cx.listener(move |console, _, window, cx| run(console, cmd, window, cx)));
    if cmd == Cmd::Delete {
        button.danger().into_any_element()
    } else {
        button.into_any_element()
    }
}

fn arrange(turns: bool, cx: &mut Context<Console>) -> Vec<AnyElement> {
    let mut buttons = vec![
        button(
            "mimic-insp-duplicate",
            "Duplicate",
            "secondary-d",
            Cmd::Duplicate,
            cx,
        ),
        button("mimic-insp-front", "To front", "]", Cmd::Front, cx),
        button("mimic-insp-back", "To back", "[", Cmd::Back, cx),
    ];
    if turns {
        buttons.push(button(
            "mimic-insp-rotate",
            "Rotate 90°",
            "r",
            Cmd::Rotate(1),
            cx,
        ));
        buttons.push(button("mimic-insp-mirror", "Mirror", "h", Cmd::Mirror, cx));
    }
    buttons.push(button(
        "mimic-insp-delete",
        "Delete",
        "backspace",
        Cmd::Delete,
        cx,
    ));
    vec![
        div()
            .flex()
            .flex_wrap()
            .gap(SPACE_2)
            .children(buttons)
            .into_any_element(),
    ]
}

/// A tag's name and key, or the placeholder.
fn tag_line(console: &Console, tag: Option<i64>, empty: &'static str, theme: &Theme) -> AnyElement {
    let known = tag.and_then(|id| console.store.get(id));
    match (tag, known) {
        (Some(_), Some(known)) => div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(
                div()
                    .text_size(FONT_BODY)
                    .text_color(theme.fg)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(known.label.clone()),
            )
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .font_family(MONO)
                    .text_color(theme.fg_dim)
                    .child(known.key_label.clone()),
            )
            .into_any_element(),
        (Some(id), None) => caption(format!("Tag #{id} (not in this plant)"), theme),
        (None, _) => caption(empty, theme),
    }
}

/// A bound tag with its Bind (Change) and clear buttons.
#[allow(clippy::too_many_arguments)]
fn tag_slot(
    console: &Console,
    tag: Option<i64>,
    empty: &'static str,
    id: &'static str,
    pick_as: Pick,
    clear: Option<Edit>,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let change = Button::new(
        SharedString::from(format!("mimic-insp-bind-{id}")),
        if tag.is_some() { "Change" } else { "Bind" },
    )
    .compact()
    .on_click(cx.listener(move |console, _, window, cx| {
        start_picking(console, pick_as, window, cx);
    }));
    let clear = clear.filter(|_| tag.is_some()).map(|clear| {
        Button::new(SharedString::from(format!("mimic-insp-clear-{id}")), "×")
            .compact()
            .ghost()
            .tooltip("Unbind")
            .on_click(cx.listener(move |console, _, window, cx| {
                unbind(console, &clear, window, cx);
            }))
    });
    div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(tag_line(console, tag, empty, theme)),
        )
        .child(change)
        .children(clear)
        .into_any_element()
}

/// The tag search, under the slot it binds.
fn picker(console: &Console, theme: &Theme, cx: &mut Context<Console>) -> AnyElement {
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let input = editor.tag_query.clone();
    let hover = theme.hover;
    let mut rows: Vec<AnyElement> = Vec::new();
    for at in found(console, cx) {
        let Some(tag) = console.store.tags.get(at) else {
            continue;
        };
        let id = tag.info.id;
        rows.push(
            div()
                .id(ElementId::NamedInteger(
                    "mimic-insp-tag".into(),
                    u64::try_from(id).unwrap_or(0),
                ))
                .flex()
                .flex_col()
                .px(SPACE_3)
                .py(SPACE_2)
                .rounded(RADIUS_SM)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |console, _, window, cx| pick(console, id, window, cx)))
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(tag.label.clone()),
                )
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .font_family(MONO)
                        .text_color(theme.fg_dim)
                        .child(tag.key_label.clone()),
                )
                .into_any_element(),
        );
    }
    let empty = rows.is_empty();
    div()
        .flex()
        .flex_col()
        .gap(SPACE_2)
        .p(SPACE_3)
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(theme.accent)
        .child(input)
        .child(
            div()
                .id("mimic-insp-tags")
                .flex()
                .flex_col()
                .max_h(px(240.0))
                .overflow_y_scroll()
                .children(rows),
        )
        .when(empty, |this| this.child(caption("No tag matches.", theme)))
        .into_any_element()
}

/// The setting edit for `key` of widget `id`.
fn set(id: &str, key: &'static str, value: Value) -> Edit {
    Edit::Config {
        id: id.to_owned(),
        key: key.to_owned(),
        value,
    }
}

/// A field's control: a switch, choices, a text or number field, a colour
/// field with the theme's swatches, or a tag slot.
fn control(
    console: &Console,
    widget: &Widget,
    field: &Field,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let value = edit::field_value(widget, field);
    let key = field.key;
    let input = edits_ref(console).and_then(|editor| {
        editor
            .fields
            .iter()
            .find(|(slot, _)| *slot == Slot::Config(key))
            .map(|(_, field)| field.clone())
    });
    match field.kind {
        FieldKind::Flag => {
            let on = edit::value_flag(&value);
            let flipped = set(&widget.id, key, Value::Bool(!on));
            div()
                .id(SharedString::from(format!("mimic-insp-flag-{key}")))
                .flex()
                .items_center()
                .justify_between()
                .gap(SPACE_4)
                .cursor_pointer()
                .on_click(cx.listener(move |console, _, window, cx| {
                    apply(console, &flipped, None, window, cx);
                }))
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(field.label),
                )
                .child(switch(on, theme.accent, theme))
                .into_any_element()
        }
        FieldKind::Select(options) => {
            let current = edit::value_text(&value);
            let hover = theme.hover;
            let mut chips: Vec<AnyElement> = Vec::with_capacity(options.len());
            for &(option, label) in options {
                let selected = current == option;
                let chosen = set(&widget.id, key, Value::from(option));
                chips.push(
                    div()
                        .id(SharedString::from(format!("mimic-insp-{key}-{option}")))
                        .px(SPACE_3)
                        .h(CONTROL_SM)
                        .flex()
                        .items_center()
                        .rounded(RADIUS_SM)
                        .border_1()
                        .border_color(if selected { theme.accent } else { theme.border })
                        .when(selected, |this| this.bg(theme.accent_soft))
                        .when(!selected, |this| this.hover(move |style| style.bg(hover)))
                        .cursor_pointer()
                        .text_size(FONT_SMALL)
                        .text_color(if selected { theme.fg } else { theme.fg_muted })
                        .on_click(cx.listener(move |console, _, window, cx| {
                            apply(console, &chosen, None, window, cx);
                        }))
                        .child(label)
                        .into_any_element(),
                );
            }
            labelled(
                field.label,
                div().flex().flex_wrap().gap(SPACE_2).children(chips),
                theme,
            )
        }
        FieldKind::Tag => {
            let tag = edit::value_tag(&value);
            let clear = set(&widget.id, key, Value::Null);
            let picking =
                edits_ref(console).and_then(|editor| editor.picking) == Some(Pick::Config(key));
            let slot = tag_slot(
                console,
                tag,
                "— None —",
                key,
                Pick::Config(key),
                Some(clear),
                theme,
                cx,
            );
            let search = picking.then(|| picker(console, theme, cx));
            labelled(
                field.label,
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .child(slot)
                    .children(search),
                theme,
            )
        }
        FieldKind::Color => {
            let mut swatches: Vec<AnyElement> = Vec::with_capacity(edit::COLORS.len());
            for color in edit::COLORS {
                let selected = value.as_str() == Some(color);
                let chosen = set(&widget.id, key, Value::from(color));
                swatches.push(
                    div()
                        .id(SharedString::from(format!("mimic-insp-{key}-{color}")))
                        .size(px(18.0))
                        .rounded(RADIUS_SM)
                        .border_1()
                        .border_color(if selected { theme.fg } else { theme.border })
                        .bg(swatch(color, theme))
                        .cursor_pointer()
                        .on_click(cx.listener(move |console, _, window, cx| {
                            apply(console, &chosen, None, window, cx);
                        }))
                        .into_any_element(),
                );
            }
            labelled(
                field.label,
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .child(div().flex().gap(SPACE_2).children(swatches))
                    .children(input),
                theme,
            )
        }
        FieldKind::Text | FieldKind::Number { .. } => {
            labelled(field.label, div().children(input), theme)
        }
    }
}

/// The colour a theme variable stands for, for its swatch.
fn swatch(setting: &str, theme: &Theme) -> Hsla {
    match setting {
        "var(--ok)" | "var(--hmi-run)" => theme.success,
        "var(--cyan)" => theme.accent,
        "var(--warn)" => theme.warning,
        "var(--crit)" => theme.danger,
        _ => theme.fg_dim,
    }
}

fn switch(on: bool, accent: Hsla, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .w(px(28.0))
        .h(px(16.0))
        .rounded(px(8.0))
        .p(px(2.0))
        .flex()
        .when(on, |this| this.justify_end().bg(accent))
        .when(!on, |this| this.bg(theme.border))
        .child(div().size(px(12.0)).rounded(px(6.0)).bg(theme.bg))
        .into_any_element()
}

fn widget_panel(
    console: &Console,
    widget: &Widget,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> Vec<AnyElement> {
    let Some(editor) = edits_ref(console) else {
        return Vec::new();
    };
    let max_tags = edit::max_tags(widget);
    let resolved = mimic::resolve(widget);
    let picking = editor.picking;
    let mut binding: Vec<AnyElement> = Vec::new();
    match max_tags {
        0 => binding.push(caption("This widget shows no tag.", theme)),
        1 => {
            let empty = match resolved {
                Some(resolved) if resolved.def.binding == mimic::Binding::None => {
                    "— Static (no tag) —"
                }
                _ => "— Not bound —",
            };
            binding.push(tag_slot(
                console,
                widget.tag(),
                empty,
                "primary",
                Pick::Primary,
                Some(Edit::Bind {
                    id: widget.id.clone(),
                    tags: Vec::new(),
                }),
                theme,
                cx,
            ));
            if picking == Some(Pick::Primary) {
                binding.push(picker(console, theme, cx));
            }
        }
        _ => {
            for (index, tag) in widget.tags.iter().enumerate() {
                let mut rest = widget.tags.clone();
                rest.remove(index);
                let clear = Edit::Bind {
                    id: widget.id.clone(),
                    tags: rest,
                };
                binding.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(div().flex_1().min_w_0().child(tag_line(
                            console,
                            Some(*tag),
                            "",
                            theme,
                        )))
                        .child(
                            Button::new(
                                ElementId::NamedInteger(
                                    "mimic-insp-untag".into(),
                                    u64::try_from(index).unwrap_or(0),
                                ),
                                "×",
                            )
                            .compact()
                            .ghost()
                            .tooltip("Remove the tag")
                            .on_click(cx.listener(
                                move |console, _, window, cx| {
                                    unbind(console, &clear, window, cx);
                                },
                            )),
                        )
                        .into_any_element(),
                );
            }
            if widget.tags.is_empty() {
                binding.push(caption("No tags: it shows the plant's first ones.", theme));
            }
            binding.push(
                Button::new("mimic-insp-add-tag", "Add a tag")
                    .icon(Icon::Plus)
                    .compact()
                    .on_click(cx.listener(|console, _, window, cx| {
                        start_picking(console, Pick::Add, window, cx);
                    }))
                    .into_any_element(),
            );
            if picking == Some(Pick::Add) {
                binding.push(picker(console, theme, cx));
            }
        }
    }
    if let Some(resolved) = resolved {
        binding.push(caption(edit::binding_hint(resolved.def.binding), theme));
    }

    let box_field = |slot: Slot, label: &'static str| {
        let input = editor
            .fields
            .iter()
            .find(|(at, _)| *at == slot)
            .map(|(_, field)| field.clone());
        div()
            .flex_1()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .child(
                div()
                    .w(px(14.0))
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(label),
            )
            .child(div().flex_1().min_w_0().children(input))
            .into_any_element()
    };
    let position = vec![
        div()
            .flex()
            .gap(SPACE_4)
            .child(box_field(Slot::X, "X"))
            .child(box_field(Slot::Y, "Y"))
            .into_any_element(),
        div()
            .flex()
            .gap(SPACE_4)
            .child(box_field(Slot::Width, "W"))
            .child(box_field(Slot::Height, "H"))
            .into_any_element(),
    ];

    let schema = edit::schema(widget);
    let mut settings: Vec<AnyElement> = Vec::with_capacity(schema.len());
    for field in &schema {
        settings.push(control(console, widget, field, theme, cx));
    }

    let mut panel = vec![
        header(edit::title(widget), widget.id.clone(), theme),
        section("Tag binding", binding, theme, cx),
        section("Position & size", position, theme, cx),
    ];
    if !settings.is_empty() {
        panel.push(section("Appearance & behaviour", settings, theme, cx));
    }
    let buttons = arrange(edit::turns(widget), cx);
    panel.push(section("Arrange", buttons, theme, cx));
    if matches!(widget.kind, WidgetKind::Other(_)) {
        panel.push(section(
            "Unknown type",
            vec![caption(
                "This console does not draw this widget type; saving keeps it as it is.",
                theme,
            )],
            theme,
            cx,
        ));
    }
    panel
}

fn many_panel(count: usize, theme: &Theme, cx: &mut Context<Console>) -> Vec<AnyElement> {
    let buttons = arrange(true, cx);
    vec![
        header(
            format!("{count} widgets"),
            "⇧-click adds or removes one".to_owned(),
            theme,
        ),
        section("Arrange", buttons, theme, cx),
    ]
}

fn widget_name(layout: &mimic::Layout, id: &str) -> String {
    layout.widget(id).map_or_else(
        || id.to_owned(),
        |widget| {
            widget
                .text("label")
                .filter(|label| !label.is_empty())
                .map_or_else(|| edit::title(widget), str::to_owned)
        },
    )
}

fn line_panel(
    layout: &mimic::Layout,
    line: &mimic::Connection,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> Vec<AnyElement> {
    let hover = theme.hover;
    let mut chips: Vec<AnyElement> = Vec::with_capacity(edit::LINE_TYPES.len());
    for (kind, label) in edit::LINE_TYPES {
        let selected = line.kind == kind;
        let retyped = Edit::Line {
            id: line.id.clone(),
            kind: kind.to_owned(),
        };
        chips.push(
            div()
                .id(SharedString::from(format!("mimic-insp-line-{kind}")))
                .px(SPACE_3)
                .h(CONTROL_SM)
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(if selected { theme.accent } else { theme.border })
                .when(selected, |this| this.bg(theme.accent_soft))
                .when(!selected, |this| this.hover(move |style| style.bg(hover)))
                .cursor_pointer()
                .text_size(FONT_SMALL)
                .on_click(cx.listener(move |console, _, window, cx| {
                    apply(console, &retyped, None, window, cx);
                }))
                .child(label)
                .into_any_element(),
        );
    }
    vec![
        header(
            "Connection".to_owned(),
            format!(
                "{} → {}",
                widget_name(layout, &line.from),
                widget_name(layout, &line.to)
            ),
            theme,
        ),
        section(
            "Type",
            vec![
                div()
                    .flex()
                    .flex_wrap()
                    .gap(SPACE_2)
                    .children(chips)
                    .into_any_element(),
                caption(
                    "Process pipes animate flow while the equipment at either end runs.",
                    theme,
                ),
            ],
            theme,
            cx,
        ),
        section(
            "Arrange",
            vec![button(
                "mimic-insp-line-delete",
                "Delete connection",
                "backspace",
                Cmd::Delete,
                cx,
            )],
            theme,
            cx,
        ),
    ]
}

/// The editor's keys, as the screen inspector lists them (`SHORTCUTS`).
const SHORTCUTS: [(&str, &str); 10] = [
    ("V · C", "Select / connect tool"),
    ("⌘Z · ⇧⌘Z", "Undo / redo"),
    ("⌘C · ⌘V · ⌘D", "Copy / paste / duplicate"),
    ("Arrows · ⇧Arrows", "Nudge 1 / 10 px"),
    ("[ · ]", "Send to back / front"),
    ("R · ⇧R · H", "Turn / turn back / mirror"),
    ("⌫", "Delete the selection"),
    ("⌘A · Tab", "Select all / the next"),
    ("⌘+ · ⌘− · ⌘0", "Zoom in / out / fit"),
    ("⌘S · E", "Save / done"),
];

fn screen_panel(
    layout: &mimic::Layout,
    tool: Tool,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> Vec<AnyElement> {
    let (widgets, lines, tags) = edit::summary(layout);
    let build = if tool == Tool::Connect {
        "Click the source equipment, then the target. V goes back to selecting."
    } else {
        "Drag equipment or widgets from the palette. Select an item to bind tags and set how it looks; the connect tool (C) pipes nozzles together. ⌥-drag draws a selection band anywhere."
    };
    let keys: Vec<AnyElement> = SHORTCUTS
        .iter()
        .map(|(keys, what)| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(SPACE_4)
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(*what),
                )
                .child(kbd(*keys, cx))
                .into_any_element()
        })
        .collect();
    vec![
        header(
            "Screen".to_owned(),
            format!("{widgets} items · {lines} connections · {tags} tags bound"),
            theme,
        ),
        section("Build", vec![caption(build, theme)], theme, cx),
        section("Shortcuts", keys, theme, cx),
    ]
}

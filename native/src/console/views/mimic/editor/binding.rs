//! The property editor's binding editor (`BindingEditor` in the web's
//! `PropertyEditor.tsx`): any bindable property of the selected component
//! (everything but a tag picker) follows a tag, an expression or a
//! state-to-value map. The draft is previewed live against the store; Apply
//! writes it as one undo step, Remove unbinds the property.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, Entity, SharedString, Subscription, Window, div, px};
use serde_json::Value as Json;
use voltrus_scada_native::designer::{self, Binding, Prop, PropType, bindings, expr};
use voltrus_scada_native::mimic::Widget;
use vt_ui::components::{Button, Segment, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::{Pick, apply, edits, edits_ref, focus_stage};
use crate::console::Console;

/// What a draft binds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Tag,
    Expr,
    Map,
}

/// One map case being edited.
pub(crate) struct CaseRow {
    op: &'static str,
    v: Entity<TextInput>,
    out: Entity<TextInput>,
    /// The result of a boolean property.
    flag: bool,
}

/// The binding editor, open on one property of one component.
pub(crate) struct BindEditor {
    pub widget: String,
    pub prop: &'static str,
    kind: PropType,
    /// The property was bound when the editor opened (Remove is offered).
    bound: bool,
    pub mode: Mode,
    /// The tag ref of a tag binding.
    pub tag: String,
    pub expr: Entity<TextInput>,
    cases: Vec<CaseRow>,
    otherwise: Entity<TextInput>,
    /// "Otherwise" of a boolean property: none, on or off.
    otherwise_flag: Option<bool>,
    _subscriptions: Vec<Subscription>,
}

fn json_text(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => String::new(),
        Some(Json::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn number_text(v: f64) -> String {
    if v.is_finite() {
        expr::js_number(v)
    } else {
        String::new()
    }
}

/// A typed result as the property takes it: a number for a number
/// property (empty or not a number: none, the fixed value stays), text
/// for the rest.
fn typed_out(text: &str, kind: PropType) -> Option<Json> {
    match kind {
        PropType::Number => {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            text.parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Json::Number)
        }
        _ => Some(Json::String(text.to_owned())),
    }
}

fn field(
    text: String,
    placeholder: &'static str,
    numeric: bool,
    cx: &mut Context<Console>,
) -> Entity<TextInput> {
    cx.new(|cx| {
        let field = TextInput::new(cx).text(text).placeholder(placeholder);
        if numeric {
            field.kind(InputKind::Number).mono()
        } else {
            field.mono()
        }
    })
}

fn listen(
    input: &Entity<TextInput>,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> Subscription {
    cx.subscribe_in(
        input,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Cancelled => close(console, window, cx),
            _ => cx.notify(),
        },
    )
}

impl BindEditor {
    /// The draft as a binding.
    pub fn draft(&self, cx: &gpui::App) -> Binding {
        let expr = self.expr.read(cx).value().to_owned();
        match self.mode {
            Mode::Tag => Binding::Tag {
                tag: self.tag.clone(),
            },
            Mode::Expr => Binding::Expr { expr },
            Mode::Map => Binding::Map {
                expr,
                cases: self
                    .cases
                    .iter()
                    .map(|case| bindings::MapCase {
                        op: case.op.to_owned(),
                        v: case
                            .v
                            .read(cx)
                            .value()
                            .trim()
                            .parse::<f64>()
                            .unwrap_or(f64::NAN),
                        out: if self.kind == PropType::Boolean {
                            Some(Json::Bool(case.flag))
                        } else {
                            typed_out(case.out.read(cx).value(), self.kind)
                        },
                    })
                    .collect(),
                otherwise: if self.kind == PropType::Boolean {
                    self.otherwise_flag.map(Json::Bool)
                } else {
                    let text = self.otherwise.read(cx).value();
                    if text.is_empty() {
                        None
                    } else {
                        typed_out(text, self.kind)
                    }
                },
            },
        }
    }

    /// Nothing to apply yet: no tag picked, or no expression typed.
    pub fn empty(&self, cx: &gpui::App) -> bool {
        match self.mode {
            Mode::Tag => self.tag.is_empty(),
            Mode::Expr | Mode::Map => self.expr.read(cx).value().trim().is_empty(),
        }
    }

    fn add_case(
        &mut self,
        op: &'static str,
        v: f64,
        window: &mut Window,
        cx: &mut Context<Console>,
    ) {
        let v = field(number_text(v), "value", true, cx);
        let out = field(String::new(), "result", self.kind == PropType::Number, cx);
        self._subscriptions.push(listen(&v, window, cx));
        self._subscriptions.push(listen(&out, window, cx));
        self.cases.push(CaseRow {
            op,
            v,
            out,
            flag: true,
        });
    }
}

/// Open the editor on `prop` of the one selected component, seeded with
/// its binding (or an empty tag binding).
pub(super) fn open(
    console: &mut Console,
    prop: &Prop,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(widget) = edits_ref(console).and_then(|editor| editor.single().cloned()) else {
        return;
    };
    if edits_ref(console)
        .and_then(|editor| editor.bind.as_ref())
        .is_some_and(|open| open.widget == widget.id && open.prop == prop.key)
    {
        close(console, window, cx);
        return;
    }
    let existing = bindings::binding_of(&widget.config, prop.key);
    let (mode, tag, text) = match &existing {
        None => (Mode::Tag, String::new(), String::new()),
        Some(Binding::Tag { tag }) => (Mode::Tag, tag.clone(), String::new()),
        Some(Binding::Expr { expr }) => (Mode::Expr, String::new(), expr.clone()),
        Some(Binding::Map { expr, .. }) => (Mode::Map, String::new(), expr.clone()),
    };
    let expr_field = field(text, "{device/tag} > 80 ? \"HIGH\" : \"OK\"", false, cx);
    let otherwise_text = match &existing {
        Some(Binding::Map { otherwise, .. }) => json_text(otherwise.as_ref()),
        _ => String::new(),
    };
    let otherwise = field(
        otherwise_text,
        "otherwise",
        prop.kind == PropType::Number,
        cx,
    );
    let subscriptions = vec![
        listen(&expr_field, window, cx),
        listen(&otherwise, window, cx),
    ];
    let mut editor = BindEditor {
        widget: widget.id.clone(),
        prop: prop.key,
        kind: prop.kind,
        bound: existing.is_some(),
        mode,
        tag,
        expr: expr_field,
        cases: Vec::new(),
        otherwise,
        otherwise_flag: match &existing {
            Some(Binding::Map { otherwise, .. }) => otherwise.as_ref().and_then(Json::as_bool),
            _ => None,
        },
        _subscriptions: subscriptions,
    };
    if let Some(Binding::Map { cases, .. }) = &existing {
        for case in cases {
            let op = bindings::OPS
                .into_iter()
                .find(|op| *op == case.op)
                .unwrap_or(">=");
            editor.add_case(op, case.v, window, cx);
            if let Some(row) = editor.cases.last_mut() {
                row.flag = case.out.as_ref().and_then(Json::as_bool).unwrap_or(false);
                let text = json_text(case.out.as_ref());
                row.out.update(cx, |field, cx| field.set_text(text, cx));
            }
        }
    }
    if let Some(state) = edits(console) {
        state.bind = Some(editor);
        state.picking = None;
    }
    cx.notify();
}

pub(super) fn close(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if let Some(editor) = edits(console) {
        editor.bind = None;
        if matches!(editor.picking, Some(Pick::BindTag | Pick::Insert)) {
            editor.picking = None;
        }
    }
    focus_stage(console, window, cx);
    cx.notify();
}

/// Switch the draft to another kind, carrying what fits over (a tag
/// becomes `{tag}` in an expression; a map gets a first case).
fn switch(console: &mut Console, mode: Mode, window: &mut Window, cx: &mut Context<Console>) {
    let Some(state) = edits(console) else {
        return;
    };
    let Some(editor) = state.bind.as_mut() else {
        return;
    };
    if editor.mode == mode {
        return;
    }
    if editor.mode == Mode::Tag && !editor.tag.is_empty() && mode != Mode::Tag {
        let seed = expr::escape_ref(&editor.tag);
        editor.expr.update(cx, |field, cx| field.set_text(seed, cx));
    }
    if mode == Mode::Map && editor.cases.is_empty() {
        editor.add_case(">=", 1.0, window, cx);
    }
    editor.mode = mode;
    cx.notify();
}

/// A tag picked for the draft: the tag of a tag binding, or `{ref}`
/// inserted into the expression.
pub(super) fn picked(console: &mut Console, pick: Pick, tag: i64, cx: &mut Context<Console>) {
    let Some(reference) = console.store.ref_of(tag).map(str::to_owned) else {
        return;
    };
    let Some(editor) = edits(console).and_then(|state| state.bind.as_mut()) else {
        return;
    };
    match pick {
        Pick::BindTag => editor.tag = reference,
        Pick::Insert => {
            let now = editor.expr.read(cx).value().to_owned();
            let gap = if now.is_empty() || now.ends_with(' ') {
                ""
            } else {
                " "
            };
            let text = format!("{now}{gap}{}", expr::escape_ref(&reference));
            editor.expr.update(cx, |field, cx| field.set_text(text, cx));
        }
        _ => {}
    }
    cx.notify();
}

/// Apply the draft (one undo step), or remove the binding.
fn commit(console: &mut Console, remove: bool, window: &mut Window, cx: &mut Context<Console>) {
    let Some(state) = edits_ref(console) else {
        return;
    };
    let Some(editor) = state.bind.as_ref() else {
        return;
    };
    let Some(widget) = state.layout().widget(&editor.widget).cloned() else {
        return;
    };
    let draft = (!remove).then(|| editor.draft(cx));
    if let Some(draft) = &draft {
        let known = |reference: &str| console.store.by_ref(reference).is_some();
        if editor.empty(cx) || !bindings::binding_errors(draft, &known).is_empty() {
            return;
        }
    }
    let edit = designer::set_binding(&widget, editor.prop, draft.as_ref());
    apply(console, &edit, None, window, cx);
    close(console, window, cx);
}

/// What the draft gives now, or what is wrong with it.
fn preview(console: &Console, editor: &BindEditor, cx: &gpui::App) -> (String, bool) {
    if editor.empty(cx) {
        return ("Nothing bound yet".into(), false);
    }
    let draft = editor.draft(cx);
    let known = |reference: &str| console.store.by_ref(reference).is_some();
    let errors = bindings::binding_errors(&draft, &known);
    if !errors.is_empty() {
        return (errors.join(" · "), true);
    }
    let read = |reference: &str| console.store.read_ref(reference);
    let mut cache = expr::Cache::default();
    let value = bindings::evaluate_binding(&draft, &read, Some(editor.kind), &mut cache);
    let text = value.map_or_else(|| "no value".to_owned(), |value| value.to_string());
    (format!("Now: {text}"), false)
}

fn switch_row(on: bool, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .w(px(28.0))
        .h(px(16.0))
        .rounded(px(8.0))
        .p(px(2.0))
        .flex()
        .when(on, |this| this.justify_end().bg(theme.accent))
        .when(!on, |this| this.bg(theme.border))
        .child(div().size(px(12.0)).rounded(px(6.0)).bg(theme.bg))
        .into_any_element()
}

/// The editor's panel, under the property's row.
pub(super) fn render(
    console: &Console,
    widget: &Widget,
    picker: Option<AnyElement>,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> Option<AnyElement> {
    let editor = edits_ref(console)?.bind.as_ref()?;
    if editor.widget != widget.id {
        return None;
    }
    let mode = editor.mode;
    let segments = [
        (Mode::Tag, "Tag"),
        (Mode::Expr, "Expression"),
        (Mode::Map, "Map"),
    ]
    .into_iter()
    .map(|(at, label)| {
        Segment::new(
            label,
            mode == at,
            cx.listener(move |console, _, window, cx| switch(console, at, window, cx)),
        )
    })
    .collect();
    let mut body: Vec<AnyElement> = vec![segmented("designer-bindkind", segments, cx)];
    let bind_colour = theme.accent;
    match mode {
        Mode::Tag => {
            let current = if editor.tag.is_empty() {
                "Pick a tag".to_owned()
            } else {
                expr::escape_ref(&editor.tag)
            };
            body.push(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(FONT_SMALL)
                            .font_family(MONO)
                            .text_color(if editor.tag.is_empty() {
                                theme.fg_dim
                            } else {
                                bind_colour
                            })
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(current),
                    )
                    .child(
                        Button::new("designer-bind-pick", "Pick a tag")
                            .compact()
                            .on_click(cx.listener(|console, _, window, cx| {
                                super::inspector::start_picking(console, Pick::BindTag, window, cx);
                            })),
                    )
                    .into_any_element(),
            );
        }
        Mode::Expr | Mode::Map => {
            body.push(editor.expr.clone().into_any_element());
            body.push(
                Button::new("designer-bind-insert", "Insert tag…")
                    .compact()
                    .ghost()
                    .on_click(cx.listener(|console, _, window, cx| {
                        super::inspector::start_picking(console, Pick::Insert, window, cx);
                    }))
                    .into_any_element(),
            );
            if mode == Mode::Expr {
                let names: Vec<&str> = expr::FUNCTIONS.iter().map(|(name, ..)| *name).collect();
                body.push(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(format!(
                            "+ − × ÷ %, comparisons, && || !, a ? b : c, \"text\". Functions: {}.",
                            names.join(", ")
                        ))
                        .into_any_element(),
                );
            }
        }
    }
    body.extend(picker);
    if mode == Mode::Map {
        body.extend(cases(editor, theme, cx));
    }
    let (said, wrong) = preview(console, editor, cx);
    body.push(
        div()
            .id("designer-binding-preview")
            .text_size(FONT_CAPTION)
            .font_family(MONO)
            .text_color(if wrong { theme.danger } else { theme.fg_muted })
            .child(said)
            .into_any_element(),
    );
    let ready = !editor.empty(cx) && !wrong;
    let mut buttons: Vec<AnyElement> = Vec::new();
    if editor.bound {
        buttons.push(
            Button::new("designer-binding-remove", "Remove binding")
                .compact()
                .danger()
                .on_click(cx.listener(|console, _, window, cx| commit(console, true, window, cx)))
                .into_any_element(),
        );
    }
    buttons.push(
        Button::new("designer-binding-cancel", "Cancel")
            .compact()
            .ghost()
            .on_click(cx.listener(|console, _, window, cx| close(console, window, cx)))
            .into_any_element(),
    );
    buttons.push(
        Button::new("designer-binding-apply", "Apply")
            .compact()
            .primary()
            .disabled(!ready)
            .on_click(cx.listener(|console, _, window, cx| commit(console, false, window, cx)))
            .into_any_element(),
    );
    body.push(
        div()
            .flex()
            .justify_end()
            .gap(SPACE_2)
            .children(buttons)
            .into_any_element(),
    );
    Some(
        div()
            .id("designer-binding-editor")
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .p(SPACE_4)
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(bind_colour)
            .bg(theme.bg)
            .children(body)
            .into_any_element(),
    )
}

/// The map's cases and its "otherwise".
fn cases(editor: &BindEditor, theme: &Theme, cx: &mut Context<Console>) -> Vec<AnyElement> {
    let boolean = editor.kind == PropType::Boolean;
    let hover = theme.hover;
    let mut rows: Vec<AnyElement> = Vec::new();
    for (index, case) in editor.cases.iter().enumerate() {
        let tag = u64::try_from(index).unwrap_or(0);
        let op = div()
            .id(ElementId::NamedInteger("designer-case-op".into(), tag))
            .flex_none()
            .w(px(36.0))
            .h(CONTROL_SM)
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .text_size(FONT_SMALL)
            .font_family(MONO)
            .child(case.op)
            .on_click(cx.listener(move |console, _, _, cx| {
                if let Some(row) = edits(console)
                    .and_then(|state| state.bind.as_mut())
                    .and_then(|editor| editor.cases.get_mut(index))
                {
                    let at = bindings::OPS
                        .iter()
                        .position(|op| *op == row.op)
                        .unwrap_or(0);
                    row.op = bindings::OPS[(at + 1) % bindings::OPS.len()];
                }
                cx.notify();
            }));
        let out: AnyElement = if boolean {
            let on = case.flag;
            div()
                .id(ElementId::NamedInteger("designer-case-flag".into(), tag))
                .cursor_pointer()
                .child(switch_row(on, theme))
                .on_click(cx.listener(move |console, _, _, cx| {
                    if let Some(row) = edits(console)
                        .and_then(|state| state.bind.as_mut())
                        .and_then(|editor| editor.cases.get_mut(index))
                    {
                        row.flag = !row.flag;
                    }
                    cx.notify();
                }))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .child(case.out.clone())
                .into_any_element()
        };
        rows.push(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(op)
                .child(div().w(px(64.0)).flex_none().child(case.v.clone()))
                .child(out)
                .child(
                    Button::new(
                        ElementId::NamedInteger("designer-case-remove".into(), tag),
                        "×",
                    )
                    .compact()
                    .ghost()
                    .tooltip("Remove the case")
                    .on_click(cx.listener(move |console, _, _, cx| {
                        if let Some(editor) = edits(console).and_then(|state| state.bind.as_mut())
                            && index < editor.cases.len()
                        {
                            editor.cases.remove(index);
                        }
                        cx.notify();
                    })),
                )
                .into_any_element(),
        );
    }
    let otherwise: AnyElement = if boolean {
        let label = match editor.otherwise_flag {
            None => "—",
            Some(true) => "on",
            Some(false) => "off",
        };
        div()
            .id("designer-otherwise-flag")
            .px(SPACE_3)
            .h(CONTROL_SM)
            .flex()
            .items_center()
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .text_size(FONT_SMALL)
            .child(label)
            .on_click(cx.listener(|console, _, _, cx| {
                if let Some(editor) = edits(console).and_then(|state| state.bind.as_mut()) {
                    editor.otherwise_flag = match editor.otherwise_flag {
                        None => Some(true),
                        Some(true) => Some(false),
                        Some(false) => None,
                    };
                }
                cx.notify();
            }))
            .into_any_element()
    } else {
        div()
            .flex_1()
            .min_w_0()
            .child(editor.otherwise.clone())
            .into_any_element()
    };
    rows.push(
        div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .child(
                div()
                    .w(px(102.0))
                    .flex_none()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child("otherwise"),
            )
            .child(otherwise)
            .into_any_element(),
    );
    rows.push(
        Button::new("designer-case-add", "+ Case")
            .compact()
            .on_click(cx.listener(|console, _, window, cx| {
                if let Some(editor) = edits(console).and_then(|state| state.bind.as_mut()) {
                    editor.add_case(">=", 0.0, window, cx);
                }
                cx.notify();
            }))
            .into_any_element(),
    );
    rows
}

/// The label of a property's row: its value now. Bound: the live value
/// and where it comes from; fixed: the value set.
pub(super) fn value_line(
    console: &Console,
    widget: &Widget,
    prop: &Prop,
    cache: &mut expr::Cache,
) -> (SharedString, bool) {
    if let Some(binding) = bindings::binding_of(&widget.config, prop.key) {
        let read = |reference: &str| console.store.read_ref(reference);
        let live = bindings::evaluate_binding(&binding, &read, Some(prop.kind), cache);
        let shown = match live {
            None => "—".to_owned(),
            Some(Json::Number(n)) => n.as_f64().map_or_else(String::new, |x| {
                expr::js_number((x * 1000.0).round() / 1000.0)
            }),
            Some(Json::String(s)) => s,
            Some(other) => other.to_string(),
        };
        return (
            format!("{shown} ← {}", bindings::describe(&binding)).into(),
            true,
        );
    }
    let fixed = match &prop.field {
        Some(field) => voltrus_scada_native::mimic::edit::value_text(
            &voltrus_scada_native::mimic::edit::field_value(widget, field),
        ),
        None => match prop.key {
            "visible" => if designer::visible(widget) {
                "yes"
            } else {
                "no"
            }
            .to_owned(),
            "opacity" => widget
                .number("opacity")
                .map_or_else(|| "1".to_owned(), expr::js_number),
            key => widget.text(key).unwrap_or_default().to_owned(),
        },
    };
    (fixed.into(), false)
}

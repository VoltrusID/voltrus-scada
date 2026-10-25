//! What the Admin group's sections share (Users, Roles, Notifications,
//! API keys): the confirmation before anything destructive, the search
//! field, the table's header and cells, a dialog's frame and footer, and
//! closing a dialog once the server has answered. The palette's "New …"
//! commands land here too.

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, Context, Div, ElementId, Entity, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, SharedString, Stateful, Subscription, Window, div, px,
};
use vt_ui::Icon;
use vt_ui::components::{Button, kbd};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::choose;
use crate::console::keys::{DataNewApiKey, DataNewRole, DataNewRule, DataNewUser};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::pages::{Page, Role};
use crate::console::{Console, Phase};

/// A table row's height in every Admin section.
pub(crate) const ROW: f32 = 32.0;
const CONFIRM: &str = "data-admin-confirm";

/// A destructive action waiting for a yes.
pub(crate) struct Confirm {
    pub title: String,
    pub body: String,
    /// The button: "Delete", "Revoke".
    pub verb: &'static str,
    /// Runs after the yes; the section keeps what it acts on in its state.
    pub run: fn(&mut Console, &mut Window, &mut Context<Console>),
}

#[derive(Default)]
struct Kit {
    confirm: Option<Confirm>,
    focus: Option<FocusHandle>,
    /// The window, for closing a dialog once the server has answered.
    window: Option<AnyWindowHandle>,
}

/// Ask before `confirm.run`: Enter or the button runs it, Esc keeps.
pub(crate) fn ask(
    console: &mut Console,
    confirm: Confirm,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let focus = cx.focus_handle();
    let kit = console.state::<Kit>();
    kit.confirm = Some(confirm);
    kit.focus = Some(focus.clone());
    let overlay = Overlay::modal(CONFIRM, px(440.0), confirm_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn confirmed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(confirm) = console.state::<Kit>().confirm.take() else {
        return;
    };
    console.close_overlay(window, cx);
    (confirm.run)(console, window, cx);
}

fn confirm_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let kit = console.state::<Kit>();
    let Some(confirm) = kit.confirm.as_ref() else {
        return div().into_any_element();
    };
    let (title, body, verb) = (confirm.title.clone(), confirm.body.clone(), confirm.verb);
    let focus = kit.focus.clone();
    div()
        .when_some(focus, |this, focus| this.track_focus(&focus))
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "enter" {
                confirmed(console, window, cx);
                cx.stop_propagation();
            }
        }))
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(vt_ui::icon(Icon::Trash, ICON_LG, theme.danger))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                ),
        )
        .child(div().text_color(theme.fg_muted).child(body))
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
                        .child(verb.to_lowercase())
                        .child(kbd("Esc", cx))
                        .child("keep"),
                )
                .child(
                    Button::new("data-admin-keep", "Keep")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.state::<Kit>().confirm = None;
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new("data-admin-confirm", verb)
                        .danger()
                        .icon(Icon::Trash)
                        .on_click(cx.listener(|console, _, window, cx| {
                            confirmed(console, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

/// Remember the window a dialog opened in, for [`close_later`].
pub(crate) fn remember_window(console: &mut Console, window: &Window) {
    console.state::<Kit>().window = Some(window.window_handle());
}

/// Close overlay `id` (if still open) after this update: replies come
/// without a window.
pub(crate) fn close_later(console: &mut Console, id: &'static str, cx: &mut Context<Console>) {
    let Some(handle) = console.state::<Kit>().window else {
        return;
    };
    let entity = cx.entity();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            entity.update(cx, |console, cx| {
                if console.overlay_is(id) {
                    console.close_overlay(window, cx);
                }
            });
        });
    });
}

/// Focus `focus` after this update (a reply's new view: Copy the key).
pub(crate) fn focus_later(console: &mut Console, focus: FocusHandle, cx: &mut Context<Console>) {
    let Some(handle) = console.state::<Kit>().window else {
        return;
    };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, _| window.focus(&focus));
    });
}

/// A section's search field: typing drops the cursor, Enter or Esc go to
/// the first row of `list`.
pub(crate) fn search_field(
    list: &'static str,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> (Entity<TextInput>, Subscription) {
    let field = cx.new(|cx| TextInput::new(cx).placeholder(placeholder));
    let subscription = cx.subscribe_in(
        &field,
        window,
        move |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                console.list(list, cx).cursor = None;
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                let nav = console.list(list, cx);
                nav.step(Move::First);
                window.focus(&nav.focus);
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    (field, subscription)
}

/// The search field's text, lower-cased and trimmed.
pub(crate) fn query(field: Option<&Entity<TextInput>>, cx: &gpui::App) -> String {
    field
        .map(|field| field.read(cx).value().trim().to_lowercase())
        .unwrap_or_default()
}

/// The search field beside its icon, for the page header.
pub(crate) fn search_control(field: Option<Entity<TextInput>>, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .w(px(220.0))
        .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
        .children(field.map(|field| div().flex_1().child(field)))
        .into_any_element()
}

/// The table's header row: labels, `None` width for the flexible column.
pub(crate) fn header(columns: &[(&'static str, Option<f32>)], theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(30.0))
        .px(SPACE_8)
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .children(columns.iter().map(|(label, width)| {
            div()
                .map(|this| match width {
                    Some(width) => this.w(px(*width)).flex_none(),
                    None => this.flex_1().min_w_0(),
                })
                .child(*label)
        }))
}

/// A table row's frame: cursor highlight, hover, the focus ring.
pub(crate) fn row_frame(id: ElementId, current: bool, ring: bool, theme: &Theme) -> Stateful<Div> {
    let hover = theme.hover;
    div()
        .id(id)
        .group("data-admin-row")
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(current, |this| this.bg(theme.selected))
        .when(!current, |this| this.hover(move |style| style.bg(hover)))
        .when(ring, |this| this.child(super::super::focus_ring(theme)))
}

/// A fixed-width cell of small text.
pub(crate) fn cell(text: impl Into<SharedString>, width: f32, color: Hsla, mono: bool) -> Div {
    div()
        .w(px(width))
        .flex_none()
        .truncate()
        .text_size(FONT_SMALL)
        .text_color(color)
        .when(mono, |this| this.font_family(MONO))
        .child(text.into())
}

/// The row's action buttons: shown on the cursor row and on hover.
pub(crate) fn actions(width: f32, current: bool) -> Div {
    div()
        .w(px(width))
        .flex_none()
        .flex()
        .justify_end()
        .gap(SPACE_1)
        .opacity(if current { 1.0 } else { 0.0 })
        .group_hover("data-admin-row", |style| style.opacity(1.0))
}

pub(crate) fn element_id(name: &'static str, id: i64) -> ElementId {
    ElementId::NamedInteger(name.into(), u64::try_from(id).unwrap_or(0))
}

/// A dialog's title bar.
pub(crate) fn dialog_header(
    icon: Icon,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(56.0))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border)
        .child(vt_ui::icon(icon, ICON_LG, theme.accent))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(subtitle.into()),
                ),
        )
}

/// A dialog's body: fields stacked with room around them.
pub(crate) fn dialog_body(children: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .px(SPACE_8)
        .py(SPACE_7)
        .children(children)
}

/// A dialog's footer: the error or the keys, Cancel and the action.
pub(crate) fn dialog_footer(
    id: &'static str,
    error: Option<String>,
    label: &'static str,
    busy: bool,
    submit: fn(&mut Console, &mut Window, &mut Context<Console>),
    cx: &mut Context<Console>,
) -> Div {
    let theme = theme(cx).clone();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(56.0))
        .px(SPACE_8)
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .map(|this| match &error {
                    Some(error) => {
                        this.child(note(error.clone(), theme.danger, Some(Icon::CircleAlert)))
                    }
                    None => this
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(kbd("↵", cx))
                        .child(label.to_lowercase())
                        .child(kbd("Tab", cx))
                        .child("next field")
                        .child(kbd("Esc", cx))
                        .child("close"),
                }),
        )
        .child(
            Button::new(SharedString::from(format!("{id}-cancel")), "Cancel")
                .ghost()
                .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx))),
        )
        .child(
            Button::new(
                SharedString::from(format!("{id}-submit")),
                if busy { "Working…" } else { label },
            )
            .primary()
            .icon(Icon::Check)
            .disabled(busy)
            .on_click(cx.listener(move |console, _, window, cx| submit(console, window, cx))),
        )
}

/// A line of small text, with an icon in front if given.
pub(crate) fn note(text: impl Into<SharedString>, color: Hsla, icon: Option<Icon>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .min_w_0()
        .text_size(FONT_SMALL)
        .text_color(color)
        .children(icon.map(|icon| vt_ui::icon(icon, ICON_SM, color)))
        .child(div().min_w_0().child(text.into()))
        .into_any_element()
}

/// A permanent line under the header explaining a server rule.
pub(crate) fn banner(text: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_3)
        .px(SPACE_9)
        .py(SPACE_3)
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .text_size(FONT_SMALL)
        .text_color(theme.fg_muted)
        .child(vt_ui::icon(Icon::Info, ICON_SM, theme.info))
        .child(div().flex_1().min_w_0().child(text.into()))
        .into_any_element()
}

/// Show the Data Manager's section `id`, if the user may open it.
pub(crate) fn open_section(
    console: &mut Console,
    id: &'static str,
    role: Role,
    what: &str,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> bool {
    if !console.allows(role) {
        let who = if role == Role::Admin {
            "the admin role"
        } else {
            "the admin, demo or integration role"
        };
        console.show_toast(format!("{what} needs {who}"), true, cx);
        return false;
    }
    if console.page != Page::DataManager {
        console.show(Page::DataManager, window, cx);
    }
    choose(console, id, cx);
    true
}

/// The palette's "New user", "New role", "New API key" and "New
/// notification rule" (and N in each section).
pub(crate) fn root_actions(root: Stateful<Div>, cx: &mut Context<Console>) -> Stateful<Div> {
    root.on_action(cx.listener(|console, _: &DataNewUser, window, cx| {
        if console.phase == Phase::Console {
            super::users::new_user(console, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &DataNewRole, window, cx| {
        if console.phase == Phase::Console {
            super::roles::new_role(console, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &DataNewApiKey, window, cx| {
        if console.phase == Phase::Console {
            super::api_keys::new_key(console, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &DataNewRule, window, cx| {
        if console.phase == Phase::Console {
            super::notification_rules::new_rule(console, window, cx);
        }
    }))
}

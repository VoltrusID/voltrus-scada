//! The sign-in card, and the new-password card the server may ask for first.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, div, px};
use vt_ui::Icon;
use vt_ui::components::{Button, brand_mark, field};
use vt_ui::theme::*;

use super::host;
use crate::console::{Console, Phase};

pub fn render(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let changing = console.phase == Phase::ChangePassword;
    let username = console
        .user
        .as_ref()
        .map_or_else(String::new, |(name, _)| name.clone());

    let (title, subtitle) = if changing {
        (
            "Choose a new password",
            format!("{username} signed in with a generated password. Replace it before you start."),
        )
    } else {
        (
            "Sign in",
            "Operator console for the Voltrus SCADA server".to_owned(),
        )
    };

    let fields = if changing {
        div()
            .flex()
            .flex_col()
            .gap(SPACE_6)
            .child(field(
                "New password",
                console.inputs.new_password.clone(),
                cx,
            ))
            .child(field(
                "Repeat it",
                console.inputs.confirm_password.clone(),
                cx,
            ))
    } else {
        div()
            .flex()
            .flex_col()
            .gap(SPACE_6)
            .child(field("Server", console.inputs.url.clone(), cx))
            .child(field("Username", console.inputs.username.clone(), cx))
            .child(field("Password", console.inputs.password.clone(), cx))
    };

    let busy = console.busy;
    let primary = if changing {
        Button::new(
            "change-password",
            if busy { "Saving…" } else { "Change password" },
        )
        .primary()
        .disabled(busy)
        .on_click(cx.listener(|console, _, _, cx| console.change_password(cx)))
    } else {
        Button::new("sign-in", if busy { "Signing in…" } else { "Sign in" })
            .primary()
            .disabled(busy)
            .on_click(cx.listener(|console, _, window, cx| console.sign_in(window, cx)))
    };

    let card = div()
        .w(px(380.0))
        .flex()
        .flex_col()
        .gap(SPACE_8)
        .p(SPACE_10)
        .rounded(RADIUS_XL)
        .border_1()
        .border_color(theme.border)
        .bg(theme.elevated)
        .shadow(theme.shadow(Elevation::Modal))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .child(
                    div()
                        .text_size(FONT_DISPLAY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(FONT_BODY)
                        .text_color(theme.fg_muted)
                        .child(subtitle),
                ),
        )
        .child(fields)
        .when_some(console.error.clone(), |this, error| {
            this.child(
                div()
                    .flex()
                    .items_start()
                    .gap(SPACE_4)
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .rounded(RADIUS_MD)
                    .bg(theme.danger.alpha(0.1))
                    .border_1()
                    .border_color(theme.danger.alpha(0.3))
                    .text_size(FONT_SMALL)
                    .text_color(theme.danger)
                    .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
                    .child(div().flex_1().min_w_0().child(error)),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(primary)
                .when(changing, |this| {
                    this.child(Button::new("cancel-change", "Sign out").ghost().on_click(
                        cx.listener(|console, _, window, cx| {
                            console.sign_out(window, cx);
                        }),
                    ))
                }),
        )
        .child(
            div()
                .flex()
                .items_start()
                .gap(SPACE_3)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(
                    div()
                        .pt(px(1.0))
                        .child(vt_ui::icon(Icon::Lock, ICON_SM, theme.fg_dim)),
                )
                .child(div().flex_1().min_w_0().child(if changing {
                    format!(
                        "At least 8 characters. Applies to {} at once.",
                        host(&console.server)
                    )
                } else {
                    "Remembers the server and username, never the password.".to_owned()
                })),
        );

    div()
        .id("sign-in")
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(SPACE_8)
        .bg(theme.chrome)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(SPACE_5)
                .child(brand_mark(px(64.0)))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Voltrus SCADA"),
                ),
        )
        .child(card)
        .child(
            div()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(concat!("Voltrus SCADA console ", env!("CARGO_PKG_VERSION"))),
        )
        .into_any_element()
}

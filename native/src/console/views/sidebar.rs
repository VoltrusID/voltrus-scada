//! Brand, server, the pages, and who is signed in.

use gpui::prelude::*;
use gpui::{Context, FontWeight, SharedString, div, px};
use vt_ui::Icon;
use vt_ui::components::{IconButton, kbd, monogram, section_label};
use vt_ui::theme::*;

use super::host;
use super::sign_in::brand_mark;
use crate::console::{Console, Page};

const WIDTH: f32 = 232.0;

pub fn render(console: &Console, cx: &mut Context<Console>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let top_pad = if cfg!(target_os = "macos") {
        TRAFFIC_LIGHT_WIDTH
    } else {
        SPACE_4
    };
    let unacknowledged = console.unacknowledged();
    let (username, role) = console.user.clone().unwrap_or_default();
    let initials: String = username
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let pages = [
        (Page::Overview, Icon::Gauge, "1", None),
        (Page::Tags, Icon::Table, "2", None),
        (
            Page::Alarms,
            Icon::CircleAlert,
            "3",
            (unacknowledged > 0).then_some(unacknowledged),
        ),
        (Page::Trends, Icon::ChartLine, "4", None),
    ];
    let shortcut = |digit: &str| {
        if cfg!(target_os = "macos") {
            format!("⌘{digit}")
        } else {
            format!("Ctrl {digit}")
        }
    };
    let nav = pages.into_iter().map(|(page, glyph, digit, badge)| {
        let active = console.page == page;
        let color = if active { theme.fg } else { theme.fg_muted };
        let hover = theme.hover;
        div()
            .id(SharedString::from(format!("nav-{}", page.title())))
            .group("nav")
            .flex()
            .items_center()
            .gap(SPACE_4)
            .mx(SPACE_4)
            .px(SPACE_4)
            .h(px(30.0))
            .rounded(RADIUS_MD)
            .cursor_pointer()
            .text_color(color)
            .font_weight(if active {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            })
            .when(active, |this| this.bg(theme.selected))
            .when(!active, |this| this.hover(move |style| style.bg(hover)))
            .on_click(cx.listener(move |console, _, _, cx| console.show(page, cx)))
            .child(vt_ui::icon(
                glyph,
                ICON_MD,
                if active { theme.accent } else { theme.fg_dim },
            ))
            .child(div().flex_1().child(page.title()))
            .when_some(badge, |this, count| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .min_w(px(18.0))
                        .h(px(16.0))
                        .px(SPACE_2)
                        .rounded_full()
                        .bg(theme.danger)
                        .text_color(theme.accent_fg)
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::BOLD)
                        .child(if count > 99 {
                            "99+".to_owned()
                        } else {
                            count.to_string()
                        }),
                )
            })
            .when(badge.is_none(), |this| this.child(kbd(shortcut(digit), cx)))
    });

    let dark = theme.dark;
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(WIDTH))
        .h_full()
        .bg(theme.chrome)
        .border_r_1()
        .border_color(theme.border)
        .child(div().flex_none().h(TITLE_BAR_HEIGHT).pl(top_pad))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .px(SPACE_7)
                .pb(SPACE_7)
                .child(brand_mark(&theme))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(FONT_EMPHASIS)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Voltrus SCADA"),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(MONO)
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(host(&console.server).to_owned()),
                        ),
                ),
        )
        .child(
            div()
                .px(SPACE_8)
                .pb(SPACE_3)
                .child(section_label("Monitor", cx)),
        )
        .child(div().flex().flex_col().gap(SPACE_1).children(nav))
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .mx(SPACE_4)
                .mb(SPACE_4)
                .p(SPACE_4)
                .rounded(RADIUS_LG)
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .child(monogram(initials, theme.accent, px(28.0)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(username),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(role_label(&role)),
                        ),
                )
                .child(
                    IconButton::new("appearance", if dark { Icon::Sun } else { Icon::Moon })
                        .tooltip(if dark {
                            "Light appearance  ⇧⌘L"
                        } else {
                            "Dark appearance  ⇧⌘L"
                        })
                        .on_click(cx.listener(|console, _, _, cx| console.toggle_appearance(cx))),
                )
                .child(
                    IconButton::new("sign-out", Icon::Unplug)
                        .tooltip("Sign out")
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.sign_out(window, cx);
                        })),
                ),
        )
}

fn role_label(role: &str) -> String {
    let mut chars = role.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Signed in".into(),
    }
}

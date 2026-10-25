//! The top bar, sharing the title bar row: brand and server, the pages, and
//! who is signed in.

use gpui::prelude::*;
use gpui::{Context, FontWeight, SharedString, div, px};
use vt_ui::Icon;
use vt_ui::components::{IconButton, Tooltip, brand_mark, monogram};
use vt_ui::theme::*;

use super::{capitalized, host};
use crate::console::{Console, Page};

pub fn render(
    console: &Console,
    unacknowledged: usize,
    cx: &mut Context<Console>,
) -> impl IntoElement {
    let theme = theme(cx).clone();
    let left = if cfg!(target_os = "macos") {
        TRAFFIC_LIGHT_WIDTH
    } else {
        SPACE_6
    };
    let (username, role) = console.user.clone().unwrap_or_default();
    let initials: String = username
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let pages = [
        (Page::Overview, Icon::Gauge, '1', None),
        (Page::Tags, Icon::Table, '2', None),
        (
            Page::Alarms,
            Icon::CircleAlert,
            '3',
            (unacknowledged > 0).then_some(unacknowledged),
        ),
        (Page::Trends, Icon::ChartLine, '4', None),
    ];
    let tabs = pages.into_iter().map(|(page, glyph, digit, badge)| {
        let active = console.page == page;
        let hover = theme.hover;
        let hover_fg = theme.fg;
        let hint = SharedString::from(if cfg!(target_os = "macos") {
            format!("{}  ⌘{digit}", page.title())
        } else {
            format!("{}  Ctrl {digit}", page.title())
        });
        div()
            .id(SharedString::from(format!("nav-{}", page.title())))
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_3)
            .h(CONTROL_MD)
            .px(SPACE_5)
            .rounded(RADIUS_MD)
            .cursor_pointer()
            .text_color(if active { theme.fg } else { theme.fg_muted })
            .font_weight(if active {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            })
            .when(active, |this| this.bg(theme.selected))
            .when(!active, |this| {
                this.hover(move |style| style.bg(hover).text_color(hover_fg))
            })
            .tooltip(move |_, cx| Tooltip::view(hint.clone(), cx))
            .on_click(cx.listener(move |console, _, _, cx| console.show(page, cx)))
            .child(vt_ui::icon(
                glyph,
                ICON_MD,
                if active { theme.accent } else { theme.fg_dim },
            ))
            .child(page.title())
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
                        .text_color(super::text_on(theme.danger, &theme))
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::BOLD)
                        .child(if count > 99 {
                            "99+".to_owned()
                        } else {
                            count.to_string()
                        }),
                )
            })
    });

    let separator = || div().flex_none().w(px(1.0)).h(px(18.0)).bg(theme.border);
    let dark = theme.dark;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .h(TITLE_BAR_HEIGHT)
        .pl(left)
        .pr(SPACE_6)
        .bg(theme.chrome)
        .border_b_1()
        .border_color(theme.border)
        .whitespace_nowrap()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .child(brand_mark(px(22.0)))
                .child(
                    div()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Voltrus SCADA"),
                ),
        )
        .child(
            div()
                .min_w(px(40.0))
                .max_w(px(220.0))
                .truncate()
                .font_family(MONO)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(host(&console.server).to_owned()),
        )
        .child(separator())
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_2)
                .children(tabs),
        )
        .child(div().flex_1())
        .child(
            IconButton::new("appearance", if dark { Icon::Sun } else { Icon::Moon })
                .tooltip(if dark {
                    "Light appearance  ⇧⌘L"
                } else {
                    "Dark appearance  ⇧⌘L"
                })
                .on_click(cx.listener(|console, _, _, cx| console.toggle_appearance(cx))),
        )
        .child(separator())
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap(SPACE_4)
                .child(monogram(initials, theme.accent, px(22.0)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
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
                ),
        )
        .child(
            IconButton::new("sign-out", Icon::Unplug)
                .tooltip("Sign out")
                .on_click(cx.listener(|console, _, window, cx| {
                    console.sign_out(window, cx);
                })),
        )
}

fn role_label(role: &str) -> String {
    if role.is_empty() {
        "Signed in".into()
    } else {
        capitalized(role)
    }
}

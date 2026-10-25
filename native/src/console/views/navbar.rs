//! The top bar, sharing the title bar row: brand and server, the pages, and
//! who is signed in. The tabs come from the page registry: pages placed as
//! tabs get one (icons only when the window is narrow), the rest sit in the
//! More menu at the end.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, SharedString, Window, canvas, div, px};
use vt_ui::Icon;
use vt_ui::components::{IconButton, Tooltip, brand_mark, kbd, monogram};
use vt_ui::theme::*;

use super::{capitalized, host};
use crate::console::keys::{
    ShowMore, ToggleAppearance, TogglePalette, TogglePerfHud, display, hint, page_keys,
};
use crate::console::pages::{PAGES, PageSpec, Placement};
use crate::console::{Console, overlay};

/// Below this window width the tabs show icons only.
const COMPACT_BELOW: f32 = 1280.0;

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let accent = theme.accent;
    let left = if cfg!(target_os = "macos") {
        TRAFFIC_LIGHT_WIDTH
    } else {
        SPACE_6
    };
    let compact = f32::from(window.viewport_size().width) < COMPACT_BELOW;
    let (username, role) = console.user.clone().unwrap_or_default();
    let initials: String = username
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let tabs: Vec<AnyElement> = PAGES
        .iter()
        .filter(|spec| spec.placement == Placement::Tab && console.allows(spec.role))
        .map(|spec| tab(console, spec, compact, cx))
        .collect();

    // The More menu: its pages, and the current one's title when it is one
    // of them.
    let more = overlay::more_pages(console.role());
    let more_active = more.contains(&console.page);
    let more_open = console.overlay_is("more");
    let anchor = console.more_anchor.clone();
    let more_button = (!more.is_empty()).then(|| {
        let hover = theme.hover;
        let hover_fg = theme.fg;
        div()
            .relative()
            .child(
                div()
                    .id("nav-more")
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
                    .text_color(if more_active {
                        theme.fg
                    } else {
                        theme.fg_muted
                    })
                    .when(more_active || more_open, |this| this.bg(theme.selected))
                    .when(!more_active, |this| {
                        this.hover(move |style| style.bg(hover).text_color(hover_fg))
                    })
                    .tooltip(|_, cx| Tooltip::view("More pages".into(), cx))
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(Box::new(ShowMore), cx);
                    }))
                    .child(vt_ui::icon(
                        Icon::Ellipsis,
                        ICON_MD,
                        if more_active {
                            theme.accent
                        } else {
                            theme.fg_dim
                        },
                    ))
                    .when(!compact, |this| {
                        this.child(if more_active {
                            SharedString::from(console.page.title())
                        } else {
                            SharedString::from("More")
                        })
                    })
                    .child(vt_ui::icon(Icon::ChevronDown, ICON_SM, theme.fg_dim)),
            )
            // Where the menu opens: under this button.
            .child(
                canvas(
                    move |bounds, _, _| anchor.set(Some(bounds)),
                    |_, (), _, _| {},
                )
                .absolute()
                .size_full(),
            )
    });

    let separator = || div().flex_none().w(px(1.0)).h(px(18.0)).bg(theme.border);
    let dark = theme.dark;
    let palette_keys = hint(&TogglePalette).unwrap_or_default();
    div()
        .size_full()
        .flex()
        .items_center()
        .gap(SPACE_6)
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
                .when(!compact, |this| {
                    this.child(
                        div()
                            .text_size(FONT_EMPHASIS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Voltrus SCADA"),
                    )
                }),
        )
        .child(
            div()
                .min_w(px(40.0))
                .max_w(px(200.0))
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
                .children(tabs)
                .children(more_button),
        )
        .child(div().flex_1().min_w(px(8.0)))
        .child(
            div()
                .id("nav-palette")
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_3)
                .h(CONTROL_SM)
                .px(SPACE_4)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .cursor_pointer()
                .tab_index(0)
                .focus(move |style| style.border_color(accent))
                .hover(move |style| style.border_color(theme.border_strong))
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .on_click(cx.listener(|_, _, window, cx| {
                    window.dispatch_action(Box::new(TogglePalette), cx);
                }))
                .child(vt_ui::icon(Icon::Search, ICON_SM, theme.fg_dim))
                .when(!compact, |this| this.child("Search"))
                .child(kbd(palette_keys, cx)),
        )
        .child(super::alarms::mute_button(console, cx))
        .child(
            IconButton::new("perf-hud", Icon::Activity)
                .active(console.hud)
                .tooltip(format!(
                    "Performance HUD  {}",
                    hint(&TogglePerfHud).unwrap_or_default()
                ))
                .on_click(cx.listener(|console, _, _, cx| {
                    console.set_hud(!console.hud, cx);
                })),
        )
        .child(
            IconButton::new("appearance", if dark { Icon::Sun } else { Icon::Moon })
                .tooltip(format!(
                    "{} appearance  {}",
                    if dark { "Light" } else { "Dark" },
                    hint(&ToggleAppearance).unwrap_or_default()
                ))
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
                .when(!compact, |this| {
                    this.child(
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
                    )
                }),
        )
        .child(
            IconButton::new("sign-out", Icon::Unplug)
                .tooltip("Sign out")
                .on_click(cx.listener(|console, _, window, cx| {
                    console.sign_out(window, cx);
                })),
        )
        .into_any_element()
}

fn tab(
    console: &Console,
    spec: &'static PageSpec,
    compact: bool,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let page = spec.page;
    let active = console.page == page;
    let hover = theme.hover;
    let hover_fg = theme.fg;
    let accent = theme.accent;
    let badge = spec.badge.map_or(0, |badge| badge(console));
    let hint = SharedString::from(match spec.digit {
        Some(digit) => format!("{}  {}", spec.title, display(&page_keys(digit))),
        None => spec.title.to_owned(),
    });
    div()
        .id(SharedString::from(format!("nav-{}", spec.id)))
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_3)
        .h(CONTROL_MD)
        .px(if compact { SPACE_4 } else { SPACE_5 })
        .rounded(RADIUS_MD)
        .cursor_pointer()
        .tab_index(0)
        .border_1()
        .border_color(gpui::transparent_black())
        .focus(move |style| style.border_color(accent))
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
        .on_click(cx.listener(move |console, _, window, cx| console.show(page, window, cx)))
        .child(vt_ui::icon(
            spec.icon,
            ICON_MD,
            if active { theme.accent } else { theme.fg_dim },
        ))
        .when(!compact, |this| this.child(spec.title))
        .when(badge > 0, |this| {
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
                    .child(if badge > 99 {
                        "99+".to_owned()
                    } else {
                        badge.to_string()
                    }),
            )
        })
        .into_any_element()
}

fn role_label(role: &str) -> String {
    if role.is_empty() {
        "Signed in".into()
    } else {
        capitalized(role)
    }
}

//! The top bar, sharing the title bar row: brand and server, the pages, and
//! who is signed in. The tabs come from the page registry: pages placed as
//! tabs get one, the rest sit in the More menu at the end.
//!
//! It fits the window at every width: `navfit` picks how much it shows from
//! the window's width and the measured widths of its labels (icon-only
//! tabs, no names, a search button, the secondary controls folded into one
//! menu, no host), so the avatar and sign-out are never clipped.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{
    Action, AnyElement, Context, FontWeight, Pixels, SharedString, TextRun, Window, canvas, div, px,
};
use vt_ui::Icon;
use vt_ui::components::{IconButton, Tooltip, brand_mark, kbd, monogram};
use vt_ui::theme::*;

use super::{ListKeys, capitalized, focus_ring, host, keyboard_list};
use crate::console::keys::{
    AlarmsToggleMute, ShellOpenSettings, ShellToggleNotifications, ShowMore, ShowNavMenu,
    ToggleAppearance, TogglePalette, TogglePerfHud, display, hint, page_keys,
};
use crate::console::navfit::{self, Fit, Parts, Tab};
use crate::console::pages::{PAGES, PageSpec, Placement};
use crate::console::shell::Shell;
use crate::console::{Console, overlay};

/// The navbar menu (the secondary controls, folded) and its keyboard list.
pub(crate) const MENU: &str = "nav-menu";
pub(crate) const MENU_WIDTH: Pixels = px(260.0);

/// The secondary controls on the navbar, left to right; folded into the
/// navbar menu in a narrow window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Notifications,
    Settings,
    Sound,
    Hud,
    Appearance,
}

const CONTROLS: [Control; 5] = [
    Control::Notifications,
    Control::Settings,
    Control::Sound,
    Control::Hud,
    Control::Appearance,
];

impl Control {
    fn action(self) -> Box<dyn Action> {
        match self {
            Self::Notifications => Box::new(ShellToggleNotifications),
            Self::Settings => Box::new(ShellOpenSettings),
            Self::Sound => Box::new(AlarmsToggleMute),
            Self::Hud => Box::new(TogglePerfHud),
            Self::Appearance => Box::new(ToggleAppearance),
        }
    }
}

/// Label widths by text, font family, size and weight (as bits).
type Widths = HashMap<(String, &'static str, u32, u32), f32>;

thread_local! {
    /// Measured label widths: the labels hardly change, so the navbar
    /// shapes each once.
    static WIDTHS: RefCell<Widths> = RefCell::new(HashMap::new());
}

/// The width of `text` in `family` at `size` and `weight`, as the text
/// system lays it out.
fn measure(
    window: &Window,
    text: &str,
    size: Pixels,
    family: &'static str,
    weight: FontWeight,
) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let key = (
        text.to_owned(),
        family,
        f32::from(size).to_bits(),
        weight.0.to_bits(),
    );
    if let Some(width) = WIDTHS.with(|widths| widths.borrow().get(&key).copied()) {
        return width;
    }
    let mut font = gpui::font(family);
    font.weight = weight;
    let run = TextRun {
        len: text.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width = f32::from(
        window
            .text_system()
            .layout_line(text, size, &[run], None)
            .width,
    )
    .ceil();
    WIDTHS.with(|widths| widths.borrow_mut().insert(key, width));
    width
}

fn badge_text(count: usize) -> String {
    if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    }
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let accent = theme.accent;
    let left = if cfg!(target_os = "macos") {
        TRAFFIC_LIGHT_WIDTH
    } else {
        SPACE_6
    };
    let (username, role) = console.user.clone().unwrap_or_default();
    let role = role_label(&role);
    let initials: String = username
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let server = host(&console.server).to_owned();
    let specs: Vec<&'static PageSpec> = PAGES
        .iter()
        .filter(|spec| spec.placement == Placement::Tab && console.allows(spec.role))
        .collect();
    let badges: Vec<usize> = specs
        .iter()
        .map(|spec| spec.badge.map_or(0, |badge| badge(console)))
        .collect();
    // The More menu: its pages, and the current one's title when it is one
    // of them.
    let more = overlay::more_pages(console.role());
    let more_active = more.contains(&console.page);
    let more_label = SharedString::from(if more_active {
        console.page.title()
    } else {
        "More"
    });
    let palette_keys = hint(&TogglePalette).unwrap_or_default();
    let edition = super::edition::badge_label(console);
    let kiosk = console.kiosk_locked();

    // What fits: every label measured as the navbar draws it.
    let parts = Parts {
        left: f32::from(left),
        wordmark: measure(
            window,
            "Voltrus SCADA",
            FONT_EMPHASIS,
            SANS,
            FontWeight::SEMIBOLD,
        ),
        host: measure(window, &server, FONT_CAPTION, MONO, FontWeight::NORMAL),
        edition: edition
            .map(|label| measure(window, label, FONT_CAPTION, SANS, FontWeight::SEMIBOLD)),
        tabs: specs
            .iter()
            .zip(&badges)
            .map(|(spec, badge)| Tab {
                label: measure(
                    window,
                    spec.title,
                    FONT_BODY,
                    SANS,
                    if console.page == spec.page {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    },
                ),
                badge: (*badge > 0).then(|| {
                    measure(
                        window,
                        &badge_text(*badge),
                        FONT_CAPTION,
                        SANS,
                        FontWeight::BOLD,
                    )
                }),
            })
            .collect(),
        more: (!more.is_empty())
            .then(|| measure(window, &more_label, FONT_BODY, SANS, FontWeight::NORMAL)),
        search_label: measure(window, "Search", FONT_SMALL, SANS, FontWeight::NORMAL),
        search_keys: measure(
            window,
            &palette_keys,
            FONT_CAPTION,
            MONO,
            FontWeight::NORMAL,
        ),
        controls: CONTROLS.len(),
        name: measure(window, &username, FONT_SMALL, SANS, FontWeight::MEDIUM).max(measure(
            window,
            &role,
            FONT_CAPTION,
            SANS,
            FontWeight::NORMAL,
        )),
        sign_out: !kiosk,
    };
    let fit = navfit::choose(&parts, f32::from(window.viewport_size().width));
    console.nav_fit = fit;

    let tabs: Vec<AnyElement> = specs
        .iter()
        .copied()
        .zip(badges)
        .map(|(spec, badge)| tab(console, spec, badge, fit, cx))
        .collect();
    let more_open = console.overlay_is("more");
    let anchor = console.more_anchor.clone();
    let more_button = (!more.is_empty()).then(|| {
        let hover = theme.hover;
        let hover_fg = theme.fg;
        div()
            .relative()
            .flex_none()
            .debug_selector(|| "nav-more".into())
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
                    .when(fit.tab_labels(), |this| this.child(more_label))
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
    let search_tip = SharedString::from(format!("Search  {palette_keys}"));
    let search = if fit.search_box() {
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
            .child("Search")
            .child(kbd(palette_keys, cx))
            .into_any_element()
    } else {
        IconButton::new("nav-palette", Icon::Search)
            .tooltip(search_tip)
            .on_click(cx.listener(|_, _, window, cx| {
                window.dispatch_action(Box::new(TogglePalette), cx);
            }))
            .into_any_element()
    };

    let user_tip = SharedString::from(format!("{username} · {role}"));
    let names = fit.names();
    let user = div()
        .id("nav-user")
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .child(monogram(initials, theme.accent, px(navfit::LOGO)))
        .when(names, |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(FONT_SMALL)
                            .font_weight(FontWeight::MEDIUM)
                            .child(username),
                    )
                    .child(
                        div()
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(role),
                    ),
            )
        })
        .when(!names, |this| {
            this.tooltip(move |_, cx| Tooltip::view(user_tip.clone(), cx))
        });

    let controls: Vec<AnyElement> = if fit.folds_controls() {
        vec![menu_button(console, cx)]
    } else {
        CONTROLS
            .iter()
            .enumerate()
            .map(|(at, control)| {
                div()
                    .flex_none()
                    .debug_selector(|| format!("nav-control-{at}"))
                    .child(control_button(console, *control, cx))
                    .into_any_element()
            })
            .collect()
    };

    div()
        .size_full()
        .flex()
        .items_center()
        .gap(px(navfit::gap()))
        .pl(left)
        .pr(px(navfit::EDGE))
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
                .debug_selector(|| "nav-brand".into())
                .child(brand_mark(px(navfit::LOGO)))
                .when(names, |this| {
                    this.child(
                        div()
                            .text_size(FONT_EMPHASIS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Voltrus SCADA"),
                    )
                }),
        )
        .when(fit.host(), |this| {
            this.child(
                div()
                    .min_w(px(navfit::HOST_MIN))
                    .max_w(px(navfit::HOST_MAX))
                    .truncate()
                    .debug_selector(|| "nav-host".into())
                    .font_family(MONO)
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(server),
            )
            .children(super::edition::badge(console, cx))
        })
        .child(separator())
        // Tabs give way before the controls and the avatar if a label
        // measured short: they clip, the avatar stays whole.
        .child(
            div()
                .flex()
                .min_w_0()
                .overflow_hidden()
                .items_center()
                .gap(SPACE_2)
                .debug_selector(|| "nav-tabs".into())
                .children(tabs)
                .children(more_button),
        )
        .child(div().flex_1().min_w(px(navfit::SPACER_MIN)))
        .child(
            div()
                .flex_none()
                .debug_selector(|| "nav-search".into())
                .child(search),
        )
        .children(controls)
        .child(separator())
        .child(user.debug_selector(|| "nav-avatar".into()))
        // The kiosk lock keeps the operator signed in.
        .when(!kiosk, |this| {
            this.child(
                div()
                    .flex_none()
                    .debug_selector(|| "nav-sign-out".into())
                    .child(
                        IconButton::new("sign-out", Icon::Unplug)
                            .tooltip("Sign out")
                            .on_click(cx.listener(|console, _, window, cx| {
                                console.sign_out(window, cx);
                            })),
                    ),
            )
        })
        .into_any_element()
}

/// One of the secondary controls as its navbar button.
fn control_button(
    console: &mut Console,
    control: Control,
    cx: &mut Context<Console>,
) -> AnyElement {
    let dark = theme(cx).dark;
    match control {
        Control::Notifications => super::notifications::bell(console, cx),
        Control::Settings => IconButton::new("settings", Icon::Settings)
            .active(console.overlay_is("settings"))
            .tooltip(format!(
                "Settings  {}",
                hint(&ShellOpenSettings).unwrap_or_default()
            ))
            .on_click(cx.listener(|_, _, window, cx| {
                window.dispatch_action(Box::new(ShellOpenSettings), cx);
            }))
            .into_any_element(),
        Control::Sound => super::alarms::mute_button(console, cx),
        Control::Hud => IconButton::new("perf-hud", Icon::Activity)
            .active(console.hud)
            .tooltip(format!(
                "Performance HUD  {}",
                hint(&TogglePerfHud).unwrap_or_default()
            ))
            .on_click(cx.listener(|console, _, _, cx| {
                console.set_hud(!console.hud, cx);
            }))
            .into_any_element(),
        Control::Appearance => {
            IconButton::new("appearance", if dark { Icon::Sun } else { Icon::Moon })
                .tooltip(format!(
                    "{} appearance  {}",
                    if dark { "Light" } else { "Dark" },
                    hint(&ToggleAppearance).unwrap_or_default()
                ))
                .on_click(cx.listener(|console, _, _, cx| console.toggle_appearance(cx)))
                .into_any_element()
        }
    }
}

/// The navbar menu's button, standing in for the folded controls: it
/// carries the bell's unread count, and the notifications popover opens
/// under it.
fn menu_button(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let unread = console.unread_notifications();
    let open = console.overlay_is(MENU);
    let anchor = console.menu_anchor.clone();
    let bell = console.state::<Shell>().bell_anchor.clone();
    div()
        .relative()
        .flex_none()
        .debug_selector(|| "nav-menu".into())
        .child(
            IconButton::new("nav-menu", Icon::Ellipsis)
                .active(open)
                .color(if unread > 0 {
                    theme.warning
                } else {
                    theme.fg_muted
                })
                .tooltip(if unread > 0 {
                    format!("Notifications, settings and view ({unread} unread)")
                } else {
                    "Notifications, settings and view".to_owned()
                })
                .on_click(cx.listener(|_, _, window, cx| {
                    window.dispatch_action(Box::new(ShowNavMenu), cx);
                })),
        )
        .when(unread > 0, |this| {
            this.child(super::notifications::unread_badge(unread, &theme))
        })
        .child(
            canvas(
                move |bounds, _, _| {
                    anchor.set(Some(bounds));
                    bell.set(Some(bounds));
                },
                |_, (), _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .into_any_element()
}

fn run_control(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(control) = CONTROLS.get(row).copied() else {
        return;
    };
    // As the palette runs a command: the menu closes, focus goes back, and
    // the control's action reaches the root.
    console.close_overlay(window, cx);
    window.dispatch_action(control.action(), cx);
}

/// The navbar menu: the secondary controls as rows, with their keys.
pub fn menu(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let unread = console.unread_notifications();
    let muted = super::alarms::muted(console);
    let hud = console.hud;
    let dark = theme.dark;
    let list = keyboard_list(
        ListKeys {
            id: MENU,
            on_move: None,
            on_activate: run_control,
        },
        CONTROLS.len(),
        CONTROLS.len(),
        console,
        cx,
    );
    let nav = console.list(MENU, cx);
    let (cursor, focused, rows_scroll) =
        (nav.cursor, nav.focus.is_focused(window), nav.rows.clone());
    let rows: Vec<AnyElement> = CONTROLS
        .iter()
        .enumerate()
        .map(|(row, control)| {
            let (glyph, label, on): (AnyElement, &str, bool) = match control {
                Control::Notifications => (
                    vt_ui::icon(Icon::Inbox, ICON_MD, theme.fg_muted).into_any_element(),
                    "Notifications",
                    false,
                ),
                Control::Settings => (
                    vt_ui::icon(Icon::Settings, ICON_MD, theme.fg_muted).into_any_element(),
                    "Settings",
                    false,
                ),
                Control::Sound => (
                    super::alarms::speaker(muted, ICON_MD, theme.fg_muted).into_any_element(),
                    if muted {
                        "Alarm sound off"
                    } else {
                        "Alarm sound on"
                    },
                    !muted,
                ),
                Control::Hud => (
                    vt_ui::icon(Icon::Activity, ICON_MD, theme.fg_muted).into_any_element(),
                    "Performance HUD",
                    hud,
                ),
                Control::Appearance => (
                    vt_ui::icon(
                        if dark { Icon::Sun } else { Icon::Moon },
                        ICON_MD,
                        theme.fg_muted,
                    )
                    .into_any_element(),
                    if dark {
                        "Light appearance"
                    } else {
                        "Dark appearance"
                    },
                    false,
                ),
            };
            let keys = hint(control.action().as_ref()).unwrap_or_default();
            let ring = focused && cursor == Some(row);
            let hover = theme.hover;
            div()
                .id(("nav-menu-row", row))
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .h(px(30.0))
                .mx(SPACE_3)
                .px(SPACE_4)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |console, _, window, cx| {
                    run_control(console, row, false, window, cx);
                }))
                .child(glyph)
                .child(div().flex_1().min_w_0().truncate().child(label))
                .when(*control == Control::Notifications && unread > 0, |this| {
                    this.child(
                        div()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.warning)
                            .child(format!("{} unread", badge_text(unread))),
                    )
                })
                .when(on, |this| {
                    this.child(vt_ui::icon(Icon::Check, ICON_MD, theme.accent))
                })
                .when(!keys.is_empty(), |this| this.child(kbd(keys, cx)))
                .when(ring, |this| this.child(focus_ring(&theme)))
                .into_any_element()
        })
        .collect();
    list.min_h_0()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .track_scroll(&rows_scroll)
        .children(rows)
        .into_any_element()
}

fn tab(
    console: &Console,
    spec: &'static PageSpec,
    badge: usize,
    fit: Fit,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let page = spec.page;
    let active = console.page == page;
    let hover = theme.hover;
    let hover_fg = theme.fg;
    let accent = theme.accent;
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
        .px(px(navfit::tab_padding(fit)))
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
        .when(fit.tab_labels(), |this| this.child(spec.title))
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
                    .child(badge_text(badge)),
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

//! The overlay slot's frame (backdrop, card, focus trap) and the built-in
//! overlays: the command palette, the shortcut help and the More menu.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, FontWeight, MouseButton, SharedString, Window, anchored, div, hsla, px,
    uniform_list,
};
use vt_ui::components::{kbd, section_label};
use vt_ui::theme::*;

use super::{ListKeys, focus_ring, keyboard_list};
use crate::console::keys::{Group, ListDown, ListUp, MENU, TogglePalette, help_rows, hint};
use crate::console::nav::Move;
use crate::console::overlay::{self, OverlayKind};
use crate::console::{Console, Page};

/// The palette's result list and the More menu's list.
pub(crate) const PALETTE_LIST: &str = "palette";
pub(crate) const MORE_LIST: &str = "more";

const PALETTE_ROW: f32 = 36.0;

/// The frame around whatever overlay is open.
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let Some(overlay) = console.overlay.as_ref() else {
        return div().into_any_element();
    };
    let kind = overlay.kind;
    let trap = overlay.focus.clone();
    let draw = overlay.render;
    let content = draw(console, window, cx);
    let theme = theme(cx).clone();
    let dim = hsla(0.0, 0.0, 0.0, if theme.dark { 0.45 } else { 0.2 });
    let border = theme.border_strong;
    let card = div()
        .track_focus(&trap)
        .flex()
        .flex_col()
        .bg(theme.elevated)
        .border_color(border)
        .shadow(theme.shadow(Elevation::Modal))
        // Clicks inside stay inside; the backdrop's close is for outside.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(content);
    let backdrop = div()
        .id("overlay")
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|console, _, window, cx| console.close_overlay(window, cx)),
        );
    match kind {
        OverlayKind::Modal { width } => backdrop
            .bg(dim)
            .flex()
            .justify_center()
            .items_start()
            .pt(px(96.0))
            .child(card.w(width).max_h(px(560.0)).border_1().rounded(RADIUS_XL)),
        OverlayKind::Sheet { width } => backdrop
            .bg(dim)
            .flex()
            .justify_end()
            .child(card.w(width).h_full().border_l_1()),
        OverlayKind::Popover { width, at } => backdrop.child(
            anchored()
                .position(at)
                .snap_to_window()
                .child(card.w(width).border_1().rounded(RADIUS_LG).py(SPACE_3)),
        ),
    }
    .into_any_element()
}

/// ⌘K: every page and command, fuzzy-searched as you type.
pub fn palette(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(palette) = console.palette.as_ref() else {
        return div().into_any_element();
    };
    let input = palette.input.clone();
    let count = palette.results.len();
    let nav = console.list(PALETTE_LIST, cx);
    nav.set_len(count);
    let scroll = nav.scroll.clone();

    let list = uniform_list(
        "palette-results",
        count,
        cx.processor(move |console: &mut Console, range: Range<usize>, _, cx| {
            let theme = vt_ui::theme::theme(cx).clone();
            let entries = overlay::entries(console.role());
            let cursor = console.list(PALETTE_LIST, cx).cursor;
            let Some(palette) = console.palette.as_ref() else {
                return Vec::new();
            };
            range
                .filter_map(|row| Some((row, *palette.results.get(row)?)))
                .filter_map(|(row, index)| {
                    let entry = entries.get(index)?;
                    let active = cursor == Some(row);
                    let hover = theme.hover;
                    Some(
                        div()
                            .id(("palette-row", row))
                            .relative()
                            .flex()
                            .items_center()
                            .gap(SPACE_5)
                            .h(px(PALETTE_ROW))
                            .w_full()
                            .px(SPACE_5)
                            .rounded(RADIUS_MD)
                            .cursor_pointer()
                            .when(active, |this| this.bg(theme.selected))
                            .when(!active, |this| this.hover(move |style| style.bg(hover)))
                            .on_click(cx.listener(move |console, _, window, cx| {
                                console.run_palette(Some(row), window, cx);
                            }))
                            .child(vt_ui::icon(entry.icon, ICON_MD, theme.fg_muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(entry.title.clone()),
                            )
                            .when(!entry.keys.is_empty(), |this| {
                                this.child(kbd(entry.keys.clone(), cx))
                            }),
                    )
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(scroll)
    .px(SPACE_3)
    .h(px(PALETTE_ROW * 9.0));

    div()
        .key_context(MENU)
        .on_action(cx.listener(|console, _: &ListUp, _, cx| {
            console.list_move(PALETTE_LIST, Move::Up, cx);
        }))
        .on_action(cx.listener(|console, _: &ListDown, _, cx| {
            console.list_move(PALETTE_LIST, Move::Down, cx);
        }))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .p(SPACE_6)
                .border_b_1()
                .border_color(theme.border)
                .child(vt_ui::icon(vt_ui::Icon::Search, ICON_LG, theme.fg_dim))
                .child(div().flex_1().child(input))
                .child(kbd(hint(&TogglePalette).unwrap_or_default(), cx)),
        )
        .child(div().py(SPACE_3).child(if count == 0 {
            div()
                .h(px(PALETTE_ROW * 2.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_dim)
                .child("Nothing matches")
                .into_any_element()
        } else {
            list.into_any_element()
        }))
        .child(footer(
            &[("↑↓", "Move"), ("↵", "Run"), ("Esc", "Close")],
            format!("{count} results"),
            cx,
        ))
        .into_any_element()
}

fn footer(
    keys: &[(&'static str, &'static str)],
    note: String,
    cx: &Context<Console>,
) -> impl IntoElement {
    let theme = theme(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .px(SPACE_6)
        .h(px(34.0))
        .border_t_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_dim)
        .children(keys.iter().map(|(keys, label)| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(kbd(*keys, cx))
                .child(*label)
        }))
        .child(div().flex_1())
        .child(note)
}

/// `?`: every shortcut, generated from the command table and the page
/// registry.
pub fn help(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let rows = help_rows();
    let scroll = console.scroll("help");
    let groups = Group::ALL.into_iter().filter_map(|group| {
        let lines: Vec<_> = rows.iter().filter(|(g, _, _)| *g == group).collect();
        (!lines.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .child(section_label(group.title(), cx))
                .children(lines.into_iter().map(|(_, label, keys)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .min_h(px(24.0))
                        .child(div().flex_1().min_w_0().child(*label))
                        .children(keys.iter().map(|keys| kbd(keys.clone(), cx)))
                }))
        })
    });
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .px(SPACE_8)
                .h(px(52.0))
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Keyboard shortcuts"),
                )
                .child(kbd("?", cx)),
        )
        .child(
            div()
                .id("help-groups")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .flex()
                .flex_col()
                .gap(SPACE_8)
                .px(SPACE_8)
                .py(SPACE_7)
                .children(groups),
        )
        .child(footer(
            &[("Esc", "Close"), ("Tab", "Next control")],
            String::new(),
            cx,
        ))
        .into_any_element()
}

fn open_more_row(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(page) = overlay::more_pages(console.role()).get(row).copied() {
        console.show(page, window, cx);
    }
}

/// The More menu: pages without a tab.
pub fn more(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let pages: Vec<Page> = overlay::more_pages(console.role());
    let keys = ListKeys {
        id: MORE_LIST,
        on_move: None,
        on_activate: open_more_row,
    };
    let list = keyboard_list(keys, pages.len(), pages.len(), console, cx);
    let nav = console.list(MORE_LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let rows: Vec<AnyElement> = pages
        .iter()
        .enumerate()
        .map(|(row, page)| {
            let spec = crate::console::pages::spec(*page);
            let ring = focused && cursor == Some(row);
            let active = console.page == *page;
            let hover = theme.hover;
            div()
                .id(("more-row", row))
                .relative()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(30.0))
                .mx(SPACE_3)
                .px(SPACE_4)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .when(active, |this| this.bg(theme.selected))
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |console, _, window, cx| {
                    open_more_row(console, row, false, window, cx);
                }))
                .child(vt_ui::icon(spec.icon, ICON_MD, theme.fg_muted))
                .child(div().flex_1().child(SharedString::from(spec.title)))
                .when_some(spec.digit, |this, digit| {
                    this.child(kbd(
                        crate::console::keys::display(&crate::console::keys::page_keys(digit)),
                        cx,
                    ))
                })
                .when(ring, |this| this.child(focus_ring(&theme)))
                .into_any_element()
        })
        .collect();
    list.flex().flex_col().children(rows).into_any_element()
}

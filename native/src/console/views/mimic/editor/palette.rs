//! The palette (`ScadaPalette.tsx`): the symbol library by category, drawn
//! by the same painter as the stage, and the widget types, filtered by the
//! search field (/). A click adds at the stage's centre, a drag drops at the
//! pointer; Enter in the search adds the first match.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Render, SharedString, Window, canvas, div, px};
use voltrus_scada_native::mimic::edit::{self, CATEGORIES, Manifest, Placing};
use voltrus_scada_native::mimic::{self, Reading, SymbolDef};
use vt_ui::components::section_label;
use vt_ui::theme::*;

use super::super::paint::{Canvas, Hmi, Tessellations, Xf};
use super::super::scene::Item;
use super::super::{State, symbols};
use super::{add, edits_ref};
use crate::console::Console;

/// A thumbnail's side, window pixels.
const THUMB: f32 = 40.0;
/// A tile's width: three to a row.
const TILE_WIDTH: f32 = 74.0;

/// What a palette drag carries, drawn under the pointer.
pub(super) struct Dragged(Placing);

impl Render for Dragged {
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
            .child(self.0.name())
    }
}

pub(super) fn render(
    console: &mut Console,
    _: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let hmi = Hmi::new(&theme);
    let Some(editor) = edits_ref(console) else {
        return div().into_any_element();
    };
    let input = editor.query.clone();
    let query = input.read(cx).value().to_owned();
    let Some(cache) = console
        .state_ref::<State>()
        .map(|state| state.cache.clone())
    else {
        return div().into_any_element();
    };

    let mut sections: Vec<AnyElement> = Vec::new();
    for category in CATEGORIES {
        let defs: Vec<&'static SymbolDef> = edit::palette_symbols(&query)
            .filter(|def| def.category == category)
            .collect();
        if defs.is_empty() {
            continue;
        }
        sections.push(heading(edit::category_name(category), cx));
        let tiles: Vec<AnyElement> = defs
            .into_iter()
            .map(|def| tile(def, hmi, cache.clone(), &theme, cx))
            .collect();
        sections.push(
            div()
                .flex()
                .flex_wrap()
                .gap(SPACE_2)
                .pb(SPACE_4)
                .children(tiles)
                .into_any_element(),
        );
    }
    let widgets: Vec<&'static Manifest> = edit::palette_widgets()
        .filter(|manifest| edit::matches(&query, &[manifest.name]))
        .collect();
    if !widgets.is_empty() {
        sections.push(heading("Widgets", cx));
        sections.push(
            div()
                .pb(SPACE_3)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child("Widgets show tags: bind them in the inspector.")
                .into_any_element(),
        );
        for manifest in widgets {
            sections.push(row(manifest, &theme, cx));
        }
    }
    if sections.is_empty() {
        sections.push(
            div()
                .py(SPACE_6)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child("Nothing matches.")
                .into_any_element(),
        );
    }

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
                .p(SPACE_4)
                .border_b_1()
                .border_color(theme.border)
                .child(input),
        )
        .child(
            div()
                .id("mimic-palette")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(SPACE_4)
                .pt(SPACE_3)
                .pb(SPACE_8)
                .children(sections),
        )
        .into_any_element()
}

fn heading(text: &'static str, cx: &Context<Console>) -> AnyElement {
    div()
        .pb(SPACE_3)
        .child(section_label(text, cx))
        .into_any_element()
}

/// No values: thumbnails draw the symbols at rest.
fn nothing(_: i64) -> Option<Reading> {
    None
}

fn tile(
    def: &'static SymbolDef,
    hmi: Hmi,
    cache: Rc<Tessellations>,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let placing = Placing::Symbol(def.key);
    let accent = theme.accent;
    div()
        .id(SharedString::from(format!("mimic-palette-{}", def.key)))
        .flex()
        .flex_col()
        .items_center()
        .gap(SPACE_2)
        .w(px(TILE_WIDTH))
        .px(SPACE_1)
        .py(SPACE_3)
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(theme.border)
        .bg(hmi.stage)
        .cursor_grab()
        .hover(move |style| style.border_color(accent))
        .on_click(cx.listener(move |console, _, window, cx| {
            add(console, placing, None, window, cx);
        }))
        .on_drag(placing, |placing, _, _, cx| cx.new(|_| Dragged(*placing)))
        .child(thumbnail(def, hmi, cache))
        .child(
            div()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_muted)
                .text_center()
                .child(def.name),
        )
        .into_any_element()
}

/// The symbol at rest, fitted in a square.
fn thumbnail(def: &'static SymbolDef, hmi: Hmi, cache: Rc<Tessellations>) -> impl IntoElement {
    let widget = Placing::Symbol(def.key).widget(String::new(), 0.0, 0.0);
    let live = mimic::symbol_live(&widget, def, &nothing);
    let symbol = mimic::resolve(&widget).map(|resolved| (resolved, live));
    let item = Item {
        widget,
        tags: Vec::new(),
        primary: None,
        symbol,
        selected: false,
        cursor: false,
        hovered: false,
    };
    canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            #[allow(clippy::cast_possible_truncation)]
            let (w, h) = (item.widget.width as f32, item.widget.height as f32);
            let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let scale = (bw / w).min(bh / h);
            let x = f32::from(bounds.origin.x) + (bw - w * scale) / 2.0;
            let y = f32::from(bounds.origin.y) + (bh - h * scale) / 2.0;
            let xf = Xf::translate(x, y).then(&Xf::scale(scale, scale));
            let mut canvas = Canvas {
                window,
                cx,
                hmi: &hmi,
                zoom: scale,
                alpha: 1.0,
                cache: &cache,
            };
            symbols::paint(&mut canvas, &item, &xf, 0.0);
        },
    )
    .w(px(THUMB))
    .h(px(THUMB))
}

fn row(manifest: &'static Manifest, theme: &Theme, cx: &mut Context<Console>) -> AnyElement {
    let placing = Placing::Widget(manifest.kind);
    let hover = theme.hover;
    div()
        .id(SharedString::from(format!("mimic-palette-{}", manifest.kind)))
        .flex()
        .items_center()
        .gap(SPACE_4)
        .h(px(28.0))
        .px(SPACE_3)
        .rounded(RADIUS_MD)
        .cursor_grab()
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |console, _, window, cx| {
            add(console, placing, None, window, cx);
        }))
        .on_drag(placing, |placing, _, _, cx| cx.new(|_| Dragged(*placing)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(FONT_BODY)
                .text_color(theme.fg)
                .child(manifest.name),
        )
        .child(
            div()
                .flex_none()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(manifest.data),
        )
        .into_any_element()
}

/// Enter in the search: add the first match.
pub(super) fn add_first(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(editor) = edits_ref(console) else {
        return;
    };
    let query = editor.query.read(cx).value().to_owned();
    let first = edit::palette_symbols(&query)
        .next()
        .map(|def| Placing::Symbol(def.key))
        .or_else(|| {
            edit::palette_widgets()
                .find(|manifest| edit::matches(&query, &[manifest.name]))
                .map(|manifest| Placing::Widget(manifest.kind))
        });
    if let Some(placing) = first {
        add(console, placing, None, window, cx);
    }
}

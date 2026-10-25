//! Screens and their card dashboards: the server's rows, and the card grid
//! the web client lays them out on (react-grid-layout with vertical
//! compaction), as pure functions. No UI here: the console draws them
//! (`console/screens.rs` holds the session's screens, `views/dashboard.rs`
//! draws the grid), the end-to-end test drives the endpoints with these
//! types, and the mimic viewer builds on the same screen list.
//!
//! # The server's side (`/api/v1`)
//!
//! | What | Endpoint | Role |
//! |---|---|---|
//! | The screens | `GET /screens` → `[Screen]` | any |
//! | A screen's tags | `GET /screens/{id}/data-sources` → `[DataSource]` in screen order | any |
//! | Its card layouts | `GET /screens/{id}/layouts` → [`ScreenLayouts`] | any |
//! | Create, rename, delete | `POST /screens {name}` → `Screen`; `PUT /screens/{id} {name}`; `DELETE /screens/{id}` | operator |
//! | Add or remove a tag | `POST /screens/{id}/data-sources {data_source_id}`; `DELETE /screens/{id}/data-sources/{ds}` | operator |
//! | Save the layouts | `PUT /screens/{id}/layouts` with every breakpoint | operator |
//!
//! The overview ([`OVERVIEW`]) is no row of `/screens`: it shows every tag
//! (`GET /data-sources`) and keeps its layouts under [`OVERVIEW_LAYOUTS`].
//! The server stores `lg`, `md` and `sm` and replaces all of them on every
//! `PUT`, so a save always sends the whole [`ScreenLayouts`].
//!
//! # The grid
//!
//! Cards sit on a grid of [`Breakpoint::cols`] columns (24 on a wide
//! window) and rows of [`ROW_HEIGHT`] pixels, [`GAP`] apart: a card is
//! `{i, x, y, w, h}` in cells, `i` its data source id. Like the web client
//! the grid compacts upwards ([`compact`]): no card floats below a gap. A
//! card moved onto others pushes them down, or lets one hop above it when
//! there is room ([`place`]); [`auto_arrange`] packs the cards row by row,
//! grouped by device.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};

/// The web client's id for the overview screen (`?screen=all`).
pub const OVERVIEW: &str = "all";
/// Where the server keeps the overview's layouts.
pub const OVERVIEW_LAYOUTS: &str = "__overview";

/// A grid row's height in pixels (the web client's `rowHeight`).
pub const ROW_HEIGHT: f32 = 50.0;
/// The space between cards in pixels (the web client's default gap).
pub const GAP: f32 = 12.0;
/// The smallest card a resize leaves, in cells.
pub const MIN_W: u32 = 2;
pub const MIN_H: u32 = 2;

/// Mirrors `voltrus::models::Device`, which the server lists as screens.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Screen {
    pub id: String,
    pub name: String,
    /// Always `screen` for the rows `/screens` answers.
    #[serde(rename = "type")]
    pub kind: String,
    pub sort_order: i64,
}

/// The id a screen's layouts are kept under: the overview's is
/// [`OVERVIEW_LAYOUTS`].
pub fn layout_id(screen: &str) -> &str {
    if screen == OVERVIEW {
        OVERVIEW_LAYOUTS
    } else {
        screen
    }
}

/// `GET` and `PUT` path of a screen's layouts.
pub fn layouts_path(screen: &str) -> String {
    format!("/api/v1/screens/{}/layouts", layout_id(screen))
}

/// `GET` path of a screen's tags; the overview has every tag.
pub fn tags_path(screen: &str) -> String {
    if screen == OVERVIEW {
        "/api/v1/data-sources".to_owned()
    } else {
        format!("/api/v1/screens/{screen}/data-sources")
    }
}

/// One card's cell rectangle, as react-grid-layout stores it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutItem {
    /// The card's data source id, as text.
    #[serde(deserialize_with = "text_or_number")]
    pub i: String,
    #[serde(deserialize_with = "cell")]
    pub x: u32,
    #[serde(deserialize_with = "cell")]
    pub y: u32,
    #[serde(deserialize_with = "cell")]
    pub w: u32,
    #[serde(deserialize_with = "cell")]
    pub h: u32,
}

impl LayoutItem {
    pub fn new(id: i64, x: u32, y: u32, w: u32, h: u32) -> Self {
        Self {
            i: id.to_string(),
            x,
            y,
            w,
            h,
        }
    }

    /// The data source id this card shows.
    pub fn id(&self) -> Option<i64> {
        self.i.parse().ok()
    }

    fn bottom(&self) -> u32 {
        self.y + self.h
    }
}

/// An id stored as `"12"` or `12`.
fn text_or_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Text(String),
        Number(serde_json::Number),
    }
    Ok(match Id::deserialize(deserializer)? {
        Id::Text(text) => text,
        Id::Number(number) => number.to_string(),
    })
}

/// A cell coordinate: a whole number of cells, never negative.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn cell<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let value = f64::deserialize(deserializer)?;
    Ok(if value.is_finite() && value > 0.0 {
        value.round().min(f64::from(u32::MAX)) as u32
    } else {
        0
    })
}

/// The web client's responsive breakpoints, widest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Breakpoint {
    Lg,
    Md,
    Sm,
    Xs,
}

impl Breakpoint {
    pub const ALL: [Self; 4] = [Self::Lg, Self::Md, Self::Sm, Self::Xs];

    pub fn cols(self) -> u32 {
        match self {
            Self::Lg => 24,
            Self::Md => 12,
            Self::Sm => 4,
            Self::Xs => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Lg => "lg",
            Self::Md => "md",
            Self::Sm => "sm",
            Self::Xs => "xs",
        }
    }

    /// The breakpoint of a grid `width` pixels wide: wider than 1200 is
    /// `lg`, than 768 `md`, than 480 `sm` (react-grid-layout's rule).
    pub fn for_width(width: f32) -> Self {
        if width > 1200.0 {
            Self::Lg
        } else if width > 768.0 {
            Self::Md
        } else if width > 480.0 {
            Self::Sm
        } else {
            Self::Xs
        }
    }

    fn position(self) -> usize {
        Self::ALL.iter().position(|bp| *bp == self).unwrap_or(0)
    }
}

/// A screen's card layouts, one per breakpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenLayouts {
    #[serde(deserialize_with = "items")]
    pub lg: Vec<LayoutItem>,
    #[serde(deserialize_with = "items")]
    pub md: Vec<LayoutItem>,
    #[serde(deserialize_with = "items")]
    pub sm: Vec<LayoutItem>,
    #[serde(deserialize_with = "items")]
    pub xs: Vec<LayoutItem>,
}

/// A breakpoint's items; `null` (a stored layout the server could not
/// parse) reads as none.
fn items<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<LayoutItem>, D::Error> {
    Ok(Option::<Vec<LayoutItem>>::deserialize(deserializer)?.unwrap_or_default())
}

impl ScreenLayouts {
    pub fn get(&self, bp: Breakpoint) -> &[LayoutItem] {
        match bp {
            Breakpoint::Lg => &self.lg,
            Breakpoint::Md => &self.md,
            Breakpoint::Sm => &self.sm,
            Breakpoint::Xs => &self.xs,
        }
    }

    pub fn set(&mut self, bp: Breakpoint, items: Vec<LayoutItem>) {
        match bp {
            Breakpoint::Lg => self.lg = items,
            Breakpoint::Md => self.md = items,
            Breakpoint::Sm => self.sm = items,
            Breakpoint::Xs => self.xs = items,
        }
    }
}

/// What a card draws, from its tag's `chart_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    Sparkline,
    Gauge,
    BigNumber,
    Image,
    Text,
}

impl CardKind {
    /// Unknown and empty chart types draw a sparkline, as on the web.
    pub fn of(chart_type: &str) -> Self {
        match chart_type {
            "gauge" => Self::Gauge,
            "bignumber" => Self::BigNumber,
            "image" => Self::Image,
            "text" => Self::Text,
            _ => Self::Sparkline,
        }
    }

    /// Whether the card shows its tag's status and value (images and text
    /// do not).
    pub fn shows_status(self) -> bool {
        !matches!(self, Self::Image | Self::Text)
    }

    /// The cells the card takes when arranged on `bp` (the web client's
    /// `CHART_SIZES`, narrowed on small breakpoints).
    pub fn size(self, bp: Breakpoint) -> (u32, u32) {
        let (w, h) = match self {
            Self::Gauge => (4, 4),
            Self::Sparkline => (6, 3),
            Self::BigNumber => (4, 3),
            Self::Image | Self::Text => (6, 4),
        };
        match bp {
            Breakpoint::Sm => (w.min(4), h.max(3)),
            Breakpoint::Xs => (2, 3),
            _ => (w, h),
        }
    }
}

/// A card to lay out: its tag, the device it groups by, what it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card<'a> {
    pub id: i64,
    pub device: &'a str,
    pub kind: CardKind,
}

/// The web client's first layout for a screen without one: every card 6×4
/// (2 wide on `xs`, 4 on `sm`), row after row.
pub fn default_layout(cards: &[Card], bp: Breakpoint) -> Vec<LayoutItem> {
    let cols = bp.cols();
    let w = match bp {
        Breakpoint::Lg | Breakpoint::Md => 6,
        Breakpoint::Sm => 4,
        Breakpoint::Xs => 2,
    }
    .min(cols);
    let h = 4;
    let (mut x, mut y) = (0, 0);
    cards
        .iter()
        .map(|card| {
            if x + w > cols {
                x = 0;
                y += h;
            }
            let item = LayoutItem::new(card.id, x, y, w, h);
            x += w;
            item
        })
        .collect()
}

/// Pack `cards` row by row from row `top`, each device's cards starting a
/// new row, each card its kind's size.
fn pack(cards: &[&Card], bp: Breakpoint, top: u32) -> Vec<LayoutItem> {
    let cols = bp.cols();
    let mut items = Vec::with_capacity(cards.len());
    let mut groups: Vec<(&str, Vec<&Card>)> = Vec::new();
    for card in cards {
        match groups.iter_mut().find(|(device, _)| *device == card.device) {
            Some((_, group)) => group.push(*card),
            None => groups.push((card.device, vec![*card])),
        }
    }
    let mut next_row = top;
    for (_, group) in groups {
        let (mut x, mut row, mut tallest) = (0, next_row, 0);
        for card in group {
            let (w, h) = card.kind.size(bp);
            let w = w.min(cols);
            if x + w > cols {
                x = 0;
                row += tallest;
                tallest = 0;
            }
            items.push(LayoutItem::new(card.id, x, row, w, h));
            tallest = tallest.max(h);
            x += w;
        }
        next_row = row + tallest;
    }
    items
}

/// Auto-arrange (the web client's Arrange button): the cards packed row by
/// row in list order, grouped by device, each its kind's size.
pub fn auto_arrange(cards: &[Card], bp: Breakpoint) -> Vec<LayoutItem> {
    let all: Vec<&Card> = cards.iter().collect();
    pack(&all, bp, 0)
}

/// [`auto_arrange`] on every breakpoint.
pub fn auto_arrange_all(cards: &[Card]) -> ScreenLayouts {
    ScreenLayouts {
        lg: auto_arrange(cards, Breakpoint::Lg),
        md: auto_arrange(cards, Breakpoint::Md),
        sm: auto_arrange(cards, Breakpoint::Sm),
        xs: auto_arrange(cards, Breakpoint::Xs),
    }
}

/// Every breakpoint as the grid shows it ([`effective`]): what an edit
/// starts from, so a saved layout places every card on every breakpoint.
pub fn materialize(layouts: &ScreenLayouts, cards: &[Card]) -> ScreenLayouts {
    ScreenLayouts {
        lg: effective(layouts, Breakpoint::Lg, cards),
        md: effective(layouts, Breakpoint::Md, cards),
        sm: effective(layouts, Breakpoint::Sm, cards),
        xs: effective(layouts, Breakpoint::Xs, cards),
    }
}

/// The cards as the grid shows them on `bp`: the saved items of the cards
/// still on the screen, kept inside the columns and compacted, then the
/// cards without an item packed below them.
///
/// Without any saved `lg` layout every breakpoint gets the
/// [`default_layout`]; a breakpoint without items borrows the nearest wider
/// one's (react-grid-layout's rule).
pub fn effective(layouts: &ScreenLayouts, bp: Breakpoint, cards: &[Card]) -> Vec<LayoutItem> {
    if layouts.lg.is_empty() {
        return default_layout(cards, bp);
    }
    let cols = bp.cols();
    let saved = if layouts.get(bp).is_empty() {
        Breakpoint::ALL[..bp.position()]
            .iter()
            .rev()
            .map(|wider| layouts.get(*wider))
            .find(|items| !items.is_empty())
            .unwrap_or(layouts.lg.as_slice())
    } else {
        layouts.get(bp)
    };
    let positions: HashMap<i64, usize> = cards
        .iter()
        .enumerate()
        .map(|(position, card)| (card.id, position))
        .collect();
    let mut placed = vec![false; cards.len()];
    let mut items: Vec<LayoutItem> = Vec::with_capacity(cards.len());
    for item in saved {
        let Some(position) = item.id().and_then(|id| positions.get(&id).copied()) else {
            continue;
        };
        if placed[position] {
            continue;
        }
        placed[position] = true;
        let w = item.w.clamp(1, cols);
        items.push(LayoutItem {
            i: item.i.clone(),
            x: item.x.min(cols - w),
            y: item.y,
            w,
            h: item.h.max(1),
        });
    }
    compact(&mut items);
    let missing: Vec<&Card> = cards
        .iter()
        .zip(&placed)
        .filter(|(_, placed)| !**placed)
        .map(|(card, _)| card)
        .collect();
    let top = bottom(&items);
    items.extend(pack(&missing, bp, top));
    items
}

/// The first row below every item.
pub fn bottom(items: &[LayoutItem]) -> u32 {
    items.iter().map(LayoutItem::bottom).max().unwrap_or(0)
}

/// Whether two different cards overlap.
pub fn collides(a: &LayoutItem, b: &LayoutItem) -> bool {
    a.i != b.i && a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// The first of `among` (positions in `items`) that overlaps `item`.
fn first_collision(items: &[LayoutItem], among: &[usize], item: &LayoutItem) -> Option<usize> {
    among
        .iter()
        .copied()
        .find(|other| collides(&items[*other], item))
}

/// Float every card up as far as it goes, top row first and left to right
/// within a row (react-grid-layout's vertical compactor). The order of
/// `items` stays.
pub fn compact(items: &mut [LayoutItem]) {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|index| (items[*index].y, items[*index].x));
    let mut placed: Vec<usize> = Vec::with_capacity(items.len());
    let mut floor = 0;
    for index in order {
        let mut item = items[index].clone();
        item.y = item.y.min(floor);
        while item.y > 0 {
            let above = LayoutItem {
                y: item.y - 1,
                ..item.clone()
            };
            if first_collision(items, &placed, &above).is_some() {
                break;
            }
            item.y -= 1;
        }
        while let Some(other) = first_collision(items, &placed, &item) {
            item.y = items[other].bottom();
        }
        floor = floor.max(item.bottom());
        items[index] = item;
        placed.push(index);
    }
}

/// Push every card that overlaps another down below it, the card at
/// `fixed` first and the rest top row first.
fn push_down(items: &mut [LayoutItem], fixed: usize) {
    let mut order: Vec<usize> = (0..items.len()).filter(|index| *index != fixed).collect();
    order.sort_by_key(|index| (items[*index].y, items[*index].x));
    let mut placed = vec![fixed];
    for index in order {
        while let Some(other) = first_collision(items, &placed, &items[index]) {
            items[index].y = items[other].bottom();
        }
        placed.push(index);
    }
}

fn index_of(items: &[LayoutItem], id: &str) -> Option<usize> {
    items.iter().position(|item| item.i == id)
}

/// Move card `id` to cell `(x, y)`, as a drag does: each card it lands on
/// hops above it when there is room there, the others move down, and the
/// grid compacts. Out-of-range cells are clamped.
pub fn place(items: &[LayoutItem], id: &str, x: u32, y: u32, cols: u32) -> Vec<LayoutItem> {
    let mut out = items.to_vec();
    let Some(index) = index_of(&out, id) else {
        return out;
    };
    let w = out[index].w.clamp(1, cols);
    out[index].w = w;
    out[index].x = x.min(cols - w);
    out[index].y = y;
    let moved = out[index].clone();
    let mut hit: Vec<usize> = (0..out.len())
        .filter(|other| *other != index && collides(&out[*other], &moved))
        .collect();
    hit.sort_by_key(|other| (out[*other].y, out[*other].x));
    for other in hit {
        let hop = LayoutItem {
            y: moved.y.saturating_sub(out[other].h),
            ..out[other].clone()
        };
        if !out
            .iter()
            .enumerate()
            .any(|(at, item)| at != other && collides(item, &hop))
        {
            out[other] = hop;
        }
    }
    push_down(&mut out, index);
    compact(&mut out);
    out
}

/// Give card `id` the size `w`×`h` (at least [`MIN_W`]×[`MIN_H`], inside
/// the columns); cards in the way move down and the grid compacts.
pub fn resize(items: &[LayoutItem], id: &str, w: u32, h: u32, cols: u32) -> Vec<LayoutItem> {
    let mut out = items.to_vec();
    let Some(index) = index_of(&out, id) else {
        return out;
    };
    let room = cols - out[index].x.min(cols - 1);
    out[index].w = w.max(MIN_W).min(room).max(1);
    out[index].h = h.max(MIN_H);
    push_down(&mut out, index);
    compact(&mut out);
    out
}

/// A direction of the arrow keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

fn spans_overlap(a: u32, a_len: u32, b: u32, b_len: u32) -> bool {
    a < b + b_len && b < a + a_len
}

/// The card the arrow `direction` goes to from card `id`: the nearest one
/// wholly on that side, preferring cards level with it.
pub fn neighbor<'a>(
    items: &'a [LayoutItem],
    id: &str,
    direction: Direction,
) -> Option<&'a LayoutItem> {
    let from = &items[index_of(items, id)?];
    let center = |item: &LayoutItem, vertical: bool| {
        if vertical {
            2 * item.y + item.h
        } else {
            2 * item.x + item.w
        }
    };
    items
        .iter()
        .filter(|item| item.i != from.i)
        .filter_map(|item| {
            let (gap, level) = match direction {
                Direction::Right => (
                    item.x.checked_sub(from.x + from.w)?,
                    spans_overlap(item.y, item.h, from.y, from.h),
                ),
                Direction::Left => (
                    from.x.checked_sub(item.x + item.w)?,
                    spans_overlap(item.y, item.h, from.y, from.h),
                ),
                Direction::Down => (
                    item.y.checked_sub(from.y + from.h)?,
                    spans_overlap(item.x, item.w, from.x, from.w),
                ),
                Direction::Up => (
                    from.y.checked_sub(item.y + item.h)?,
                    spans_overlap(item.x, item.w, from.x, from.w),
                ),
            };
            let vertical = matches!(direction, Direction::Left | Direction::Right);
            let off_axis = center(item, vertical).abs_diff(center(from, vertical));
            Some(((!level, gap, off_axis), item))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, item)| item)
}

/// The keyboard's move of card `id` one step in `direction`: up swaps it
/// with the card above, down with the card below; left and right step one
/// column, or trade places with a card beside it in the same row.
pub fn nudge(items: &[LayoutItem], id: &str, direction: Direction, cols: u32) -> Vec<LayoutItem> {
    let Some(index) = index_of(items, id) else {
        return items.to_vec();
    };
    let card = &items[index];
    match direction {
        Direction::Up => {
            if card.y == 0 {
                items.to_vec()
            } else {
                place(items, id, card.x, card.y - 1, cols)
            }
        }
        Direction::Down => {
            let below = items
                .iter()
                .filter(|item| {
                    item.i != card.i
                        && item.y >= card.bottom()
                        && spans_overlap(item.x, item.w, card.x, card.w)
                })
                .min_by_key(|item| (item.y, item.x));
            match below {
                Some(below) => {
                    let y = (below.bottom().saturating_sub(card.h)).max(card.y + 1);
                    place(items, id, card.x, y, cols)
                }
                None => items.to_vec(),
            }
        }
        Direction::Left | Direction::Right => {
            let right = direction == Direction::Right;
            let adjacent = |item: &LayoutItem| {
                if right {
                    item.x == card.x + card.w
                } else {
                    item.x + item.w == card.x
                }
            };
            let beside = items
                .iter()
                .position(|item| item.i != card.i && item.y == card.y && adjacent(item));
            if let Some(other) = beside {
                let mut out = items.to_vec();
                if right {
                    out[other].x = card.x;
                    out[index].x = card.x + out[other].w;
                } else {
                    out[index].x = items[other].x;
                    out[other].x = items[other].x + card.w;
                }
                push_down(&mut out, index);
                compact(&mut out);
                out
            } else if right && card.x + card.w < cols {
                place(items, id, card.x + 1, card.y, cols)
            } else if !right && card.x > 0 {
                place(items, id, card.x - 1, card.y, cols)
            } else {
                items.to_vec()
            }
        }
    }
}

/// The keyboard's resize of card `id`: right and down grow it a cell, left
/// and up shrink it.
pub fn grow(items: &[LayoutItem], id: &str, direction: Direction, cols: u32) -> Vec<LayoutItem> {
    let Some(card) = index_of(items, id).map(|index| &items[index]) else {
        return items.to_vec();
    };
    let (w, h) = match direction {
        Direction::Right => (card.w + 1, card.h),
        Direction::Left => (card.w.saturating_sub(1), card.h),
        Direction::Down => (card.w, card.h + 1),
        Direction::Up => (card.w, card.h.saturating_sub(1)),
    };
    resize(items, id, w, h, cols)
}

/// A rectangle in pixels from the grid's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// The grid's pixel geometry: `cols` columns across `width` pixels, cards
/// [`GAP`] apart and rows [`ROW_HEIGHT`] tall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub cols: u32,
    pub width: f32,
}

impl Grid {
    pub fn new(bp: Breakpoint, width: f32) -> Self {
        Self {
            cols: bp.cols(),
            width,
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn col_width(&self) -> f32 {
        let cols = self.cols.max(1) as f32;
        ((self.width - GAP * (cols - 1.0)) / cols).max(1.0)
    }

    /// Where `item` sits.
    #[allow(clippy::cast_precision_loss)]
    pub fn rect(&self, item: &LayoutItem) -> Rect {
        let col = self.col_width();
        Rect {
            x: item.x as f32 * (col + GAP),
            y: item.y as f32 * (ROW_HEIGHT + GAP),
            w: item.w as f32 * col + item.w.saturating_sub(1) as f32 * GAP,
            h: item.h as f32 * ROW_HEIGHT + item.h.saturating_sub(1) as f32 * GAP,
        }
    }

    /// The height of `rows` rows.
    #[allow(clippy::cast_precision_loss)]
    pub fn height(&self, rows: u32) -> f32 {
        rows as f32 * ROW_HEIGHT + rows.saturating_sub(1) as f32 * GAP
    }

    /// The cell nearest a card of `w` columns whose corner is at `(left,
    /// top)` pixels, inside the columns.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn cell(&self, left: f32, top: f32, w: u32) -> (u32, u32) {
        let x = (left / (self.col_width() + GAP)).round().max(0.0) as u32;
        let y = (top / (ROW_HEIGHT + GAP)).round().max(0.0) as u32;
        (x.min(self.cols.saturating_sub(w.min(self.cols))), y)
    }

    /// The cells nearest a card `width`×`height` pixels starting at column
    /// `x`: at least [`MIN_W`]×[`MIN_H`], inside the columns.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn span(&self, x: u32, width: f32, height: f32) -> (u32, u32) {
        let w = ((width + GAP) / (self.col_width() + GAP)).round().max(0.0) as u32;
        let h = ((height + GAP) / (ROW_HEIGHT + GAP)).round().max(0.0) as u32;
        let room = self.cols.saturating_sub(x).max(1);
        (w.max(MIN_W).min(room), h.max(MIN_H))
    }
}

/// The image format of fetched bytes, as a MIME type: from the server's
/// `Content-Type` when it names an image, else from the bytes' signature.
pub fn image_mime(content_type: Option<&str>, bytes: &[u8]) -> Option<&'static str> {
    let declared = content_type
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .and_then(|mime| match mime.as_str() {
            "image/png" => Some("image/png"),
            "image/jpeg" | "image/jpg" => Some("image/jpeg"),
            "image/gif" => Some("image/gif"),
            "image/webp" => Some("image/webp"),
            "image/svg+xml" => Some("image/svg+xml"),
            "image/bmp" => Some("image/bmp"),
            "image/tiff" => Some("image/tiff"),
            _ => None,
        });
    declared.or_else(|| {
        let text_start = String::from_utf8_lossy(&bytes[..bytes.len().min(256)])
            .trim_start()
            .to_ascii_lowercase();
        if bytes.starts_with(b"\x89PNG") {
            Some("image/png")
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Some("image/jpeg")
        } else if bytes.starts_with(b"GIF8") {
            Some("image/gif")
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
            Some("image/webp")
        } else if bytes.starts_with(b"BM") {
            Some("image/bmp")
        } else if text_start.starts_with("<svg") || text_start.starts_with("<?xml") {
            Some("image/svg+xml")
        } else {
            None
        }
    })
}

/// Fetch an image card's picture: `url` absolute, or a path on the server
/// (sent with the session's cookie, like the browser would). The answer:
/// the bytes and their MIME type.
pub async fn fetch_image(
    api: &crate::api::VoltrusApi,
    url: &str,
) -> anyhow::Result<(Vec<u8>, &'static str)> {
    let own = url.starts_with('/') || url.starts_with(&api.base);
    let full = if url.starts_with('/') {
        format!("{}{url}", api.base)
    } else {
        url.to_owned()
    };
    let mut request = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?
        .get(&full);
    if own && let Some(token) = &api.token {
        request = request.header("Cookie", format!("{}={token}", crate::api::TOKEN_COOKIE));
    }
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("the image answered {status}");
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response.bytes().await?.to_vec();
    let mime = image_mime(content_type.as_deref(), &bytes)
        .ok_or_else(|| anyhow::anyhow!("not an image the console can draw"))?;
    Ok((bytes, mime))
}

/// The settings an image or text card reads from its tag's `source_config`
/// (JSON text): `image_url` and `text_content`.
pub fn card_config(source_config: &str, field: &str) -> String {
    serde_json::from_str::<serde_json::Value>(source_config)
        .ok()
        .and_then(|config| config.get(field)?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, x: u32, y: u32, w: u32, h: u32) -> LayoutItem {
        LayoutItem::new(id, x, y, w, h)
    }

    fn cards(kinds: &[(i64, &'static str, CardKind)]) -> Vec<Card<'static>> {
        kinds
            .iter()
            .map(|(id, device, kind)| Card {
                id: *id,
                device: *device,
                kind: *kind,
            })
            .collect()
    }

    fn at(items: &[LayoutItem], id: i64) -> (u32, u32, u32, u32) {
        let found = items
            .iter()
            .find(|item| item.i == id.to_string())
            .expect("the card is laid out");
        (found.x, found.y, found.w, found.h)
    }

    #[test]
    fn the_servers_screens_and_layouts_decode() {
        // `GET /api/v1/screens`: `voltrus::models::Device` rows.
        let screens: Vec<Screen> = serde_json::from_str(
            r#"[{"id":"sma_inv_1","name":"SMA INV 1","type":"screen","sort_order":0},
                {"id":"grid_meter","name":"GRID METER","type":"screen","sort_order":1}]"#,
        )
        .unwrap();
        assert_eq!(screens[0].id, "sma_inv_1");
        assert_eq!(screens[1].kind, "screen");
        assert_eq!(screens[1].sort_order, 1);
        // `GET /screens/{id}/layouts`: lg, md and sm, each as react-grid-layout
        // saved it (extra keys, `i` as text), or `null` when unreadable.
        let layouts: ScreenLayouts = serde_json::from_str(
            r#"{"lg":[{"i":"7","x":0,"y":0,"w":6,"h":3,"moved":false,"static":false}],
                "md":[{"i":8,"x":2.0,"y":-1,"w":4,"h":4}],
                "sm":null}"#,
        )
        .unwrap();
        assert_eq!(layouts.lg, [item(7, 0, 0, 6, 3)]);
        assert_eq!(layouts.md, [item(8, 2, 0, 4, 4)], "numbers and negatives");
        assert!(layouts.sm.is_empty() && layouts.xs.is_empty());
        // A save sends every breakpoint; the server keeps lg, md and sm.
        let saved = serde_json::to_value(&layouts).unwrap();
        assert_eq!(saved["lg"][0]["i"], "7");
        assert!(saved["sm"].as_array().unwrap().is_empty());
    }

    #[test]
    fn the_overview_has_its_own_paths() {
        assert_eq!(layouts_path(OVERVIEW), "/api/v1/screens/__overview/layouts");
        assert_eq!(layouts_path("plts_1"), "/api/v1/screens/plts_1/layouts");
        assert_eq!(tags_path(OVERVIEW), "/api/v1/data-sources");
        assert_eq!(tags_path("plts_1"), "/api/v1/screens/plts_1/data-sources");
    }

    #[test]
    fn widths_pick_the_web_clients_breakpoints() {
        assert_eq!(Breakpoint::for_width(1392.0), Breakpoint::Lg);
        assert_eq!(Breakpoint::for_width(1200.0), Breakpoint::Md, "wider than");
        assert_eq!(Breakpoint::for_width(976.0), Breakpoint::Md);
        assert_eq!(Breakpoint::for_width(600.0), Breakpoint::Sm);
        assert_eq!(Breakpoint::for_width(300.0), Breakpoint::Xs);
        assert_eq!(Breakpoint::Lg.cols(), 24);
        assert_eq!(CardKind::of(""), CardKind::Sparkline);
        assert_eq!(CardKind::of("bignumber"), CardKind::BigNumber);
        assert!(!CardKind::of("text").shows_status());
    }

    #[test]
    fn auto_arrange_packs_rows_by_device_like_the_web() {
        let list = cards(&[
            (1, "inv", CardKind::Gauge),
            (2, "inv", CardKind::Sparkline),
            (3, "inv", CardKind::BigNumber),
            (4, "inv", CardKind::Sparkline),
            (5, "inv", CardKind::Image),
            (6, "meter", CardKind::Sparkline),
        ]);
        let lg = auto_arrange(&list, Breakpoint::Lg);
        assert_eq!(at(&lg, 1), (0, 0, 4, 4));
        assert_eq!(at(&lg, 2), (4, 0, 6, 3));
        assert_eq!(at(&lg, 3), (10, 0, 4, 3));
        assert_eq!(at(&lg, 4), (14, 0, 6, 3));
        // 20 + 6 > 24: a new row below the tallest card of the first.
        assert_eq!(at(&lg, 5), (0, 4, 6, 4));
        // Another device starts below the last row.
        assert_eq!(at(&lg, 6), (0, 8, 6, 3));
        let xs = auto_arrange(&list, Breakpoint::Xs);
        assert_eq!(at(&xs, 2), (0, 3, 2, 3));
        let sm = auto_arrange(&list, Breakpoint::Sm);
        assert_eq!(at(&sm, 2), (0, 4, 4, 3), "4 wide at most, 3 tall at least");
        assert_eq!(auto_arrange_all(&list).md.len(), 6);
    }

    #[test]
    fn a_screen_without_layouts_gets_the_default_grid() {
        let list = cards(&[
            (1, "a", CardKind::Gauge),
            (2, "a", CardKind::Gauge),
            (3, "a", CardKind::Gauge),
            (4, "a", CardKind::Gauge),
            (5, "a", CardKind::Gauge),
        ]);
        let items = effective(&ScreenLayouts::default(), Breakpoint::Lg, &list);
        assert_eq!(at(&items, 4), (18, 0, 6, 4));
        assert_eq!(at(&items, 5), (0, 4, 6, 4));
        let all = materialize(&ScreenLayouts::default(), &list);
        assert_eq!(all.lg, items);
        assert_eq!(all.xs, default_layout(&list, Breakpoint::Xs));
        assert_eq!(
            effective(&all, Breakpoint::Md, &list),
            all.md,
            "a materialized layout shows as saved"
        );
    }

    #[test]
    fn saved_layouts_are_kept_compacted_and_completed() {
        let layouts = ScreenLayouts {
            lg: vec![
                item(1, 0, 3, 6, 3),
                item(2, 20, 0, 8, 2),
                item(99, 0, 0, 6, 3),
                item(1, 6, 0, 6, 3),
            ],
            ..ScreenLayouts::default()
        };
        let list = cards(&[
            (1, "a", CardKind::Sparkline),
            (2, "a", CardKind::Sparkline),
            (3, "a", CardKind::Gauge),
        ]);
        let items = effective(&layouts, Breakpoint::Lg, &list);
        assert_eq!(items.len(), 3, "a gone tag drops, a duplicate counts once");
        assert_eq!(at(&items, 1), (0, 0, 6, 3), "floats up to the top");
        assert_eq!(at(&items, 2), (16, 0, 8, 2), "kept inside 24 columns");
        assert_eq!(at(&items, 3), (0, 3, 4, 4), "a new tag goes below");
        // md has no items: it borrows lg's, narrowed to 12 columns.
        let md = effective(&layouts, Breakpoint::Md, &list);
        assert_eq!(at(&md, 2), (4, 0, 8, 2));
        assert_eq!(
            at(&md, 1),
            (0, 2, 6, 3),
            "below card 2, which shares columns"
        );
    }

    #[test]
    fn compaction_closes_gaps_without_overlaps() {
        let mut items = vec![
            item(1, 0, 5, 6, 2),
            item(2, 0, 9, 12, 2),
            item(3, 6, 1, 6, 4),
        ];
        compact(&mut items);
        assert_eq!(at(&items, 3), (6, 0, 6, 4));
        assert_eq!(at(&items, 1), (0, 0, 6, 2));
        assert_eq!(at(&items, 2), (0, 4, 12, 2), "below the taller card");
    }

    #[test]
    fn dragging_a_card_onto_another_makes_room() {
        let items = vec![
            item(1, 0, 0, 6, 4),
            item(2, 0, 4, 6, 4),
            item(3, 6, 0, 6, 4),
        ];
        // Down onto the card below by its whole height: it hops above.
        let moved = place(&items, "1", 0, 4, 24);
        assert_eq!(at(&moved, 2), (0, 0, 6, 4));
        assert_eq!(at(&moved, 1), (0, 4, 6, 4));
        // Less than that: the card comes back when it is let go.
        assert_eq!(place(&items, "1", 0, 2, 24), items);
        // Up onto the card above: they trade places.
        let up = place(&items, "2", 0, 3, 24);
        assert_eq!(at(&up, 2), (0, 0, 6, 4));
        assert_eq!(at(&up, 1), (0, 4, 6, 4));
        // Sideways onto a neighbour: it moves down out of the way.
        let side = place(&items, "1", 4, 0, 24);
        assert_eq!(at(&side, 1), (4, 0, 6, 4));
        assert_eq!(at(&side, 3), (6, 4, 6, 4));
        assert_eq!(at(&side, 2), (0, 4, 6, 4));
        // Clamped inside the columns.
        assert_eq!(at(&place(&items, "3", 40, 0, 24), 3), (18, 0, 6, 4));
        assert_eq!(place(&items, "404", 0, 0, 24), items);
    }

    #[test]
    fn resizing_pushes_cards_down_and_keeps_a_minimum() {
        let items = vec![item(1, 0, 0, 6, 4), item(2, 6, 0, 6, 4)];
        let wider = resize(&items, "1", 8, 4, 24);
        assert_eq!(at(&wider, 1), (0, 0, 8, 4));
        assert_eq!(at(&wider, 2), (6, 4, 6, 4));
        let tiny = resize(&items, "2", 0, 0, 24);
        assert_eq!(at(&tiny, 2), (6, 0, MIN_W, MIN_H));
        let huge = resize(&items, "2", 99, 5, 24);
        assert_eq!(at(&huge, 2), (6, 0, 18, 5), "no wider than the room left");
    }

    #[test]
    fn arrow_keys_move_and_resize_the_focused_card() {
        let items = vec![
            item(1, 0, 0, 6, 4),
            item(2, 6, 0, 6, 4),
            item(3, 0, 4, 6, 3),
        ];
        let right = nudge(&items, "1", Direction::Right, 24);
        assert_eq!(
            at(&right, 1),
            (6, 0, 6, 4),
            "trades places with its neighbour"
        );
        assert_eq!(at(&right, 2), (0, 0, 6, 4));
        let further = nudge(&items, "2", Direction::Right, 24);
        assert_eq!(at(&further, 2), (7, 0, 6, 4), "free room: one column");
        let down = nudge(&items, "1", Direction::Down, 24);
        assert_eq!(at(&down, 3), (0, 0, 6, 3), "the card below comes up");
        assert_eq!(at(&down, 1), (0, 3, 6, 4));
        let up = nudge(&items, "3", Direction::Up, 24);
        assert_eq!(at(&up, 3), (0, 0, 6, 3));
        assert_eq!(at(&up, 1), (0, 3, 6, 4));
        assert_eq!(
            nudge(&items, "1", Direction::Up, 24),
            items,
            "top row stays"
        );
        assert_eq!(
            nudge(&items, "1", Direction::Left, 24),
            items,
            "left edge stays"
        );
        let wide = grow(&items, "2", Direction::Right, 24);
        assert_eq!(at(&wide, 2), (6, 0, 7, 4));
        let short = grow(&items, "1", Direction::Up, 24);
        assert_eq!(at(&short, 1), (0, 0, 6, 3));
        assert_eq!(at(&short, 3), (0, 3, 6, 3), "the card below follows up");
    }

    #[test]
    fn arrows_find_the_nearest_card_on_that_side() {
        let items = vec![
            item(1, 0, 0, 6, 4),
            item(2, 6, 0, 6, 4),
            item(3, 18, 0, 6, 2),
            item(4, 0, 4, 12, 3),
            item(5, 12, 4, 6, 3),
        ];
        let go =
            |from: &str, direction| neighbor(&items, from, direction).map(|item| item.i.clone());
        assert_eq!(go("1", Direction::Right).as_deref(), Some("2"));
        assert_eq!(go("2", Direction::Right).as_deref(), Some("3"));
        assert_eq!(go("3", Direction::Right), None);
        assert_eq!(go("2", Direction::Down).as_deref(), Some("4"));
        assert_eq!(
            go("3", Direction::Down).as_deref(),
            Some("5"),
            "the nearest below when none is level"
        );
        assert_eq!(go("5", Direction::Left).as_deref(), Some("4"));
        assert_eq!(go("4", Direction::Up).as_deref(), Some("1"));
        assert_eq!(go("9", Direction::Up), None);
    }

    #[test]
    fn the_grid_maps_cells_to_pixels_and_back() {
        let grid = Grid::new(Breakpoint::Lg, 24.0 * 40.0 + 23.0 * GAP);
        assert!((grid.col_width() - 40.0).abs() < 1e-3);
        let rect = grid.rect(&item(1, 2, 1, 3, 2));
        assert!((rect.x - 104.0).abs() < 1e-3);
        assert!((rect.y - 62.0).abs() < 1e-3);
        assert!((rect.w - 144.0).abs() < 1e-3);
        assert!((rect.h - 112.0).abs() < 1e-3);
        assert!((grid.height(2) - 112.0).abs() < 1e-3);
        assert!(grid.height(0).abs() < 1e-6);
        assert_eq!(grid.cell(rect.x + 10.0, rect.y - 20.0, 3), (2, 1));
        assert_eq!(grid.cell(-50.0, -50.0, 3), (0, 0));
        assert_eq!(grid.cell(5000.0, 0.0, 6), (18, 0), "inside the columns");
        assert_eq!(grid.span(2, rect.w, rect.h), (3, 2));
        assert_eq!(grid.span(20, 1000.0, 10.0), (4, MIN_H));
    }

    #[test]
    fn image_and_text_cards_read_their_settings() {
        assert_eq!(
            card_config(r#"{"image_url":"/uploads/plant.png"}"#, "image_url"),
            "/uploads/plant.png"
        );
        assert_eq!(card_config("not json", "text_content"), "");
        assert_eq!(card_config("{}", "text_content"), "");
        assert_eq!(
            image_mime(Some("image/png; charset=binary"), b""),
            Some("image/png")
        );
        assert_eq!(image_mime(None, b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(
            image_mime(Some("text/html"), b"\xff\xd8\xff\xe0"),
            Some("image/jpeg")
        );
        assert_eq!(image_mime(None, b"  <svg xmlns=..."), Some("image/svg+xml"));
        assert_eq!(image_mime(None, b"<html>"), None);
    }
}

//! The overlay slot: one modal, sheet or popover at a time above the page,
//! with a focus trap (Tab cycles inside it) and Esc to close. The command
//! palette (⌘K), the shortcut help (`?`) and the More menu use it; later
//! dialogs (shelve, export, settings) open their own [`Overlay`] the same
//! way: `console.open_overlay(Overlay::modal(..), window, cx)`.
//!
//! Every overlay fits the window: [`place`] caps its width and height to
//! the window less [`MARGIN`] and keeps a popover inside it (flipped to
//! its button's other edge, else shifted), and the frame scrolls what
//! still does not fit.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Action, App, Bounds, Entity, FocusHandle, Pixels, ScrollHandle, SharedString, Size,
    Subscription, px,
};
use vt_ui::Icon;
use vt_ui::text_input::TextInput;
use vt_ui::theme::TITLE_BAR_HEIGHT;

use super::keys::{COMMANDS, display, page_keys};
use super::pages::{PAGES, Page, Render, Role};

/// Where a button was last drawn, for the popover it opens (set by a
/// `canvas` in the button's prepaint).
pub type Anchor = Rc<Cell<Option<Bounds<Pixels>>>>;

/// Which edge of its button a popover lines up with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// How an overlay sits on the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayKind {
    /// Centred near the top over a dimmed page; a click outside closes it.
    Modal { width: Pixels },
    /// Along the right edge, full height, over a dimmed page.
    Sheet { width: Pixels },
    /// Below its button, lined up with the button's `align` edge, over an
    /// undimmed page.
    Popover { width: Pixels, align: Align },
}

pub struct Overlay {
    /// Names it, e.g. for `is_open`.
    pub id: &'static str,
    pub kind: OverlayKind,
    /// A popover's button, read again on every draw so the popover follows
    /// it (a resize, the navbar stepping down).
    pub anchor: Option<Anchor>,
    /// Draws the overlay's content; the frame (backdrop, card, trap) is
    /// the shell's.
    pub render: Render,
    /// The trap: focus stays inside the element tracking this handle. The
    /// overlay focuses it on opening unless its content focuses a field.
    pub focus: FocusHandle,
    /// The card's scroll, when its content is taller than the window
    /// allows.
    pub scroll: ScrollHandle,
}

impl Overlay {
    pub fn modal(id: &'static str, width: Pixels, render: Render, cx: &mut App) -> Self {
        Self::new(id, OverlayKind::Modal { width }, None, render, cx)
    }

    pub fn sheet(id: &'static str, width: Pixels, render: Render, cx: &mut App) -> Self {
        Self::new(id, OverlayKind::Sheet { width }, None, render, cx)
    }

    pub fn popover(
        id: &'static str,
        width: Pixels,
        anchor: Anchor,
        align: Align,
        render: Render,
        cx: &mut App,
    ) -> Self {
        Self::new(
            id,
            OverlayKind::Popover { width, align },
            Some(anchor),
            render,
            cx,
        )
    }

    fn new(
        id: &'static str,
        kind: OverlayKind,
        anchor: Option<Anchor>,
        render: Render,
        cx: &mut App,
    ) -> Self {
        Self {
            id,
            kind,
            anchor,
            render,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }
}

/// Room an overlay keeps from the window's edges.
pub const MARGIN: Pixels = px(12.0);
/// A modal's height at most, however tall the window.
const MODAL_MAX_HEIGHT: Pixels = px(560.0);
/// A modal's distance from the window's top at most (less in a short
/// window, never into the navbar).
const MODAL_TOP: Pixels = px(96.0);
/// Between a popover and its button.
const POPOVER_GAP: Pixels = px(4.0);

/// Where an overlay's card goes in the window, and how big it may be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub left: Pixels,
    pub top: Pixels,
    pub width: Pixels,
    pub max_height: Pixels,
}

/// Place an overlay of `kind` in a window of `viewport`, a popover below
/// its button (`anchor`; before the button is drawn, under the navbar at
/// the window's edge). Width and height are capped to the window less
/// [`MARGIN`]; a popover that would cross the window's edge lines up
/// with its button's other edge if that fits, else is shifted inside.
pub fn place(kind: OverlayKind, anchor: Option<Bounds<Pixels>>, viewport: Size<Pixels>) -> Placed {
    let room = |width: Pixels| width.min((viewport.width - MARGIN * 2.0).max(px(0.0)));
    match kind {
        OverlayKind::Modal { width } => {
            let width = room(width);
            let top = (viewport.height * 0.12).clamp(TITLE_BAR_HEIGHT + MARGIN, MODAL_TOP);
            Placed {
                left: ((viewport.width - width) / 2.0).max(px(0.0)),
                top,
                width,
                max_height: MODAL_MAX_HEIGHT.min((viewport.height - top - MARGIN).max(px(0.0))),
            }
        }
        OverlayKind::Sheet { width } => {
            let width = room(width);
            Placed {
                left: viewport.width - width,
                top: TITLE_BAR_HEIGHT,
                width,
                max_height: (viewport.height - TITLE_BAR_HEIGHT).max(px(0.0)),
            }
        }
        OverlayKind::Popover { width, align } => {
            let width = room(width);
            let right_edge = viewport.width - MARGIN;
            let (top, left) = match anchor {
                Some(button) => {
                    let left_aligned = button.left();
                    let right_aligned = button.right() - width;
                    let left = match align {
                        Align::Left if left_aligned + width > right_edge => {
                            if right_aligned >= MARGIN {
                                right_aligned
                            } else {
                                left_aligned
                            }
                        }
                        Align::Left => left_aligned,
                        Align::Right if right_aligned < MARGIN => {
                            if left_aligned + width <= right_edge {
                                left_aligned
                            } else {
                                right_aligned
                            }
                        }
                        Align::Right => right_aligned,
                    };
                    (button.bottom() + POPOVER_GAP, left)
                }
                None => (
                    TITLE_BAR_HEIGHT + POPOVER_GAP,
                    match align {
                        Align::Left => MARGIN,
                        Align::Right => right_edge - width,
                    },
                ),
            };
            // Shifted inside: off the right edge first, then off the left.
            let left = left.min(right_edge - width).max(MARGIN);
            let top = top.min((viewport.height - MARGIN).max(px(0.0)));
            Placed {
                left,
                top,
                width,
                max_height: (viewport.height - top - MARGIN).max(px(0.0)),
            }
        }
    }
}

/// The ⌘K palette's state while it is open.
pub struct Palette {
    pub input: Entity<TextInput>,
    /// Positions in [`entries`] matching the query, best first.
    pub results: Vec<usize>,
    pub _subscription: Subscription,
}

/// One thing the palette can do.
pub struct Entry {
    pub title: SharedString,
    /// The shortcut, if any.
    pub keys: SharedString,
    pub icon: Icon,
    pub action: Box<dyn Action>,
}

/// Everything the palette offers a user with `role`: every page they may
/// open, then every palette command.
pub fn entries(role: Option<Role>) -> Vec<Entry> {
    let allowed = |needed: Role| role.is_some_and(|role| role >= needed);
    let pages = PAGES
        .iter()
        .filter(|spec| super::pages::opens(spec, role))
        .map(|spec| Entry {
            title: format!("Go to {}", spec.title).into(),
            keys: spec
                .digit
                .map(|digit| display(&page_keys(digit)))
                .unwrap_or_default()
                .into(),
            icon: spec.icon,
            action: Box::new(super::keys::ShowPage(spec.page)),
        });
    let commands = COMMANDS
        .iter()
        .filter(|command| command.palette && allowed(command.role))
        .map(|command| Entry {
            title: command.label.into(),
            keys: command
                .keys
                .first()
                .map(|keys| display(keys))
                .unwrap_or_default()
                .into(),
            icon: command.icon,
            action: (command.action)(),
        });
    pages.chain(commands).collect()
}

/// Positions of the entries whose title matches `query`, best first; all of
/// them, in order, for an empty query.
pub fn search(entries: &[Entry], query: &str) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| score(query, &entry.title).map(|score| (score, index)))
        .collect();
    // Stable: equal scores keep table order.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, index)| index).collect()
}

/// A fuzzy match: the query's characters in order, case-insensitive.
/// Matches at word starts and in runs score higher, gaps lower.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.chars().collect();
    let mut wanted = 0;
    let mut last: Option<usize> = None;
    let mut score = 0_i32;
    for (index, character) in text.iter().enumerate() {
        if wanted == query.len() {
            break;
        }
        if !character.to_lowercase().eq(std::iter::once(query[wanted])) {
            continue;
        }
        let word_start = index == 0 || !text[index - 1].is_alphanumeric();
        score += 10;
        if word_start {
            score += 8;
        }
        match last {
            Some(previous) if previous + 1 == index => score += 6,
            Some(previous) => {
                score -= i32::try_from((index - previous - 1).min(8)).unwrap_or(8);
            }
            None => {}
        }
        last = Some(index);
        wanted += 1;
    }
    (wanted == query.len()).then_some(score)
}

/// The More menu's pages: those without a tab that `role` may open.
pub fn more_pages(role: Option<Role>) -> Vec<Page> {
    PAGES
        .iter()
        .filter(|spec| {
            spec.placement == super::pages::Placement::More && super::pages::opens(spec, role)
        })
        .map(|spec| spec.page)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fuzzy_score_prefers_word_starts_and_runs() {
        assert_eq!(score("", "anything"), Some(0));
        assert!(score("ack", "Acknowledge all alarms").is_some());
        assert!(score("gtr", "Go to Trends").is_some());
        assert_eq!(score("xyz", "Go to Trends"), None);
        assert_eq!(
            score("trendsx", "Go to Trends"),
            None,
            "every character must match"
        );
        // "al" at the start of "alarms" beats the scattered a…l.
        assert!(score("al", "Go to Alarms") > score("al", "Toggle a fill"));
        assert!(score("TRE", "go to trends").is_some(), "case-insensitive");
    }

    #[test]
    fn the_palette_offers_pages_and_commands_by_role() {
        let none = entries(None);
        assert!(none.is_empty(), "nothing before signing in");
        let all = entries(Some(Role::Any));
        assert_eq!(all[0].title.as_ref(), "Go to Overview");
        assert!(
            all.iter()
                .any(|entry| entry.title.as_ref() == "Acknowledge all alarms")
        );
        assert!(
            !all.iter()
                .any(|entry| entry.title.as_ref() == "Command palette"),
            "the palette does not list itself"
        );
        let found = search(&all, "trends");
        assert_eq!(all[found[0]].title.as_ref(), "Go to Trends");
        assert_eq!(search(&all, "").len(), all.len());
        // The Data Manager has sections open to every user (System, EDR).
        assert_eq!(
            more_pages(Some(Role::Any)),
            [Page::Performance, Page::DataManager, Page::Commands]
        );
        assert!(more_pages(None).is_empty());
    }

    fn inside(placed: Placed, viewport: Size<Pixels>) {
        assert!(placed.left >= px(0.0), "{placed:?}");
        assert!(placed.left + placed.width <= viewport.width, "{placed:?}");
        assert!(placed.top >= px(0.0), "{placed:?}");
        assert!(
            placed.top + placed.max_height <= viewport.height,
            "{placed:?}"
        );
    }

    #[test]
    fn every_overlay_fits_the_window() {
        let windows = [
            gpui::size(px(1440.0), px(900.0)),
            gpui::size(px(1280.0), px(800.0)),
            gpui::size(px(1024.0), px(684.0)),
            gpui::size(px(1024.0), px(640.0)),
            gpui::size(px(700.0), px(480.0)),
            gpui::size(px(300.0), px(200.0)),
        ];
        let button = |left: f32| {
            Bounds::new(
                gpui::point(px(left), px(8.0)),
                gpui::size(px(22.0), px(22.0)),
            )
        };
        for viewport in windows {
            for width in [240.0, 380.0, 600.0, 2000.0] {
                let width = px(width);
                for kind in [
                    OverlayKind::Modal { width },
                    OverlayKind::Sheet { width },
                    OverlayKind::Popover {
                        width,
                        align: Align::Left,
                    },
                    OverlayKind::Popover {
                        width,
                        align: Align::Right,
                    },
                ] {
                    inside(place(kind, None, viewport), viewport);
                    for left in [
                        0.0,
                        30.0,
                        f32::from(viewport.width) / 2.0,
                        f32::from(viewport.width) - 22.0,
                    ] {
                        let placed = place(kind, Some(button(left)), viewport);
                        inside(placed, viewport);
                        if matches!(kind, OverlayKind::Popover { .. })
                            && viewport.width > width + MARGIN * 2.0
                        {
                            assert!(
                                placed.left >= MARGIN
                                    && placed.left + placed.width <= viewport.width - MARGIN,
                                "{placed:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_popover_near_the_edge_flips_to_its_buttons_other_edge() {
        let viewport = gpui::size(px(1024.0), px(684.0));
        let width = px(240.0);
        // A menu button near the right edge: the menu ends at its right.
        let near_right = Bounds::new(
            gpui::point(px(900.0), px(8.0)),
            gpui::size(px(60.0), px(26.0)),
        );
        let placed = place(
            OverlayKind::Popover {
                width,
                align: Align::Left,
            },
            Some(near_right),
            viewport,
        );
        assert_eq!(placed.left, near_right.right() - width);
        assert_eq!(placed.top, near_right.bottom() + px(4.0));
        // A right-aligned menu under a button near the left edge starts at its left.
        let near_left = Bounds::new(
            gpui::point(px(20.0), px(8.0)),
            gpui::size(px(22.0), px(22.0)),
        );
        let placed = place(
            OverlayKind::Popover {
                width,
                align: Align::Right,
            },
            Some(near_left),
            viewport,
        );
        assert_eq!(placed.left, near_left.left());
        // In the open it lines up as asked.
        let middle = Bounds::new(
            gpui::point(px(400.0), px(8.0)),
            gpui::size(px(60.0), px(26.0)),
        );
        let placed = place(
            OverlayKind::Popover {
                width,
                align: Align::Left,
            },
            Some(middle),
            viewport,
        );
        assert_eq!(placed.left, px(400.0));
        // Its height is what is left below the button.
        assert_eq!(placed.max_height, viewport.height - placed.top - MARGIN);
    }

    #[test]
    fn a_modal_moves_up_and_shrinks_in_a_short_window() {
        let tall = place(
            OverlayKind::Modal { width: px(600.0) },
            None,
            gpui::size(px(1440.0), px(900.0)),
        );
        assert_eq!((tall.top, tall.max_height), (px(96.0), px(560.0)));
        let short = place(
            OverlayKind::Modal { width: px(600.0) },
            None,
            gpui::size(px(1024.0), px(640.0)),
        );
        assert!(short.top < px(96.0) && short.top >= TITLE_BAR_HEIGHT + MARGIN);
        assert_eq!(short.max_height, px(640.0) - short.top - MARGIN);
    }
}

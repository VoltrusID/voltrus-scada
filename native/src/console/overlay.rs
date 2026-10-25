//! The overlay slot: one modal, sheet or popover at a time above the page,
//! with a focus trap (Tab cycles inside it) and Esc to close. The command
//! palette (⌘K), the shortcut help (`?`) and the More menu use it; later
//! dialogs (shelve, export, settings) open their own [`Overlay`] the same
//! way: `console.open_overlay(Overlay::modal(..), window, cx)`.

use gpui::{Action, App, Entity, FocusHandle, Pixels, Point, SharedString, Subscription};
use vt_ui::Icon;
use vt_ui::text_input::TextInput;

use super::keys::{COMMANDS, display, page_keys};
use super::pages::{PAGES, Page, Render, Role};

/// How an overlay sits on the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayKind {
    /// Centred near the top over a dimmed page; a click outside closes it.
    Modal { width: Pixels },
    /// Along the right edge, full height, over a dimmed page.
    Sheet { width: Pixels },
    /// Below a point (a menu button), over an undimmed page.
    Popover { width: Pixels, at: Point<Pixels> },
}

pub struct Overlay {
    /// Names it, e.g. for `is_open`.
    pub id: &'static str,
    pub kind: OverlayKind,
    /// Draws the overlay's content; the frame (backdrop, card, trap) is
    /// the shell's.
    pub render: Render,
    /// The trap: focus stays inside the element tracking this handle. The
    /// overlay focuses it on opening unless its content focuses a field.
    pub focus: FocusHandle,
}

impl Overlay {
    pub fn modal(id: &'static str, width: Pixels, render: Render, cx: &mut App) -> Self {
        Self {
            id,
            kind: OverlayKind::Modal { width },
            render,
            focus: cx.focus_handle(),
        }
    }

    pub fn sheet(id: &'static str, width: Pixels, render: Render, cx: &mut App) -> Self {
        Self {
            id,
            kind: OverlayKind::Sheet { width },
            render,
            focus: cx.focus_handle(),
        }
    }

    pub fn popover(
        id: &'static str,
        width: Pixels,
        at: Point<Pixels>,
        render: Render,
        cx: &mut App,
    ) -> Self {
        Self {
            id,
            kind: OverlayKind::Popover { width, at },
            render,
            focus: cx.focus_handle(),
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
        .filter(|spec| allowed(spec.role))
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
    scored.sort_by(|a, b| b.0.cmp(&a.0));
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
            spec.placement == super::pages::Placement::More
                && role.is_some_and(|role| role >= spec.role)
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
        assert_eq!(more_pages(Some(Role::Any)), [Page::Performance]);
        assert!(more_pages(None).is_empty());
    }
}

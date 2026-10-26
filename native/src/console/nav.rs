//! Keyboard lists: a focusable list with a cursor row. j/k and the arrows
//! move it, PgUp/PgDn a screenful, Home/End to the ends, Enter and Space act
//! on the row (see `views::keyboard_list`, which wires the keys).

use gpui::{App, FocusHandle, ScrollHandle, ScrollStrategy, UniformListScrollHandle};

/// A cursor move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
    PageUp,
    PageDown,
    First,
    Last,
}

/// Where the cursor lands: `None` without rows; the first row from no
/// cursor; otherwise clamped to the list.
pub fn moved(cursor: Option<usize>, len: usize, step: Move, page: usize) -> Option<usize> {
    let last = len.checked_sub(1)?;
    let page = page.max(1);
    let Some(at) = cursor.map(|at| at.min(last)) else {
        return Some(match step {
            Move::Last => last,
            _ => 0,
        });
    };
    Some(match step {
        Move::Up => at.saturating_sub(1),
        Move::Down => (at + 1).min(last),
        Move::PageUp => at.saturating_sub(page),
        Move::PageDown => (at + page).min(last),
        Move::First => 0,
        Move::Last => last,
    })
}

pub struct ListNav {
    pub focus: FocusHandle,
    /// The row the keys act on; drawn with a focus ring while the list has
    /// focus.
    pub cursor: Option<usize>,
    /// Rows at the last render.
    pub len: usize,
    /// Rows in a screenful, for PgUp/PgDn.
    pub page: usize,
    /// For `uniform_list`s: scrolled to keep the cursor in view.
    pub scroll: UniformListScrollHandle,
    /// For lists of plain rows (menus): the scrolling element whose
    /// children are the rows tracks it, and it keeps the cursor in view.
    pub rows: ScrollHandle,
}

impl ListNav {
    pub fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            cursor: None,
            len: 0,
            page: 10,
            scroll: UniformListScrollHandle::new(),
            rows: ScrollHandle::new(),
        }
    }

    /// Record the rows this render shows; a cursor past the end moves to
    /// the last row.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.cursor = len
            .checked_sub(1)
            .and_then(|last| self.cursor.map(|at| at.min(last)));
    }

    /// Move the cursor and scroll it into view; the new row.
    pub fn step(&mut self, step: Move) -> Option<usize> {
        self.cursor = moved(self.cursor, self.len, step, self.page);
        if let Some(row) = self.cursor {
            self.reveal(row);
        }
        self.cursor
    }

    /// Put the cursor on `row` (a click), scrolled into view.
    pub fn select(&mut self, row: usize) {
        self.cursor = Some(row);
        self.reveal(row);
    }

    /// Scroll `row` into view, whichever kind of list tracks the handles.
    pub fn reveal(&self, row: usize) {
        self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        self.rows.scroll_to_item(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_starts_at_an_end_and_stays_inside() {
        assert_eq!(moved(None, 0, Move::Down, 10), None, "no rows, no cursor");
        assert_eq!(moved(None, 5, Move::Down, 10), Some(0));
        assert_eq!(moved(None, 5, Move::Up, 10), Some(0));
        assert_eq!(moved(None, 5, Move::Last, 10), Some(4));
        assert_eq!(moved(Some(4), 5, Move::Down, 10), Some(4));
        assert_eq!(moved(Some(0), 5, Move::Up, 10), Some(0));
        assert_eq!(moved(Some(2), 50, Move::PageDown, 10), Some(12));
        assert_eq!(moved(Some(45), 50, Move::PageDown, 10), Some(49));
        assert_eq!(moved(Some(5), 50, Move::PageUp, 10), Some(0));
        assert_eq!(moved(Some(9), 50, Move::First, 10), Some(0));
        assert_eq!(
            moved(Some(9), 3, Move::Up, 10),
            Some(1),
            "a stale cursor is clamped first"
        );
        assert_eq!(
            moved(Some(3), 9, Move::PageDown, 0),
            Some(4),
            "a page is at least a row"
        );
    }
}

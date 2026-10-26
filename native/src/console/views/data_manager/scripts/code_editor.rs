//! A code editor for the Scripts section: monospace, a line-number gutter,
//! the caret's line highlighted, JavaScript colours ([`super::js_syntax`]),
//! no soft wrap (long lines scroll sideways), Tab and ⇧Tab indenting,
//! Enter keeping the line's indent (one more after `{`, `[` or `(`), a
//! closing bracket typed on a blank line stepping back, ⌘/ commenting
//! lines, word moves, undo and redo. Only the lines on screen are shaped
//! and coloured.
//!
//! Built like `console::text_area` (which stays the plain multi-line field
//! of the shift handover): it answers the same `text_input` actions, so its
//! key context is `TextInput CodeEditor` and the `!TextInput` keys stay off
//! while typing; its own keys live in the `CodeEditor` context
//! ([`key_bindings`], which `shortcuts::rebind` includes in the keymap).
//! Emits `Changed`, `Submitted` (⌘↵),
//! `Cancelled` (Esc) and `Blurred` like `TextInput`. IME-aware through
//! `EntityInputHandler`.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun, UTF16Selection,
    UnderlineStyle, Window, actions, div, fill, point, px, relative, size,
};
use vt_ui::text_input::{
    Backspace, Cancel, Copy, Cut, Delete, End, Home, InputEvent, Left, Paste, Right, SelectAll,
    SelectEnd, SelectHome, SelectLeft, SelectRight, Submit,
};
use vt_ui::theme::*;

use super::js_syntax::{self, Kind, LineState};

actions!(
    code_editor,
    [
        Up,
        Down,
        SelectUp,
        SelectDown,
        PageUp,
        PageDown,
        SelectPageUp,
        SelectPageDown,
        DocumentStart,
        DocumentEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DeleteWordLeft,
        DeleteToLineStart,
        Indent,
        Outdent,
        ToggleComment,
        Undo,
        Redo,
        Commit,
    ]
);

/// The editor's key context: `TextInput` for the shared bindings (and so
/// that `!TextInput` keys stay off while typing), `CodeEditor` for its own.
const KEY_CONTEXT: &str = "TextInput CodeEditor";
const CONTEXT: &str = "CodeEditor";
const LINE_HEIGHT: Pixels = px(18.0);
/// Spaces per indent level (the web editor's `tabSize: 2`).
pub(crate) const INDENT: usize = 2;
/// Undo steps kept.
const HISTORY: usize = 200;

/// The keys only the code editor has. `shortcuts::rebind` must include
/// them when it rebuilds the keymap.
pub fn key_bindings() -> Vec<KeyBinding> {
    let mac = cfg!(target_os = "macos");
    let word = if mac { "alt" } else { "ctrl" };
    let mut bindings = vec![
        KeyBinding::new("up", Up, Some(CONTEXT)),
        KeyBinding::new("down", Down, Some(CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("pageup", PageUp, Some(CONTEXT)),
        KeyBinding::new("pagedown", PageDown, Some(CONTEXT)),
        KeyBinding::new("shift-pageup", SelectPageUp, Some(CONTEXT)),
        KeyBinding::new("shift-pagedown", SelectPageDown, Some(CONTEXT)),
        KeyBinding::new("secondary-up", DocumentStart, Some(CONTEXT)),
        KeyBinding::new("secondary-down", DocumentEnd, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-up", SelectDocumentStart, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-down", SelectDocumentEnd, Some(CONTEXT)),
        KeyBinding::new(&format!("{word}-left"), WordLeft, Some(CONTEXT)),
        KeyBinding::new(&format!("{word}-right"), WordRight, Some(CONTEXT)),
        KeyBinding::new(&format!("{word}-shift-left"), SelectWordLeft, Some(CONTEXT)),
        KeyBinding::new(
            &format!("{word}-shift-right"),
            SelectWordRight,
            Some(CONTEXT),
        ),
        KeyBinding::new(&format!("{word}-backspace"), DeleteWordLeft, Some(CONTEXT)),
        KeyBinding::new("tab", Indent, Some(CONTEXT)),
        KeyBinding::new("shift-tab", Outdent, Some(CONTEXT)),
        KeyBinding::new("secondary-/", ToggleComment, Some(CONTEXT)),
        KeyBinding::new("secondary-z", Undo, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-z", Redo, Some(CONTEXT)),
        KeyBinding::new("secondary-enter", Commit, Some(CONTEXT)),
    ];
    if mac {
        bindings.push(KeyBinding::new(
            "cmd-backspace",
            DeleteToLineStart,
            Some(CONTEXT),
        ));
    } else {
        bindings.push(KeyBinding::new("ctrl-y", Redo, Some(CONTEXT)));
        bindings.push(KeyBinding::new("ctrl-home", DocumentStart, Some(CONTEXT)));
        bindings.push(KeyBinding::new("ctrl-end", DocumentEnd, Some(CONTEXT)));
    }
    bindings
}

/// What an edit was, so a run of typing undoes as one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

struct Snapshot {
    content: String,
    selection: Range<usize>,
}

pub struct CodeEditor {
    focus_handle: FocusHandle,
    content: String,
    /// Byte offset of each line's first character.
    starts: Vec<usize>,
    /// Characters in the longest line: how far the text scrolls sideways.
    widest: usize,
    /// The tokenizer's state at the start of each line, for a prefix of
    /// the lines (cut back at an edit, extended as lines are drawn).
    states: Vec<LineState>,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    /// The caret's column through a run of ↑/↓, so a short line on the way
    /// does not pull it left.
    goal_column: Option<usize>,
    scroll: Point<Pixels>,
    /// Scroll the caret into view at the next layout (it moved or typed).
    reveal: bool,
    layout: Option<Layout>,
    is_selecting: bool,
    was_focused: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<EditKind>,
}

impl EventEmitter<InputEvent> for CodeEditor {}

impl CodeEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            content: String::new(),
            starts: vec![0],
            widest: 0,
            states: vec![LineState::default()],
            placeholder: SharedString::default(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            goal_column: None,
            scroll: point(px(0.0), px(0.0)),
            reveal: true,
            layout: None,
            is_selecting: false,
            was_focused: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
        }
    }

    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn value(&self) -> &str {
        &self.content
    }

    /// Replace the whole text (a script opened): caret at the start, no
    /// undo history, nothing emitted.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = super::model::unix_lines(text);
        self.reindex(0);
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.goal_column = None;
        self.scroll = point(px(0.0), px(0.0));
        self.reveal = true;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// Line starts and the widest line after the text changed from byte
    /// `from` on; the colours from that line on are worked out again.
    fn reindex(&mut self, from: usize) {
        self.starts = text::line_starts(&self.content);
        self.widest = self
            .content
            .split('\n')
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let line = text::line_of(&self.starts, from.min(self.content.len()));
        self.states.truncate(line + 1);
        if self.states.is_empty() {
            self.states.push(LineState::default());
        }
    }

    /// The tokenizer's state at the start of `line`.
    fn state_at(&mut self, line: usize) -> LineState {
        while self.states.len() <= line {
            let index = self.states.len() - 1;
            let mut state = self.states[index].clone();
            let range = self.line_range(index);
            js_syntax::advance(&self.content[range], &mut state);
            self.states.push(state);
        }
        self.states[line].clone()
    }

    /// Byte range of `line`, without its line break.
    fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.starts[line];
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.content.len(), |next| next - 1);
        start..end
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = text::clamp_to_char_boundary(&self.content, offset);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.reveal = true;
        self.last_edit = None;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = text::clamp_to_char_boundary(&self.content, offset);
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.reveal = true;
        self.last_edit = None;
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        if self.selected_range.is_empty() {
            self.move_to(text::previous_char(&self.content, self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        if self.selected_range.is_empty() {
            self.move_to(text::next_char(&self.content, self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.select_to(text::previous_char(&self.content, self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.select_to(text::next_char(&self.content, self.cursor_offset()), cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.move_to(text::word_left(&self.content, self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.move_to(text::word_right(&self.content, self.cursor_offset()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.select_to(text::word_left(&self.content, self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.select_to(text::word_right(&self.content, self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    /// Home: the line's first non-blank, then its very start.
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.move_to(text::smart_home(&self.content, self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.move_to(text::line_end(&self.content, self.cursor_offset()), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.select_to(text::smart_home(&self.content, self.cursor_offset()), cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.select_to(text::line_end(&self.content, self.cursor_offset()), cx);
    }

    fn document_start(&mut self, _: &DocumentStart, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.move_to(0, cx);
    }

    fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_column = None;
        self.move_to(self.content.len(), cx);
    }

    fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.goal_column = None;
        self.select_to(0, cx);
    }

    fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.goal_column = None;
        self.select_to(self.content.len(), cx);
    }

    /// The offset `lines` lines above (< 0) or below the caret, at the
    /// run's column; the text's ends past its first and last lines.
    fn vertical(&mut self, lines: isize) -> usize {
        let offset = self.cursor_offset();
        let line = text::line_of(&self.starts, offset);
        let column = *self
            .goal_column
            .get_or_insert_with(|| self.content[self.starts[line]..offset].chars().count());
        let target = line.cast_signed() + lines;
        if target < 0 {
            return 0;
        }
        let target = target.cast_unsigned();
        if target >= self.starts.len() {
            return self.content.len();
        }
        let range = self.line_range(target);
        range.start + text::byte_at_column(&self.content[range], column)
    }

    /// Lines in a screenful, for PgUp/PgDn.
    fn page(&self) -> isize {
        self.layout.as_ref().map_or(20, |layout| {
            #[allow(clippy::cast_possible_truncation)]
            let rows = (layout.bounds.size.height / layout.line_height).floor() as isize;
            (rows - 1).max(1)
        })
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(-1);
        self.keep_goal(|editor| editor.move_to(offset, cx));
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(1);
        self.keep_goal(|editor| editor.move_to(offset, cx));
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(-1);
        self.keep_goal(|editor| editor.select_to(offset, cx));
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(1);
        self.keep_goal(|editor| editor.select_to(offset, cx));
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(-self.page());
        self.keep_goal(|editor| editor.move_to(offset, cx));
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(self.page());
        self.keep_goal(|editor| editor.move_to(offset, cx));
    }

    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(-self.page());
        self.keep_goal(|editor| editor.select_to(offset, cx));
    }

    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(self.page());
        self.keep_goal(|editor| editor.select_to(offset, cx));
    }

    /// Run a vertical move without losing the goal column.
    fn keep_goal(&mut self, step: impl FnOnce(&mut Self)) {
        let goal = self.goal_column;
        step(self);
        self.goal_column = goal;
    }

    // ── Editing ──────────────────────────────────────────────────────────

    /// Replace `range` with `text` as one undoable edit of `kind`; the
    /// caret goes to `caret` (an offset in the new text).
    fn edit(
        &mut self,
        range: Range<usize>,
        text: &str,
        kind: EditKind,
        caret: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if text.is_empty() && range.is_empty() {
            return;
        }
        if kind == EditKind::Other || self.last_edit != Some(kind) {
            self.push_undo();
        }
        self.redo.clear();
        self.content.replace_range(range.clone(), text);
        self.reindex(range.start);
        let end = range.start + text.len();
        self.selected_range = caret.unwrap_or(end..end);
        self.selection_reversed = false;
        self.marked_range = None;
        self.goal_column = None;
        self.reveal = true;
        self.last_edit = Some(kind);
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn push_undo(&mut self) {
        self.undo.push(Snapshot {
            content: self.content.clone(),
            selection: self.selected_range.clone(),
        });
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.content = snapshot.content;
        self.reindex(0);
        self.selected_range = snapshot.selection;
        self.selection_reversed = false;
        self.marked_range = None;
        self.goal_column = None;
        self.reveal = true;
        self.last_edit = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.undo.pop() {
            self.redo.push(Snapshot {
                content: self.content.clone(),
                selection: self.selected_range.clone(),
            });
            self.restore(snapshot, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.redo.pop() {
            self.undo.push(Snapshot {
                content: self.content.clone(),
                selection: self.selected_range.clone(),
            });
            self.restore(snapshot, cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let range = if self.selected_range.is_empty() {
            let caret = self.cursor_offset();
            text::backspace_range(&self.content, caret)
        } else {
            self.selected_range.clone()
        };
        self.edit(range, "", EditKind::Deleting, None, cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let range = if self.selected_range.is_empty() {
            let caret = self.cursor_offset();
            caret..text::next_char(&self.content, caret)
        } else {
            self.selected_range.clone()
        };
        self.edit(range, "", EditKind::Deleting, None, cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let range = if self.selected_range.is_empty() {
            let caret = self.cursor_offset();
            text::word_left(&self.content, caret)..caret
        } else {
            self.selected_range.clone()
        };
        self.edit(range, "", EditKind::Other, None, cx);
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = if self.selected_range.is_empty() {
            let caret = self.cursor_offset();
            text::line_start(&self.content, caret)..caret
        } else {
            self.selected_range.clone()
        };
        self.edit(range, "", EditKind::Other, None, cx);
    }

    /// Enter: a new line at the line's indent (one level more after an
    /// opening bracket; a closing one right after the caret goes to its own
    /// line).
    fn newline(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        let (range, insert, caret) = text::newline(&self.content, self.selected_range.clone());
        let at = range.start + caret;
        self.edit(range, &insert, EditKind::Other, Some(at..at), cx);
    }

    /// Tab: spaces to the next indent stop, or every selected line one
    /// level in.
    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        let selection = self.selected_range.clone();
        if self.content[selection.clone()].contains('\n') {
            let edit = text::indent_lines(&self.content, selection);
            self.apply_lines(edit, cx);
        } else {
            let column = self.content
                [text::line_start(&self.content, selection.start)..selection.start]
                .chars()
                .count();
            let spaces = " ".repeat(INDENT - column % INDENT);
            self.edit(selection, &spaces, EditKind::Other, None, cx);
        }
    }

    /// ⇧Tab: the caret's line, or every selected line, one level out.
    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        let edit = text::outdent_lines(&self.content, self.selected_range.clone());
        self.apply_lines(edit, cx);
    }

    fn toggle_comment(&mut self, _: &ToggleComment, _: &mut Window, cx: &mut Context<Self>) {
        let edit = text::toggle_comment(&self.content, self.selected_range.clone());
        self.apply_lines(edit, cx);
    }

    fn apply_lines(&mut self, edit: text::LinesEdit, cx: &mut Context<Self>) {
        if edit.replacement == self.content[edit.range.clone()] {
            return;
        }
        let reversed = self.selection_reversed;
        self.edit(
            edit.range,
            &edit.replacement,
            EditKind::Other,
            Some(edit.selection),
            cx,
        );
        self.selection_reversed = reversed && !self.selected_range.is_empty();
    }

    fn commit(&mut self, _: &Commit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submitted);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancelled);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.last_edit = None;
            self.replace_text_in_range(None, &text, window, cx);
            self.last_edit = None;
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_owned(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_owned(),
            ));
            let range = self.selected_range.clone();
            self.edit(range, "", EditKind::Other, None, cx);
        }
    }

    // ── Mouse ────────────────────────────────────────────────────────────

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;
        self.goal_column = None;
        let offset = self.offset_for_mouse(event.position);
        match event.click_count {
            2 => {
                let word = text::word_at(&self.content, offset);
                self.move_to(word.start, cx);
                self.select_to(word.end, cx);
            }
            count if count >= 3 => {
                let start = text::line_start(&self.content, offset);
                let end = text::line_end(&self.content, offset);
                self.move_to(start, cx);
                self.select_to(text::next_char(&self.content, end).max(end), cx);
            }
            _ if event.modifiers.shift => self.select_to(offset, cx),
            _ => self.move_to(offset, cx),
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let offset = self.offset_for_mouse(event.position);
            self.select_to(offset, cx);
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(LINE_HEIGHT);
        self.scroll.x -= delta.x;
        self.scroll.y -= delta.y;
        cx.stop_propagation();
        cx.notify();
    }

    fn offset_for_mouse(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = self.layout.as_ref() else {
            return self.content.len();
        };
        let y = position.y - layout.bounds.top() + self.scroll.y;
        if y < px(0.0) {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let line = (y / layout.line_height).floor() as usize;
        if line >= self.starts.len() {
            return self.content.len();
        }
        let range = self.line_range(line);
        let x = (position.x - layout.text_left + self.scroll.x).max(px(0.0));
        let local = match layout.shaped(line) {
            Some(shaped) => shaped.closest_index_for_x(x),
            None => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let column = (x / layout.char_width).round() as usize;
                text::byte_at_column(&self.content[range.clone()], column)
            }
        };
        text::clamp_to_char_boundary(&self.content, range.start + local.min(range.len()))
    }

    // ── IME ──────────────────────────────────────────────────────────────

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        text::utf8_to_utf16(&self.content, range.start)
            ..text::utf8_to_utf16(&self.content, range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        text::utf16_to_utf8(&self.content, range.start.min(range.end))
            ..text::utf16_to_utf8(&self.content, range.start.max(range.end))
    }

    /// The range an input-method edit replaces: the one it names, else the
    /// composition, else the selection.
    fn edit_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        let range = range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = text::clamp_to_char_boundary(&self.content, range.start);
        let end = text::clamp_to_char_boundary(&self.content, range.end);
        start.min(end)..start.max(end)
    }

    /// Where `offset` is drawn, from the window's top-left, while laid out.
    fn position_of(&self, offset: usize) -> Option<Point<Pixels>> {
        let layout = self.layout.as_ref()?;
        let line = text::line_of(&self.starts, offset);
        let start = self.starts[line];
        let x = match layout.shaped(line) {
            Some(shaped) => shaped.x_for_index(offset - start),
            None => layout.char_width * self.content[start..offset].chars().count() as f32,
        };
        Some(point(
            layout.text_left + x - self.scroll.x,
            layout.bounds.top() + layout.line_height * line as f32 - self.scroll.y,
        ))
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut range = self.edit_range(range_utf16.as_ref());
        self.marked_range = None;
        let text = text::clean(new_text);
        if text.is_empty() && range.is_empty() {
            cx.notify();
            return;
        }
        // A closing bracket typed on a blank line steps back a level.
        if range.is_empty()
            && let Some(dedent) = text::dedent_for_closer(&self.content, range.start, &text)
        {
            range = dedent.start..range.end;
        }
        let kind = if range.is_empty() && text.chars().count() == 1 && text != "\n" {
            EditKind::Typing
        } else {
            EditKind::Other
        };
        self.edit(range, &text, kind, None, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.edit_range(range_utf16.as_ref());
        let text = text::clean(new_text);
        if text.is_empty() && range.is_empty() {
            return;
        }
        let start = range.start;
        self.edit(range, &text, EditKind::Typing, None, cx);
        let end = start + text.len();
        self.marked_range = (!text.is_empty()).then_some(start..end);
        self.selected_range = match new_selected_range_utf16 {
            Some(selected) if text == new_text => {
                start + text::utf16_to_utf8(&text, selected.start)
                    ..start + text::utf16_to_utf8(&text, selected.end)
            }
            _ => end..end,
        };
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let at = self.position_of(range.start)?;
        let line_height = self.layout.as_ref()?.line_height;
        Some(Bounds::new(at, size(px(2.0), line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.layout.as_ref()?;
        let offset = self.offset_for_mouse(point);
        Some(text::utf8_to_utf16(&self.content, offset))
    }
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        if self.was_focused && !focused {
            cx.emit(InputEvent::Blurred);
        }
        self.was_focused = focused;
        let theme = theme(cx);
        div()
            .id(ElementId::from(("code-editor", cx.entity_id())))
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::toggle_comment))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .rounded(RADIUS_MD)
            .bg(theme.bg)
            .border_1()
            .border_color(if focused {
                theme.accent
            } else {
                theme.border_strong
            })
            .font_family(MONO)
            .text_size(FONT_BODY)
            .line_height(LINE_HEIGHT)
            .text_color(theme.fg)
            .child(EditorElement {
                editor: cx.entity(),
            })
    }
}

/// The colour of a token.
fn token_color(kind: Kind, theme: &Theme) -> Hsla {
    match kind {
        Kind::Keyword | Kind::Interpolation => theme.series(3),
        Kind::Literal | Kind::Number => theme.warning,
        Kind::Builtin => theme.info,
        Kind::Function => theme.series(5),
        Kind::String | Kind::Template => theme.success,
        Kind::Comment => theme.fg_dim,
    }
}

/// The editor as last laid out.
struct Layout {
    /// The element's bounds (inside the border).
    bounds: Bounds<Pixels>,
    /// Where column 0 is drawn when not scrolled.
    text_left: Pixels,
    line_height: Pixels,
    char_width: Pixels,
    /// The first line shaped, and the lines on screen from it.
    first: usize,
    lines: Vec<ShapedLine>,
}

impl Layout {
    fn shaped(&self, line: usize) -> Option<&ShapedLine> {
        line.checked_sub(self.first)
            .and_then(|index| self.lines.get(index))
    }
}

struct EditorElement {
    editor: Entity<CodeEditor>,
}

struct Prepaint {
    gutter: Bounds<Pixels>,
    text: Bounds<Pixels>,
    current_line: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    /// Each shown line, with where it is drawn.
    lines: Vec<(ShapedLine, Point<Pixels>)>,
    numbers: Vec<(ShapedLine, Point<Pixels>)>,
    placeholder: Option<(ShapedLine, Point<Pixels>)>,
    gutter_bg: Hsla,
    gutter_border: Hsla,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Fill the editor's box; scrolling shows the rest.
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.flex_grow = 1.0;
        style.flex_shrink = 1.0;
        style.flex_basis = px(0.0).into();
        style.min_size.height = (window.line_height() * 3.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.editor
            .update(cx, |editor, cx| editor.prepaint(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        let line_height = window.line_height();
        window.paint_quad(fill(prepaint.gutter, prepaint.gutter_bg));
        window.paint_quad(fill(
            Bounds::new(
                point(prepaint.gutter.right() - px(1.0), prepaint.gutter.top()),
                size(px(1.0), prepaint.gutter.size.height),
            ),
            prepaint.gutter_border,
        ));
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(quad) = prepaint.current_line.take() {
                window.paint_quad(quad);
            }
        });
        window.with_content_mask(
            Some(ContentMask {
                bounds: prepaint.gutter,
            }),
            |window| {
                for (number, at) in &prepaint.numbers {
                    let _ = number.paint(*at, line_height, window, cx);
                }
            },
        );
        window.with_content_mask(
            Some(ContentMask {
                bounds: prepaint.text,
            }),
            |window| {
                for quad in prepaint.selection.drain(..) {
                    window.paint_quad(quad);
                }
                if let Some((placeholder, at)) = &prepaint.placeholder {
                    let _ = placeholder.paint(*at, line_height, window, cx);
                }
                for (line, at) in &prepaint.lines {
                    let _ = line.paint(*at, line_height, window, cx);
                }
                if focus_handle.is_focused(window)
                    && let Some(cursor) = prepaint.cursor.take()
                {
                    window.paint_quad(cursor);
                }
            },
        );
    }
}

impl CodeEditor {
    /// Lay out the lines on screen: scroll to the caret if it moved, shape
    /// and colour only what shows, and keep the geometry for the mouse.
    #[allow(clippy::too_many_lines)]
    fn prepaint(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Prepaint {
        let theme = theme(cx).clone();
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let text_system = window.text_system().clone();
        let font_id = text_system.resolve_font(&font);
        let char_width = text_system
            .advance(font_id, font_size, '0')
            .map_or(font_size * 0.6, |advance| advance.width);

        let line_count = self.starts.len();
        let digits = line_count.max(99).to_string().len();
        let gutter_width = char_width * digits as f32 + SPACE_5 * 2.0;
        let gutter = Bounds::new(bounds.origin, size(gutter_width, bounds.size.height));
        let text_bounds = Bounds::from_corners(
            point(bounds.left() + gutter_width, bounds.top()),
            bounds.bottom_right(),
        );
        let text_left = text_bounds.left() + SPACE_4;
        let visible_width = (text_bounds.size.width - SPACE_4 * 2.0).max(char_width);
        let height = bounds.size.height;

        let caret = self.cursor_offset();
        let caret_line = text::line_of(&self.starts, caret);

        // Vertically: the caret's line in view after it moved; never past
        // the text.
        let mut scroll_y = self.scroll.y;
        if self.reveal {
            let top = line_height * caret_line as f32;
            if top < scroll_y {
                scroll_y = top;
            } else if top + line_height > scroll_y + height {
                scroll_y = top + line_height - height;
            }
        }
        let most_y = (line_height * line_count as f32 - height).max(px(0.0));
        scroll_y = scroll_y.clamp(px(0.0), most_y);

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let first = ((scroll_y / line_height).floor() as usize).min(line_count - 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let last =
            (((scroll_y + height) / line_height).ceil() as usize).clamp(first + 1, line_count);

        let run = |len: usize, color: Hsla| TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked = self.marked_range.clone();
        let mut shaped = Vec::with_capacity(last - first);
        for line in first..last {
            let range = self.line_range(line);
            let mut state = self.state_at(line);
            let source = &self.content[range.clone()];
            let tokens = js_syntax::tokenize_line(source, &mut state);
            if self.states.len() == line + 1 {
                self.states.push(state);
            }
            let mut runs: Vec<TextRun> = Vec::new();
            for (span, kind) in js_syntax::spans(source.len(), &tokens) {
                let color = kind.map_or(theme.fg, |kind| token_color(kind, &theme));
                // An input method's composition is underlined.
                let cuts = marked.as_ref().map_or_else(Vec::new, |marked| {
                    [marked.start, marked.end]
                        .into_iter()
                        .filter_map(|at| at.checked_sub(range.start))
                        .filter(|at| span.start < *at && *at < span.end)
                        .collect::<Vec<_>>()
                });
                let mut at = span.start;
                for end in cuts.into_iter().chain([span.end]) {
                    let underlined = marked.as_ref().is_some_and(|marked| {
                        range.start + at >= marked.start && range.start + end <= marked.end
                    });
                    let mut piece = run(end - at, color);
                    if underlined {
                        piece.underline = Some(UnderlineStyle {
                            color: Some(color),
                            thickness: px(1.0),
                            wavy: false,
                        });
                    }
                    runs.push(piece);
                    at = end;
                }
            }
            if runs.is_empty() {
                runs.push(run(0, theme.fg));
            }
            // Tabs (from a script saved elsewhere) draw as one space; both
            // are one byte, so offsets hold.
            let display: SharedString = if source.contains('\t') {
                source.replace('\t', " ").into()
            } else {
                SharedString::from(source.to_owned())
            };
            shaped.push(text_system.shape_line(display, font_size, &runs, None));
        }

        // Sideways: the caret in view, with a few columns of margin.
        let caret_x = shaped.get(caret_line.wrapping_sub(first)).map_or_else(
            || char_width * self.content[self.starts[caret_line]..caret].chars().count() as f32,
            |line: &ShapedLine| line.x_for_index(caret - self.starts[caret_line]),
        );
        let mut scroll_x = self.scroll.x;
        if self.reveal {
            let margin = char_width * 4.0;
            if caret_x < scroll_x {
                scroll_x = (caret_x - margin).max(px(0.0));
            } else if caret_x + char_width > scroll_x + visible_width {
                scroll_x = caret_x + margin - visible_width;
            }
        }
        let most_x = (char_width * (self.widest + 4) as f32 - visible_width).max(px(0.0));
        scroll_x = scroll_x.clamp(px(0.0), most_x);

        let line_top = |line: usize| bounds.top() + line_height * line as f32 - scroll_y;
        let origin_x = text_left - scroll_x;

        let selection_range = self.selected_range.clone();
        let mut selection = Vec::new();
        if !selection_range.is_empty() {
            for (index, line) in shaped.iter().enumerate() {
                let number = first + index;
                let range = self.line_range(number);
                if selection_range.end < range.start || selection_range.start > range.end {
                    continue;
                }
                let from = selection_range.start.max(range.start) - range.start;
                let to = selection_range.end.min(range.end) - range.start;
                let left = line.x_for_index(from);
                let mut right = line.x_for_index(to);
                // The line break a selection runs over shows as a sliver.
                if selection_range.end > range.end {
                    right += char_width * 0.5;
                }
                if right > left {
                    let top = line_top(number);
                    selection.push(fill(
                        Bounds::from_corners(
                            point(origin_x + left, top),
                            point(origin_x + right, top + line_height),
                        ),
                        theme.accent.alpha(0.28),
                    ));
                }
            }
        }

        let focused = self.focus_handle.is_focused(window);
        let single_line = !self.content[selection_range.clone()].contains('\n');
        let current_line = (focused && single_line).then(|| {
            fill(
                Bounds::new(
                    point(bounds.left(), line_top(caret_line)),
                    size(bounds.size.width, line_height),
                ),
                theme.hover,
            )
        });
        let cursor = selection_range.is_empty().then(|| {
            fill(
                Bounds::new(
                    point(origin_x + caret_x, line_top(caret_line)),
                    size(px(1.5), line_height),
                ),
                theme.accent,
            )
        });

        let mut numbers = Vec::with_capacity(shaped.len());
        for line in first..last {
            let label: SharedString = (line + 1).to_string().into();
            let color = if line == caret_line {
                theme.fg_muted
            } else {
                theme.fg_dim
            };
            let number =
                text_system.shape_line(label.clone(), font_size, &[run(label.len(), color)], None);
            let x = gutter.right() - SPACE_5 - number.width;
            numbers.push((number, point(x, line_top(line))));
        }

        let placeholder = (self.content.is_empty() && !self.placeholder.is_empty()).then(|| {
            let line = text_system.shape_line(
                self.placeholder.clone(),
                font_size,
                &[run(self.placeholder.len(), theme.fg_dim)],
                None,
            );
            (line, point(origin_x, line_top(0)))
        });

        let lines = shaped
            .iter()
            .enumerate()
            .map(|(index, line)| (line.clone(), point(origin_x, line_top(first + index))))
            .collect();

        self.scroll = point(scroll_x, scroll_y);
        self.reveal = false;
        self.layout = Some(Layout {
            bounds,
            text_left,
            line_height,
            char_width,
            first,
            lines: shaped,
        });

        Prepaint {
            gutter,
            text: text_bounds,
            current_line,
            selection,
            cursor,
            lines,
            numbers,
            placeholder,
            gutter_bg: theme.surface,
            gutter_border: theme.border,
        }
    }
}

/// The editor's text rules, pure so they are tested without a window.
pub(crate) mod text {
    use std::ops::Range;

    use super::INDENT;

    /// Byte offset of each line's first character.
    pub(crate) fn line_starts(text: &str) -> Vec<usize> {
        std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect()
    }

    /// The line holding byte `offset`.
    pub(crate) fn line_of(starts: &[usize], offset: usize) -> usize {
        starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1)
    }

    pub(crate) fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub(crate) fn previous_char(text: &str, offset: usize) -> usize {
        text[..clamp_to_char_boundary(text, offset)]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    pub(crate) fn next_char(text: &str, offset: usize) -> usize {
        let offset = clamp_to_char_boundary(text, offset);
        text[offset..]
            .chars()
            .next()
            .map_or(text.len(), |character| offset + character.len_utf8())
    }

    pub(crate) fn line_start(text: &str, offset: usize) -> usize {
        let offset = clamp_to_char_boundary(text, offset);
        text[..offset].rfind('\n').map_or(0, |index| index + 1)
    }

    pub(crate) fn line_end(text: &str, offset: usize) -> usize {
        let offset = clamp_to_char_boundary(text, offset);
        text[offset..]
            .find('\n')
            .map_or(text.len(), |index| offset + index)
    }

    /// The line's first non-blank; its start when the caret is already
    /// there (or the line is blank up to the caret).
    pub(crate) fn smart_home(text: &str, offset: usize) -> usize {
        let start = line_start(text, offset);
        let end = line_end(text, offset);
        let first = start + leading_blank(&text[start..end]);
        if offset == first { start } else { first }
    }

    /// Bytes of spaces and tabs at the start of `line`.
    fn leading_blank(line: &str) -> usize {
        line.len() - line.trim_start_matches([' ', '\t']).len()
    }

    /// The byte in `line` at character `column`, or its end.
    pub(crate) fn byte_at_column(line: &str, column: usize) -> usize {
        line.char_indices()
            .nth(column)
            .map_or(line.len(), |(index, _)| index)
    }

    fn is_word(character: char) -> bool {
        character.is_alphanumeric() || character == '_' || character == '$'
    }

    /// The start of the word before `offset` (over blanks first, then a
    /// word or a run of punctuation).
    pub(crate) fn word_left(text: &str, offset: usize) -> usize {
        let offset = clamp_to_char_boundary(text, offset);
        let mut chars = text[..offset].char_indices().rev().peekable();
        let mut at = offset;
        while let Some((index, character)) = chars.peek().copied() {
            let blank = character.is_whitespace() && character != '\n';
            let first_break = character == '\n' && at == offset;
            if !(blank || first_break) {
                break;
            }
            at = index;
            chars.next();
            if first_break {
                return at;
            }
        }
        let Some((_, first)) = chars.peek().copied() else {
            return at;
        };
        let word = is_word(first);
        for (index, character) in chars {
            if character.is_whitespace() || is_word(character) != word {
                break;
            }
            at = index;
        }
        at
    }

    /// The end of the word after `offset`.
    pub(crate) fn word_right(text: &str, offset: usize) -> usize {
        let offset = clamp_to_char_boundary(text, offset);
        let mut chars = text[offset..].char_indices().peekable();
        let mut at = offset;
        while let Some((index, character)) = chars.peek().copied() {
            if character == '\n' {
                return if at == offset { offset + index + 1 } else { at };
            }
            if character.is_whitespace() {
                at = offset + index + character.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        let Some((_, first)) = chars.peek().copied() else {
            return at;
        };
        let word = is_word(first);
        for (index, character) in chars {
            if character.is_whitespace() || is_word(character) != word {
                break;
            }
            at = offset + index + character.len_utf8();
        }
        at
    }

    /// The word around `offset`, or the character there.
    pub(crate) fn word_at(text: &str, offset: usize) -> Range<usize> {
        let offset = clamp_to_char_boundary(text, offset);
        let start = text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, character)| is_word(*character))
            .last()
            .map_or(offset, |(index, _)| index);
        let end = text[offset..]
            .char_indices()
            .find(|(_, character)| !is_word(*character))
            .map_or(text.len(), |(index, _)| offset + index);
        if start == end {
            start..next_char(text, offset)
        } else {
            start..end
        }
    }

    /// What Backspace takes at `offset`: back to the previous indent stop
    /// in a line's leading blanks, else one character.
    pub(crate) fn backspace_range(text: &str, offset: usize) -> Range<usize> {
        let offset = clamp_to_char_boundary(text, offset);
        let start = line_start(text, offset);
        let before = &text[start..offset];
        if !before.is_empty() && before.bytes().all(|byte| byte == b' ') {
            let column = before.len();
            let back = match column % INDENT {
                0 => INDENT,
                rest => rest,
            };
            return offset - back.min(column)..offset;
        }
        previous_char(text, offset)..offset
    }

    /// Typed or pasted text as the editor keeps it: line breaks as `\n`,
    /// tabs as an indent's spaces, no other control characters.
    pub(crate) fn clean(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for character in text.replace("\r\n", "\n").chars() {
            match character {
                '\n' | '\r' => out.push('\n'),
                '\t' => out.push_str(&" ".repeat(INDENT)),
                other if other.is_control() => {}
                other => out.push(other),
            }
        }
        out
    }

    /// Enter over `selection`: the range replaced, the text put there and
    /// where the caret lands in it.
    pub(crate) fn newline(text: &str, selection: Range<usize>) -> (Range<usize>, String, usize) {
        let start = line_start(text, selection.start);
        let before = &text[start..selection.start];
        let indent = &before[..leading_blank(before)];
        let opener = before.trim_end().chars().last();
        let rest_of_line = &text[selection.end..line_end(text, selection.end)];
        let blanks = rest_of_line.len() - rest_of_line.trim_start_matches(' ').len();
        let closer = rest_of_line[blanks..].chars().next();
        let pair = matches!(
            (opener, closer),
            (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('('), Some(')'))
        );
        let deeper = matches!(opener, Some('{' | '[' | '('));
        let inner = format!(
            "\n{indent}{}",
            if deeper {
                " ".repeat(INDENT)
            } else {
                String::new()
            }
        );
        if pair {
            let insert = format!("{inner}\n{indent}");
            (selection.start..selection.end + blanks, insert, inner.len())
        } else {
            let caret = inner.len();
            (selection, inner, caret)
        }
    }

    /// Typing `typed` at `offset`: the start of the blanks to take back
    /// first, when it is a closing bracket on a line blank up to it.
    pub(crate) fn dedent_for_closer(
        text: &str,
        offset: usize,
        typed: &str,
    ) -> Option<Range<usize>> {
        if !matches!(typed, "}" | "]" | ")") {
            return None;
        }
        let start = line_start(text, offset);
        let before = &text[start..offset];
        (before.len() >= INDENT && before.bytes().all(|byte| byte == b' '))
            .then(|| offset - INDENT..offset)
    }

    /// A change to whole lines: the bytes replaced, their new text and the
    /// selection after it.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct LinesEdit {
        pub range: Range<usize>,
        pub replacement: String,
        pub selection: Range<usize>,
    }

    /// The lines `selection` touches: a selection ending at a line's very
    /// start leaves that line out.
    fn touched_lines(text: &str, selection: &Range<usize>) -> Range<usize> {
        let starts = line_starts(text);
        let first = line_of(&starts, selection.start);
        let mut last = line_of(&starts, selection.end);
        if last > first && starts[last] == selection.end {
            last -= 1;
        }
        let start = starts[first];
        let end = starts.get(last + 1).map_or(text.len(), |next| next - 1);
        start..end
    }

    /// One change inside a line: `removed` bytes at `at` (absolute) give
    /// way to `inserted`.
    struct Change {
        at: usize,
        removed: usize,
        inserted: &'static str,
    }

    /// Apply `changes` (in order) to the lines of `block`, mapping
    /// `selection` through them.
    fn apply(
        text: &str,
        block: Range<usize>,
        changes: &[Change],
        selection: &Range<usize>,
    ) -> LinesEdit {
        let mut replacement = String::with_capacity(block.len() + changes.len() * INDENT);
        let mut at = block.start;
        for change in changes {
            replacement.push_str(&text[at..change.at]);
            replacement.push_str(change.inserted);
            at = change.at + change.removed;
        }
        replacement.push_str(&text[at..block.end]);
        // An offset where text is put stays before it unless `after`; one
        // inside removed text goes to where it was.
        let map = |offset: usize, after: bool| {
            let mut shift: isize = 0;
            for change in changes {
                let end = change.at + change.removed;
                if offset > end || (offset == end && (change.removed > 0 || after)) {
                    shift += change.inserted.len().cast_signed() - change.removed.cast_signed();
                } else if offset > change.at {
                    return (change.at.cast_signed() + shift).cast_unsigned();
                } else {
                    break;
                }
            }
            (offset.cast_signed() + shift).cast_unsigned()
        };
        // A caret moves with its text; a selection grows over what is put
        // at its ends.
        let caret = selection.is_empty();
        LinesEdit {
            range: block,
            replacement,
            selection: map(selection.start, caret)..map(selection.end, true),
        }
    }

    /// Every touched line one level in (blank lines stay blank).
    pub(crate) fn indent_lines(text: &str, selection: Range<usize>) -> LinesEdit {
        let block = touched_lines(text, &selection);
        let mut changes = Vec::new();
        let mut start = block.start;
        for line in text[block.clone()].split('\n') {
            if !line.trim().is_empty() {
                changes.push(Change {
                    at: start,
                    removed: 0,
                    inserted: "  ",
                });
            }
            start += line.len() + 1;
        }
        apply(text, block, &changes, &selection)
    }

    /// Every touched line one level out (up to an indent of blanks).
    pub(crate) fn outdent_lines(text: &str, selection: Range<usize>) -> LinesEdit {
        let block = touched_lines(text, &selection);
        let mut changes = Vec::new();
        let mut start = block.start;
        for line in text[block.clone()].split('\n') {
            let removed = if line.starts_with('\t') {
                1
            } else {
                leading_blank(line).min(INDENT)
            };
            if removed > 0 {
                changes.push(Change {
                    at: start,
                    removed,
                    inserted: "",
                });
            }
            start += line.len() + 1;
        }
        apply(text, block, &changes, &selection)
    }

    /// `// ` before every touched line at their shallowest indent, or, when
    /// every non-blank one is already commented, the comment marks off.
    pub(crate) fn toggle_comment(text: &str, selection: Range<usize>) -> LinesEdit {
        let block = touched_lines(text, &selection);
        let lines: Vec<&str> = text[block.clone()].split('\n').collect();
        let filled = || lines.iter().filter(|line| !line.trim().is_empty());
        let commented = filled().count() > 0
            && filled().all(|line| line[leading_blank(line)..].starts_with("//"));
        let indent = filled().map(|line| leading_blank(line)).min().unwrap_or(0);
        let mut changes = Vec::new();
        let mut start = block.start;
        for line in &lines {
            let blank = leading_blank(line);
            let body = &line[blank..];
            if commented {
                if body.starts_with("//") {
                    let removed = if body[2..].starts_with(' ') { 3 } else { 2 };
                    changes.push(Change {
                        at: start + blank,
                        removed,
                        inserted: "",
                    });
                }
            } else if !line.trim().is_empty() || lines.len() == 1 {
                changes.push(Change {
                    at: start + indent.min(blank),
                    removed: 0,
                    inserted: "// ",
                });
            }
            start += line.len() + 1;
        }
        apply(text, block, &changes, &selection)
    }

    /// Byte offset in `text` of UTF-16 offset `offset`.
    pub(crate) fn utf16_to_utf8(text: &str, offset: usize) -> usize {
        let (mut utf8, mut utf16) = (0, 0);
        for character in text.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += character.len_utf16();
            utf8 += character.len_utf8();
        }
        utf8
    }

    /// UTF-16 offset in `text` of byte offset `offset`.
    pub(crate) fn utf8_to_utf16(text: &str, offset: usize) -> usize {
        let (mut utf8, mut utf16) = (0, 0);
        for character in text.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += character.len_utf8();
            utf16 += character.len_utf16();
        }
        utf16
    }
}

#[cfg(test)]
mod tests {
    use super::text::*;

    /// Apply a lines edit to `text`.
    fn applied(text: &str, edit: &LinesEdit) -> String {
        let mut out = text.to_owned();
        out.replace_range(edit.range.clone(), &edit.replacement);
        out
    }

    #[test]
    fn lines_are_indexed_and_found() {
        let text = "a\nbc\n\nd";
        let starts = line_starts(text);
        assert_eq!(starts, [0, 2, 5, 6]);
        assert_eq!(line_of(&starts, 0), 0);
        assert_eq!(line_of(&starts, 1), 0, "the line break belongs to its line");
        assert_eq!(line_of(&starts, 2), 1);
        assert_eq!(line_of(&starts, 5), 2);
        assert_eq!(line_of(&starts, text.len()), 3);
        assert_eq!(line_starts(""), [0]);
        assert_eq!(byte_at_column("héllo", 2), 3, "é is two bytes");
        assert_eq!(byte_at_column("ab", 9), 2, "past the end: the end");
    }

    #[test]
    fn home_goes_to_the_first_non_blank_then_the_start() {
        let text = "x\n    return 1;";
        let end = text.len();
        assert_eq!(smart_home(text, end), 6);
        assert_eq!(smart_home(text, 6), 2);
        assert_eq!(smart_home(text, 3), 6, "inside the indent: to the code");
    }

    #[test]
    fn words_are_crossed_like_an_editor_does() {
        let text = "const total = input.value + 1;";
        assert_eq!(word_left(text, 11), 6, "back over `total`");
        assert_eq!(word_left(text, 12), 6, "over the blank, then the word");
        assert_eq!(word_right(text, 0), 5);
        assert_eq!(word_right(text, 5), 11, "the blank, then `total`");
        assert_eq!(word_right(text, 19), 20, "a run of punctuation is a word");
        let lines = "a\n  b";
        assert_eq!(word_left(lines, 2), 1, "a line break stops the move");
        assert_eq!(word_right(lines, 1), 2);
        assert_eq!(&text[word_at(text, 22)], "value");
        assert_eq!(word_at(text, 12), 12..13, "the character when no word");
    }

    #[test]
    fn backspace_takes_an_indent_level_in_leading_blanks() {
        let text = "{\n    x";
        assert_eq!(backspace_range(text, 6), 4..6, "a level of two");
        assert_eq!(backspace_range(text, 5), 4..5, "back to the stop");
        assert_eq!(backspace_range(text, 7), 6..7, "in code: one character");
        assert_eq!(backspace_range("é", 2), 0..2);
    }

    #[test]
    fn enter_keeps_and_deepens_the_indent() {
        let text = "  if (x) {";
        let (range, insert, caret) = newline(text, text.len()..text.len());
        assert_eq!(range, text.len()..text.len());
        assert_eq!(insert, "\n    ");
        assert_eq!(caret, insert.len());

        let text = "  return 1;";
        let (_, insert, _) = newline(text, text.len()..text.len());
        assert_eq!(insert, "\n  ", "the same indent");

        // Between a pair, the closer goes to its own line.
        let text = "function main(input) {}";
        let at = text.len() - 1;
        let (range, insert, caret) = newline(text, at..at);
        assert_eq!(range, at..at);
        assert_eq!(insert, "\n  \n");
        assert_eq!(caret, 3);
        let mut out = text.to_owned();
        out.replace_range(range, &insert);
        assert_eq!(out, "function main(input) {\n  \n}");

        // A selection is replaced.
        let text = "ab cd";
        let (range, insert, _) = newline(text, 2..3);
        assert_eq!((range, insert.as_str()), (2..3, "\n"));
    }

    #[test]
    fn a_closer_on_a_blank_line_steps_back() {
        let text = "{\n    ";
        assert_eq!(dedent_for_closer(text, text.len(), "}"), Some(4..6));
        assert_eq!(dedent_for_closer(text, text.len(), "x"), None);
        assert_eq!(dedent_for_closer("{\n  x ", 6, "}"), None, "code before it");
        assert_eq!(dedent_for_closer("{\n", 2, "}"), None, "nothing to take");
    }

    #[test]
    fn tab_and_shift_tab_move_whole_lines() {
        let text = "a\nb\n\nc";
        let edit = indent_lines(text, 0..text.len());
        assert_eq!(
            applied(text, &edit),
            "  a\n  b\n\n  c",
            "blank lines stay blank"
        );
        assert_eq!(edit.selection, 0..text.len() + 6);

        let edit = outdent_lines("  a\n    b\nc", 0..9);
        assert_eq!(applied("  a\n    b\nc", &edit), "a\n  b\nc");

        // A selection ending at a line's start leaves that line be.
        let text = "a\nb\nc";
        let edit = indent_lines(text, 0..2);
        assert_eq!(applied(text, &edit), "  a\nb\nc");
        assert_eq!(edit.selection, 0..4, "the selection grows with its line");

        // The caret's line alone, the caret keeping its place in the code.
        let text = "x\n    y";
        let edit = outdent_lines(text, 7..7);
        assert_eq!(applied(text, &edit), "x\n  y");
        assert_eq!(edit.selection, 5..5);
    }

    #[test]
    fn comments_toggle_at_the_shallowest_indent() {
        let text = "  a\n    b\n\n  c";
        let edit = toggle_comment(text, 0..text.len());
        let commented = applied(text, &edit);
        assert_eq!(commented, "  // a\n  //   b\n\n  // c");
        let back = toggle_comment(&commented, 0..commented.len());
        assert_eq!(applied(&commented, &back), text);

        // Mixed: comment them all.
        let text = "// a\nb";
        let edit = toggle_comment(text, 0..text.len());
        assert_eq!(applied(text, &edit), "// // a\n// b");

        // The caret's line, the caret moving with its text.
        let text = "x = 1";
        let edit = toggle_comment(text, 2..2);
        assert_eq!(applied(text, &edit), "// x = 1");
        assert_eq!(edit.selection, 5..5);
    }

    #[test]
    fn comments_toggle_past_unicode_blanks() {
        // Not indent blanks: they stay in the body, and must not be
        // sliced through.
        for text in ["\u{3000}// a", "\u{a0}// a"] {
            let edit = toggle_comment(text, 0..text.len());
            assert_eq!(applied(text, &edit), format!("// {text}"));
        }
    }

    #[test]
    fn pasted_text_keeps_line_breaks_and_indents_tabs() {
        assert_eq!(clean("a\r\nb\rc\td\u{7}"), "a\nb\nc  d");
    }

    #[test]
    fn utf16_offsets_round_trip() {
        let text = "a😀b\nc";
        assert_eq!(utf8_to_utf16(text, 5), 3);
        assert_eq!(utf16_to_utf8(text, 3), 5);
        assert_eq!(utf16_to_utf8(text, 99), text.len());
    }
}

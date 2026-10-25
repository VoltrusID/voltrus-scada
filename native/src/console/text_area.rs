//! A multi-line text field. vt-ui's `TextInput` is one line; shift handover
//! notes are paragraphs. The field answers the same `text_input` actions as
//! `TextInput` (so its key bindings, the Edit menu and the `!TextInput` key
//! contexts hold for it too), with Enter starting a new line, ↑/↓ moving
//! between lines and ⌘↵ submitting. It wraps at its width, shows a fixed
//! number of rows and scrolls to keep the caret in view. IME-aware through
//! `EntityInputHandler`, like `TextInput`. A candidate for vt-ui.

use std::cell::Cell;
use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ScrollWheelEvent, SharedString, Style, TextAlign, TextRun, UTF16Selection, UnderlineStyle,
    Window, WrappedLine, actions, div, fill, point, px, relative, size,
};
use vt_ui::text_input::{
    Backspace, Cancel, Copy, Cut, Delete, End, Home, InputEvent, Left, Paste, Right, SelectAll,
    SelectEnd, SelectHome, SelectLeft, SelectRight, Submit,
};
use vt_ui::theme::*;

actions!(text_area, [Up, Down, SelectUp, SelectDown, Commit]);

/// The field's key context: `TextInput` for the shared bindings (and so
/// that `!TextInput` keys stay off while typing), `TextArea` for its own.
const KEY_CONTEXT: &str = "TextInput TextArea";
const CONTEXT: &str = "TextArea";
const LINE_HEIGHT: Pixels = px(18.0);

thread_local! {
    static BOUND: Cell<bool> = const { Cell::new(false) };
}

/// The keys only a multi-line field has. `shortcuts::rebind` includes
/// them when it rebuilds the keymap.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", Up, Some(CONTEXT)),
        KeyBinding::new("down", Down, Some(CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("secondary-enter", Commit, Some(CONTEXT)),
    ]
}

/// Bound once with the first field (the keymap may already have them).
fn bind_keys(cx: &mut App) {
    if BOUND.with(|bound| bound.replace(true)) {
        return;
    }
    cx.bind_keys(key_bindings());
}

pub struct TextArea {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    /// Rows of text the field shows.
    rows: usize,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    /// The caret's x through a run of ↑/↓, so a short line on the way does
    /// not pull it left.
    goal_x: Option<Pixels>,
    scroll_y: Pixels,
    /// Scroll the caret into view at the next layout (it moved or typed).
    reveal: bool,
    layout: Option<Layout>,
    is_selecting: bool,
    was_focused: bool,
}

impl EventEmitter<InputEvent> for TextArea {}

impl TextArea {
    /// A field showing `rows` rows of text. Emits `Changed`, `Submitted`
    /// (⌘↵), `Cancelled` (Esc) and `Blurred` like `TextInput`.
    pub fn new(rows: usize, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            content: String::new(),
            placeholder: SharedString::default(),
            rows: rows.max(1),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            goal_x: None,
            scroll_y: px(0.0),
            reveal: true,
            layout: None,
            is_selecting: false,
            was_focused: false,
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

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = clamp_to_char_boundary(&self.content, offset);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.reveal = true;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = clamp_to_char_boundary(&self.content, offset);
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
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        if self.selected_range.is_empty() {
            self.move_to(previous_char(&self.content, self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        if self.selected_range.is_empty() {
            self.move_to(next_char(&self.content, self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(previous_char(&self.content, self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(next_char(&self.content, self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(line_start(&self.content, self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(line_end(&self.content, self.cursor_offset()), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(line_start(&self.content, self.cursor_offset()), cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(line_end(&self.content, self.cursor_offset()), cx);
    }

    /// The offset a row above (`rows` < 0) or below the caret, at the run's
    /// x; the field's ends past its first and last rows.
    fn vertical(&mut self, rows: f32) -> usize {
        let Some(layout) = self.layout.as_ref() else {
            return self.cursor_offset();
        };
        let at = layout.position(self.cursor_offset());
        let x = *self.goal_x.get_or_insert(at.x);
        let y = at.y + layout.line_height * rows + layout.line_height / 2.0;
        if y < px(0.0) {
            0
        } else if y >= layout.height {
            self.content.len()
        } else {
            clamp_to_char_boundary(&self.content, layout.offset_at(point(x, y)))
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(-1.0);
        self.move_to(offset, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(1.0);
        self.move_to(offset, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(-1.0);
        self.select_to(offset, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.vertical(1.0);
        self.select_to(offset, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(previous_char(&self.content, self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(next_char(&self.content, self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    /// Enter: a new line (⌘↵ submits).
    fn newline(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn commit(&mut self, _: &Commit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submitted);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancelled);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_owned(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_owned(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;
        self.goal_x = None;
        let offset = self.offset_for_mouse(event.position);
        if event.click_count >= 2 {
            let word = word_at(&self.content, offset);
            self.move_to(word.start, cx);
            self.select_to(word.end, cx);
        } else if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
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
        self.scroll_y -= delta.y;
        cx.notify();
    }

    fn offset_for_mouse(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = self.layout.as_ref() else {
            return self.content.len();
        };
        let local = point(
            position.x - layout.bounds.left(),
            position.y - layout.bounds.top() + self.scroll_y,
        );
        if local.y < px(0.0) {
            0
        } else if local.y >= layout.height {
            self.content.len()
        } else {
            clamp_to_char_boundary(&self.content, layout.offset_at(local))
        }
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf8_to_utf16(&self.content, range.start)..utf8_to_utf16(&self.content, range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf16_to_utf8(&self.content, range.start.min(range.end))
            ..utf16_to_utf8(&self.content, range.start.max(range.end))
    }

    /// The range an input-method edit replaces: the one it names, else the
    /// composition, else the selection.
    fn edit_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        let range = range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = clamp_to_char_boundary(&self.content, range.start);
        let end = clamp_to_char_boundary(&self.content, range.end);
        start.min(end)..start.max(end)
    }

    fn splice(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) -> usize {
        self.content.replace_range(range.clone(), text);
        self.selection_reversed = false;
        self.goal_x = None;
        self.reveal = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
        range.start + text.len()
    }
}

impl EntityInputHandler for TextArea {
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
        let range = self.edit_range(range_utf16.as_ref());
        self.marked_range = None;
        let text = clean(new_text);
        if text.is_empty() && range.is_empty() {
            cx.notify();
            return;
        }
        let caret = self.splice(range, &text, cx);
        self.selected_range = caret..caret;
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
        let text = clean(new_text);
        if text.is_empty() && range.is_empty() {
            return;
        }
        let start = range.start;
        let end = self.splice(range, &text, cx);
        self.marked_range = (!text.is_empty()).then_some(start..end);
        self.selected_range = match new_selected_range_utf16 {
            Some(selected) if text == new_text => {
                start + utf16_to_utf8(&text, selected.start)
                    ..start + utf16_to_utf8(&text, selected.end)
            }
            _ => end..end,
        };
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let at = layout.position(range.start);
        Some(Bounds::new(
            point(bounds.left() + at.x, bounds.top() + at.y - self.scroll_y),
            size(px(2.0), layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.layout.as_ref()?;
        let offset = self.offset_for_mouse(point);
        Some(utf8_to_utf16(&self.content, offset))
    }
}

impl Focusable for TextArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        if self.was_focused && !focused {
            cx.emit(InputEvent::Blurred);
        }
        self.was_focused = focused;
        let theme = theme(cx);
        div()
            .id(ElementId::from(("text-area", cx.entity_id())))
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
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
            .w_full()
            .px(SPACE_4)
            .py(SPACE_3)
            .rounded(RADIUS_MD)
            .bg(theme.bg)
            .border_1()
            .border_color(if focused {
                theme.accent
            } else {
                theme.border_strong
            })
            .text_size(FONT_BODY)
            .line_height(LINE_HEIGHT)
            .text_color(theme.fg)
            .font_family(SANS)
            .child(AreaElement { area: cx.entity() })
    }
}

/// The text as last laid out: one wrapped line per paragraph.
struct Layout {
    lines: Vec<WrappedLine>,
    /// Content offset of each paragraph's first byte.
    starts: Vec<usize>,
    /// Each paragraph's top, from the text's top.
    tops: Vec<Pixels>,
    height: Pixels,
    line_height: Pixels,
    /// Where the text was drawn (the field inside its padding).
    bounds: Bounds<Pixels>,
}

impl Layout {
    fn paragraph_at(&self, offset: usize) -> usize {
        self.starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1)
    }

    /// Where `offset` sits, from the text's top-left.
    fn position(&self, offset: usize) -> Point<Pixels> {
        let index = self.paragraph_at(offset);
        let local = offset - self.starts[index];
        let at = self.lines[index]
            .position_for_index(local, self.line_height)
            .unwrap_or_default();
        point(at.x, self.tops[index] + at.y)
    }

    /// The content offset nearest `at`, from the text's top-left.
    fn offset_at(&self, at: Point<Pixels>) -> usize {
        let index = self
            .tops
            .partition_point(|top| *top <= at.y)
            .saturating_sub(1);
        let line = &self.lines[index];
        let height = line.size(self.line_height).height;
        let y = (at.y - self.tops[index]).clamp(px(0.0), height - px(1.0));
        let local = line
            .closest_index_for_position(point(at.x.max(px(0.0)), y), self.line_height)
            .unwrap_or_else(|index| index);
        self.starts[index] + local.min(line.len())
    }

    /// The start offsets of each row of paragraph `index`, and its end.
    fn rows(&self, index: usize) -> Vec<usize> {
        let line = &self.lines[index];
        let mut rows = vec![0];
        rows.extend(
            line.wrap_boundaries()
                .iter()
                .map(|boundary| line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index),
        );
        rows.push(line.len());
        rows
    }

    /// Rectangles covering `selection`, from the text's top-left.
    fn selection(&self, selection: &Range<usize>) -> Vec<Bounds<Pixels>> {
        let mut quads = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let start = self.starts[index];
            let end = start + line.len();
            if selection.end < start || selection.start > end {
                continue;
            }
            let from = selection.start.max(start) - start;
            let to = selection.end.min(end) - start;
            // The line break a selection runs over shows as a sliver.
            let past_end = selection.end > end;
            let rows = self.rows(index);
            for (row, bounds) in rows.windows(2).enumerate() {
                let (row_start, row_end) = (bounds[0], bounds[1]);
                let last = row + 2 == rows.len();
                if to < row_start || from > row_end || (from == row_end && !last) {
                    continue;
                }
                let x = |offset: usize| {
                    line.position_for_index(offset, self.line_height)
                        .map_or(px(0.0), |at| at.x)
                };
                let left = if from <= row_start { px(0.0) } else { x(from) };
                let mut right = if to >= row_end { x(row_end) } else { x(to) };
                if last && past_end {
                    right += px(5.0);
                }
                if right > left {
                    let top = self.tops[index] + self.line_height * row as f32;
                    quads.push(Bounds::from_corners(
                        point(left, top),
                        point(right, top + self.line_height),
                    ));
                }
            }
        }
        quads
    }
}

struct AreaElement {
    area: Entity<TextArea>,
}

struct Prepaint {
    origin: Point<Pixels>,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for AreaElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AreaElement {
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
        let rows = self.area.read(cx).rows;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * rows as f32).into();
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
        let theme = theme(cx).clone();
        let area = self.area.read(cx);
        let style = window.text_style();
        let line_height = window.line_height();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let empty = area.content.is_empty();
        let (text, color): (SharedString, _) = if empty {
            (area.placeholder.clone(), theme.fg_dim)
        } else {
            (area.content.clone().into(), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match area
            .marked_range
            .as_ref()
            .filter(|marked| !empty && marked.end <= text.len())
        {
            Some(marked) => vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..run
                },
            ],
            None => vec![run],
        };
        let lines: Vec<WrappedLine> = window
            .text_system()
            .shape_text(text, font_size, &runs, Some(bounds.size.width), None)
            .map(|lines| lines.into_iter().collect())
            .unwrap_or_default();

        let mut starts = Vec::with_capacity(lines.len());
        let mut tops = Vec::with_capacity(lines.len());
        let (mut start, mut top) = (0, px(0.0));
        for line in &lines {
            starts.push(start);
            tops.push(top);
            start += line.len() + 1;
            top += line.size(line_height).height;
        }
        let layout = Layout {
            lines,
            starts,
            tops,
            height: top,
            line_height,
            bounds,
        };

        let cursor = if empty { 0 } else { area.cursor_offset() };
        let caret = layout.position(cursor);
        // Keep the caret in view after it moved; otherwise the wheel's
        // scroll stands. Never past the text.
        let mut scroll_y = area.scroll_y;
        if area.reveal {
            if caret.y < scroll_y {
                scroll_y = caret.y;
            } else if caret.y + line_height > scroll_y + bounds.size.height {
                scroll_y = caret.y + line_height - bounds.size.height;
            }
        }
        let most = (layout.height - bounds.size.height).max(px(0.0));
        scroll_y = scroll_y.clamp(px(0.0), most);
        let origin = point(bounds.left(), bounds.top() - scroll_y);

        let selection = if empty || area.selected_range.is_empty() {
            Vec::new()
        } else {
            layout
                .selection(&area.selected_range)
                .into_iter()
                .map(|quad| fill(quad + origin, theme.accent.alpha(0.28)))
                .collect()
        };
        let cursor = area.selected_range.is_empty().then(|| {
            fill(
                Bounds::new(
                    point(origin.x + caret.x, origin.y + caret.y),
                    size(px(1.5), line_height),
                ),
                theme.accent,
            )
        });
        self.area.update(cx, |area, _| {
            area.scroll_y = scroll_y;
            area.reveal = false;
            area.layout = Some(layout);
        });
        Prepaint {
            origin,
            selection,
            cursor,
        }
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
        let focus_handle = self.area.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.area.clone()),
            cx,
        );
        let Some(layout) = self.area.update(cx, |area, _| area.layout.take()) else {
            return;
        };
        let origin = prepaint.origin;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            for (line, top) in layout.lines.iter().zip(&layout.tops) {
                let at = point(origin.x, origin.y + *top);
                let height = line.size(layout.line_height).height;
                if at.y + height < bounds.top() || at.y > bounds.bottom() {
                    continue;
                }
                let _ = line.paint(at, layout.line_height, TextAlign::Left, None, window, cx);
            }
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        self.area.update(cx, |area, _| area.layout = Some(layout));
    }
}

/// Typed or pasted text as the field keeps it: line breaks as `\n`, tabs
/// as spaces, no other control characters.
fn clean(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter_map(|character| match character {
            '\n' => Some('\n'),
            '\r' => Some('\n'),
            '\t' => Some(' '),
            other if other.is_control() => None,
            other => Some(other),
        })
        .collect()
}

fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn previous_char(text: &str, offset: usize) -> usize {
    text[..clamp_to_char_boundary(text, offset)]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_char(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    text[offset..]
        .chars()
        .next()
        .map_or(text.len(), |character| offset + character.len_utf8())
}

/// The first offset of the line (between line breaks) holding `offset`.
fn line_start(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    text[..offset].rfind('\n').map_or(0, |index| index + 1)
}

/// The offset of the line break ending the line holding `offset`, or the
/// text's end.
fn line_end(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    text[offset..]
        .find('\n')
        .map_or(text.len(), |index| offset + index)
}

/// The word around `offset` (letters, digits, `_`, `@`), or the character
/// there.
fn word_at(text: &str, offset: usize) -> Range<usize> {
    let offset = clamp_to_char_boundary(text, offset);
    let word =
        |character: char| character.is_alphanumeric() || character == '_' || character == '@';
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, character)| word(*character))
        .last()
        .map_or(offset, |(index, _)| index);
    let end = text[offset..]
        .char_indices()
        .find(|(_, character)| !word(*character))
        .map_or(text.len(), |(index, _)| offset + index);
    if start == end {
        start..next_char(text, offset)
    } else {
        start..end
    }
}

/// Byte offset in `text` of UTF-16 offset `offset`.
fn utf16_to_utf8(text: &str, offset: usize) -> usize {
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
fn utf8_to_utf16(text: &str, offset: usize) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_words_are_found_on_character_boundaries() {
        let text = "first line\nsécond @maria\n";
        assert_eq!(line_start(text, 3), 0);
        assert_eq!(line_end(text, 3), 10);
        assert_eq!(line_start(text, 13), 11);
        assert_eq!(line_end(text, 13), text.len() - 1);
        assert_eq!(line_start(text, text.len()), text.len());
        assert_eq!(previous_char(text, 14), 12, "é is two bytes");
        assert_eq!(next_char(text, 12), 14);
        let mention = word_at(text, text.find("maria").unwrap() + 2);
        assert_eq!(&text[mention], "@maria");
        assert_eq!(word_at(text, 10), 6..10, "the word the click ends");
        assert_eq!(word_at("a  b", 2), 2..3, "a space alone");
    }

    #[test]
    fn pasted_text_keeps_line_breaks_and_drops_control_characters() {
        assert_eq!(clean("a\r\nb\rc\td\u{7}"), "a\nb\nc d");
    }

    #[test]
    fn utf16_offsets_round_trip() {
        let text = "a😀b\nc";
        assert_eq!(utf8_to_utf16(text, 5), 3);
        assert_eq!(utf16_to_utf8(text, 3), 5);
        assert_eq!(utf16_to_utf8(text, 99), text.len());
    }
}

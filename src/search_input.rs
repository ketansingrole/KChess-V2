use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, IntoElement, IsZero,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle,
    Window, div, fill, point, prelude::*, px, rgb, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    SearchBackspace, SearchCopy, SearchCut, SearchDelete, SearchDeleteToEnd, SearchDeleteToStart,
    SearchDeleteWordBackward, SearchDeleteWordForward, SearchEnd, SearchHome, SearchLeft,
    SearchPaste, SearchRedo, SearchRight, SearchSelectAll, SearchSelectLeft, SearchSelectRight,
    SearchSelectWordLeft, SearchSelectWordRight, SearchUndo, SearchWordLeft, SearchWordRight,
    ShowCharacterPalette, theme::ThemeState,
};

const MAX_UNDO_STEPS: usize = 100;

#[derive(Clone)]
struct InputSnapshot {
    content: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    scroll_x: Pixels,
}

pub struct SearchInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    scroll_x: Pixels,
    undo_stack: Vec<InputSnapshot>,
    redo_stack: Vec<InputSnapshot>,
    is_selecting: bool,
    theme_state: Option<Entity<ThemeState>>,
}

impl SearchInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            content: "".into(),
            placeholder: "Search".into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll_x: px(0.0),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            is_selecting: false,
            theme_state: None,
        }
    }

    pub fn set_theme_state(&mut self, theme_state: Entity<ThemeState>, cx: &mut Context<Self>) {
        self.theme_state = Some(theme_state);
        cx.notify();
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_owned().into();
        let len = self.content.len();
        self.selected_range = len..len;
        self.selection_reversed = false;
        self.marked_range = None;
        self.scroll_x = px(0.0);
        self.clear_history();
        cx.notify();
    }

    pub fn set_placeholder(&mut self, placeholder: &str, cx: &mut Context<Self>) {
        self.placeholder = placeholder.to_owned().into();
        cx.notify();
    }

    pub fn text(&self) -> String {
        self.content.to_string()
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn keyboard_focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    fn snapshot(&self) -> InputSnapshot {
        InputSnapshot {
            content: self.content.clone(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
            marked_range: self.marked_range.clone(),
            scroll_x: self.scroll_x,
        }
    }

    fn restore_snapshot(&mut self, snapshot: InputSnapshot, cx: &mut Context<Self>) {
        self.content = snapshot.content;
        self.selected_range = snapshot.selected_range;
        self.selection_reversed = snapshot.selection_reversed;
        self.marked_range = snapshot.marked_range;
        self.scroll_x = snapshot.scroll_x;
        cx.notify();
    }

    fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    fn push_undo_snapshot(&mut self, snapshot: InputSnapshot) {
        if self.undo_stack.len() >= MAX_UNDO_STEPS {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
    }

    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        let mut index = offset.min(self.content.len());
        while index > 0 {
            let prev = self.previous_boundary(index);
            let ch = self.content[prev..index].chars().next().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            index = prev;
        }

        if index == 0 {
            return 0;
        }

        let prev = self.previous_boundary(index);
        let class = Self::is_word_char(self.content[prev..index].chars().next().unwrap_or(' '));

        while index > 0 {
            let prev = self.previous_boundary(index);
            let ch = self.content[prev..index].chars().next().unwrap_or(' ');
            if ch.is_whitespace() || Self::is_word_char(ch) != class {
                break;
            }
            index = prev;
        }

        index
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        let len = self.content.len();
        let mut index = offset.min(len);
        while index < len {
            let next = self.next_boundary(index);
            let ch = self.content[index..next].chars().next().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            index = next;
        }

        if index >= len {
            return len;
        }

        let next = self.next_boundary(index);
        let class = Self::is_word_char(self.content[index..next].chars().next().unwrap_or(' '));

        while index < len {
            let next = self.next_boundary(index);
            let ch = self.content[index..next].chars().next().unwrap_or(' ');
            if ch.is_whitespace() || Self::is_word_char(ch) != class {
                break;
            }
            index = next;
        }

        index
    }

    fn delete_byte_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if range.is_empty() {
            return;
        }
        self.selected_range = range;
        self.selection_reversed = false;
        self.replace_text_in_range(None, "", window, cx);
    }

    fn apply_horizontal_scroll(&mut self, delta_x: Pixels, cx: &mut Context<Self>) -> bool {
        if self.content.is_empty() {
            if !self.scroll_x.is_zero() {
                self.scroll_x = px(0.0);
                cx.notify();
            }
            return false;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return false;
        };

        let viewport_width = f32::from(bounds.size.width).max(0.0);
        let content_width = f32::from(line.x_for_index(self.content.len()));
        let min_scroll_x = (viewport_width - content_width).min(0.0);
        if min_scroll_x >= 0.0 {
            if !self.scroll_x.is_zero() {
                self.scroll_x = px(0.0);
                cx.notify();
            }
            return false;
        }

        let current = f32::from(self.scroll_x);
        let next = (current - f32::from(delta_x)).clamp(min_scroll_x, 0.0);
        if (next - current).abs() < 0.01 {
            return false;
        }

        self.scroll_x = px(next);
        cx.notify();
        true
    }

    fn left(&mut self, _: &SearchLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &SearchRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SearchSelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SearchSelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn word_left(&mut self, _: &SearchWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selected_range.is_empty() {
            self.previous_word_boundary(self.cursor_offset())
        } else {
            self.previous_word_boundary(self.selected_range.start)
        };
        self.move_to(target, cx);
    }

    fn word_right(&mut self, _: &SearchWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selected_range.is_empty() {
            self.next_word_boundary(self.cursor_offset())
        } else {
            self.next_word_boundary(self.selected_range.end)
        };
        self.move_to(target, cx);
    }

    fn select_word_left(
        &mut self,
        _: &SearchSelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.previous_word_boundary(self.cursor_offset()), cx);
    }

    fn select_word_right(
        &mut self,
        _: &SearchSelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.next_word_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SearchSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &SearchHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &SearchEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &SearchBackspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &SearchDelete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_word_backward(
        &mut self,
        _: &SearchDeleteWordBackward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        let cursor = self.cursor_offset();
        let start = self.previous_word_boundary(cursor);
        self.delete_byte_range(start..cursor, window, cx);
    }

    fn delete_word_forward(
        &mut self,
        _: &SearchDeleteWordForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        let cursor = self.cursor_offset();
        let end = self.next_word_boundary(cursor);
        self.delete_byte_range(cursor..end, window, cx);
    }

    fn delete_to_start(
        &mut self,
        _: &SearchDeleteToStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        let cursor = self.cursor_offset();
        self.delete_byte_range(0..cursor, window, cx);
    }

    fn delete_to_end(
        &mut self,
        _: &SearchDeleteToEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        let cursor = self.cursor_offset();
        self.delete_byte_range(cursor..self.content.len(), window, cx);
    }

    fn undo(&mut self, _: &SearchUndo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.undo_stack.pop() else {
            return;
        };
        self.redo_stack.push(self.snapshot());
        self.restore_snapshot(snapshot, cx);
    }

    fn redo(&mut self, _: &SearchRedo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.redo_stack.pop() else {
            return;
        };
        self.push_undo_snapshot(self.snapshot());
        self.restore_snapshot(snapshot, cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height());
        let horizontal = if !delta.x.is_zero() {
            delta.x
        } else if event.modifiers.shift {
            delta.y
        } else {
            Pixels::ZERO
        };

        if horizontal.is_zero() {
            return;
        }

        if self.apply_horizontal_scroll(horizontal, cx) {
            cx.stop_propagation();
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &SearchPaste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace("\n", " "), window, cx);
        }
    }

    fn copy(&mut self, _: &SearchCopy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &SearchCut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };

        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }

        line.closest_index_for_x(position.x - bounds.left() - self.scroll_x)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };

        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }

        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for SearchInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.snapshot();
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        if range.is_empty() && new_text.is_empty() && self.marked_range.is_none() {
            return;
        }

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.push_undo_snapshot(before);
        self.redo_stack.clear();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.snapshot();
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }

        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        self.push_undo_snapshot(before);
        self.redo_stack.clear();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);

        Some(Bounds::from_corners(
            point(
                bounds.left() + self.scroll_x + last_layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + self.scroll_x + last_layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;
        let utf8_index = last_layout.index_for_x(line_point.x - self.scroll_x)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

struct SearchTextElement {
    input: Entity<SearchInput>,
}

struct SearchPrepaintState {
    line: Option<ShapedLine>,
    scroll_x: Pixels,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for SearchTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SearchTextElement {
    type RequestLayoutState = ();
    type PrepaintState = SearchPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let showing_placeholder = content.is_empty();
        let style = window.text_style();
        let palette = input
            .theme_state
            .as_ref()
            .map(|theme| theme.read(cx).palette());

        let (display_text, text_color) = if content.is_empty() {
            (
                input.placeholder.clone(),
                palette
                    .map(|palette| palette.text_muted.into())
                    .unwrap_or_else(|| gpui::hsla(0.0, 0.0, 0.38, 0.65)),
            )
        } else {
            (content, style.color)
        };

        let base_run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let runs = if let Some(marked_range) = input.marked_range.as_ref() {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..base_run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(base_run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..base_run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![base_run]
        };

        let display_text_len = display_text.len();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        let mut scroll_x = input.scroll_x;
        let cursor_pos = line.x_for_index(cursor);
        if showing_placeholder {
            scroll_x = px(0.0);
        } else {
            let viewport_width = f32::from(bounds.size.width).max(0.0);
            let content_width = f32::from(line.x_for_index(display_text_len));
            let cursor_x = f32::from(cursor_pos);
            let mut scroll_x_value = f32::from(scroll_x);
            let edge_padding = 2.0;
            let visible_cursor_x = cursor_x + scroll_x_value;

            if visible_cursor_x < edge_padding {
                scroll_x_value = edge_padding - cursor_x;
            } else if visible_cursor_x > viewport_width - edge_padding {
                scroll_x_value = (viewport_width - edge_padding) - cursor_x;
            }

            let min_scroll_x = (viewport_width - content_width).min(0.0);
            scroll_x = px(scroll_x_value.clamp(min_scroll_x, 0.0));
        }

        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + scroll_x + cursor_pos, bounds.top()),
                        size(px(1.8), bounds.bottom() - bounds.top()),
                    ),
                    palette
                        .map(|palette| palette.accent)
                        .unwrap_or_else(|| rgb(0x1d6de5)),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + scroll_x + line.x_for_index(selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + scroll_x + line.x_for_index(selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    palette
                        .map(|palette| palette.selection_bg)
                        .unwrap_or_else(|| rgba(0x3f7fe533)),
                )),
                None,
            )
        };

        SearchPrepaintState {
            line: Some(line),
            scroll_x,
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection)
        }

        let line = prepaint.line.take().expect("search text line available");
        line.paint(
            point(bounds.left() + prepaint.scroll_x, bounds.top()),
            window.line_height(),
            window,
            cx,
        )
        .unwrap();

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            input.scroll_x = prepaint.scroll_x;
        });
    }
}

impl Render for SearchInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text_color = self
            .theme_state
            .as_ref()
            .map(|theme| theme.read(cx).palette().text_primary)
            .unwrap_or_else(|| rgb(0x20252b));

        div()
            .flex()
            .w_full()
            .min_w(px(0.0))
            .overflow_hidden()
            .key_context("SearchInput")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
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
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::delete_to_start))
            .on_action(cx.listener(Self::delete_to_end))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .line_height(px(20.0))
            .text_size(px(13.0))
            .text_color(text_color)
            .child(
                div()
                    .h(px(20.0))
                    .min_w(px(0.0))
                    .w_full()
                    .overflow_hidden()
                    .child(SearchTextElement { input: cx.entity() }),
            )
    }
}

impl Focusable for SearchInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

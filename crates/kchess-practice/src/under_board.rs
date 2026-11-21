//! The row under a drill's board, as the game pages have it under theirs: a box to type the
//! square where a square is the answer, and Voice at the row's end, with what voice says on a line
//! below. Coordinates and Knight paths share it, so Voice sits where the game pages have it.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::theme::Palette;
use crate::widgets;

const FILES: &str = "abcdefgh";
const RANKS: &str = "12345678";

/// The height the row takes under the board, kept whether or not it shows anything, so the
/// board holds its size.
pub fn height() -> Pixels {
    widgets::STACK_GAP + widgets::PLAYER_LINE
}

/// A box in the move box's place for a square: a file, then a rank, answers at once, with no
/// Return.
pub struct SquareBox {
    input: Entity<InputState>,
}

impl SquareBox {
    /// A box that hands `typed` each square typed whole.
    pub fn new<V: 'static>(
        typed: fn(&mut V, &str, &mut Context<V>),
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> (SquareBox, Subscription) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a square"));
        let subscription =
            cx.subscribe_in(&input, window, move |this, input, event, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_lowercase();
                let (kept, square) = square_typed(&value);
                if let Some(square) = square {
                    typed(this, &square, cx);
                }
                if value != kept {
                    input.update(cx, |input, cx| input.set_value(kept, window, cx));
                }
            });
        (SquareBox { input }, subscription)
    }

    /// Whether the keys go to the box, so the page leaves them to it.
    pub fn focused(&self, window: &Window, cx: &App) -> bool {
        self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Empty the box, as a run starts: a file left in it is no part of the new run.
    pub fn clear(&self, window: &mut Window, cx: &mut App) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn entry(&self, enabled: bool, palette: &Palette) -> Div {
        widgets::text_entry(&self.input, enabled, "Type the square", false, palette)
            // The practice page takes the keys back on a click elsewhere; here the box keeps them.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }
}

/// What the box keeps of `value`, and the square typed whole, if one was: a file waits for its
/// rank, and anything else is dropped.
fn square_typed(value: &str) -> (String, Option<String>) {
    let mut chars = value.chars().filter(|c| !c.is_whitespace());
    match (chars.next(), chars.next()) {
        (Some(file), Some(rank)) if FILES.contains(file) && RANKS.contains(rank) => {
            (String::new(), Some(format!("{file}{rank}")))
        }
        (Some(file), _) if FILES.contains(file) => (file.to_string(), None),
        _ => (String::new(), None),
    }
}

/// The row: the square box (`square`, and whether a square may be typed now) and `voice` at its
/// end; empty, at the same height, with neither.
pub fn row(
    square: Option<(&SquareBox, bool)>,
    voice: Option<AnyElement>,
    palette: &Palette,
) -> Div {
    let entry = square.map_or_else(div, |(square, enabled)| square.entry(enabled, palette));
    match voice {
        Some(voice) => widgets::input_row(entry, voice),
        None => widgets::lone_entry(entry),
    }
    .h(widgets::PLAYER_LINE)
}

/// The board with the row under it, and the voice line under that while it has something to say.
pub fn column(
    id: &'static str,
    board_size: Pixels,
    board: impl IntoElement,
    row: Div,
    voice_line: Option<AnyElement>,
) -> impl IntoElement {
    v_flex()
        .id(id)
        .test_support()
        .w(board_size)
        .flex_none()
        .gap(widgets::STACK_GAP)
        .child(board)
        .child(row)
        .children(voice_line)
}

#[cfg(test)]
mod tests {
    use super::square_typed;

    #[test]
    fn a_square_answers_once_typed_whole() {
        assert_eq!(square_typed(""), (String::new(), None));
        assert_eq!(square_typed("e"), ("e".into(), None));
        assert_eq!(square_typed("e4"), (String::new(), Some("e4".into())));
        assert_eq!(square_typed("e 4"), (String::new(), Some("e4".into())));
        // A rank first, or a letter that is no file, is dropped; a file keeps waiting.
        assert_eq!(square_typed("4"), (String::new(), None));
        assert_eq!(square_typed("x"), (String::new(), None));
        assert_eq!(square_typed("ex"), ("e".into(), None));
        assert_eq!(square_typed("e9"), ("e".into(), None));
    }
}

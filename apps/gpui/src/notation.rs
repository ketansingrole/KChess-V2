//! How moves are written on screen: SAN with letters (Nf3), or with figurines, the piece drawn in
//! place of its letter, as Lichess's piece notation setting offers. Only what is shown changes:
//! accessible names, copied PGN and typed moves keep the letters.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::settings::SettingsHandle;

/// How figurines are drawn, or `None` while moves are written with letters.
pub type Figurine = Option<Figurines>;

/// A piece set's silhouettes, in the colour of the text they stand in.
#[derive(Clone)]
pub struct Figurines {
    set: SharedString,
    color: Hsla,
}

impl Figurines {
    /// The same figurines in another text colour.
    pub fn in_color(&self, color: Hsla) -> Figurines {
        Figurines {
            set: self.set.clone(),
            color,
        }
    }
}

/// How moves are shown now: the board's piece set in the body text's colour, when figurines are
/// on.
pub fn figurine(cx: &App) -> Figurine {
    let frontend = &cx.global::<SettingsHandle>().0.read(cx).settings.frontend;
    frontend.figurine_notation.then(|| Figurines {
        set: crate::board::looks::piece_set(&frontend.piece_set)
            .to_string()
            .into(),
        color: cx.global::<crate::theme::Palette>().text,
    })
}

/// A figurine's side: the height of a capital letter at the move list's text sizes.
const FIGURINE: Pixels = px(15.);

/// `san` as shown: as written, or with each piece letter drawn as that piece (a promotion's and a
/// drop's too, "e8=♕", "♘@f7"), a silhouette in the text's colour.
pub fn shown(san: &str, figurine: &Figurine) -> AnyElement {
    let Some(Figurines { set, color }) = figurine else {
        return san.to_string().into_any_element();
    };
    let mut parts: Vec<AnyElement> = Vec::new();
    let mut text = String::new();
    for part in split(san) {
        match part {
            Part::Text(c) => text.push(c),
            Part::Piece(letter) => {
                if !text.is_empty() {
                    parts.push(std::mem::take(&mut text).into_any_element());
                }
                parts.push(
                    svg()
                        .path(SharedString::from(format!("pieces/{set}/b{letter}.svg")))
                        .flex_none()
                        .size(FIGURINE)
                        .text_color(*color)
                        .into_any_element(),
                );
            }
        }
    }
    if !text.is_empty() {
        parts.push(text.into_any_element());
    }
    div()
        .flex()
        .flex_row()
        .justify_center()
        .items_center()
        .when(parts.len() > 1, |row| row.gap(px(1.)))
        .children(parts)
        .into_any_element()
}

#[derive(Debug, PartialEq)]
enum Part {
    Text(char),
    Piece(char),
}

/// A move's characters, its piece letters told apart from the rest (squares, captures, checks).
fn split(san: &str) -> Vec<Part> {
    san.chars()
        .map(|c| match c {
            'K' | 'Q' | 'R' | 'B' | 'N' => Part::Piece(c),
            c => Part::Text(c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Part, split};

    fn pieces(san: &str) -> String {
        split(san)
            .into_iter()
            .map(|part| match part {
                Part::Piece(letter) => format!("[{letter}]"),
                Part::Text(c) => c.to_string(),
            })
            .collect()
    }

    #[test]
    fn piece_letters_become_figurines() {
        assert_eq!(pieces("Nf3"), "[N]f3");
        assert_eq!(pieces("exd8=Q+"), "exd8=[Q]+");
        assert_eq!(pieces("Rxe1#"), "[R]xe1#");
        assert_eq!(pieces("O-O-O"), "O-O-O");
        assert_eq!(pieces("bxc3"), "bxc3");
        // Crazyhouse drops name their piece too.
        assert_eq!(pieces("N@f7"), "[N]@f7");
    }
}

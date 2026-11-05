//! How moves are written on screen: SAN with letters (Nf3), or with piece symbols (♘f3), as
//! Lichess's Piece notation setting offers. Only what is shown changes: accessible names, copied
//! PGN and typed moves keep the letters.

use gpui_kit::App;

use crate::settings::SettingsHandle;

/// Whether moves are shown with piece symbols.
pub fn figurine(cx: &App) -> bool {
    cx.global::<SettingsHandle>()
        .0
        .read(cx)
        .settings
        .frontend
        .figurine_notation
}

/// `san` as shown: its piece letters as symbols when `figurine`, a promotion's too ("e8=♕").
/// Castling, squares and the files of pawn captures stay as they are.
pub fn shown(san: &str, figurine: bool) -> String {
    if !figurine {
        return san.to_string();
    }
    san.chars()
        .map(|c| match c {
            'K' => '♔',
            'Q' => '♕',
            'R' => '♖',
            'B' => '♗',
            'N' => '♘',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::shown;

    #[test]
    fn pieces_become_symbols() {
        assert_eq!(shown("Nf3", true), "♘f3");
        assert_eq!(shown("exd8=Q+", true), "exd8=♕+");
        assert_eq!(shown("Rxe1#", true), "♖xe1#");
        assert_eq!(shown("O-O-O", true), "O-O-O");
        assert_eq!(shown("bxc3", true), "bxc3");
        // Crazyhouse drops name their piece too.
        assert_eq!(shown("N@f7", true), "♘@f7");
        assert_eq!(shown("Nf3", false), "Nf3");
    }
}

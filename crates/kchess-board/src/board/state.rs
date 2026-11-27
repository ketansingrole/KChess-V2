//! What a page shows on the board and hears from it: the position and its marks, arrows,
//! circles and badges, the editor's tools, and the board's events.

use super::*;

/// The checkmated king's "#" badge, Lichess's red.
const MATE_BADGE: u32 = 0xdf5353;

/// A move's label as a badge over its square's top-right corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    pub square: Square,
    pub glyph: &'static str,
    pub color: Hsla,
}

/// The board editor's tools: move pieces (dragged off the board, they are gone),
/// put one piece down (or take it off again where it already stands), or take pieces off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTool {
    Pointer,
    Place(Piece),
    Trash,
}

/// A spare piece dragged onto the editor's board.
pub struct SparePiece(pub Piece);

/// A ring over a square in a Chessground brush color (`yellow`, `green`, `red`).
#[derive(Clone, Debug, PartialEq)]
pub struct Circle {
    pub square: Square,
    pub color: u32,
}

impl Circle {
    /// A circle in one of Chessground's brushes.
    pub fn brush(brush: &str, square: Square) -> Circle {
        Circle {
            square,
            color: brush_color(brush),
        }
    }

    pub fn yellow(square: Square) -> Circle {
        Circle::brush("yellow", square)
    }

    pub fn green(square: Square) -> Circle {
        Circle::brush("green", square)
    }

    pub fn red(square: Square) -> Circle {
        Circle::brush("red", square)
    }
}

/// An arrow from one square to another: Chessground's brush color and opacity, and its line
/// width in 64ths of a square.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrow {
    pub from: Square,
    pub to: Square,
    pub color: u32,
    pub opacity: f32,
    pub width: f32,
    /// What a screen reader hears it as, for an arrow that tells the player a move (a hint, a
    /// solution); drawings and the engine's lines are seen, not said.
    pub spoken: Option<&'static str>,
}

/// Chessground's brushes by name: `green`, `red`, `blue` and `yellow`.
pub fn brush_color(brush: &str) -> u32 {
    match brush {
        "red" => 0x882020,
        "blue" => 0x003088,
        "yellow" => 0xe68f00,
        _ => 0x15781b,
    }
}

impl Arrow {
    /// An arrow in one of Chessground's brushes (a lesson's drawings).
    pub fn brush(brush: &str, from: Square, to: Square) -> Arrow {
        Arrow {
            from,
            to,
            color: brush_color(brush),
            opacity: 1.,
            width: 10.,
            spoken: None,
        }
    }

    /// The solution's next move, shown (the `green` brush).
    pub fn solution(from: Square, to: Square) -> Arrow {
        Arrow {
            spoken: Some("Solution"),
            ..Arrow::brush("green", from, to)
        }
    }

    /// A move shown on request (the `blue` brush).
    pub fn hint(from: Square, to: Square) -> Arrow {
        Arrow {
            spoken: Some("Hint"),
            ..Arrow::brush("blue", from, to)
        }
    }

    /// The analysis board's arrows: the best line pale blue and bold, the others pale grey and
    /// thinner.
    pub fn engine(from: Square, to: Square, rank: usize) -> Arrow {
        let (color, opacity) = if rank == 0 {
            (0x003088, 0.4)
        } else {
            (0x4a4a4a, 0.35)
        };
        Arrow {
            from,
            to,
            color,
            opacity,
            width: if rank == 0 {
                12.
            } else {
                (9. - rank as f32 * 2.).max(4.)
            },
            spoken: None,
        }
    }
}

/// What the page shows on the board.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardState {
    pub fen: String,
    pub orientation: Color,
    /// The side whose pieces may move now; `None` while nothing moves.
    pub movable: Option<Color>,
    /// Each piece's destinations, as `position::dests` lists them for a board.
    pub dests: HashMap<Square, Vec<Square>>,
    pub last_move: Option<(Square, Square)>,
    pub check: Option<Square>,
    /// Hide the pieces (blindfold play); highlights and moves stay.
    pub blindfold: bool,
    /// A small picture of a position: no coordinates, nothing to touch.
    pub preview: bool,
    /// Arrows over the pieces (the engine's choices), drawn as Chessground draws them.
    pub arrows: Vec<Arrow>,
    /// A puzzle hint: the piece to move, ringed under it.
    pub hint: Option<Square>,
    /// Rings over squares (a drill's target, a right or wrong answer).
    pub circles: Vec<Circle>,
    /// Coordinates other than Settings' (the coordinate drills hide or show them themselves).
    pub coordinates: Option<CoordinateMode>,
    /// Every square's name on it.
    pub square_names: bool,
    /// The board editor's tool: pieces move anywhere, are put down or taken off.
    pub edit: Option<EditTool>,
    /// A review's label on the move just played.
    pub badge: Option<Badge>,
    /// The side that may queue a move for its next turn while the other side is to move.
    pub premove: Option<Color>,
    /// Crazyhouse: the pieces each side holds in hand.
    pub pockets: Option<ByColor<ByRole<u8>>>,
    /// Crazyhouse: where each piece in the mover's pocket may be dropped.
    pub drops: HashMap<Role, Vec<Square>>,
    /// The board's corners, which its corner squares take too: `radii::BOARD`, or those of the
    /// box it sits flush in (a card's picture, `widgets::card_picture_corners`).
    pub corners: Corners<Pixels>,
}

impl BoardState {
    /// The standard starting position, White at the bottom and nothing to move: a board before
    /// its page shows anything.
    pub fn start() -> BoardState {
        BoardState::of(
            &VariantPosition::Chess(shakmaty::Chess::default()),
            Color::White,
            None,
            None,
        )
    }

    /// A position as the board shows it; `movable` is the side the player may move.
    pub fn of(
        position: &VariantPosition,
        orientation: Color,
        movable: Option<Color>,
        last_move: Option<(Square, Square)>,
    ) -> BoardState {
        let can_move = movable == Some(position.turn());
        let dests = if can_move {
            kchess_domain::position::dests(position, kchess_domain::position::DestMode::Board)
                .into_iter()
                .collect()
        } else {
            HashMap::new()
        };
        let check = position
            .is_check()
            .then(|| position.board().king_of(position.turn()))
            .flatten();
        // Checkmate marks the king as Lichess does: a red "#" over its square's corner.
        let badge = check
            .filter(|_| position.is_checkmate())
            .map(|square| Badge {
                square,
                glyph: "#",
                color: rgb(MATE_BADGE).into(),
            });
        let drops = if can_move {
            kchess_domain::position::drop_dests(position)
                .into_iter()
                .collect()
        } else {
            HashMap::new()
        };
        BoardState {
            fen: kchess_domain::rules::make_fen(position),
            orientation,
            movable: can_move.then_some(position.turn()),
            dests,
            last_move,
            check,
            blindfold: false,
            preview: false,
            arrows: Vec::new(),
            hint: None,
            circles: Vec::new(),
            coordinates: None,
            square_names: false,
            edit: None,
            badge,
            premove: None,
            pockets: position.pockets().copied(),
            drops,
            corners: Corners::all(radii::BOARD),
        }
    }
}

impl BoardState {
    /// A position only looked at (a live game, a broadcast board): its FEN, which may be just the
    /// placement and the side to move, and the last move in UCI.
    pub fn shown(
        fen: &str,
        variant: kchess_domain::variant::Variant,
        orientation: Color,
        last: Option<&str>,
    ) -> BoardState {
        let fields = fen.split_whitespace().count();
        let full = if fields >= 4 {
            fen.to_string()
        } else {
            format!(
                "{} - - 0 1",
                fen.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
            )
        };
        let last = last.and_then(last_squares);
        match kchess_domain::rules::setup_start(variant, &full).or_else(|| {
            kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, &full)
        }) {
            Some(position) => BoardState::of(&position, orientation, None, last),
            None => {
                let mut state = BoardState::of(
                    &VariantPosition::Chess(shakmaty::Chess::default()),
                    orientation,
                    None,
                    last,
                );
                state.fen = fen.to_string();
                state.check = None;
                state
            }
        }
    }
}

impl BoardState {
    /// A small board of a position that is only looked at: no keys or coordinates of its own.
    pub fn preview(
        fen: &str,
        variant: kchess_domain::variant::Variant,
        orientation: Color,
        last: Option<&str>,
    ) -> BoardState {
        let mut state = BoardState::shown(fen, variant, orientation, last);
        state.preview = true;
        state
    }
}

/// The squares a UCI move marks as the last move; a drop (`N@f3`) marks the one it landed on.
pub fn last_squares(uci: &str) -> Option<(Square, Square)> {
    if uci.get(1..2) == Some("@") {
        let to = uci.get(2..4)?.parse().ok()?;
        return Some((to, to));
    }
    let from = uci.get(0..2)?.parse().ok()?;
    let to = uci.get(2..4)?.parse().ok()?;
    Some((from, to))
}

/// A position's preview board's width, in the tooltip a move shows on hover.
pub const PREVIEW_TIP: Pixels = px(200.);

/// The tooltip of a move in a line: a small board of `fen`, reached by `uci` (marked as the last
/// move), seen from `orientation`. The board is made when the tooltip opens and let go with it.
pub fn preview_tip(
    fen: String,
    uci: String,
    variant: kchess_domain::variant::Variant,
    orientation: Color,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| {
        let board = cx
            .new(|_| ChessBoard::new(BoardState::preview(&fen, variant, orientation, Some(&uci))));
        gpui_kit::component::tooltip::Tooltip::element(move |_, _| {
            div().w(PREVIEW_TIP).py(px(6.)).child(board.clone())
        })
        .build(window, cx)
    }
}

/// What the board tells its page.
pub enum BoardEvent {
    /// A move the player made, in UCI (`e2e4`, `e7e8q`, castling as king to rook or two squares,
    /// a Crazyhouse drop as `N@f3`).
    Move(String),
    /// A square was clicked: the coordinate drills' answers.
    Select(Square),
    /// The editor's pieces changed: their placement (a FEN's first field).
    Edited(String),
}

#[cfg(test)]
mod preview_tests {
    use super::BoardState;
    use shakmaty::{Color, Square};

    /// A move's preview shows the position it reaches from the side the board beside it is seen
    /// from, the move marked, and takes no part in the page.
    #[test]
    fn a_preview_shows_where_a_move_leads() {
        let state = BoardState::preview(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
            kchess_domain::variant::Variant::Standard,
            Color::Black,
            Some("e2e4"),
        );
        assert!(state.preview);
        assert_eq!(state.orientation, Color::Black);
        assert_eq!(state.last_move, Some((Square::E2, Square::E4)));
        assert_eq!(
            state.fen,
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
        );
    }
}

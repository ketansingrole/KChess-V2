//! The chessboard: the chosen board
//! and pieces, the last move, check, the selected piece and where it can go, coordinates,
//! click-to-move and drag-and-drop, and the promotion picker. Squares, highlights and pieces are
//! painted in one pass; the page gives the board its position and hears its moves.

mod input;
mod keyboard;
pub mod looks;
mod paint;
mod pocket;
pub mod previews;
mod state;
mod view;

use input::Drawn;
use paint::{Frame, check_rings, paint_frame};
pub use previews::Previews;
pub use state::*;

pub use pocket::{PieceGhost, pocket};

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{CoordinateMode, PromotionMode};
use shakmaty::variant::VariantPosition;
use shakmaty::{Board, ByColor, ByRole, Color, File, Piece, Position as _, Rank, Role, Square};

use crate::settings::SettingsHandle;
use crate::theme::radii;
use crate::widgets::Pressable as _;

/// Chessground's square colors.
const LAST_MOVE: u32 = 0x9bc70069; // rgba(155, 199, 0, 0.41)
const SELECTED: u32 = 0x14551e80; // rgba(20, 85, 30, 0.5)
const DEST_DOT: u32 = 0x14551e80; // rgba(20, 85, 30, 0.5)
const DEST_CORNER: u32 = 0x1455004d; // rgba(20, 85, 0, 0.3)
/// The strip right of and below the board that holds coordinates drawn outside it, inside the
/// board's own bounds so the controls beside it cannot cover them.
const OUTSIDE_GUTTER: Pixels = px(16.);
const DEST_HOVER: u32 = 0x14551e4d; // rgba(20, 85, 30, 0.3)
/// Chessground's queued premove and its targets.
const PREMOVE: u32 = 0x141e5580; // rgba(20, 30, 85, 0.5)
const PREMOVE_DOT: u32 = 0x141e5580;
/// A press moves this far before it is a drag rather than a click.
const DRAG_THRESHOLD: Pixels = px(4.);

struct Drag {
    from: Square,
    piece: Piece,
    /// Where the pointer is, in window coordinates.
    position: Point<Pixels>,
    start: Point<Pixels>,
    moved: bool,
    /// The piece was already selected when pressed: let go of it on release unless dragged.
    again: bool,
}

/// Pieces sliding from one square to another, timed by the executor's clock: the real one in the
/// app, the test's own in tests, where a slide holds still until the test moves the clock.
struct Animation {
    moves: Vec<(Piece, Square, Square)>,
    start: std::time::Instant,
    duration: std::time::Duration,
}

impl Animation {
    fn ended(&self, now: std::time::Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.duration
    }

    /// How far along at `now`, eased as Chessground eases (cubic in, then out).
    fn progress(&self, now: std::time::Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start);
        let t = (elapsed.as_secs_f32() / self.duration.as_secs_f32()).min(1.);
        if t < 0.5 {
            4. * t * t * t
        } else {
            (t - 1.) * (2. * t - 2.) * (2. * t - 2.) + 1.
        }
    }
}

/// Whether a new position slides its pieces, for the Settings speed in milliseconds:
/// - not with Reduce motion on, which stills movement;
/// - not while the last slide is still under way: positions coming faster than a slide (an
///   arrow key held through a game, moves played back quickly) are shown at once, where
///   restarting each slide would keep the pieces always in motion and behind the moves;
/// - not when the board turned: a flipped board is redrawn, not slid.
fn slides_now(duration_ms: f64, reduce_motion: bool, running: bool, flipped: bool) -> bool {
    duration_ms > 0. && !reduce_motion && !running && !flipped
}

/// The pieces that moved from `old` to `new`: each piece that left a square, paired with the
/// nearest square a like piece arrived on. More than a few is a new position, not a move.
fn slides(old: &Board, new: &Board) -> Vec<(Piece, Square, Square)> {
    let left: Vec<(Square, Piece)> = old
        .occupied()
        .into_iter()
        .filter_map(|square| {
            let piece = old.piece_at(square)?;
            (new.piece_at(square) != Some(piece)).then_some((square, piece))
        })
        .collect();
    let mut arrived: Vec<(Square, Piece)> = new
        .occupied()
        .into_iter()
        .filter_map(|square| {
            let piece = new.piece_at(square)?;
            (old.piece_at(square) != Some(piece)).then_some((square, piece))
        })
        .collect();
    let mut moves = Vec::new();
    for (from, piece) in left {
        let nearest = arrived
            .iter()
            .enumerate()
            .filter(|(_, (_, p))| *p == piece)
            .min_by_key(|(_, (to, _))| from.distance(*to))
            .map(|(index, _)| index);
        if let Some(index) = nearest {
            let (to, _) = arrived.remove(index);
            moves.push((piece, from, to));
        }
    }
    if moves.len() > 4 { Vec::new() } else { moves }
}

/// Where a piece could go next turn, whatever stands in the way (Chessground's `premove`).
fn premove_targets(pieces: &Board, from: Square) -> Vec<Square> {
    let Some(piece) = pieces.piece_at(from) else {
        return Vec::new();
    };
    let (x1, y1) = (i32::from(from.file()), i32::from(from.rank()));
    let forward = if piece.color == Color::White { 1 } else { -1 };
    let start_rank = if piece.color == Color::White { 1 } else { 6 };
    let fits = |x2: i32, y2: i32| -> bool {
        let (dx, dy) = ((x2 - x1).abs(), (y2 - y1).abs());
        match piece.role {
            Role::Pawn => {
                (dx == 0 && y2 - y1 == forward)
                    || (dx == 0 && y1 == start_rank && y2 - y1 == 2 * forward)
                    || (dx == 1 && y2 - y1 == forward)
            }
            Role::Knight => (dx == 1 && dy == 2) || (dx == 2 && dy == 1),
            Role::Bishop => dx == dy,
            Role::Rook => dx == 0 || dy == 0,
            Role::Queen => dx == dy || dx == 0 || dy == 0,
            Role::King => {
                let home = if piece.color == Color::White { 0 } else { 7 };
                let castling = y1 == home && y2 == home && x1 == 4 && (x2 == 2 || x2 == 6) && {
                    let rook = if x2 == 6 { 7 } else { 0 };
                    pieces.piece_at(Square::from_coords(File::new(rook), Rank::new(home as u32)))
                        == Some(Piece {
                            color: piece.color,
                            role: Role::Rook,
                        })
                };
                (dx <= 1 && dy <= 1) || castling
            }
        }
    };
    Square::ALL
        .into_iter()
        .filter(|&to| to != from && fits(i32::from(to.file()), i32::from(to.rank())))
        .collect()
}

pub struct ChessBoard {
    state: BoardState,
    pieces: Board,
    /// The player's right-button drawings, until a left click clears them.
    drawn: Vec<Drawn>,
    /// A drawing under way: where it started, its brush, and the square under the pointer.
    drawing: Option<(Square, &'static str, Option<Square>)>,
    /// A move queued for the next turn (a premove).
    queued: Option<(Square, Square)>,
    /// Pieces sliding to where the new position put them.
    animation: Option<Animation>,
    _animating: Option<Task<()>>,
    selected: Option<Square>,
    /// A Crazyhouse pocket piece picked to be dropped.
    selected_drop: Option<Role>,
    drag: Option<Drag>,
    /// A pawn move waiting for its promotion piece.
    promotion: Option<(Square, Square)>,
    /// The promotion piece the keyboard has marked (`keyboard::PROMOTION`).
    promotion_choice: usize,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The board's own keys, which Tab gives it: made with its first frame.
    focus: Option<FocusHandle>,
    /// The square the keyboard is on; it starts where `keyboard::start` puts it.
    cursor: Option<Square>,
}

/// What a press on a square does, from the mouse or the keyboard alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    /// The picked pocket piece is dropped there.
    Drop(Role),
    /// The selected piece moves there, from this square.
    Play(Square),
    /// A premove was queued, or one waiting was taken back.
    Queued,
    /// A piece that may move was picked up; `again` when it was already selected.
    Pick { piece: Piece, again: bool },
    /// Nothing to move there: the selection goes.
    Clear,
}

impl EventEmitter<BoardEvent> for ChessBoard {}

impl ChessBoard {
    pub fn new(state: BoardState) -> ChessBoard {
        ChessBoard {
            pieces: board_of(&state.fen),
            drawn: Vec::new(),
            drawing: None,
            queued: None,
            animation: None,
            _animating: None,
            state,
            selected: None,
            selected_drop: None,
            drag: None,
            promotion: None,
            promotion_choice: 0,
            bounds: Rc::new(Cell::new(Bounds::default())),
            focus: None,
            cursor: None,
        }
    }

    /// The position shown, and how.
    #[cfg(any(test, feature = "test-support"))]
    pub fn state(&self) -> &BoardState {
        &self.state
    }

    /// Show another position (after a move, a takeback, a new game).
    pub fn set_state(&mut self, state: BoardState, cx: &mut Context<Self>) {
        if state == self.state {
            return;
        }
        if state.fen != self.state.fen {
            let next = board_of(&state.fen);
            if !state.preview {
                self.animate(&next, state.orientation, cx);
            }
            self.pieces = next;
            self.promotion = None;
        }
        // A selection, and a promotion being chosen, survive only while their piece can still
        // move: a game that ended on the same position left the picker open, still playing.
        if self
            .selected
            .is_some_and(|square| !state.dests.contains_key(&square))
        {
            self.selected = None;
        }
        if self
            .promotion
            .is_some_and(|(from, _)| !state.dests.contains_key(&from))
        {
            self.promotion = None;
        }
        if self
            .selected_drop
            .is_some_and(|role| !state.drops.contains_key(&role))
        {
            self.selected_drop = None;
        }
        self.state = state;
        self.play_queued(cx);
        cx.notify();
    }

    /// Put the pieces back as the state has them: a move the board played at once and the page
    /// refused goes back, which `set_state` cannot see when the position did not change.
    pub fn undo_move(&mut self, cx: &mut Context<Self>) {
        self.pieces = board_of(&self.state.fen);
        self.promotion = None;
        cx.notify();
    }

    /// Slide the pieces from `self.pieces` to `next` (Chessground's `animate`): each piece that
    /// left a square goes to the nearest square a like piece arrived on.
    fn animate(&mut self, next: &Board, orientation: Color, cx: &mut Context<Self>) {
        let choice = crate::settings::frontend(cx).piece_animation;
        let choice = serde_json::to_value(choice)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "normal".into());
        let duration = looks::animation_ms(&choice);
        let moves = slides(&self.pieces, next);
        let now = cx.background_executor().now();
        let running = self
            .animation
            .as_ref()
            .is_some_and(|animation| !animation.ended(now));
        let flipped = orientation != self.state.orientation;
        if !slides_now(duration, cx.reduce_motion(), running, flipped) || moves.is_empty() {
            self.animation = None;
            return;
        }
        let duration = std::time::Duration::from_secs_f64(duration / 1000.);
        self.animation = Some(Animation {
            moves,
            start: now,
            duration,
        });
        // The frames between are drawn on the display's own clock (`render` asks for the next
        // one while the slide runs), so a 120 Hz screen slides at 120 Hz, in step with its
        // refresh; this only ends the slide, for a window that draws no more frames (hidden,
        // or a test's, which has no frame clock).
        self._animating = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            this.update(cx, |this, cx| {
                let now = cx.background_executor().now();
                if this
                    .animation
                    .as_ref()
                    .is_none_or(|animation| animation.ended(now))
                {
                    this.animation = None;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// A queued premove plays once its side is to move and it is legal; otherwise it is dropped.
    fn play_queued(&mut self, cx: &mut Context<Self>) {
        let Some((from, to)) = self.queued else {
            return;
        };
        let mine = self.pieces.piece_at(from).map(|piece| piece.color);
        if self.state.movable.is_some() && self.state.movable == mine {
            self.queued = None;
            if self.reaches(from, to) {
                self.play_move(from, to, true, cx);
            }
        } else if self.state.premove.is_none() || self.state.premove != mine {
            self.queued = None;
        }
    }

    fn square_size(&self) -> Pixels {
        self.bounds.get().size.width / 8.
    }

    /// Where a square is drawn, relative to the board: column and row from the top left.
    fn cell(&self, square: Square) -> (u32, u32) {
        let (file, rank) = (u32::from(square.file()), u32::from(square.rank()));
        match self.state.orientation {
            Color::White => (file, 7 - rank),
            Color::Black => (7 - file, rank),
        }
    }

    /// The square under a point in window coordinates.
    fn square_at(&self, point: Point<Pixels>) -> Option<Square> {
        let bounds = self.bounds.get();
        let side = self.square_size();
        if side <= px(0.) || !bounds.contains(&point) {
            return None;
        }
        let column = ((point.x - bounds.origin.x) / side).floor() as u32;
        let row = ((point.y - bounds.origin.y) / side).floor() as u32;
        if column > 7 || row > 7 {
            return None;
        }
        let (file, rank) = match self.state.orientation {
            Color::White => (column, 7 - row),
            Color::Black => (7 - column, row),
        };
        Some(Square::from_coords(File::new(file), Rank::new(rank)))
    }

    fn can_move_from(&self, square: Square) -> bool {
        self.state
            .dests
            .get(&square)
            .is_some_and(|targets| !targets.is_empty())
    }

    /// A premove from `from` to `to` can be queued.
    fn premoves_to(&self, from: Square, to: Square) -> bool {
        self.state.movable.is_none()
            && self
                .pieces
                .piece_at(from)
                .is_some_and(|piece| Some(piece.color) == self.state.premove)
            && premove_targets(&self.pieces, from).contains(&to)
    }

    fn reaches(&self, from: Square, to: Square) -> bool {
        self.state
            .dests
            .get(&from)
            .is_some_and(|targets| targets.contains(&to))
    }

    /// A pocket piece of this role can be dropped on the square.
    fn drops_to(&self, role: Role, square: Square) -> bool {
        self.state
            .drops
            .get(&role)
            .is_some_and(|squares| squares.contains(&square))
    }

    /* ── The keyboard ─────────────────────────────────────────────────── */
}

/// The pieces of a FEN's placement; a Crazyhouse pocket (`[Pn]`, or a ninth rank) and promoted
/// marks (`~`) are not pieces on the board.
fn board_of(fen: &str) -> Board {
    let field = fen.split(' ').next().unwrap_or_default();
    let field = field.split('[').next().unwrap_or_default();
    let placement = field
        .split('/')
        .take(8)
        .collect::<Vec<_>>()
        .join("/")
        .replace('~', "");
    Board::from_ascii_board_fen(placement.as_bytes()).unwrap_or_else(|_| Board::empty())
}

/// The board as a cached view (`Entity::cached`): its last frame is reused while only its parent
/// redraws (a clock tick, an engine line) and drawn again when the board itself changes
/// (`set_state`, a drag, an animation step) or the windows refresh (settings, preferences). It
/// fills its slot's width and is square, as its own root is.
pub fn cached(board: &Entity<ChessBoard>) -> AnyElement {
    let mut style = StyleRefinement::default();
    style.size.width = Some(relative(1.).into());
    style.aspect_ratio = Some(1.);
    board.clone().cached(style).into_any_element()
}

/// Whether a page's keys reach it: the page itself has them, or the board on it does (a board
/// takes the arrows, Enter, Space and Escape and passes every other key on to its page).
pub fn page_has_keys(page: &FocusHandle, window: &Window, cx: &App) -> bool {
    page.is_focused(window)
        || (page.contains_focused(window, cx)
            && window
                .context_stack()
                .last()
                .is_some_and(|context| context.contains(keyboard::CONTEXT)))
}

/// The strip a board that follows Settings keeps right of and below its squares for coordinates
/// drawn outside, so what stands beside it (the evaluation bar) can end where the squares end.
pub fn outside_gutter(cx: &App) -> Pixels {
    let settings = &cx.global::<SettingsHandle>().0.read(cx).settings;
    if settings.frontend.coordinates == CoordinateMode::Outside {
        OUTSIDE_GUTTER
    } else {
        px(0.)
    }
}

#[cfg(test)]
mod premove_tests {
    use super::{board_of, premove_targets, slides, slides_now};
    use shakmaty::Square;

    #[test]
    fn premoves_ignore_what_stands_in_the_way() {
        let start = board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let rook = premove_targets(&start, Square::A1);
        assert!(rook.contains(&Square::A8) && rook.contains(&Square::H1));
        let pawn = premove_targets(&start, Square::E2);
        assert_eq!(pawn.len(), 4); // e3, e4, d3, f3
        let king = premove_targets(&start, Square::E1);
        assert!(king.contains(&Square::G1) && king.contains(&Square::C1));
    }

    #[test]
    fn a_move_slides_its_piece_and_a_new_game_jumps() {
        let before = board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let after = board_of("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1");
        let moves = slides(&before, &after);
        assert_eq!(moves.len(), 1);
        assert_eq!((moves[0].1, moves[0].2), (Square::E2, Square::E4));
        let castled = board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQ1RK1 b kq - 0 1");
        let castle = board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQK2R w KQkq - 0 1");
        assert_eq!(slides(&castle, &castled).len(), 2);
        let empty = board_of("8/8/8/8/8/8/8/8 w - - 0 1");
        assert!(slides(&empty, &before).is_empty());
    }

    /// A slide still under way when the next position comes (an arrow key held) is cut short
    /// and the new position stands at once; Reduce motion, the None speed and a flip never slide.
    #[test]
    fn pieces_slide_only_when_there_is_time_to() {
        assert!(slides_now(180., false, false, false));
        assert!(
            !slides_now(180., false, true, false),
            "the last slide still runs"
        );
        assert!(!slides_now(180., true, false, false), "Reduce motion");
        assert!(!slides_now(0., false, false, false), "the None speed");
        assert!(!slides_now(180., false, false, true), "a flipped board");
    }
}

#[cfg(test)]
mod fen_tests {
    use super::board_of;
    use shakmaty::{Board, Square};

    /// Crazyhouse FENs once showed an empty board: the pocket made the placement unreadable.
    #[test]
    fn a_crazyhouse_placement_keeps_its_pieces() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";
        for fen in [
            format!("{start}[PPpb] w KQkq - 0 1"),
            format!("{start}/PPpb w KQkq - 0 1"),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQ~KBNR[] w - - 0 1".to_string(),
        ] {
            assert_eq!(board_of(&fen), Board::default(), "{fen}");
        }
        assert!(
            board_of("4k3/8/8/8/8/8/8/3Q~K3[] w - - 0 1")
                .piece_at(Square::D1)
                .is_some()
        );
    }
}

#[cfg(test)]
mod keyboard_tests {
    use super::{BoardState, ChessBoard, Press, keyboard};
    use kchess_domain::variant::Variant;
    use shakmaty::{Color, Role, Square};

    fn board(fen: &str, variant: Variant, orientation: Color) -> ChessBoard {
        let position = kchess_domain::rules::setup_start(variant, fen).expect("a position");
        let turn = kchess_domain::rules::fen_turn(fen);
        ChessBoard::new(BoardState::of(&position, orientation, Some(turn), None))
    }

    fn walk(cursor: Square, orientation: Color, keys: &[&str]) -> Square {
        keys.iter().fold(cursor, |square, key| {
            keyboard::step(square, orientation, key).expect("an arrow")
        })
    }

    /// The cursor walked with the arrows, then Enter on each end: the piece is picked, a square it
    /// cannot reach lets go of it as a click does, and a square it can reach plays it.
    #[test]
    fn enter_picks_a_piece_and_plays_it_where_the_cursor_is() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let mut board = board(start, Variant::Standard, Color::White);
        let cursor = board.cursor();
        assert_eq!(cursor, Square::E2);
        assert!(matches!(
            board.press_key(cursor),
            Press::Pick { again: false, .. }
        ));
        assert_eq!(board.selected, Some(Square::E2));
        assert!(board.targets().contains(&Square::E4));
        // Three up is e5, which the pawn cannot reach: the selection goes.
        let e5 = walk(cursor, Color::White, &["up", "up", "up"]);
        assert_eq!(e5, Square::E5);
        assert_eq!(board.press_key(e5), Press::Clear);
        assert_eq!(board.selected, None);
        // Enter twice on the same piece lets go of it.
        board.press_key(Square::E2);
        assert!(matches!(
            board.press_key(Square::E2),
            Press::Pick { again: true, .. }
        ));
        assert_eq!(board.selected, None);
        board.press_key(Square::E2);
        let e4 = walk(e5, Color::White, &["down"]);
        assert_eq!(board.press_key(e4), Press::Play(Square::E2));
        assert_eq!(board.selected, None);
    }

    /// From Black's side the same keys walk the board turned round.
    #[test]
    fn a_turned_board_plays_from_the_keyboard_too() {
        let after = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let mut board = board(after, Variant::Standard, Color::Black);
        assert_eq!(board.cursor(), Square::E7);
        // Right, from Black's side, is toward the a-file.
        let d7 = walk(board.cursor(), Color::Black, &["right"]);
        assert_eq!(d7, Square::D7);
        board.press_key(d7);
        let d5 = walk(d7, Color::Black, &["up", "up"]);
        assert_eq!(d5, Square::D5);
        assert_eq!(board.press_key(d5), Press::Play(Square::D7));
    }

    /// A pawn reaching the last rank is played as any move; the picker then asks which piece,
    /// which the keyboard answers (`keyboard::promotion_key`).
    #[test]
    fn a_promotion_is_played_from_the_keyboard() {
        let mut board = board(
            "4k3/P7/8/8/8/8/8/4K3 w - - 0 1",
            Variant::Standard,
            Color::White,
        );
        board.press_key(Square::A7);
        assert!(board.targets().contains(&Square::A8));
        assert_eq!(board.press_key(Square::A8), Press::Play(Square::A7));
        assert_eq!(
            keyboard::promotion_key(0, "left"),
            Some(keyboard::PromotionKey::Highlight(3))
        );
        assert_eq!(
            keyboard::promotion_key(3, "enter"),
            Some(keyboard::PromotionKey::Pick(Role::Knight))
        );
    }

    /// The pieces to promote to were left on offer when the game ended on the same position, and
    /// picking one still played the move.
    #[gpui_kit::test]
    fn the_promotion_picker_closes_when_its_pawn_can_no_longer_move(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::AppContext as _;
        let fen = "4k3/P7/8/8/8/8/8/4K3 w - - 0 1";
        let board = cx.new(|_| board(fen, Variant::Standard, Color::White));
        let open = board.update(cx, |board, cx| {
            board.promotion = Some((Square::A7, Square::A8));
            board.set_state(
                BoardState::shown(
                    fen,
                    kchess_domain::variant::Variant::Standard,
                    Color::White,
                    None,
                ),
                cx,
            );
            board.promotion
        });
        assert_eq!(open, None);
    }

    /// A pocket piece picked, Enter drops it on the cursor's square where it may land.
    #[test]
    fn enter_drops_a_picked_pocket_piece() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[N] w KQkq - 0 1";
        let mut board = board(fen, Variant::Crazyhouse, Color::White);
        board.selected_drop = Some(Role::Knight);
        assert!(board.targets().contains(&Square::E4));
        assert_eq!(board.press_key(Square::E4), Press::Drop(Role::Knight));
        assert_eq!(board.selected_drop, None);
        // On a piece, the pocket piece goes and the piece is picked instead.
        board.selected_drop = Some(Role::Knight);
        assert!(matches!(board.press_key(Square::E2), Press::Pick { .. }));
        assert_eq!(board.selected_drop, None);
    }

    /// A board only looked at takes no moves from the keyboard, though its squares can be read.
    #[test]
    fn a_shown_board_takes_no_moves() {
        let mut board = ChessBoard::new(BoardState::shown(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            kchess_domain::variant::Variant::Standard,
            Color::White,
            None,
        ));
        assert!(!board.interactive());
        assert_eq!(board.press_key(Square::E2), Press::Clear);
        assert_eq!(board.selected, None);
    }
}

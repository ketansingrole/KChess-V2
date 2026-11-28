//! The board under the pointer and the keys: pressing, dragging and dropping, drawing arrows and
//! circles, editing a position, promotion, and the cursor the keyboard moves.

use super::*;

/// A shape the player drew with the right button: an arrow, or a circle where `from == to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Drawn {
    pub(super) from: Square,
    pub(super) to: Square,
    pub(super) brush: &'static str,
}

/// Chessground's brush for a drawing (`eventBrush`): with the right button Shift or Ctrl red, Alt
/// or ⌘ blue, both yellow, else green; a Shift-drag of the left button draws green, or blue with
/// Alt or ⌘.
fn drawing_brush(modifiers: &Modifiers, right: bool) -> &'static str {
    let a = (modifiers.shift || modifiers.control) && right;
    let b = modifiers.alt || modifiers.platform;
    ["green", "red", "blue", "yellow"][usize::from(a) + 2 * usize::from(b)]
}

/// Draw `shape`: the same shape again takes it off, the same squares in another brush recolour
/// it.
fn toggle_shape(shapes: &mut Vec<Drawn>, shape: Drawn) {
    match shapes
        .iter()
        .position(|s| s.from == shape.from && s.to == shape.to)
    {
        Some(index) if shapes[index].brush == shape.brush => {
            shapes.remove(index);
        }
        Some(index) => shapes[index] = shape,
        None => shapes.push(shape),
    }
}

/// Whether the pointer went far enough from where it was pressed to be a drag.
fn far(start: Point<Pixels>, now: Point<Pixels>) -> bool {
    let (dx, dy) = ((now.x - start.x).as_f32(), (now.y - start.y).as_f32());
    dx * dx + dy * dy >= DRAG_THRESHOLD.as_f32().powi(2)
}

impl ChessBoard {
    /// Pick a pocket piece to drop (or let go of it, picked again); a board piece's selection
    /// goes.
    pub fn select_drop(&mut self, role: Role, cx: &mut Context<Self>) {
        if !self.state.drops.contains_key(&role) || self.promotion.is_some() {
            return;
        }
        self.selected_drop = (self.selected_drop != Some(role)).then_some(role);
        self.selected = None;
        cx.notify();
    }

    /// Drop a pocket piece: it lands at once and the turn passes, and the page's next position
    /// confirms or undoes it.
    fn drop_piece(&mut self, role: Role, to: Square, cx: &mut Context<Self>) {
        if let Some(color) = self.state.movable {
            self.pieces.set_piece_at(to, Piece { color, role });
        }
        self.selected_drop = None;
        self.moved(to, to);
        cx.emit(BoardEvent::Move(format!(
            "{}@{to}",
            role.char().to_ascii_uppercase()
        )));
        cx.notify();
    }

    /// A pocket piece dragged onto the board.
    pub(super) fn drop_pocket(
        &mut self,
        role: Role,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        match self.square_at(position) {
            Some(square) if self.drops_to(role, square) => self.drop_piece(role, square, cx),
            _ => {
                self.selected_drop = None;
                cx.notify();
            }
        }
    }

    /// A left press (Chessground's `startDragOrDraw`): it ends a drag or a drawing under way,
    /// Shift starts a drawing, and otherwise it plays, premoves or picks up.
    pub(super) fn press(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.promotion.is_some() || self.state.preview {
            return;
        }
        if self.cancel_gesture(cx) {
            return;
        }
        if event.modifiers.shift && self.state.edit.is_none() {
            return self.start_drawing(event, false, cx);
        }
        let Some(square) = self.square_at(event.position) else {
            return;
        };
        if let Some(tool) = self.state.edit {
            return self.edit_press(tool, square, event.position, cx);
        }
        // A press with nothing selected clears the drawings (`eraseOnMovablePieceClick`).
        if self.selected.is_none() && !self.drawn.is_empty() {
            self.drawn.clear();
        }
        cx.emit(BoardEvent::Select(square));
        match self.decide(square) {
            Press::Drop(role) => return self.drop_piece(role, square, cx),
            Press::Play(from) => {
                // A click-to-move slides its piece, and the next piece pressed waits for a drag.
                self.dragged = false;
                return self.play_move(from, square, false, true, cx);
            }
            Press::Pick { piece, again } => {
                // The piece lifts as it is pressed, unless the last move was clicked; a second
                // press on the selected piece lets go of it on release, unless it is dragged.
                self.drag = Some(Drag {
                    from: square,
                    piece,
                    position: event.position,
                    start: event.position,
                    moved: self.dragged,
                    again,
                    left: false,
                });
            }
            // A premove clicked in counts as a click-to-move.
            Press::Queued if self.queued.is_some() => self.dragged = false,
            Press::Queued | Press::Clear => {}
        }
        cx.notify();
    }

    /// A press while a drag or a drawing is under way ends it, and does nothing else. Whether
    /// there was one.
    fn cancel_gesture(&mut self, cx: &mut Context<Self>) -> bool {
        if self.drag.take().is_some() {
            self.selected = None;
        } else if self.drawing.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    /// What pressing `square` does to the selection, a picked pocket piece and a queued premove,
    /// and what is left to play (Chessground's `selectSquare` and the press around it): the
    /// selected piece moves or premoves there, or the press picks up a piece of the side to move
    /// or to premove, even one with nowhere to go. A premove waiting stays while a piece is
    /// picked up, which a release ends, and goes on any other press.
    fn decide(&mut self, square: Square) -> Press {
        if let Some(role) = self.selected_drop.take()
            && self.drops_to(role, square)
        {
            self.selected = None;
            return Press::Drop(role);
        }
        let before = self.selected;
        if let Some(from) = before
            && from != square
        {
            if self.reaches(from, square) {
                self.selected = None;
                return Press::Play(from);
            }
            self.selected = None;
            if self.premoves_to(from, square) {
                self.queued = Some((from, square));
                return Press::Queued;
            }
        }
        match self.pieces.piece_at(square) {
            Some(piece)
                if Some(piece.color) == self.state.movable
                    || (Some(piece.color) == self.state.premove
                        && self.state.movable.is_none()) =>
            {
                self.selected = Some(square);
                Press::Pick {
                    piece,
                    again: before == Some(square),
                }
            }
            _ => {
                self.selected = None;
                if self.queued.take().is_some() {
                    Press::Queued
                } else {
                    Press::Clear
                }
            }
        }
    }

    /// A right press: it ends a drag or a drawing under way, or starts a drawing.
    pub(super) fn right_press(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.state.preview || self.cancel_gesture(cx) {
            return;
        }
        self.start_drawing(event, true, cx);
    }

    /// Start a drawing on a square (the right button, or Shift with the left). As on Lichess it
    /// takes back a queued premove and lets go of the piece picked (`cancelMove`).
    fn start_drawing(&mut self, event: &MouseDownEvent, right: bool, cx: &mut Context<Self>) {
        if self.state.preview || self.state.edit.is_some() {
            return;
        }
        self.queued = None;
        self.selected = None;
        self.selected_drop = None;
        if let Some(square) = self.square_at(event.position) {
            self.drawing = Some((square, drawing_brush(&event.modifiers, right), Some(square)));
        }
        cx.notify();
    }

    pub(super) fn draw_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let square = self.square_at(position);
        if let Some((_, _, current)) = &mut self.drawing
            && *current != square
        {
            *current = square;
            cx.notify();
        }
    }

    /// Letting go draws the arrow (or the circle, on the same square), or takes it off.
    pub(super) fn finish_drawing(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((from, brush, _)) = self.drawing.take() else {
            return;
        };
        if let Some(to) = self.square_at(position) {
            toggle_shape(&mut self.drawn, Drawn { from, to, brush });
        }
        cx.notify();
    }

    pub(super) fn pointer_moved(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let square = self.square_at(position);
        let Some(drag) = &mut self.drag else {
            return;
        };
        drag.position = position;
        if far(drag.start, position) {
            drag.moved = true;
        }
        if drag.moved && square != Some(drag.from) {
            drag.left = true;
        }
        cx.notify();
    }

    /// The pointer over the board: a square a selected piece could go to fills under it.
    pub(super) fn hover_at(&mut self, position: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        let square = position.and_then(|position| self.square_at(position));
        if square != self.hover {
            self.hover = square;
            if self.selected.is_some() || self.selected_drop.is_some() {
                cx.notify();
            }
        }
    }

    /// The editor's press: put a piece down or take it off, or pick one up to move it.
    fn edit_press(
        &mut self,
        tool: EditTool,
        square: Square,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        match tool {
            EditTool::Place(piece) => {
                if self.pieces.piece_at(square) == Some(piece) {
                    self.pieces.discard_piece_at(square);
                } else {
                    self.pieces.set_piece_at(square, piece);
                }
                self.edited(cx);
            }
            EditTool::Trash => {
                if self.pieces.remove_piece_at(square).is_some() {
                    self.edited(cx);
                }
            }
            EditTool::Pointer => {
                if let Some(piece) = self.pieces.piece_at(square) {
                    self.drag = Some(Drag {
                        from: square,
                        piece,
                        position,
                        start: position,
                        moved: false,
                        again: false,
                        left: false,
                    });
                }
            }
        }
        cx.notify();
    }

    /// A spare piece dropped on the board.
    pub(super) fn drop_spare(
        &mut self,
        piece: Piece,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.state.edit.is_none() {
            return;
        }
        if let Some(square) = self.square_at(position) {
            self.pieces.set_piece_at(square, piece);
            self.edited(cx);
        }
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        cx.emit(BoardEvent::Edited(self.pieces.to_string()));
        cx.notify();
    }

    pub(super) fn release(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        // A Shift-drag of the left button draws.
        if self.drawing.is_some() {
            return self.finish_drawing(position, cx);
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        let target = self.square_at(position);
        if self.state.edit.is_some() {
            // Dragged anywhere on the board, the piece moves there; dragged off it, it is gone.
            if drag.moved || far(drag.start, position) {
                match target {
                    Some(to) if to == drag.from => {}
                    Some(to) => {
                        self.pieces.discard_piece_at(drag.from);
                        self.pieces.set_piece_at(to, drag.piece);
                        self.edited(cx);
                    }
                    None => {
                        self.pieces.discard_piece_at(drag.from);
                        self.edited(cx);
                    }
                }
            }
            cx.notify();
            return;
        }
        // As Chessground's `end`: a premove waiting goes, and a drag that left its square moves
        // there, premoves there or lets go. The moves of a quick drag may all have come before
        // the board was listening.
        self.queued = None;
        let started = drag.moved || far(drag.start, position);
        if let Some(to) = target
            && started
            && to != drag.from
        {
            self.selected = None;
            if self.reaches(drag.from, to) {
                self.dragged = true;
                self.play_move(drag.from, to, false, false, cx);
            } else if self.premoves_to(drag.from, to) {
                self.dragged = true;
                self.queued = Some((drag.from, to));
            }
        }
        // Let go of a piece pressed again, or dragged away and back or off the board.
        if (drag.again || drag.left) && (target == Some(drag.from) || target.is_none()) {
            self.selected = None;
        }
        cx.notify();
    }

    /// Play a move the player made: it lands at once (the rook too when castling, and a pawn
    /// taken en passant goes) and the turn passes, as Chessground's own turn does until the
    /// page's next position confirms or undoes it. A clicked move or a premove slides; a dragged
    /// one is already there. A pawn reaching the last rank becomes a queen when Settings say
    /// so, or asks which piece.
    pub(super) fn play_move(
        &mut self,
        from: Square,
        to: Square,
        premove: bool,
        slide: bool,
        cx: &mut Context<Self>,
    ) {
        let promotes = self.pieces.piece_at(from).is_some_and(|piece| {
            piece.role == Role::Pawn && matches!(to.rank(), Rank::First | Rank::Eighth)
        });
        if promotes {
            let mode = crate::settings::frontend(cx).promotion;
            if mode == PromotionMode::Queen || (premove && mode == PromotionMode::Premove) {
                self.land(from, to, Some(Role::Queen), slide, cx);
                cx.emit(BoardEvent::Move(format!("{from}{to}q")));
            } else {
                self.promotion = Some((from, to));
                self.promotion_choice = 0;
            }
        } else {
            self.land(from, to, None, slide, cx);
            cx.emit(BoardEvent::Move(format!("{from}{to}")));
        }
        cx.notify();
    }

    /// Put the player's move on the board and pass the turn.
    fn land(
        &mut self,
        from: Square,
        to: Square,
        promotion: Option<Role>,
        slide: bool,
        cx: &mut Context<Self>,
    ) {
        let mut next = self.pieces.clone();
        apply_move(&mut next, from, to, promotion);
        if slide {
            self.animate(&next, self.state.orientation, cx);
        } else {
            self.animation = None;
        }
        self.pieces = next;
        self.moved(from, to);
    }

    /// The turn passes after the player's move (Chessground's `baseUserMove`): the move is the
    /// last one, no king is in check and nothing moves until the page's next position, though a
    /// premove can be queued at once.
    pub(super) fn moved(&mut self, from: Square, to: Square) {
        self.state.last_move = Some((from, to));
        self.state.check = None;
        self.state.badge = None;
        self.state.movable = None;
        self.state.dests.clear();
        self.state.drops.clear();
    }

    pub(super) fn promote(&mut self, role: Role, cx: &mut Context<Self>) {
        if let Some((from, to)) = self.promotion.take() {
            cx.emit(BoardEvent::Move(format!("{from}{to}{}", role.char())));
            cx.notify();
        }
    }

    pub(super) fn cancel_promotion(&mut self, cx: &mut Context<Self>) {
        self.promotion = None;
        cx.notify();
    }

    /// Whether the board itself has the keys (Tab gave them to it).
    pub fn has_keys(&self, window: &Window) -> bool {
        self.focus
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window))
    }

    /// The board's keys, made the first time they are asked for.
    pub fn focus_handle(&mut self, cx: &mut App) -> FocusHandle {
        self.focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone()
    }

    /// The square the keyboard is on.
    pub(super) fn cursor(&self) -> Square {
        self.cursor
            .unwrap_or_else(|| keyboard::start(self.state.orientation, self.state.last_move))
    }

    /// Whether the player can do anything on the board now: move, premove, drop or edit.
    pub(super) fn interactive(&self) -> bool {
        !self.state.preview
            && (self.state.edit.is_some()
                || self.state.movable.is_some()
                || self.state.premove.is_some())
    }

    /// The arrows move the cursor, Enter or Space presses its square and Escape lets go; while
    /// the promotion picker is open, they choose its piece. Other keys, and these when the board
    /// has nothing to do with them, go on to the page.
    pub(super) fn key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_keys(window) || event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        // Held for push to talk, Space is the voice's.
        if key == "space" && crate::space_talks(cx) {
            return;
        }
        if self.keyboard(key, cx) {
            cx.stop_propagation();
        }
    }

    /// A key on the board; whether it was taken.
    fn keyboard(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if self.promotion.is_some() {
            match keyboard::promotion_key(self.promotion_choice, key) {
                Some(keyboard::PromotionKey::Highlight(choice)) => {
                    self.promotion_choice = choice;
                    cx.notify();
                }
                Some(keyboard::PromotionKey::Pick(role)) => self.promote(role, cx),
                Some(keyboard::PromotionKey::Cancel) => self.cancel_promotion(cx),
                None => return false,
            }
            return true;
        }
        if let Some(square) = keyboard::step(self.cursor(), self.state.orientation, key) {
            self.cursor = Some(square);
            cx.notify();
            return true;
        }
        match key {
            "enter" | "space" => self.activate(self.cursor(), cx),
            "escape" => self.let_go(cx),
            _ => false,
        }
    }

    /// Enter on a square: what a click there does, without a drag; a second press on the
    /// selected piece lets go of it. Whether the board took the key.
    pub(super) fn activate(&mut self, square: Square, cx: &mut Context<Self>) -> bool {
        if self.state.preview {
            return false;
        }
        if let Some(tool) = self.state.edit {
            self.edit_key(tool, square, cx);
            return true;
        }
        let busy = self.selected.is_some() || self.selected_drop.is_some() || self.queued.is_some();
        cx.emit(BoardEvent::Select(square));
        let press = self.press_key(square);
        match press {
            Press::Drop(role) => self.drop_piece(role, square, cx),
            Press::Play(from) => {
                self.dragged = false;
                self.play_move(from, square, false, true, cx);
            }
            Press::Pick { .. } | Press::Queued | Press::Clear => {}
        }
        cx.notify();
        busy || press != Press::Clear || self.interactive()
    }

    /// What Enter on `square` does to the selection: as a click, but a second press on the
    /// selected piece lets go of it, as a click's release does.
    pub(super) fn press_key(&mut self, square: Square) -> Press {
        let press = self.decide(square);
        // A piece picked up ends a premove waiting, as a release does.
        if let Press::Pick { again, .. } = press {
            self.queued = None;
            if again {
                self.selected = None;
            }
        }
        press
    }

    /// The editor from the keyboard: its tool puts down or takes off a piece, and its pointer
    /// picks a piece up on one press and puts it down on the next.
    fn edit_key(&mut self, tool: EditTool, square: Square, cx: &mut Context<Self>) {
        match tool {
            EditTool::Place(_) | EditTool::Trash => {
                self.edit_press(tool, square, point(px(0.), px(0.)), cx);
            }
            EditTool::Pointer => match self.selected.take() {
                Some(from) if from != square => {
                    if let Some(piece) = self.pieces.remove_piece_at(from) {
                        self.pieces.set_piece_at(square, piece);
                        self.edited(cx);
                    }
                }
                Some(_) => {}
                None => self.selected = self.pieces.piece_at(square).map(|_| square),
            },
        }
        cx.notify();
    }

    /// Escape: the selection, a picked pocket piece and a queued premove go. Whether there was
    /// any.
    fn let_go(&mut self, cx: &mut Context<Self>) -> bool {
        let busy = self.selected.take().is_some()
            | self.selected_drop.take().is_some()
            | self.queued.take().is_some();
        if busy {
            cx.notify();
        }
        busy
    }

    /// Where the piece picked can go: its moves, its premoves or a pocket piece's drops.
    pub(super) fn targets(&self) -> Vec<Square> {
        if let Some(role) = self.selected_drop {
            return self.state.drops.get(&role).cloned().unwrap_or_default();
        }
        let Some(square) = self.selected else {
            return Vec::new();
        };
        if self.state.edit.is_some() {
            Vec::new()
        } else if self.state.movable.is_some() {
            self.state.dests.get(&square).cloned().unwrap_or_default()
        } else if self
            .pieces
            .piece_at(square)
            .is_some_and(|piece| Some(piece.color) == self.state.premove)
        {
            self.premove_targets(square)
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod drawing_tests {
    use super::{Drawn, drawing_brush, toggle_shape};
    use gpui_kit::Modifiers;
    use shakmaty::Square;

    /// Space presses the board's square, unless the voice holds it for push to talk: then the
    /// board leaves it alone.
    #[gpui_kit::test]
    fn space_is_the_boards_unless_the_voice_holds_it(cx: &mut gpui_kit::TestAppContext) {
        use std::cell::Cell;
        use std::rc::Rc;

        cx.update(|cx| {
            gpui_kit::init(cx);
            kchess_app::settings::init_for_tests(cx);
            let colors = kchess_ui::theme::colors("kchess", &[], false);
            cx.set_global(kchess_ui::theme::Palette::of(&colors, false));
        });
        let window = cx.add_window(|_, _| {
            let mut state = super::BoardState::shown(
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                kchess_domain::variant::Variant::Standard,
                shakmaty::Color::White,
                None,
            );
            state.movable = Some(shakmaty::Color::White);
            super::ChessBoard::new(state)
        });
        let selected = Rc::new(Cell::new(0));
        let board = window.root(cx).unwrap();
        let _subscription = cx.update(|cx| {
            let selected = selected.clone();
            cx.subscribe(&board, move |_, event, _| {
                if matches!(event, super::BoardEvent::Select(_)) {
                    selected.set(selected.get() + 1);
                }
            })
        });
        window
            .update(cx, |board, window, cx| {
                let focus = board.focus_handle(cx);
                window.focus(&focus, cx);
            })
            .unwrap();
        let press = |cx: &mut gpui_kit::TestAppContext| {
            cx.run_until_parked();
            cx.simulate_keystrokes(window.into(), "space");
            cx.run_until_parked();
        };
        press(cx);
        assert_eq!(selected.get(), 1, "the board takes Space");
        cx.update(|cx| cx.set_global(crate::SpaceTalks(|_| true)));
        press(cx);
        assert_eq!(selected.get(), 1, "the voice holds Space");
        cx.update(|cx| cx.set_global(crate::SpaceTalks(|_| false)));
        press(cx);
        assert_eq!(selected.get(), 2, "the voice let go of Space");
    }

    #[test]
    fn modifiers_pick_chessgrounds_brushes() {
        let none = Modifiers::default();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let both = Modifiers {
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        for (modifiers, right, brush) in [
            (&none, true, "green"),
            (&shift, true, "red"),
            (&alt, true, "blue"),
            (&both, true, "yellow"),
            // A Shift-drag of the left button draws, but never in red.
            (&shift, false, "green"),
            (&both, false, "blue"),
        ] {
            assert_eq!(
                drawing_brush(modifiers, right),
                brush,
                "{modifiers:?} {right}"
            );
        }
    }

    #[test]
    fn drawing_again_takes_off_or_recolours() {
        let arrow = |brush| Drawn {
            from: Square::E2,
            to: Square::E4,
            brush,
        };
        let mut shapes = Vec::new();
        toggle_shape(&mut shapes, arrow("green"));
        assert_eq!(shapes, [arrow("green")]);
        toggle_shape(&mut shapes, arrow("red"));
        assert_eq!(shapes, [arrow("red")]);
        toggle_shape(&mut shapes, arrow("red"));
        assert!(shapes.is_empty());
    }
}

/// The pointer on the board, as Chessground's `drag.ts` and `board.ts` answer it.
#[cfg(test)]
mod pointer_tests {
    use super::super::{BoardEvent, BoardState, ChessBoard, apply_move, board_of};
    use gpui_kit::{
        AppContext as _, Bounds, Entity, MouseDownEvent, Point, TestAppContext, point, px, size,
    };
    use shakmaty::{Color, Square};
    use std::cell::RefCell;
    use std::rc::Rc;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    /// A board 800 px across, White to move and to premove, and the moves it sends.
    fn board(cx: &mut TestAppContext, fen: &str) -> (Entity<ChessBoard>, Rc<RefCell<Vec<String>>>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            kchess_app::settings::init_for_tests(cx);
        });
        let board = cx.new(|_| {
            let position =
                kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, fen)
                    .expect("a position");
            let mut state = BoardState::of(&position, Color::White, Some(Color::White), None);
            state.premove = Some(Color::White);
            let board = ChessBoard::new(state);
            board
                .bounds
                .set(Bounds::new(Point::default(), size(px(800.), px(800.))));
            board
        });
        let moves = Rc::new(RefCell::new(Vec::new()));
        let sent = moves.clone();
        cx.update(|cx| {
            cx.subscribe(&board, move |_, event, _| {
                if let BoardEvent::Move(uci) = event {
                    sent.borrow_mut().push(uci.clone());
                }
            })
            .detach();
        });
        (board, moves)
    }

    fn at(square: Square) -> Point<gpui_kit::Pixels> {
        let (file, rank) = (u32::from(square.file()), u32::from(square.rank()));
        point(
            px(file as f32 * 100. + 50.),
            px((7 - rank) as f32 * 100. + 50.),
        )
    }

    fn press(board: &Entity<ChessBoard>, square: Square, cx: &mut TestAppContext) {
        board.update(cx, |board, cx| {
            let event = MouseDownEvent {
                position: at(square),
                click_count: 1,
                ..MouseDownEvent::default()
            };
            board.press(&event, cx);
        });
    }

    fn drag_to(board: &Entity<ChessBoard>, squares: &[Square], cx: &mut TestAppContext) {
        board.update(cx, |board, cx| {
            for square in squares {
                board.pointer_moved(at(*square), cx);
            }
        });
    }

    fn release(board: &Entity<ChessBoard>, square: Square, cx: &mut TestAppContext) {
        board.update(cx, |board, cx| board.release(at(square), cx));
    }

    fn click(board: &Entity<ChessBoard>, square: Square, cx: &mut TestAppContext) {
        press(board, square, cx);
        release(board, square, cx);
    }

    /// A clicked move slid in from its square and a dragged one landed where it was let go
    /// (`anim` wraps only `selectSquare`); the piece pressed next lifts at once only after a
    /// drag (`autoDistance && stats.dragged`).
    #[gpui_kit::test]
    fn a_clicked_move_slides_and_a_dragged_one_lands(cx: &mut TestAppContext) {
        let (clicked, moves) = board(cx, START);
        click(&clicked, Square::E2, cx);
        click(&clicked, Square::E4, cx);
        assert_eq!(*moves.borrow(), ["e2e4"]);
        clicked.update(cx, |board, _| {
            assert!(board.animation.is_some(), "a clicked move slides");
            assert!(!board.dragged);
        });
        let (dragged, moves) = board(cx, START);
        press(&dragged, Square::E2, cx);
        drag_to(&dragged, &[Square::E3, Square::E4], cx);
        release(&dragged, Square::E4, cx);
        assert_eq!(*moves.borrow(), ["e2e4"]);
        dragged.update(cx, |board, _| {
            assert!(board.animation.is_none(), "a dragged move is already there");
            assert!(board.dragged);
        });
        press(&dragged, Square::G1, cx);
        dragged.update(cx, |board, _| {
            assert!(
                board.drag.as_ref().is_some_and(|drag| drag.moved),
                "lifted at once"
            );
        });
    }

    /// The move played, the turn passed at once (`baseUserMove`): nothing more moves, and a
    /// premove can be queued before the page has answered.
    #[gpui_kit::test]
    fn the_turn_passes_with_the_move_so_a_premove_can_follow(cx: &mut TestAppContext) {
        let (board, moves) = board(cx, START);
        click(&board, Square::E2, cx);
        click(&board, Square::E4, cx);
        click(&board, Square::G1, cx);
        click(&board, Square::F3, cx);
        assert_eq!(
            *moves.borrow(),
            ["e2e4"],
            "the knight's move waits for its turn"
        );
        board.update(cx, |board, _| {
            assert_eq!(board.state.movable, None);
            assert_eq!(board.queued, Some((Square::G1, Square::F3)));
        });
    }

    /// A premove waiting went once another piece was picked up and let go (`drag.end`
    /// `unsetPremove`), and the piece stayed selected.
    #[gpui_kit::test]
    fn picking_up_another_piece_takes_back_the_premove(cx: &mut TestAppContext) {
        let (board, _) = board(cx, START);
        click(&board, Square::E2, cx);
        click(&board, Square::E4, cx);
        click(&board, Square::G1, cx);
        click(&board, Square::F3, cx);
        click(&board, Square::B1, cx);
        board.update(cx, |board, _| {
            assert_eq!(board.queued, None);
            assert_eq!(board.selected, Some(Square::B1));
        });
    }

    /// A piece dragged off its square and back is let go of (`keyHasChanged`); one only
    /// pressed stays selected, and pressed again lets go.
    #[gpui_kit::test]
    fn a_piece_dragged_away_and_back_is_let_go(cx: &mut TestAppContext) {
        let (board, moves) = board(cx, START);
        press(&board, Square::E2, cx);
        drag_to(&board, &[Square::E3, Square::E2], cx);
        release(&board, Square::E2, cx);
        board.update(cx, |board, _| assert_eq!(board.selected, None));
        click(&board, Square::E2, cx);
        board.update(cx, |board, _| assert_eq!(board.selected, Some(Square::E2)));
        click(&board, Square::E2, cx);
        board.update(cx, |board, _| assert_eq!(board.selected, None));
        assert!(moves.borrow().is_empty());
    }

    /// Castling put the rook beside the king, the king dragged onto its rook castled too, and a
    /// pawn taken en passant left the board, all before the page's next position.
    #[test]
    fn castling_and_en_passant_move_both_pieces_at_once() {
        let castles = "4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1";
        for (to, king, rook, empty) in [
            (Square::G1, Square::G1, Square::F1, Square::H1),
            (Square::H1, Square::G1, Square::F1, Square::H1),
            (Square::C1, Square::C1, Square::D1, Square::A1),
            (Square::A1, Square::C1, Square::D1, Square::A1),
        ] {
            let mut pieces = board_of(castles);
            apply_move(&mut pieces, Square::E1, to, None);
            assert_eq!(
                pieces.piece_at(king).map(|p| p.role),
                Some(shakmaty::Role::King),
                "{to}"
            );
            assert_eq!(
                pieces.piece_at(rook).map(|p| p.role),
                Some(shakmaty::Role::Rook),
                "{to}"
            );
            assert_eq!(pieces.piece_at(empty), None, "{to}");
            assert_eq!(pieces.piece_at(Square::E1), None, "{to}");
        }
        let mut pieces = board_of("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1");
        apply_move(&mut pieces, Square::E5, Square::D6, None);
        assert_eq!(pieces.piece_at(Square::D5), None);
        assert!(pieces.piece_at(Square::D6).is_some());
    }
}

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

/// Chessground's brush for a right-button drawing: Shift or Ctrl red, Alt or ⌘
/// blue, both yellow, else green.
fn drawing_brush(modifiers: &Modifiers) -> &'static str {
    let a = modifiers.shift || modifiers.control;
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
    (dx * dx + dy * dy).sqrt() > DRAG_THRESHOLD.as_f32()
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

    /// Drop a pocket piece: it lands at once, and the page's next position confirms or undoes it.
    fn drop_piece(&mut self, role: Role, to: Square, cx: &mut Context<Self>) {
        if let Some(color) = self.state.movable {
            self.pieces.set_piece_at(to, Piece { color, role });
        }
        self.selected_drop = None;
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

    pub(super) fn press(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.promotion.is_some() || self.state.preview {
            return;
        }
        // A left click clears the drawings; Shift keeps them.
        if !event.modifiers.shift && !self.drawn.is_empty() {
            self.drawn.clear();
            cx.notify();
        }
        let Some(square) = self.square_at(event.position) else {
            return;
        };
        if let Some(tool) = self.state.edit {
            return self.edit_press(tool, square, event.position, cx);
        }
        cx.emit(BoardEvent::Select(square));
        match self.decide(square) {
            Press::Drop(role) => return self.drop_piece(role, square, cx),
            Press::Play(from) => return self.play(from, square, cx),
            Press::Pick { piece, again } => {
                // A second press on the selected piece lets go of it on release, unless it is
                // dragged.
                self.drag = Some(Drag {
                    from: square,
                    piece,
                    position: event.position,
                    start: event.position,
                    moved: false,
                    again,
                });
            }
            Press::Queued | Press::Clear => {}
        }
        cx.notify();
    }

    /// What pressing `square` does to the selection, a picked pocket piece and a queued premove,
    /// and what is left to play.
    fn decide(&mut self, square: Square) -> Press {
        if let Some(role) = self.selected_drop.take()
            && self.drops_to(role, square)
        {
            self.selected = None;
            return Press::Drop(role);
        }
        // A press while a premove waits takes it back.
        if self.queued.take().is_some() {
            self.selected = None;
            return Press::Queued;
        }
        if let Some(from) = self.selected
            && from != square
            && self.reaches(from, square)
        {
            self.selected = None;
            return Press::Play(from);
        }
        if let Some(from) = self.selected
            && from != square
            && self.premoves_to(from, square)
        {
            self.selected = None;
            self.queued = Some((from, square));
            return Press::Queued;
        }
        match self.pieces.piece_at(square) {
            Some(piece)
                if (Some(piece.color) == self.state.movable && self.can_move_from(square))
                    || (Some(piece.color) == self.state.premove
                        && self.state.movable.is_none()) =>
            {
                let again = self.selected == Some(square);
                self.selected = Some(square);
                Press::Pick { piece, again }
            }
            _ => {
                self.selected = None;
                Press::Clear
            }
        }
    }

    /// The right button starts a drawing on a square.
    pub(super) fn start_drawing(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.state.preview || self.state.edit.is_some() {
            return;
        }
        // The right button also takes back a queued premove, as on Lichess.
        if self.queued.take().is_some() {
            cx.notify();
        }
        if let Some(square) = self.square_at(event.position) {
            self.drawing = Some((square, drawing_brush(&event.modifiers), Some(square)));
            cx.notify();
        }
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
        let Some(drag) = &mut self.drag else {
            return;
        };
        drag.position = position;
        if far(drag.start, position) {
            drag.moved = true;
        }
        cx.notify();
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
        // The moves of a quick drag may all have come before the board was listening.
        if drag.moved || far(drag.start, position) {
            match target {
                Some(to) if self.reaches(drag.from, to) => {
                    self.selected = None;
                    self.play(drag.from, to, cx);
                }
                Some(to) if to != drag.from && self.premoves_to(drag.from, to) => {
                    self.selected = None;
                    self.queued = Some((drag.from, to));
                }
                // Dropped back on its square: still selected, as Chessground keeps it.
                Some(to) if to == drag.from => {}
                _ => self.selected = None,
            }
        } else if target == Some(drag.from) && self.selected == Some(drag.from) && drag.again {
            self.selected = None;
        }
        cx.notify();
    }

    /// Play a move the player made; a pawn reaching the last rank asks which piece it becomes.
    fn play(&mut self, from: Square, to: Square, cx: &mut Context<Self>) {
        self.play_move(from, to, false, cx);
    }

    /// Play a move; a premove's promotion becomes a queen when Settings say so for premoves.
    pub(super) fn play_move(
        &mut self,
        from: Square,
        to: Square,
        premove: bool,
        cx: &mut Context<Self>,
    ) {
        let promotes = self.pieces.piece_at(from).is_some_and(|piece| {
            piece.role == Role::Pawn && matches!(to.rank(), Rank::First | Rank::Eighth)
        });
        if promotes {
            let mode = crate::settings::frontend(cx).promotion;
            if mode == PromotionMode::Queen || (premove && mode == PromotionMode::Premove) {
                cx.emit(BoardEvent::Move(format!("{from}{to}q")));
            } else {
                self.promotion = Some((from, to));
                self.promotion_choice = 0;
            }
        } else {
            // The piece lands at once; the page's next position confirms or undoes it.
            if let Some(piece) = self.pieces.remove_piece_at(from) {
                self.pieces.set_piece_at(to, piece);
            }
            cx.emit(BoardEvent::Move(format!("{from}{to}")));
        }
        cx.notify();
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
            Press::Play(from) => self.play(from, square, cx),
            Press::Pick { .. } | Press::Queued | Press::Clear => {}
        }
        cx.notify();
        busy || press != Press::Clear || self.interactive()
    }

    /// What Enter on `square` does to the selection: as a click, but a second press on the
    /// selected piece lets go of it, as a click's release does.
    pub(super) fn press_key(&mut self, square: Square) -> Press {
        let press = self.decide(square);
        if let Press::Pick { again: true, .. } = press {
            self.selected = None;
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
            premove_targets(&self.pieces, square)
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
        assert_eq!(drawing_brush(&none), "green");
        assert_eq!(drawing_brush(&shift), "red");
        assert_eq!(drawing_brush(&alt), "blue");
        assert_eq!(drawing_brush(&both), "yellow");
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

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

/// A drawing under way: where it started, its brush, the square it ends on so far, and whether
/// that square snaps to the nearest one on a queen or knight line from the start, as Lichess's
/// does until the pointer leaves the board.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drawing {
    pub(super) from: Square,
    pub(super) brush: &'static str,
    pub(super) to: Option<Square>,
    snap: bool,
}

/// The square on a queen or knight line from `from` (or `from` itself) whose centre is nearest
/// `point`, in board cells from the top left (Chessground's `getSnappedKeyAtDomPos`).
fn snapped(from: (f32, f32), point: (f32, f32)) -> (i32, i32) {
    let (fx, fy) = (from.0 as i32, from.1 as i32);
    let mut best = ((fx, fy), f32::MAX);
    for x in 0..8 {
        for y in 0..8 {
            let (dx, dy) = ((x - fx).abs(), (y - fy).abs());
            let line = dx == 0 || dy == 0 || dx == dy;
            let knight = (dx == 1 && dy == 2) || (dx == 2 && dy == 1);
            if !(line || knight) {
                continue;
            }
            let (cx, cy) = (x as f32 + 0.5 - point.0, y as f32 + 0.5 - point.1);
            let distance = cx * cx + cy * cy;
            if distance < best.1 {
                best = ((x, y), distance);
            }
        }
    }
    best.0
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
        self.ctrl = event.modifiers.control;
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
            Press::Queued if self.queued.is_some() => {
                self.dragged = false;
                self.ask_pre_promotion(cx);
            }
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
    /// takes back a queued premove and lets go of the piece picked (`cancelMove`); with Ctrl it
    /// only lets go of the piece.
    fn start_drawing(&mut self, event: &MouseDownEvent, right: bool, cx: &mut Context<Self>) {
        if self.state.preview || self.state.edit.is_some() {
            return;
        }
        if !event.modifiers.control {
            self.queued = None;
        }
        self.selected = None;
        self.selected_drop = None;
        if let Some(square) = self.square_at(event.position) {
            self.drawing = Some(Drawing {
                from: square,
                brush: drawing_brush(&event.modifiers, right),
                to: Some(square),
                snap: true,
            });
        }
        cx.notify();
    }

    pub(super) fn draw_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(mut drawing) = self.drawing else {
            return;
        };
        let under = self.square_at(position);
        if under.is_none() {
            drawing.snap = false;
        }
        let to = if drawing.snap {
            let side = self.square_size();
            let origin = self.bounds.get().origin;
            let (column, row) = self.cell(drawing.from);
            let point = (
                (position.x - origin.x) / side,
                (position.y - origin.y) / side,
            );
            let (x, y) = snapped((column as f32, row as f32), point);
            self.square_at(gpui_kit::point(
                origin.x + side * (x as f32 + 0.5),
                origin.y + side * (y as f32 + 0.5),
            ))
        } else {
            under
        };
        let changed = to != drawing.to;
        drawing.to = to;
        self.drawing = Some(drawing);
        if changed {
            cx.notify();
        }
    }

    /// Letting go draws the arrow (or the circle, on the same square) to the square it reached,
    /// or takes it off.
    pub(super) fn finish_drawing(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.draw_to(position, cx);
        let Some(Drawing {
            from, brush, to, ..
        }) = self.drawing.take()
        else {
            return;
        };
        if let Some(to) = to {
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
                self.ask_pre_promotion(cx);
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
            let chosen = self
                .pre_promotion
                .take()
                .filter(|&(f, t, _)| premove && (f, t) == (from, to))
                .map(|(_, _, role)| role);
            let auto = !self.ctrl
                && (mode == PromotionMode::Queen || (premove && mode == PromotionMode::Premove));
            if let Some(role) = chosen.or(auto.then_some(Role::Queen)) {
                self.land(from, to, Some(role), slide, cx);
                cx.emit(BoardEvent::Move(format!("{from}{to}{}", role.char())));
            } else {
                // The pawn goes to the last rank while the picker asks, as on Lichess.
                self.land(from, to, None, slide, cx);
                self.promotion = Some((from, to));
                self.promotion_pre = false;
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
        // A slide still running (the opponent's move a premove answers) gives way to this one,
        // as Chessground replaces its plan.
        self.animation = None;
        if slide {
            self.animate(&next, self.state.orientation, cx);
        }
        self.pieces = next;
        self.moved(from, to);
        self.landed = Some((from, to));
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

    /// A promotion premove just queued: it becomes a queen at once where Settings say so
    /// (unless Ctrl was held), or the picker asks now, while there is time.
    fn ask_pre_promotion(&mut self, cx: &mut Context<Self>) {
        // A piece chosen for an earlier premove is not this one's.
        self.pre_promotion = None;
        let Some((from, to)) = self.queued else {
            return;
        };
        let promotes = self.pieces.piece_at(from).is_some_and(|piece| {
            piece.role == Role::Pawn && matches!(to.rank(), Rank::First | Rank::Eighth)
        });
        if !promotes {
            return;
        }
        let mode = crate::settings::frontend(cx).promotion;
        if !self.ctrl && mode != PromotionMode::Ask {
            self.pre_promotion = Some((from, to, Role::Queen));
        } else {
            self.promotion = Some((from, to));
            self.promotion_pre = true;
            self.promotion_choice = 0;
        }
    }

    pub(super) fn promote(&mut self, role: Role, cx: &mut Context<Self>) {
        let Some((from, to)) = self.promotion.take() else {
            return;
        };
        if self.promotion_pre {
            self.pre_promotion = Some((from, to, role));
        } else {
            if let Some(pawn) = self.pieces.piece_at(to) {
                self.pieces.set_piece_at(
                    to,
                    Piece {
                        color: pawn.color,
                        role,
                    },
                );
            }
            cx.emit(BoardEvent::Move(format!("{from}{to}{}", role.char())));
        }
        cx.notify();
    }

    /// The picker closed unanswered: the pawn goes back, or the premove is taken back.
    pub(super) fn cancel_promotion(&mut self, cx: &mut Context<Self>) {
        if self.promotion.take().is_none() {
            return;
        }
        if self.promotion_pre {
            self.queued = None;
            self.pre_promotion = None;
            cx.notify();
        } else {
            self.undo_move(cx);
        }
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
        if let Some((_, to)) = self.promotion {
            // Up and Down follow the column, which hangs from the bottom edge for the side at
            // the top.
            let white = to.rank() == Rank::Eighth;
            let from_top = (self.state.orientation == Color::White) == white;
            let key = match key {
                "up" if !from_top => "down",
                "down" if !from_top => "up",
                key => key,
            };
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
        // The keyboard holds no Ctrl.
        self.ctrl = false;
        let press = self.press_key(square);
        match press {
            Press::Drop(role) => self.drop_piece(role, square, cx),
            Press::Play(from) => {
                self.dragged = false;
                self.play_move(from, square, false, true, cx);
            }
            Press::Queued if self.queued.is_some() => self.ask_pre_promotion(cx),
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

    /// An arrow drawn toward a square off every line from its start ends on the nearest square
    /// that is on one (`getSnappedKeyAtDomPos`); once the pointer has left the board it ends
    /// where it is let go. Ctrl keeps a queued premove while drawing.
    #[gpui_kit::test]
    fn an_arrow_snaps_to_a_line_from_its_square(cx: &mut TestAppContext) {
        let (board, _) = board(cx, START);
        let right = |square: Square, control: bool| MouseDownEvent {
            button: gpui_kit::MouseButton::Right,
            position: at(square),
            modifiers: gpui_kit::Modifiers {
                control,
                ..Default::default()
            },
            click_count: 1,
            ..MouseDownEvent::default()
        };
        // g5, nudged toward h5: no line from e2 reaches g5, and h5 is the nearest that does.
        let near_g5 = at(Square::G5) + point(px(20.), px(-20.));
        board.update(cx, |board, cx| {
            board.right_press(&right(Square::E2, false), cx);
            board.draw_to(near_g5, cx);
            board.finish_drawing(near_g5, cx);
            assert_eq!(
                board.drawn.last().map(|d| (d.from, d.to)),
                Some((Square::E2, Square::H5))
            );
            board.right_press(&right(Square::E2, false), cx);
            board.draw_to(point(px(900.), px(900.)), cx);
            board.finish_drawing(near_g5, cx);
            assert_eq!(
                board.drawn.last().map(|d| (d.from, d.to)),
                Some((Square::E2, Square::G5))
            );
        });
        click(&board, Square::E2, cx);
        click(&board, Square::E4, cx);
        click(&board, Square::G1, cx);
        click(&board, Square::F3, cx);
        board.update(cx, |board, cx| {
            board.right_press(&right(Square::A7, true), cx);
            board.finish_drawing(at(Square::A7), cx);
            assert_eq!(
                board.queued,
                Some((Square::G1, Square::F3)),
                "Ctrl keeps the premove"
            );
            board.right_press(&right(Square::A7, false), cx);
            board.finish_drawing(at(Square::A7), cx);
            assert_eq!(board.queued, None);
        });
    }

    /// The pawn stood on the last rank, its turn over, while the picker asked; the piece picked
    /// took its place, and closing the picker put the pawn back.
    #[gpui_kit::test]
    fn the_pawn_waits_on_the_last_rank_while_the_picker_asks(cx: &mut TestAppContext) {
        use shakmaty::{Piece, Role};
        let fen = "4k3/P7/8/8/8/8/8/4K3 w - - 0 1";
        let pawn = Piece {
            color: Color::White,
            role: Role::Pawn,
        };
        let (board, moves) = board(cx, fen);
        click(&board, Square::A7, cx);
        click(&board, Square::A8, cx);
        board.update(cx, |board, cx| {
            assert_eq!(board.promotion, Some((Square::A7, Square::A8)));
            assert_eq!(board.pieces.piece_at(Square::A8), Some(pawn));
            assert_eq!(board.state.movable, None);
            board.promote(Role::Knight, cx);
            assert_eq!(
                board.pieces.piece_at(Square::A8).map(|p| p.role),
                Some(Role::Knight)
            );
        });
        assert_eq!(*moves.borrow(), ["a7a8n"]);
        let (board, moves) = board_and_cancel(cx, fen);
        board.update(cx, |board, _| {
            assert_eq!(board.pieces.piece_at(Square::A7), Some(pawn));
            assert_eq!(board.pieces.piece_at(Square::A8), None);
            assert_eq!(board.state.movable, Some(Color::White));
        });
        assert!(moves.borrow().is_empty());
    }

    fn board_and_cancel(
        cx: &mut TestAppContext,
        fen: &str,
    ) -> (Entity<ChessBoard>, Rc<RefCell<Vec<String>>>) {
        let (board, moves) = board(cx, fen);
        click(&board, Square::A7, cx);
        click(&board, Square::A8, cx);
        board.update(cx, |board, cx| board.cancel_promotion(cx));
        (board, moves)
    }

    /// A promotion premove asked which piece as it was queued when Ctrl was held (lila's
    /// `ctrlKey`), showed the piece waiting, and became it once the turn came; without Ctrl it
    /// is a queen at once, as Settings have it.
    #[gpui_kit::test]
    fn a_promotion_premove_asks_when_it_is_queued(cx: &mut TestAppContext) {
        use shakmaty::Role;
        let fen = "4k3/P7/8/8/8/8/8/4K3 w - - 0 1";
        let (board, moves) = board(cx, fen);
        click(&board, Square::E1, cx);
        click(&board, Square::E2, cx);
        click(&board, Square::A7, cx);
        board.update(cx, |board, cx| {
            let event = MouseDownEvent {
                position: at(Square::A8),
                modifiers: gpui_kit::Modifiers {
                    control: true,
                    ..Default::default()
                },
                click_count: 1,
                ..MouseDownEvent::default()
            };
            board.press(&event, cx);
            board.release(at(Square::A8), cx);
            assert_eq!(board.queued, Some((Square::A7, Square::A8)));
            assert!(board.promotion_pre, "the picker asks for the premove");
            board.promote(Role::Knight, cx);
            assert_eq!(
                board.queued,
                Some((Square::A7, Square::A8)),
                "the premove stays"
            );
            // Black answers, and the premove plays as the knight chosen.
            let position = kchess_domain::rules::setup_start(
                kchess_domain::variant::Variant::Standard,
                "3k4/P7/8/8/8/8/4K3/8 w - - 0 2",
            )
            .expect("a position");
            board.set_state(
                BoardState::of(&position, Color::White, Some(Color::White), None),
                cx,
            );
        });
        assert_eq!(*moves.borrow(), ["e1e2", "a7a8n"]);
        let (board, moves) = self::board(cx, fen);
        for square in [Square::E1, Square::E2, Square::A7, Square::A8] {
            click(&board, square, cx);
        }
        board.update(cx, |board, _| {
            assert_eq!(board.promotion, None, "no picker without Ctrl");
            assert_eq!(
                board.pre_promotion,
                Some((Square::A7, Square::A8, Role::Queen))
            );
        });
        assert_eq!(*moves.borrow(), ["e1e2"]);
    }

    /// Blindfold play hid the piece being dragged and its ghost too, which showed where every
    /// other piece was hidden; with Highlight off the ghost goes and the piece stays, as
    /// Lichess's `showGhost` follows that setting.
    #[gpui_kit::test]
    fn blindfold_and_highlight_hide_what_lichess_hides(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;
        let settings = cx.update(|cx| {
            gpui_kit::init(cx);
            let settings = kchess_app::settings::init_for_tests(cx);
            let colors = kchess_ui::theme::colors("kchess", &[], false);
            cx.set_global(kchess_ui::theme::Palette::of(&colors, false));
            settings
        });
        for (blindfold, highlight, held, ghost) in [
            (false, true, true, true),
            (true, true, false, false),
            (false, false, true, false),
        ] {
            settings.update(cx, |settings, cx| {
                settings.update(|s| s.frontend.highlight_moves = highlight, cx)
            });
            let window = cx.add_window(|_, _| {
                let position = kchess_domain::rules::setup_start(
                    kchess_domain::variant::Variant::Standard,
                    START,
                )
                .expect("a position");
                let mut state = BoardState::of(&position, Color::White, Some(Color::White), None);
                state.blindfold = blindfold;
                ChessBoard::new(state)
            });
            cx.run_until_parked();
            let shown = window
                .update(cx, |board, window, cx| {
                    board
                        .bounds
                        .set(Bounds::new(Point::default(), size(px(800.), px(800.))));
                    let e2 = at(Square::E2);
                    let side = px(100.);
                    let event = MouseDownEvent {
                        position: e2,
                        click_count: 1,
                        ..MouseDownEvent::default()
                    };
                    board.press(&event, cx);
                    board.pointer_moved(e2 + point(side, -side), cx);
                    window.refresh();
                })
                .and_then(|_| {
                    cx.run_until_parked();
                    window.update(cx, |_, window, _| {
                        (
                            window.try_find("board-held").is_some(),
                            window.try_find("board-ghost").is_some(),
                        )
                    })
                })
                .expect("the window");
            assert_eq!(
                shown,
                (held, ghost),
                "blindfold {blindfold}, highlight {highlight}"
            );
        }
    }

    /// The page drew the position again with new marks (the Analysis board does on every engine
    /// update) before answering the move: the move stayed, and the picker with it. The same
    /// position with the turn changed is a refusal, and the move went back without sliding on.
    #[gpui_kit::test]
    fn a_redraw_keeps_the_move_and_a_refusal_takes_it_back(cx: &mut TestAppContext) {
        use shakmaty::Role;
        let fen = "4k3/P7/8/8/8/8/8/4K3 w - - 0 1";
        let (board, _) = board(cx, fen);
        click(&board, Square::A7, cx);
        click(&board, Square::A8, cx);
        board.update(cx, |board, cx| {
            let mut marked = board.given.clone();
            marked
                .arrows
                .push(crate::board::Arrow::hint(Square::E1, Square::E2));
            board.set_state(marked, cx);
            assert_eq!(
                board.promotion,
                Some((Square::A7, Square::A8)),
                "the picker stays"
            );
            assert_eq!(
                board.pieces.piece_at(Square::A8).map(|p| p.role),
                Some(Role::Pawn)
            );
            assert_eq!(board.state.movable, None);
            assert_eq!(board.state.arrows.len(), 1, "the new marks show");
            let mut refused = board.given.clone();
            refused.movable = None;
            refused.dests.clear();
            board.set_state(refused, cx);
            assert_eq!(board.pieces.piece_at(Square::A8), None);
            assert_eq!(
                board.pieces.piece_at(Square::A7).map(|p| p.role),
                Some(Role::Pawn)
            );
        });
        let (board, _) = self::board(cx, START);
        click(&board, Square::E2, cx);
        click(&board, Square::E4, cx);
        board.update(cx, |board, cx| {
            assert!(board.animation.is_some());
            board.undo_move(cx);
            assert!(
                board.animation.is_none(),
                "no slide over the piece put back"
            );
        });
    }

    /// A premove played as the opponent's move slid in, and slid itself (Chessground's plan is
    /// replaced), where the running slide used to stop both.
    #[gpui_kit::test]
    fn a_premove_slides_when_it_plays(cx: &mut TestAppContext) {
        let (board, moves) = board(cx, START);
        for square in [Square::E2, Square::E4, Square::G1, Square::F3] {
            click(&board, square, cx);
        }
        // The player's own slide is over before the opponent answers.
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        board.update(cx, |board, cx| {
            let position = kchess_domain::rules::setup_start(
                kchess_domain::variant::Variant::Standard,
                "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
            )
            .expect("a position");
            board.set_state(
                BoardState::of(&position, Color::White, Some(Color::White), None),
                cx,
            );
            let sliding = board
                .animation
                .as_ref()
                .map(|a| a.moves.iter().map(|m| (m.1, m.2)).collect::<Vec<_>>());
            assert_eq!(sliding, Some(vec![(Square::G1, Square::F3)]));
        });
        assert_eq!(*moves.borrow(), ["e2e4", "g1f3"]);
    }

    /// The piece chosen for a promotion premove went with it: taken back and queued again, the
    /// picker asks afresh, with no knight left waiting from before.
    #[gpui_kit::test]
    fn a_premove_taken_back_forgets_its_piece(cx: &mut TestAppContext) {
        use shakmaty::Role;
        let fen = "4k3/P7/8/8/8/8/8/4K3 w - - 0 1";
        let (board, _) = board(cx, fen);
        click(&board, Square::E1, cx);
        click(&board, Square::E2, cx);
        click(&board, Square::A7, cx);
        board.update(cx, |board, cx| {
            let event = MouseDownEvent {
                position: at(Square::A8),
                modifiers: gpui_kit::Modifiers {
                    control: true,
                    ..Default::default()
                },
                click_count: 1,
                ..MouseDownEvent::default()
            };
            board.press(&event, cx);
            board.release(at(Square::A8), cx);
            board.promote(Role::Knight, cx);
            assert!(board.let_go(cx), "Escape takes the premove back");
        });
        click(&board, Square::A7, cx);
        board.update(cx, |board, cx| {
            let event = MouseDownEvent {
                position: at(Square::A8),
                modifiers: gpui_kit::Modifiers {
                    control: true,
                    ..Default::default()
                },
                click_count: 1,
                ..MouseDownEvent::default()
            };
            board.press(&event, cx);
            board.release(at(Square::A8), cx);
            assert!(board.promotion_pre, "the picker asks again");
            assert_eq!(board.pre_promotion, None);
        });
    }

    /// With Black at the bottom, White's picker hangs from the bottom edge, queen lowest: Up
    /// goes to the knight above it.
    #[gpui_kit::test]
    fn the_pickers_arrows_follow_its_column(cx: &mut TestAppContext) {
        let (board, _) = board(cx, "4k3/P7/8/8/8/8/8/4K3 w - - 0 1");
        board.update(cx, |board, cx| {
            board.promotion = Some((Square::A7, Square::A8));
            board.state.orientation = Color::Black;
            assert!(board.keyboard("up", cx));
            assert_eq!(board.promotion_choice, 1, "the knight");
            board.state.orientation = Color::White;
            board.promotion_choice = 0;
            assert!(board.keyboard("down", cx));
            assert_eq!(board.promotion_choice, 1, "the knight, under the queen");
        });
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

#![cfg(feature = "gpui")]

use std::{collections::HashSet, path::Path, time::Instant};

use gpui::{
    canvas, div, img, point, prelude::*, px, rgb, App, Context, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, PathBuilder, PathStyle, Render, Rgba,
    ScrollHandle, StrokeOptions, Window,
};
use lyon_tessellation::{LineCap, LineJoin};

use crate::board::{
    BoardError, MoveHistoryRow, MoveId, MoveRequest, Perspective, Piece, PieceKind, Side, Square,
    VisualBoard,
};

#[derive(Clone, Copy, Debug)]
pub struct BoardTheme {
    pub light_square: Rgba,
    pub dark_square: Rgba,
    pub selected_outline: u32,
}

impl Default for BoardTheme {
    fn default() -> Self {
        Self {
            light_square: rgb(0xf0d9b5),
            dark_square: rgb(0xb58863),
            selected_outline: 0x38bdf8,
        }
    }
}

#[derive(Debug)]
pub struct ChessBoardView {
    board: VisualBoard,
    drag: Option<DragState>,
    arrow_draft: Option<ArrowDraft>,
    arrows: Vec<BoardArrow>,
    highlighted_squares: HashSet<Square>,
    legal_move_source: Option<Square>,
    legal_move_targets: HashSet<Square>,
    pending_legal_moves_request: Option<Square>,
    perspective: Perspective,
    window_origin_x: f32,
    window_origin_y: f32,
    cell_px: f32,
    piece_scale: f32,
    picked_piece_scale: f32,
    auto_confirm_pending: bool,
    theme: BoardTheme,
    move_history_scroll_handle: ScrollHandle,
}

#[derive(Clone, Copy, Debug)]
struct DragState {
    from: Square,
    piece: Piece,
    pointer_x: f32,
    pointer_y: f32,
    drop_target: Option<Square>,
}

#[derive(Clone, Debug)]
struct ArrowDraft {
    from: Square,
    pointer_x: f32,
    pointer_y: f32,
    to: Option<Square>,
    points: Vec<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrowKind {
    Straight,
    KnightL,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoardArrow {
    pub from: Square,
    pub to: Square,
    pub kind: ArrowKind,
}

impl ChessBoardView {
    pub fn new(board: VisualBoard) -> Self {
        Self {
            board,
            drag: None,
            arrow_draft: None,
            arrows: Vec::new(),
            highlighted_squares: HashSet::new(),
            legal_move_source: None,
            legal_move_targets: HashSet::new(),
            pending_legal_moves_request: None,
            perspective: Perspective::White,
            window_origin_x: 0.0,
            window_origin_y: 0.0,
            cell_px: 72.0,
            piece_scale: 1.0,
            picked_piece_scale: 1.18,
            auto_confirm_pending: true,
            theme: BoardTheme::default(),
            move_history_scroll_handle: ScrollHandle::new(),
        }
    }

    pub fn board(&self) -> &VisualBoard {
        &self.board
    }

    pub fn board_mut(&mut self) -> &mut VisualBoard {
        &mut self.board
    }

    pub fn move_history(&self) -> Vec<MoveHistoryRow> {
        self.board.move_history()
    }

    pub fn displayed_ply(&self) -> usize {
        self.board.displayed_ply()
    }

    pub fn is_viewing_latest(&self) -> bool {
        self.board.is_viewing_latest()
    }

    pub fn can_step_back_view(&self) -> bool {
        self.board.displayed_ply() > 0
    }

    pub fn can_step_forward_view(&self) -> bool {
        !self.board.is_viewing_latest()
    }

    pub fn move_history_scroll_handle(&self) -> ScrollHandle {
        self.move_history_scroll_handle.clone()
    }

    pub fn set_theme(&mut self, theme: BoardTheme) {
        self.theme = theme;
    }

    pub fn theme(&self) -> BoardTheme {
        self.theme
    }

    pub fn set_cell_size(&mut self, cell_px: f32) {
        self.cell_px = cell_px.max(16.0);
    }

    pub fn set_piece_scale(&mut self, piece_scale: f32) {
        self.piece_scale = piece_scale.clamp(0.6, 1.4);
    }

    pub fn set_picked_piece_scale(&mut self, piece_scale: f32) {
        self.picked_piece_scale = piece_scale.clamp(0.7, 1.6);
    }

    pub fn set_perspective(&mut self, perspective: Perspective) {
        self.perspective = perspective;
    }

    pub fn set_auto_confirm_pending(&mut self, auto_confirm_pending: bool) {
        self.auto_confirm_pending = auto_confirm_pending;
    }

    pub fn arrows(&self) -> &[BoardArrow] {
        &self.arrows
    }

    pub fn clear_arrows(&mut self) {
        self.arrows.clear();
    }

    pub fn highlighted_squares(&self) -> &HashSet<Square> {
        &self.highlighted_squares
    }

    pub fn clear_highlights(&mut self) {
        self.highlighted_squares.clear();
    }

    pub fn clear_annotations(&mut self) {
        self.clear_arrows();
        self.clear_highlights();
    }

    pub fn reset_to_start(&mut self) {
        self.board.reset_to_start();
        self.drag = None;
        self.arrow_draft = None;
        self.pending_legal_moves_request = None;
        self.legal_move_source = None;
        self.legal_move_targets.clear();
        self.clear_annotations();
        self.move_history_scroll_handle
            .set_offset(point(px(0.0), px(0.0)));
    }

    pub fn set_arrows(&mut self, arrows: impl IntoIterator<Item = BoardArrow>) {
        self.arrows.clear();
        self.arrows.extend(arrows);
    }

    pub fn set_legal_moves_for(
        &mut self,
        source: Square,
        targets: impl IntoIterator<Item = Square>,
    ) {
        self.legal_move_source = Some(source);
        self.legal_move_targets.clear();
        self.legal_move_targets.extend(targets);
    }

    pub fn set_legal_move_targets(&mut self, targets: impl IntoIterator<Item = Square>) {
        self.legal_move_targets.clear();
        self.legal_move_targets.extend(targets);
    }

    pub fn clear_legal_moves(&mut self) {
        self.legal_move_source = None;
        self.legal_move_targets.clear();
    }

    pub fn legal_move_source(&self) -> Option<Square> {
        self.legal_move_source
    }

    pub fn legal_move_targets(&self) -> &HashSet<Square> {
        &self.legal_move_targets
    }

    pub fn take_legal_moves_request(&mut self) -> Option<Square> {
        self.pending_legal_moves_request.take()
    }

    pub fn selected_square(&self) -> Option<Square> {
        self.drag.map(|drag| drag.from)
    }

    pub fn confirm_pending(&mut self, move_id: MoveId) -> Result<(), BoardError> {
        let result = self.board.confirm_pending(move_id);
        if result.is_ok() {
            self.move_history_scroll_handle.scroll_to_bottom();
        }
        result
    }

    pub fn rollback_pending(&mut self, move_id: MoveId) -> Result<(), BoardError> {
        self.board.rollback_pending(move_id, Instant::now())
    }

    pub fn undo_last(&mut self) -> Result<MoveId, BoardError> {
        self.board.undo_last(Instant::now())
    }

    pub fn apply_visual_move(&mut self, request: MoveRequest) -> Result<MoveId, BoardError> {
        let result = self.board.apply_move(request, Instant::now());
        if result.is_ok() {
            self.move_history_scroll_handle.scroll_to_bottom();
        }
        result
    }

    pub fn step_back_view(&mut self) -> bool {
        let changed = self.board.step_back_view();
        if changed {
            self.drag = None;
            self.pending_legal_moves_request = None;
            self.legal_move_source = None;
            self.legal_move_targets.clear();
            self.sync_move_history_scroll_to_view();
        }
        changed
    }

    pub fn step_forward_view(&mut self) -> bool {
        let changed = self.board.step_forward_view();
        if changed {
            self.drag = None;
            self.pending_legal_moves_request = None;
            self.legal_move_source = None;
            self.legal_move_targets.clear();
            self.sync_move_history_scroll_to_view();
        }
        changed
    }

    pub fn jump_to_move(&mut self, move_id: MoveId) -> bool {
        let changed = self.board.jump_to_move(move_id);
        if changed {
            self.drag = None;
            self.pending_legal_moves_request = None;
            self.legal_move_source = None;
            self.legal_move_targets.clear();
            self.sync_move_history_scroll_to_view();
        }
        changed
    }

    pub fn jump_to_start(&mut self) -> bool {
        let old_ply = self.board.displayed_ply();
        self.board.jump_to_start();
        let changed = old_ply != self.board.displayed_ply();
        if changed {
            self.drag = None;
            self.pending_legal_moves_request = None;
            self.legal_move_source = None;
            self.legal_move_targets.clear();
            self.sync_move_history_scroll_to_view();
        }
        changed
    }

    pub fn jump_to_latest(&mut self) -> bool {
        let old_ply = self.board.displayed_ply();
        self.board.jump_to_latest();
        let changed = old_ply != self.board.displayed_ply();
        if changed {
            self.drag = None;
            self.pending_legal_moves_request = None;
            self.legal_move_source = None;
            self.legal_move_targets.clear();
            self.sync_move_history_scroll_to_view();
        }
        changed
    }

    fn sync_move_history_scroll_to_view(&self) {
        let history_len = self.board.history_len();
        if history_len == 0 {
            return;
        }

        let displayed_ply = self.board.displayed_ply();
        if displayed_ply == 0 {
            self.move_history_scroll_handle.scroll_to_top_of_item(0);
            return;
        }

        let row_index = (displayed_ply - 1) / 2;
        self.move_history_scroll_handle.scroll_to_item(row_index);
    }

    fn square_for_row_col(&self, row: u8, col: u8) -> Square {
        let (file, rank) = match self.perspective {
            Perspective::White => (col, 7 - row),
            Perspective::Black => (7 - col, row),
        };

        Square::from_file_rank(file, rank).expect("row/col map to valid square")
    }

    fn square_color(&self, square: Square) -> Rgba {
        if (square.file() + square.rank()).is_multiple_of(2) {
            self.theme.light_square
        } else {
            self.theme.dark_square
        }
    }

    fn board_px(&self) -> f32 {
        self.cell_px * 8.0
    }

    fn local_pointer_from_window(&self, x: f32, y: f32) -> (f32, f32) {
        (x - self.window_origin_x, y - self.window_origin_y)
    }

    fn square_from_local_position(&self, x: f32, y: f32) -> Option<Square> {
        let board_px = self.board_px();
        if x < 0.0 || y < 0.0 || x >= board_px || y >= board_px {
            return None;
        }

        let col = (x / self.cell_px).floor() as u8;
        let row = (y / self.cell_px).floor() as u8;
        Some(self.square_for_row_col(row, col))
    }

    fn square_to_xy(&self, square: Square) -> (f32, f32) {
        let file = square.file() as f32;
        let rank = square.rank() as f32;

        match self.perspective {
            Perspective::White => (file, 7.0 - rank),
            Perspective::Black => (7.0 - file, rank),
        }
    }

    fn local_pointer_to_board_xy(&self, x: f32, y: f32) -> (f32, f32) {
        let screen_x = (x - self.cell_px * 0.5) / self.cell_px;
        let screen_y = (y - self.cell_px * 0.5) / self.cell_px;
        match self.perspective {
            Perspective::White => (screen_x, screen_y),
            Perspective::Black => (7.0 - screen_x, 7.0 - screen_y),
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }

        if !self.board.is_viewing_latest() {
            return;
        }

        let (local_x, local_y) = self
            .local_pointer_from_window(f32::from(event.position.x), f32::from(event.position.y));
        let square = self.square_from_local_position(local_x, local_y);

        let mut cleared_annotations = false;
        if square.is_some() && (!self.highlighted_squares.is_empty() || !self.arrows.is_empty()) {
            self.clear_annotations();
            cleared_annotations = true;
        }

        if self
            .board
            .active_animation()
            .is_some_and(|animation| !animation.is_finished(Instant::now()))
        {
            if cleared_annotations {
                cx.notify();
            }
            return;
        }

        let Some(square) = square else {
            if cleared_annotations {
                cx.notify();
            }
            return;
        };
        let Some(piece) = self.board.piece_at(square) else {
            if cleared_annotations {
                cx.notify();
            }
            return;
        };

        self.pending_legal_moves_request = Some(square);
        self.legal_move_source = None;
        self.legal_move_targets.clear();

        self.drag = Some(DragState {
            from: square,
            piece,
            pointer_x: local_x,
            pointer_y: local_y,
            drop_target: Some(square),
        });

        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut drag) = self.drag else {
            return;
        };

        let (local_x, local_y) = self
            .local_pointer_from_window(f32::from(event.position.x), f32::from(event.position.y));
        drag.pointer_x = local_x;
        drag.pointer_y = local_y;
        drag.drop_target = self.square_from_local_position(local_x, local_y);
        self.drag = Some(drag);
        cx.notify();
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }

        if !self.board.is_viewing_latest() {
            return;
        }

        let Some(drag) = self.drag.take() else {
            return;
        };

        let (local_x, local_y) = self
            .local_pointer_from_window(f32::from(event.position.x), f32::from(event.position.y));
        let to = self
            .square_from_local_position(local_x, local_y)
            .or(drag.drop_target);

        if let Some(to) = to.filter(|to| *to != drag.from) {
            let request = MoveRequest::new(drag.from, to);
            let animation_from = self.local_pointer_to_board_xy(local_x, local_y);
            if let Ok(id) = self.board.apply_move_pending_with_animation_from(
                request,
                Instant::now(),
                animation_from,
            ) {
                if self.auto_confirm_pending {
                    let _ = self.confirm_pending(id);
                }
                self.legal_move_source = None;
                self.legal_move_targets.clear();
            }
        }

        cx.notify();
    }

    fn on_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Right {
            return;
        }

        let (local_x, local_y) = self
            .local_pointer_from_window(f32::from(event.position.x), f32::from(event.position.y));
        let Some(square) = self.square_from_local_position(local_x, local_y) else {
            return;
        };

        self.arrow_draft = Some(ArrowDraft {
            from: square,
            pointer_x: local_x,
            pointer_y: local_y,
            to: Some(square),
            points: vec![(local_x, local_y)],
        });
        cx.notify();
    }

    fn on_right_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Right) {
            return;
        }

        let (local_x, local_y) = self
            .local_pointer_from_window(f32::from(event.position.x), f32::from(event.position.y));
        let target = self.square_from_local_position(local_x, local_y);

        let Some(draft) = self.arrow_draft.as_mut() else {
            return;
        };

        draft.pointer_x = local_x;
        draft.pointer_y = local_y;
        draft.to = target;
        if draft.points.last().is_none_or(|(last_x, last_y)| {
            let dx = local_x - *last_x;
            let dy = local_y - *last_y;
            (dx * dx + dy * dy).sqrt() > self.cell_px * 0.1
        }) {
            draft.points.push((local_x, local_y));
        }
        cx.notify();
    }

    fn on_right_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Right {
            return;
        }

        let Some(draft) = self.arrow_draft.take() else {
            return;
        };

        let (local_x, local_y) = self
            .local_pointer_from_window(f32::from(event.position.x), f32::from(event.position.y));
        let to = self
            .square_from_local_position(local_x, local_y)
            .or(draft.to)
            .unwrap_or(draft.from);

        if to != draft.from {
            let kind = choose_arrow_kind(draft.from, to, &draft.points, self.cell_px);
            let arrow = BoardArrow {
                from: draft.from,
                to,
                kind,
            };
            if let Some(ix) = self.arrows.iter().position(|candidate| *candidate == arrow) {
                self.arrows.remove(ix);
            } else {
                self.arrows.push(arrow);
            }
        } else if !self.highlighted_squares.insert(draft.from) {
            self.highlighted_squares.remove(&draft.from);
        }

        cx.notify();
    }
}

impl Render for ChessBoardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let board_px = self.cell_px * 8.0;
        let now = Instant::now();
        let piece_px = self.cell_px * self.piece_scale;
        let piece_offset = (piece_px - self.cell_px) / 2.0;
        let picked_piece_px = self.cell_px * self.picked_piece_scale;
        let picked_piece_offset = picked_piece_px / 2.0;

        self.board.prune_finished_animation(now);

        let animating = self.board.is_viewing_latest()
            && self
                .board
                .active_animation()
                .is_some_and(|animation| !animation.is_finished(now));
        if animating {
            window.request_animation_frame();
            cx.notify();
        }

        let mut grid = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(board_px))
            .h(px(board_px))
            .flex()
            .flex_col();

        for row in 0..8u8 {
            let mut row_div = div().flex().flex_row().h(px(self.cell_px));
            for col in 0..8u8 {
                let square = self.square_for_row_col(row, col);
                let square_div = div()
                    .id(("sq", square.index()))
                    .w(px(self.cell_px))
                    .h(px(self.cell_px))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(self.square_color(square))
                    .when(row == 0 && col == 0, |this| this.rounded_tl_xl())
                    .when(row == 0 && col == 7, |this| this.rounded_tr_xl())
                    .when(row == 7 && col == 0, |this| this.rounded_bl_xl())
                    .when(row == 7 && col == 7, |this| this.rounded_br_xl());
                row_div = row_div.child(square_div);
            }
            grid = grid.child(row_div);
        }

        let render_pieces = self.board.render_pieces(now, self.perspective);
        let moving_piece_index = if animating {
            render_pieces.len().checked_sub(1)
        } else {
            None
        };
        let dragged_source_xy = self.drag.map(|drag| self.square_to_xy(drag.from));
        let legal_source = self
            .legal_move_source
            .filter(|_| !self.legal_move_targets.is_empty());
        let legal_targets: Vec<Square> = self.legal_move_targets.iter().copied().collect();
        let highlighted_squares: Vec<Square> = self.highlighted_squares.iter().copied().collect();
        let arrows = self.arrows.clone();
        let arrow_draft = self.arrow_draft.clone();
        let cell_px = self.cell_px;
        let perspective = self.perspective;
        let arrow_layer_opacity = 0.8;
        let arrow_color = rgb(0x16a34a);
        let arrow_preview_color = rgb(0x4ade80);

        let mut piece_layer = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(board_px))
            .h(px(board_px));

        let mut legal_layer = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(board_px))
            .h(px(board_px));

        if let Some(source) = legal_source {
            let (x, y) = self.square_to_xy(source);
            legal_layer = legal_layer.child(
                div()
                    .id(("legal-source", source.index()))
                    .absolute()
                    .left(px(x * self.cell_px + self.cell_px * 0.06))
                    .top(px(y * self.cell_px + self.cell_px * 0.06))
                    .w(px(self.cell_px * 0.88))
                    .h(px(self.cell_px * 0.88))
                    .bg(with_alpha(rgb(self.theme.selected_outline), 0.26)),
            );
        }

        for target in legal_targets {
            let (x, y) = self.square_to_xy(target);
            legal_layer = legal_layer.child(
                div()
                    .id(("legal-target", target.index()))
                    .absolute()
                    .left(px(x * self.cell_px + self.cell_px * 0.3))
                    .top(px(y * self.cell_px + self.cell_px * 0.3))
                    .w(px(self.cell_px * 0.4))
                    .h(px(self.cell_px * 0.4))
                    .bg(with_alpha(rgb(self.theme.selected_outline), 0.42)),
            );
        }

        let mut highlight_layer = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(board_px))
            .h(px(board_px));

        for square in highlighted_squares {
            let (x, y) = self.square_to_xy(square);
            highlight_layer = highlight_layer.child(
                div()
                    .id(("hl", square.index()))
                    .absolute()
                    .left(px(x * self.cell_px))
                    .top(px(y * self.cell_px))
                    .w(px(self.cell_px))
                    .h(px(self.cell_px))
                    .bg(with_alpha(rgb(0x16a34a), 0.34))
                    .when(x == 0.0 && y == 0.0, |this| this.rounded_tl_xl())
                    .when(x == 7.0 && y == 0.0, |this| this.rounded_tr_xl())
                    .when(x == 0.0 && y == 7.0, |this| this.rounded_bl_xl())
                    .when(x == 7.0 && y == 7.0, |this| this.rounded_br_xl()),
            );
        }

        let origin_entity = cx.entity();
        let arrow_layer = canvas(
            move |bounds, _window, cx| {
                origin_entity.update(cx, |this, _| {
                    this.window_origin_x = f32::from(bounds.left());
                    this.window_origin_y = f32::from(bounds.top());
                });
            },
            move |bounds, _, window, _| {
                let origin_x = f32::from(bounds.left());
                let origin_y = f32::from(bounds.top());
                for arrow in &arrows {
                    let (sx, sy) = square_center_for_perspective(arrow.from, perspective, cell_px);
                    let (tx, ty) = square_center_for_perspective(arrow.to, perspective, cell_px);
                    let start = (sx + origin_x, sy + origin_y);
                    let end = (tx + origin_x, ty + origin_y);
                    paint_arrow(window, arrow.kind, start, end, cell_px, arrow_color);
                }

                if let Some(draft) = arrow_draft {
                    let (sx, sy) = square_center_for_perspective(draft.from, perspective, cell_px);
                    let (tx, ty) = if let Some(to) = draft.to {
                        square_center_for_perspective(to, perspective, cell_px)
                    } else {
                        (draft.pointer_x, draft.pointer_y)
                    };
                    let kind = if let Some(to) = draft.to {
                        choose_arrow_kind(draft.from, to, &draft.points, cell_px)
                    } else if looks_like_l_gesture(&draft.points, cell_px) {
                        ArrowKind::KnightL
                    } else {
                        ArrowKind::Straight
                    };
                    paint_arrow(
                        window,
                        kind,
                        (sx + origin_x, sy + origin_y),
                        (tx + origin_x, ty + origin_y),
                        cell_px,
                        arrow_preview_color,
                    );
                }
            },
        )
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .w(px(board_px))
        .h(px(board_px))
        .opacity(arrow_layer_opacity);

        for (idx, piece) in render_pieces.into_iter().enumerate() {
            if let Some((drag_x, drag_y)) = dragged_source_xy {
                if piece.piece == self.drag.map(|drag| drag.piece).unwrap_or(piece.piece)
                    && (piece.x - drag_x).abs() < 0.001
                    && (piece.y - drag_y).abs() < 0.001
                {
                    continue;
                }
            }

            let is_moving_piece = moving_piece_index == Some(idx);
            let left = piece.x * self.cell_px - piece_offset;
            let top = piece.y * self.cell_px - piece_offset;
            let width = piece_px;
            let height = piece_px;

            let (left, top, width, height) = if is_moving_piece {
                (left, top, width, height)
            } else {
                (left.round(), top.round(), width.round(), height.round())
            };

            piece_layer = piece_layer.child(
                div()
                    .id(("piece", idx))
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(Path::new(piece_asset_path(piece.piece)))
                            .w_full()
                            .h_full()
                            .object_fit(ObjectFit::Contain),
                    ),
            );
        }

        if let Some(drag) = self.drag {
            piece_layer = piece_layer.child(
                div()
                    .id("drag-piece")
                    .absolute()
                    .left(px((drag.pointer_x - picked_piece_offset).round()))
                    .top(px((drag.pointer_y - picked_piece_offset).round()))
                    .w(px(picked_piece_px.round()))
                    .h(px(picked_piece_px.round()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(Path::new(piece_asset_path(drag.piece)))
                            .w_full()
                            .h_full()
                            .object_fit(ObjectFit::Contain),
                    ),
            );
        }

        div()
            .id("board-root")
            .relative()
            .w(px(board_px))
            .h(px(board_px))
            .rounded_xl()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_move(cx.listener(Self::on_right_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_right_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Right, cx.listener(Self::on_right_mouse_up))
            .child(grid)
            .child(highlight_layer)
            .child(legal_layer)
            .child(piece_layer)
            .child(arrow_layer)
    }
}

fn square_center_for_perspective(
    square: Square,
    perspective: Perspective,
    cell_px: f32,
) -> (f32, f32) {
    let file = square.file() as f32;
    let rank = square.rank() as f32;
    let (x, y) = match perspective {
        Perspective::White => (file, 7.0 - rank),
        Perspective::Black => (7.0 - file, rank),
    };
    ((x + 0.5) * cell_px, (y + 0.5) * cell_px)
}

fn paint_arrow(
    window: &mut Window,
    kind: ArrowKind,
    start: (f32, f32),
    end: (f32, f32),
    cell_px: f32,
    color: gpui::Rgba,
) {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let length = (dx * dx + dy * dy).sqrt();
    if length < cell_px * 0.4 {
        return;
    }

    let head_len = (cell_px * 0.42).clamp(12.0, 30.0).min(length * 0.68);
    let head_width = (cell_px * 0.24).clamp(8.0, 16.0);
    let shaft_width = (cell_px * 0.14).clamp(5.0, 11.0);

    match kind {
        ArrowKind::Straight => {
            paint_straight_arrow(window, start, end, shaft_width, head_len, head_width, color);
        }
        ArrowKind::KnightL => {
            let abs_dx = dx.abs();
            let abs_dy = dy.abs();
            let elbow = if abs_dx >= abs_dy {
                (end.0, start.1)
            } else {
                (start.0, end.1)
            };

            let final_dx = end.0 - elbow.0;
            let final_dy = end.1 - elbow.1;
            let final_len = (final_dx * final_dx + final_dy * final_dy).sqrt();
            if final_len < head_len + shaft_width * 0.5 {
                return;
            }
            let fnx = final_dx / final_len;
            let fny = final_dy / final_len;
            let head_base = (end.0 - fnx * head_len, end.1 - fny * head_len);

            paint_polyline_shaft(
                window,
                &[start, elbow, head_base],
                shaft_width,
                LineCap::Round,
                LineCap::Butt,
                LineJoin::Round,
                color,
            );
            paint_arrow_head(window, head_base, end, head_width, color);
        }
    }
}

fn paint_straight_arrow(
    window: &mut Window,
    start: (f32, f32),
    tip: (f32, f32),
    shaft_width: f32,
    head_len: f32,
    head_half_width: f32,
    color: gpui::Rgba,
) {
    let dx = tip.0 - start.0;
    let dy = tip.1 - start.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < head_len + shaft_width {
        return;
    }

    let nx = dx / len;
    let ny = dy / len;
    let head_base = (tip.0 - nx * head_len, tip.1 - ny * head_len);

    paint_polyline_shaft(
        window,
        &[start, head_base],
        shaft_width,
        LineCap::Round,
        LineCap::Butt,
        LineJoin::Round,
        color,
    );
    paint_arrow_head(window, head_base, tip, head_half_width, color);
}

fn paint_arrow_head(
    window: &mut Window,
    base: (f32, f32),
    tip: (f32, f32),
    head_half_width: f32,
    color: gpui::Rgba,
) {
    let dx = tip.0 - base.0;
    let dy = tip.1 - base.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 {
        return;
    }
    let nx = dx / len;
    let ny = dy / len;
    let perp_x = -ny;
    let perp_y = nx;
    let left = (
        base.0 + perp_x * head_half_width,
        base.1 + perp_y * head_half_width,
    );
    let right = (
        base.0 - perp_x * head_half_width,
        base.1 - perp_y * head_half_width,
    );
    paint_polygon(window, &[tip, left, right], color);
}

fn paint_polyline_shaft(
    window: &mut Window,
    points: &[(f32, f32)],
    width: f32,
    start_cap: LineCap,
    end_cap: LineCap,
    line_join: LineJoin,
    color: gpui::Rgba,
) {
    if points.len() < 2 {
        return;
    }
    let stroke_options = StrokeOptions::default()
        .with_line_width(width)
        .with_start_cap(start_cap)
        .with_end_cap(end_cap)
        .with_line_join(line_join);
    let mut path = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(stroke_options));
    path.move_to(point(px(points[0].0), px(points[0].1)));
    for (x, y) in points.iter().skip(1) {
        path.line_to(point(px(*x), px(*y)));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_polygon(window: &mut Window, points: &[(f32, f32)], color: gpui::Rgba) {
    if points.len() < 3 {
        return;
    }
    let polygon: Vec<_> = points.iter().map(|(x, y)| point(px(*x), px(*y))).collect();
    let mut path = PathBuilder::fill();
    path.add_polygon(&polygon, true);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha.clamp(0.0, 1.0);
    color
}

fn choose_arrow_kind(from: Square, to: Square, points: &[(f32, f32)], cell_px: f32) -> ArrowKind {
    if is_knight_move(from, to) || looks_like_l_gesture(points, cell_px) {
        ArrowKind::KnightL
    } else {
        ArrowKind::Straight
    }
}

fn is_knight_move(from: Square, to: Square) -> bool {
    let df = (from.file() as i32 - to.file() as i32).abs();
    let dr = (from.rank() as i32 - to.rank() as i32).abs();
    (df == 1 && dr == 2) || (df == 2 && dr == 1)
}

fn looks_like_l_gesture(points: &[(f32, f32)], cell_px: f32) -> bool {
    if points.len() < 4 {
        return false;
    }

    let mut first = None;
    let mut last = None;
    for window in points.windows(2) {
        let dx = window[1].0 - window[0].0;
        let dy = window[1].1 - window[0].1;
        let len = (dx * dx + dy * dy).sqrt();
        if len < cell_px * 0.06 {
            continue;
        }
        let vector = (dx / len, dy / len, len);
        if first.is_none() {
            first = Some(vector);
        }
        last = Some(vector);
    }

    let Some((fx, fy, flen)) = first else {
        return false;
    };
    let Some((lx, ly, llen)) = last else {
        return false;
    };

    if flen < cell_px * 0.35 || llen < cell_px * 0.35 {
        return false;
    }

    let dot = (fx * lx + fy * ly).abs();
    if dot > 0.42 {
        return false;
    }

    let (sx, sy) = points[0];
    let (ex, ey) = points[points.len() - 1];
    let dx = (ex - sx).abs();
    let dy = (ey - sy).abs();
    dx > cell_px * 0.6 && dy > cell_px * 0.6
}

const CBURNETT_WK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/wK.svg");
const CBURNETT_WQ: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/wQ.svg");
const CBURNETT_WR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/wR.svg");
const CBURNETT_WB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/wB.svg");
const CBURNETT_WN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/wN.svg");
const CBURNETT_WP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/wP.svg");
const CBURNETT_BK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/bK.svg");
const CBURNETT_BQ: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/bQ.svg");
const CBURNETT_BR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/bR.svg");
const CBURNETT_BB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/bB.svg");
const CBURNETT_BN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/bN.svg");
const CBURNETT_BP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pieces/cburnett/bP.svg");

fn piece_asset_path(piece: Piece) -> &'static str {
    match (piece.side, piece.kind) {
        (Side::White, PieceKind::King) => CBURNETT_WK,
        (Side::White, PieceKind::Queen) => CBURNETT_WQ,
        (Side::White, PieceKind::Rook) => CBURNETT_WR,
        (Side::White, PieceKind::Bishop) => CBURNETT_WB,
        (Side::White, PieceKind::Knight) => CBURNETT_WN,
        (Side::White, PieceKind::Pawn) => CBURNETT_WP,
        (Side::Black, PieceKind::King) => CBURNETT_BK,
        (Side::Black, PieceKind::Queen) => CBURNETT_BQ,
        (Side::Black, PieceKind::Rook) => CBURNETT_BR,
        (Side::Black, PieceKind::Bishop) => CBURNETT_BB,
        (Side::Black, PieceKind::Knight) => CBURNETT_BN,
        (Side::Black, PieceKind::Pawn) => CBURNETT_BP,
    }
}

pub fn launch_demo() {
    gpui::Application::new().run(|cx: &mut App| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|_| ChessBoardView::new(VisualBoard::standard()))
        })
        .expect("failed to open GPUI window");
    });
}

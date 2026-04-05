use gpui::{
    App, Context, Entity, FocusHandle, Focusable, ModifiersChangedEvent, MouseDownEvent,
    MouseUpEvent, Window,
};
use kchess_board::{ChessBoardView, GameState, Perspective, Side};

use crate::app::command_palette::{CommandAction, CommandMatch, match_commands};
use crate::app::navigation::{ActivePane, NavigationState};
use crate::computer::ComputerStrength;
use crate::engine::stockfish::{self, BestMove};
use crate::engine_state::EngineState;
use crate::pages::{HistoryPage, SettingsPage};
use crate::search_input::SearchInput;
use crate::storage;
use crate::theme::ThemeState;
use crate::{
    FlipBoard, FocusSearch, OpenHistory, OpenPlayWithComputer, OpenSettings, PlayHistoryStepBack,
    PlayHistoryStepForward, SearchCancel, SearchConfirm, SearchDown, SearchUp, ShowHelp,
};

mod controllers;
mod sidebar;

use self::controllers::{ComputerControllerState, PlayScreenState, SearchControllerState};

const SIDEBAR_WIDTH: f32 = 248.0;
const TOP_CHROME_HEIGHT: f32 = 38.0;
const SEARCH_BOX_HEIGHT: f32 = 30.0;
const TOP_SEARCH_MAX_WIDTH: f32 = 620.0;
const TOP_SEARCH_OVERLAY_TOP: f32 = 34.0;
pub const BOARD_CELL_PX: f32 = 64.0;

const ICON_SEARCH: &str = "assets/icons/search.svg";

pub struct KChessApp {
    root_focus: FocusHandle,
    play_focus: FocusHandle,
    play_surface_focus: FocusHandle,
    history_focus: FocusHandle,
    settings_focus: FocusHandle,
    navigation: NavigationState,
    board_view: Entity<ChessBoardView>,
    history_view: Entity<HistoryPage>,
    settings_view: Entity<SettingsPage>,
    search_input: Entity<SearchInput>,
    engine_state: Entity<EngineState>,
    theme_state: Entity<ThemeState>,
    search_controller: SearchControllerState,
    play_screen: PlayScreenState,
    computer_controller: ComputerControllerState,
    cmd_held: bool,
}

impl KChessApp {
    pub fn new(
        cx: &mut Context<Self>,
        board_view: Entity<ChessBoardView>,
        history_view: Entity<HistoryPage>,
        settings_view: Entity<SettingsPage>,
        search_input: Entity<SearchInput>,
        engine_state: Entity<EngineState>,
        theme_state: Entity<ThemeState>,
    ) -> Self {
        Self {
            root_focus: cx.focus_handle().tab_index(-1).tab_stop(false),
            play_focus: cx.focus_handle().tab_index(1).tab_stop(true),
            play_surface_focus: cx.focus_handle().tab_index(-1).tab_stop(false),
            history_focus: cx.focus_handle().tab_index(2).tab_stop(true),
            settings_focus: cx.focus_handle().tab_index(3).tab_stop(true),
            navigation: NavigationState::new(),
            board_view,
            history_view,
            settings_view,
            search_input,
            engine_state,
            theme_state,
            search_controller: SearchControllerState::default(),
            play_screen: PlayScreenState::default(),
            computer_controller: ComputerControllerState::default(),
            cmd_held: false,
        }
    }

    fn focus_search_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search_input.read(cx).keyboard_focus_handle();
        window.focus(&handle);
    }

    fn focus_play_surface(&self, window: &mut Window) {
        window.focus(&self.play_surface_focus);
    }

    fn current_search_query(&self, cx: &mut Context<Self>) -> String {
        self.search_input.read(cx).text()
    }

    fn current_command_matches(&self, cx: &mut Context<Self>) -> Vec<CommandMatch> {
        match_commands(&self.current_search_query(cx))
    }

    fn selected_command(&self, matches: &[CommandMatch]) -> Option<CommandMatch> {
        matches
            .get(self.navigation.search_selection)
            .copied()
            .or_else(|| matches.first().copied())
    }

    fn execute_command(
        &mut self,
        action: CommandAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            CommandAction::OpenPane(pane) => {
                self.select_tab(pane, true, window, cx);
            }
            CommandAction::SetAppearance(mode) => {
                let window_appearance = window.appearance();
                self.theme_state.update(cx, |theme, theme_cx| {
                    theme.set_mode(mode, window_appearance, theme_cx);
                });

                if let Err(err) = storage::save_app_appearance_mode(mode) {
                    eprintln!("Failed to persist appearance mode from command palette: {err}");
                }

                self.close_search(window, cx);
            }
        }
    }

    fn activate_search(&mut self, clear_query: bool, window: &mut Window, cx: &mut Context<Self>) {
        if clear_query {
            self.search_input
                .update(cx, |input, cx| input.set_text("", cx));
        }

        let result_len = self.current_command_matches(cx).len();
        self.navigation.activate_search(result_len);
        self.focus_search_input(window, cx);
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigation.close_search();
        if self.navigation.active_pane == ActivePane::PlayWithComputer {
            self.focus_play_surface(window);
        } else {
            window.focus(&self.root_focus);
        }
        cx.notify();
    }

    fn select_tab(
        &mut self,
        pane: ActivePane,
        close_search: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigation.select_pane(pane, close_search);
        if close_search {
            if pane == ActivePane::PlayWithComputer {
                self.focus_play_surface(window);
            } else {
                window.focus(&self.root_focus);
            }
        }
        cx.notify();
    }

    fn apply_search_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let command_matches = self.current_command_matches(cx);
        if let Some(command) = self.selected_command(&command_matches) {
            self.execute_command(command.action, window, cx);
        }
    }

    fn side_label(side: Side) -> &'static str {
        match side {
            Side::White => "White",
            Side::Black => "Black",
        }
    }

    fn perspective_user_side(&self, cx: &App) -> Side {
        let perspective = self.board_view.read(cx).perspective();
        match perspective {
            Perspective::White => Side::White,
            Perspective::Black => Side::Black,
        }
    }

    pub fn toggle_computer_dropdown(&mut self, cx: &mut Context<Self>) {
        self.play_screen.toggle_computer_dropdown();
        cx.notify();
    }

    pub fn select_computer_strength(&mut self, strength: ComputerStrength, cx: &mut Context<Self>) {
        self.play_screen.close_computer_dropdown();
        if self.engine_state.read(cx).resolved_engine_path().is_none() {
            self.computer_controller.set_error(Some(
                "Stockfish is not installed. Open Settings > Chess Engine to install it."
                    .to_string(),
            ));
            self.computer_controller.set_status(None);
            cx.notify();
            return;
        }

        self.computer_controller.set_strength(Some(strength));
        self.computer_controller
            .set_user_side(Some(self.perspective_user_side(cx)));
        self.computer_controller.set_error(None);
        self.computer_controller.clear_active_request();
        self.computer_controller.set_thinking(false);

        let user_side = self.computer_controller.user_side().unwrap_or(Side::White);
        self.board_view.update(cx, |view, board_cx| {
            view.reset_to_start();
            view.set_interaction_side_lock(Some(user_side));
            board_cx.notify();
        });

        self.computer_controller.set_status(Some(format!(
            "Computer level set to {}. You are {}.",
            strength.display_label(),
            Self::side_label(user_side)
        )));
        self.maybe_schedule_computer_move(cx);
        cx.notify();
    }

    fn maybe_schedule_computer_move(&mut self, cx: &mut Context<Self>) {
        if self.computer_controller.thinking() {
            return;
        }

        let Some(strength) = self.computer_controller.strength() else {
            return;
        };
        let Some(user_side) = self.computer_controller.user_side() else {
            return;
        };
        let Some(engine_path) = self.engine_state.read(cx).resolved_engine_path() else {
            self.computer_controller.set_error(Some(
                "Stockfish engine unavailable. Install or configure it in Settings.".to_string(),
            ));
            self.computer_controller.set_status(None);
            return;
        };

        let (viewing_latest, game_state, side_to_move, history) = {
            let board_view = self.board_view.read(cx);
            let board = board_view.board();
            (
                board_view.is_viewing_latest(),
                board.current_game_state(),
                board.side_to_move(),
                board
                    .history()
                    .iter()
                    .map(|entry| entry.request)
                    .collect::<Vec<_>>(),
            )
        };

        if !viewing_latest {
            return;
        }

        match game_state {
            GameState::Ongoing => {}
            GameState::Checkmate { winner } => {
                self.computer_controller.set_status(Some(format!(
                    "{} wins by checkmate.",
                    Self::side_label(winner)
                )));
                self.computer_controller.set_error(None);
                return;
            }
            GameState::Stalemate => {
                self.computer_controller
                    .set_status(Some("Draw by stalemate.".to_string()));
                self.computer_controller.set_error(None);
                return;
            }
        }

        if side_to_move == user_side {
            return;
        }

        let profile = strength.engine_profile();
        let position_fingerprint = format!(
            "{}|limit:{}|elo:{:?}|movetime:{}|user:{}",
            stockfish::build_position_command(&history),
            profile.limit_strength,
            profile.clamped_elo(),
            profile.movetime_ms,
            Self::side_label(user_side)
        );

        if self
            .computer_controller
            .active_request()
            .is_some_and(|(_, fingerprint)| fingerprint == position_fingerprint.as_str())
        {
            return;
        }

        let request_id = self
            .computer_controller
            .begin_request(position_fingerprint.clone());
        self.computer_controller.set_thinking(true);
        self.computer_controller.set_error(None);
        self.computer_controller
            .set_status(Some("Computer is thinking...".to_string()));
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { stockfish::compute_best_move(&engine_path, &history, profile) })
                .await;

            let _ = this.update(cx, |view, cx| {
                view.finish_computer_move(request_id, &position_fingerprint, result, cx);
            });
        })
        .detach();
    }

    fn finish_computer_move(
        &mut self,
        request_id: u64,
        position_fingerprint: &str,
        result: Result<BestMove, stockfish::StockfishError>,
        cx: &mut Context<Self>,
    ) {
        let Some((active_id, active_fingerprint)) = self.computer_controller.active_request()
        else {
            return;
        };

        if active_id != request_id || active_fingerprint != position_fingerprint {
            return;
        }

        self.computer_controller.clear_active_request();
        self.computer_controller.set_thinking(false);

        match result {
            Ok(BestMove::NoMove) => {
                let game_state = self.board_view.read(cx).board().current_game_state();
                self.computer_controller.set_status(match game_state {
                    GameState::Ongoing => Some("No legal move available.".to_string()),
                    GameState::Checkmate { winner } => {
                        Some(format!("{} wins by checkmate.", Self::side_label(winner)))
                    }
                    GameState::Stalemate => Some("Draw by stalemate.".to_string()),
                });
                self.computer_controller.set_error(None);
            }
            Ok(BestMove::Move(request)) => {
                let user_side = self.computer_controller.user_side();
                let apply_result = self.board_view.update(cx, |board, board_cx| {
                    if !board.is_viewing_latest() {
                        return Ok(());
                    }
                    board
                        .apply_visual_move(request)
                        .map_err(|err| err.to_string())?;
                    if let Some(side) = user_side {
                        board.set_interaction_side_lock(Some(side));
                    }
                    board_cx.notify();
                    Ok::<(), String>(())
                });

                match apply_result {
                    Ok(()) => {
                        self.computer_controller.set_status(None);
                        self.computer_controller.set_error(None);
                    }
                    Err(err) => {
                        self.computer_controller
                            .set_error(Some(format!("Failed to apply engine move: {err}")));
                        self.computer_controller.set_status(None);
                    }
                }
            }
            Err(err) => {
                self.computer_controller.set_error(Some(err.to_string()));
                self.computer_controller.set_status(None);
            }
        }

        self.maybe_schedule_computer_move(cx);
        cx.notify();
    }

    fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cmd_held = event.modifiers.platform;
        if self.cmd_held != cmd_held {
            self.cmd_held = cmd_held;
            cx.notify();
        }
    }

    fn on_search_click(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.activate_search(false, window, cx);
    }

    fn on_root_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.navigation.search_active {
            self.search_controller.reset_outside_click_guard();
            return;
        }

        if self.search_controller.consume_outside_click_guard() {
            return;
        }

        self.close_search(window, cx);
    }

    fn on_search_region_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.search_controller.mark_inside_region_click();
    }

    fn on_computer_control_region_mouse_down(&mut self, _cx: &mut Context<Self>) {
        self.play_screen.mark_inside_computer_region_click();
    }

    fn on_play_surface_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_play_surface(window);
        if !self.play_screen.computer_dropdown_open() {
            self.play_screen.reset_outside_computer_click_guard();
            return;
        }

        if self.play_screen.consume_outside_computer_click_guard() {
            return;
        }

        if self.play_screen.close_computer_dropdown() {
            cx.notify();
        }
    }

    fn focus_search_action(
        &mut self,
        _: &FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.search_active {
            self.close_search(window, cx);
        } else {
            self.activate_search(true, window, cx);
        }
    }

    fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_tab(ActivePane::Settings, true, window, cx);
    }

    fn open_play_action(
        &mut self,
        _: &OpenPlayWithComputer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_tab(ActivePane::PlayWithComputer, true, window, cx);
    }

    fn open_history_action(
        &mut self,
        _: &OpenHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_tab(ActivePane::History, true, window, cx);
    }

    fn search_up_action(&mut self, _: &SearchUp, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.navigation.search_active {
            return;
        }

        let result_len = self.current_command_matches(cx).len();
        if self.navigation.move_selection_up(result_len) {
            cx.notify();
        }
    }

    fn search_down_action(&mut self, _: &SearchDown, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.navigation.search_active {
            return;
        }

        let result_len = self.current_command_matches(cx).len();
        if self.navigation.move_selection_down(result_len) {
            cx.notify();
        }
    }

    fn search_confirm_action(
        &mut self,
        _: &SearchConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.search_active {
            self.apply_search_selection(window, cx);
            return;
        }

        if self.play_focus.is_focused(window) {
            self.select_tab(ActivePane::PlayWithComputer, true, window, cx);
        } else if self.history_focus.is_focused(window) {
            self.select_tab(ActivePane::History, true, window, cx);
        } else if self.settings_focus.is_focused(window) {
            self.select_tab(ActivePane::Settings, true, window, cx);
        }
    }

    fn search_cancel_action(
        &mut self,
        _: &SearchCancel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.search_active {
            self.close_search(window, cx);
        }
    }

    fn show_help_action(&mut self, _: &ShowHelp, _window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub fn on_window_appearance_changed(&mut self, window: &Window, cx: &mut Context<Self>) {
        let window_appearance = window.appearance();
        self.theme_state.update(cx, |theme, cx| {
            theme.update_for_window_appearance(window_appearance, cx);
        });
    }

    fn play_history_step_back_action(
        &mut self,
        _: &PlayHistoryStepBack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.search_active
            || self.navigation.active_pane != ActivePane::PlayWithComputer
        {
            return;
        }

        self.board_view.update(cx, |view, board_cx| {
            if view.step_back_view() {
                board_cx.notify();
            }
        });
    }

    fn play_history_step_forward_action(
        &mut self,
        _: &PlayHistoryStepForward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.search_active
            || self.navigation.active_pane != ActivePane::PlayWithComputer
        {
            return;
        }

        self.board_view.update(cx, |view, board_cx| {
            if view.step_forward_view() {
                board_cx.notify();
            }
        });
    }

    fn apply_flip_board(&mut self, cx: &mut Context<Self>) {
        let restart_strength = self.computer_controller.strength();
        self.board_view.update(cx, |view, board_cx| {
            view.toggle_perspective();
            board_cx.notify();
        });

        if let Some(strength) = restart_strength {
            self.select_computer_strength(strength, cx);
            return;
        }

        cx.notify();
    }

    fn flip_board_action(&mut self, _: &FlipBoard, _window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation.search_active
            || self.navigation.active_pane != ActivePane::PlayWithComputer
        {
            return;
        }

        self.apply_flip_board(cx);
    }

    fn show_play_with_computer(
        &mut self,
        _: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_tab(ActivePane::PlayWithComputer, true, window, cx);
    }

    fn show_settings(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab(ActivePane::Settings, true, window, cx);
    }

    fn show_history(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab(ActivePane::History, true, window, cx);
    }
}

impl Focusable for KChessApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root_focus.clone()
    }
}

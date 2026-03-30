use gpui::{
    App, Context, Entity, FocusHandle, Focusable, ModifiersChangedEvent, MouseDownEvent,
    MouseUpEvent, Window,
};
use kchess_board::ChessBoardView;

use crate::app::navigation::{ActivePane, NavigationState, TAB_OPTIONS};
use crate::pages::{HistoryPage, SettingsPage};
use crate::search_input::SearchInput;
use crate::window_state::save_window_bounds;
use crate::{
    FocusSearch, OpenHistory, OpenPlayWithComputer, OpenSettings, PlayHistoryStepBack,
    PlayHistoryStepForward, SearchCancel, SearchConfirm, SearchDown, SearchUp, ShowHelp,
};

mod sidebar;

const SIDEBAR_WIDTH: f32 = 248.0;
const TOP_CHROME_HEIGHT: f32 = 38.0;
const SEARCH_BOX_HEIGHT: f32 = 30.0;
pub const BOARD_CELL_PX: f32 = 64.0;

const ICON_SEARCH: &str = "assets/icons/search.svg";

pub struct KChessApp {
    root_focus: FocusHandle,
    play_focus: FocusHandle,
    history_focus: FocusHandle,
    settings_focus: FocusHandle,
    navigation: NavigationState,
    board_view: Entity<ChessBoardView>,
    history_view: Entity<HistoryPage>,
    settings_view: Entity<SettingsPage>,
    search_input: Entity<SearchInput>,
    cmd_held: bool,
    ignore_next_outside_search_click: bool,
}

impl KChessApp {
    pub fn new(
        cx: &mut Context<Self>,
        board_view: Entity<ChessBoardView>,
        history_view: Entity<HistoryPage>,
        settings_view: Entity<SettingsPage>,
        search_input: Entity<SearchInput>,
    ) -> Self {
        Self {
            root_focus: cx.focus_handle().tab_index(-1).tab_stop(false),
            play_focus: cx.focus_handle().tab_index(1).tab_stop(true),
            history_focus: cx.focus_handle().tab_index(2).tab_stop(true),
            settings_focus: cx.focus_handle().tab_index(3).tab_stop(true),
            navigation: NavigationState::new(),
            board_view,
            history_view,
            settings_view,
            search_input,
            cmd_held: false,
            ignore_next_outside_search_click: false,
        }
    }

    fn focus_search_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search_input.read(cx).keyboard_focus_handle();
        window.focus(&handle);
    }

    fn current_search_query(&self, cx: &mut Context<Self>) -> String {
        self.search_input.read(cx).text()
    }

    fn activate_search(&mut self, clear_query: bool, window: &mut Window, cx: &mut Context<Self>) {
        if clear_query {
            self.search_input
                .update(cx, |input, cx| input.set_text("", cx));
        }

        let query = self.current_search_query(cx);
        self.navigation.activate_search(&query);
        self.focus_search_input(window, cx);
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigation.close_search();
        window.focus(&self.root_focus);
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
            window.focus(&self.root_focus);
        }
        cx.notify();
    }

    fn apply_search_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.current_search_query(cx);
        if let Some(pane) = self.navigation.selected_pane_for_query(&query) {
            self.select_tab(pane, true, window, cx);
        }
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
            self.ignore_next_outside_search_click = false;
            return;
        }

        if self.ignore_next_outside_search_click {
            self.ignore_next_outside_search_click = false;
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
        self.ignore_next_outside_search_click = true;
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

        if self
            .navigation
            .move_selection_up(&self.current_search_query(cx))
        {
            cx.notify();
        }
    }

    fn search_down_action(&mut self, _: &SearchDown, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.navigation.search_active {
            return;
        }

        if self
            .navigation
            .move_selection_down(&self.current_search_query(cx))
        {
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

    pub(crate) fn save_current_window_bounds(
        &mut self,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let _ = save_window_bounds(window.window_bounds());
    }
}

impl Focusable for KChessApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root_focus.clone()
    }
}

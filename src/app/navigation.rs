#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivePane {
    PlayWithComputer,
    History,
    Settings,
}

#[derive(Clone, Copy)]
pub struct TabOption {
    pub pane: ActivePane,
    pub label: &'static str,
    pub icon_path: &'static str,
    pub aliases: &'static [&'static str],
}

const ICON_COMPUTER: &str = "assets/icons/computer.svg";
const ICON_HISTORY: &str = "assets/icons/history.svg";
const ICON_SETTINGS: &str = "assets/icons/settings.svg";

pub const TAB_OPTIONS: [TabOption; 3] = [
    TabOption {
        pane: ActivePane::PlayWithComputer,
        label: "Play with Computer",
        icon_path: ICON_COMPUTER,
        aliases: &["play", "computer", "engine", "board", "ai"],
    },
    TabOption {
        pane: ActivePane::History,
        label: "History",
        icon_path: ICON_HISTORY,
        aliases: &["history", "games", "lichess", "accounts", "archive"],
    },
    TabOption {
        pane: ActivePane::Settings,
        label: "Settings",
        icon_path: ICON_SETTINGS,
        aliases: &["settings", "preferences", "config", "options"],
    },
];

#[derive(Clone, Debug)]
pub struct NavigationState {
    pub active_pane: ActivePane,
    pub search_active: bool,
    pub last_search_query: String,
    pub search_selection: usize,
}

impl NavigationState {
    pub fn new() -> Self {
        Self {
            active_pane: ActivePane::PlayWithComputer,
            search_active: false,
            last_search_query: String::new(),
            search_selection: 0,
        }
    }

    pub fn sync_selection(&mut self, result_len: usize) {
        if result_len == 0 {
            self.search_selection = 0;
        } else if self.search_selection >= result_len {
            self.search_selection = result_len - 1;
        }
    }

    pub fn reset_selection(&mut self, result_len: usize) {
        self.search_selection = 0;
        self.sync_selection(result_len);
    }

    pub fn activate_search(&mut self, result_len: usize) {
        self.search_active = true;
        self.reset_selection(result_len);
    }

    pub fn close_search(&mut self) {
        self.search_active = false;
    }

    pub fn select_pane(&mut self, pane: ActivePane, close_search: bool) {
        self.active_pane = pane;
        if close_search {
            self.search_active = false;
        }
    }

    pub fn move_selection_up(&mut self, result_len: usize) -> bool {
        if result_len == 0 {
            return false;
        }

        if self.search_selection == 0 {
            self.search_selection = result_len - 1;
        } else {
            self.search_selection -= 1;
        }

        true
    }

    pub fn move_selection_down(&mut self, result_len: usize) -> bool {
        if result_len == 0 {
            return false;
        }

        self.search_selection = (self.search_selection + 1) % result_len;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_and_sync_clamp_selection() {
        let mut nav = NavigationState::new();
        nav.search_selection = 99;
        nav.sync_selection(3);
        assert_eq!(nav.search_selection, 2);

        nav.reset_selection(2);
        assert_eq!(nav.search_selection, 0);
    }

    #[test]
    fn move_up_and_down_wrap_with_non_empty_results() {
        let mut nav = NavigationState::new();
        nav.search_selection = 0;
        assert!(nav.move_selection_up(3));
        assert_eq!(nav.search_selection, 2);

        assert!(nav.move_selection_down(3));
        assert_eq!(nav.search_selection, 0);
    }

    #[test]
    fn move_does_nothing_for_empty_results() {
        let mut nav = NavigationState::new();
        nav.search_selection = 1;
        assert!(!nav.move_selection_up(0));
        assert_eq!(nav.search_selection, 1);
        assert!(!nav.move_selection_down(0));
        assert_eq!(nav.search_selection, 1);
    }
}

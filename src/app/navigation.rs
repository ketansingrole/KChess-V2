#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivePane {
    PlayWithComputer,
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
const ICON_SETTINGS: &str = "assets/icons/settings.svg";

pub const TAB_OPTIONS: [TabOption; 2] = [
    TabOption {
        pane: ActivePane::PlayWithComputer,
        label: "Play with Computer",
        icon_path: ICON_COMPUTER,
        aliases: &["play", "computer", "engine", "board", "ai"],
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

    pub fn matching_tabs_for_query(query: &str) -> Vec<TabOption> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return TAB_OPTIONS.to_vec();
        }

        TAB_OPTIONS
            .iter()
            .copied()
            .filter(|tab| {
                tab.label.to_lowercase().contains(&q)
                    || tab.aliases.iter().any(|alias| alias.contains(&q))
            })
            .collect()
    }

    pub fn sync_search_state_from_query(&mut self, query: &str) {
        self.last_search_query = query.to_owned();

        let len = Self::matching_tabs_for_query(query).len();
        if len == 0 {
            self.search_selection = 0;
        } else if self.search_selection >= len {
            self.search_selection = len - 1;
        }
    }

    pub fn reset_selection_for_query(&mut self, query: &str) {
        self.search_selection = 0;
        self.sync_search_state_from_query(query);
    }

    pub fn activate_search(&mut self, query: &str) {
        self.search_active = true;
        self.reset_selection_for_query(query);
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

    pub fn selected_pane_for_query(&self, query: &str) -> Option<ActivePane> {
        let suggestions = Self::matching_tabs_for_query(query);
        suggestions
            .get(self.search_selection)
            .copied()
            .or_else(|| suggestions.first().copied())
            .map(|tab| tab.pane)
    }

    pub fn move_selection_up(&mut self, query: &str) -> bool {
        let suggestions = Self::matching_tabs_for_query(query);
        if suggestions.is_empty() {
            return false;
        }

        if self.search_selection == 0 {
            self.search_selection = suggestions.len() - 1;
        } else {
            self.search_selection -= 1;
        }

        true
    }

    pub fn move_selection_down(&mut self, query: &str) -> bool {
        let suggestions = Self::matching_tabs_for_query(query);
        if suggestions.is_empty() {
            return false;
        }

        self.search_selection = (self.search_selection + 1) % suggestions.len();
        true
    }
}

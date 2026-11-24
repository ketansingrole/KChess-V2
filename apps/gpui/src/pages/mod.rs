//! The app's pages, one per sidebar entry: each describes itself once as a `PageView`
//! (`kchess_nav::page`), and the registry below lists them in the sidebar's order.

pub mod analysis;
pub mod computer;
pub use kchess_database as database;
pub mod editor;
pub mod friends;
pub use kchess_history as history;
pub use kchess_home as home;
pub mod insights;
pub use kchess_local as local;
pub mod online;
pub mod openings;
pub mod players;
pub use kchess_practice as practice;
pub use kchess_puzzles as puzzles;
pub mod settings;
pub use kchess_studies as studies;
pub mod study_bar;
pub mod tournaments;
pub use kchess_watch as watch;

pub use kchess_nav::page::*;

static REGISTRY: [Registered; 18] = PAGES;

const PAGES: [Registered; 18] = [
    register::<home::HomePage>(),
    register::<computer::ComputerPage>(),
    register::<local::LocalPage>(),
    register::<puzzles::PuzzlesPage>(),
    register::<practice::PracticePage>(),
    register::<openings::OpeningsPage>(),
    register::<analysis::AnalysisPage>(),
    register::<history::HistoryPage>(),
    register::<studies::StudiesPage>(),
    register::<database::DatabasePage>(),
    register::<editor::EditorPage>(),
    register::<online::OnlinePage>(),
    register::<tournaments::TournamentsPage>(),
    register::<watch::WatchPage>(),
    register::<insights::InsightsPage>(),
    register::<friends::FriendsPage>(),
    register::<players::PlayersPage>(),
    register::<settings::SettingsPage>(),
];

// The registry is indexed by `Page`: each page sits at its own place, or the build fails.
const _: () = {
    let mut index = 0;
    while index < PAGES.len() {
        assert!(
            PAGES[index].page() as usize == index,
            "a page out of its place"
        );
        index += 1;
    }
};

/// Register the app's pages with the shell's (`kchess_nav::page::set_registry`): before anything
/// asks about a page, as the app starts and in tests.
pub fn register_pages() {
    set_registry(&REGISTRY);
}

#[cfg(test)]
mod tests {
    use super::Page;

    fn registered() {
        super::register_pages();
    }

    #[test]
    fn a_remembered_page_is_found_by_its_name() {
        registered();
        assert_eq!(Page::named("Analysis"), Some(Page::Analysis));
        assert_eq!(Page::named("Settings"), Some(Page::Settings));
        assert_eq!(Page::named("Gone"), None);
    }

    /// Shortened under their headings, the open sidebar's rows still name one page each, and
    /// each group's pages sit together.
    #[test]
    fn sidebar_rows_name_one_page_each() {
        registered();
        let mut shown: Vec<&str> = Page::all().map(|page| page.info().short).collect();
        shown.sort_unstable();
        let count = shown.len();
        shown.dedup();
        assert_eq!(shown.len(), count, "two rows read the same");
        let groups: Vec<_> = Page::all().filter_map(|page| page.info().group).collect();
        let mut seen = Vec::new();
        for group in groups {
            if seen.last() != Some(&group) {
                assert!(!seen.contains(&group), "{group:?} is split");
                seen.push(group);
            }
        }
    }
}

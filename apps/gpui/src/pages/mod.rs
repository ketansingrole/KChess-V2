//! The app's pages, one per sidebar entry, created when first shown and kept while the app runs,
//! as the Electron app keeps its page stores.

pub mod analysis;
pub mod computer;
pub mod computer_options;
pub mod editor;
pub mod friends;
pub mod history;
pub mod home;
pub mod local;
pub mod openings;
pub mod players;
pub mod practice;
pub mod puzzles;
pub mod settings;
pub mod studies;
pub mod study_bar;

use std::collections::HashMap;

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Dashboard,
    Computer,
    Local,
    Puzzles,
    Practice,
    Openings,
    Analysis,
    History,
    Studies,
    Editor,
    Online,
    Tournaments,
    Watch,
    Insights,
    Friends,
    Players,
    Settings,
}

impl Page {
    /// A game page, where zen mode hides everything but the game (`useZenSurface(true)`).
    pub fn zen_surface(self) -> bool {
        matches!(self, Page::Computer | Page::Local)
    }
}

/// A page and where its keys go.
#[derive(Clone)]
struct Shown {
    view: AnyView,
    focus: Option<FocusHandle>,
}

pub struct Pages {
    current: Page,
    views: HashMap<Page, Shown>,
}

impl Pages {
    pub fn new<T>(window: &mut Window, cx: &mut Context<T>) -> Pages {
        let mut pages = Pages {
            current: Page::Dashboard,
            views: HashMap::new(),
        };
        pages.show(Page::Dashboard, window, cx);
        pages
    }

    pub fn current(&self) -> Page {
        self.current
    }

    /// Show a page, creating it the first time, with its keys going to it.
    pub fn show<T>(&mut self, page: Page, window: &mut Window, cx: &mut Context<T>) {
        self.current = page;
        let shown = self
            .views
            .entry(page)
            .or_insert_with(|| create(page, window, cx))
            .clone();
        if let Some(focus) = shown.focus {
            window.focus(&focus, cx);
        }
    }

    /// The Settings page, once it has been shown.
    pub fn settings(&self) -> Option<Entity<settings::SettingsPage>> {
        self.view_of(Page::Settings)
    }

    /// A page's view, once it has been shown.
    pub fn view_of<V: 'static>(&self, page: Page) -> Option<Entity<V>> {
        self.views
            .get(&page)
            .and_then(|shown| shown.view.clone().downcast().ok())
    }

    /// A page's view, made now (without showing it) if it has not been shown.
    pub fn view_made<V: 'static, T>(
        &mut self,
        page: Page,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Option<Entity<V>> {
        self.views
            .entry(page)
            .or_insert_with(|| create(page, window, cx))
            .view
            .clone()
            .downcast()
            .ok()
    }

    pub fn view(&self) -> AnyView {
        self.views
            .get(&self.current)
            .map(|shown| shown.view.clone())
            .expect("the current page was created by show")
    }
}

fn create<T>(page: Page, window: &mut Window, cx: &mut Context<T>) -> Shown {
    fn focused<V: Focusable + Render>(view: Entity<V>, cx: &App) -> Shown {
        Shown {
            focus: Some(view.read(cx).focus_handle(cx)),
            view: view.into(),
        }
    }
    match page {
        Page::Computer => {
            let view = cx.new(|cx| computer::ComputerPage::new(window, cx));
            focused(view, cx)
        }
        Page::Local => {
            let view = cx.new(|cx| local::LocalPage::new(window, cx));
            focused(view, cx)
        }
        Page::Analysis => {
            let view = cx.new(|cx| analysis::AnalysisPage::new(window, cx));
            focused(view, cx)
        }
        Page::Dashboard => {
            let view = cx.new(home::HomePage::new);
            focused(view, cx)
        }
        Page::History => {
            let view = cx.new(|cx| history::HistoryPage::new(window, cx));
            focused(view, cx)
        }
        Page::Openings => {
            let view = cx.new(|cx| openings::OpeningsPage::new(window, cx));
            focused(view, cx)
        }
        Page::Practice => {
            let view = cx.new(practice::PracticePage::new);
            focused(view, cx)
        }
        Page::Puzzles => {
            let view = cx.new(|cx| puzzles::PuzzlesPage::new(window, cx));
            focused(view, cx)
        }
        Page::Editor => {
            let view = cx.new(|cx| editor::EditorPage::new(window, cx));
            focused(view, cx)
        }
        Page::Studies => {
            let view = cx.new(|cx| studies::StudiesPage::new(window, cx));
            focused(view, cx)
        }
        Page::Settings => {
            let view = cx.new(|cx| settings::SettingsPage::new(window, cx));
            focused(view, cx)
        }
        Page::Friends => {
            let view = cx.new(|cx| friends::FriendsPage::new(window, cx));
            focused(view, cx)
        }
        Page::Players => {
            let view = cx.new(|cx| players::PlayersPage::new(window, cx));
            focused(view, cx)
        }
        page => Shown {
            view: cx.new(|_| Placeholder { page }).into(),
            focus: None,
        },
    }
}

/// A page this app does not draw yet.
struct Placeholder {
    page: Page,
}

impl Render for Placeholder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child(format!("{:?}", self.page))
    }
}

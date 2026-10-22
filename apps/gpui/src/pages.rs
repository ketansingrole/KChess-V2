//! The app's pages, one per sidebar entry, created when first shown and kept while the app runs,
//! as the Electron app keeps its page stores.

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

pub struct Pages {
    current: Page,
    views: HashMap<Page, AnyView>,
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

    pub fn show<T>(&mut self, page: Page, window: &mut Window, cx: &mut Context<T>) {
        self.current = page;
        self.views
            .entry(page)
            .or_insert_with(|| create(page, window, cx));
    }

    pub fn view(&self) -> AnyView {
        self.views
            .get(&self.current)
            .cloned()
            .expect("the current page was created by show")
    }
}

fn create<T>(page: Page, _window: &mut Window, cx: &mut Context<T>) -> AnyView {
    cx.new(|_| Placeholder { page }).into()
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

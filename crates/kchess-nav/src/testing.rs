//! What the pages' tests share: a shell holding one page.

use std::cell::RefCell;

use gpui_kit::{AnyView, App, Entity, Window};

use crate::page::{Page, PageView};
use crate::sections::Section;
use crate::shell::Shell;
use crate::swipe::Direction;

/// A shell holding only page `page`, which keeps what it was asked to open: how a page's test
/// sees another page open it.
pub struct OnlyPage {
    page: Page,
    view: AnyView,
    /// The pages opened, in order, and Settings' categories chosen as `Settings:Accounts`.
    pub opened: RefCell<Vec<String>>,
}

impl OnlyPage {
    pub fn of<V: PageView>(view: Entity<V>) -> OnlyPage {
        OnlyPage {
            page: V::INFO.page,
            view: view.into(),
            opened: RefCell::default(),
        }
    }
}

impl Shell for OnlyPage {
    fn open(&self, page: Page, _: &mut Window, _: &mut App) {
        self.opened.borrow_mut().push(format!("{page:?}"));
    }

    fn current(&self, _: &App) -> Option<Page> {
        Some(self.page)
    }

    fn view(&self, page: Page, _: &App) -> Option<AnyView> {
        (page == self.page).then(|| self.view.clone())
    }

    fn made(&self, page: Page, _: &mut Window, cx: &mut App) -> Option<AnyView> {
        self.view(page, cx)
    }

    fn go(&self, _: Direction, _: &mut Window, _: &mut App) {}

    fn choose_section(&self, section: Section, _: Option<&'static str>, _: &mut App) {
        self.opened
            .borrow_mut()
            .push(format!("Settings:{section:?}"));
    }
}

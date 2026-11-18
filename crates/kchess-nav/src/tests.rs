//! The registry and the shell as a page reaches them, with stand-in pages and a stand-in shell.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    AnyView, App, AppContext as _, Context, FocusHandle, Focusable, IntoElement, Render,
    TestAppContext, Window, div,
};

use crate::page::{NavGroup, Page, PageInfo, PageView, Registered, register, set_registry};
use crate::shell::{self, Navigator, Shell};
use crate::swipe::Direction;

const ORDER: [Page; 17] = [
    Page::Dashboard,
    Page::Computer,
    Page::Local,
    Page::Puzzles,
    Page::Practice,
    Page::Openings,
    Page::Analysis,
    Page::History,
    Page::Studies,
    Page::Editor,
    Page::Online,
    Page::Tournaments,
    Page::Watch,
    Page::Insights,
    Page::Friends,
    Page::Players,
    Page::Settings,
];

/// The page at `N` in the sidebar's order, counting what it was asked to do.
struct Stand<const N: usize> {
    focus: FocusHandle,
    acted: usize,
}

impl<const N: usize> Render for Stand<N> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl<const N: usize> Focusable for Stand<N> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<const N: usize> PageView for Stand<N> {
    const INFO: PageInfo = PageInfo {
        page: ORDER[N],
        label: "A page",
        short: "Page",
        icon: "circle",
        group: Some(NavGroup::Play),
        keywords: "",
    };
    type Action = std::convert::Infallible;

    fn create(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Stand {
            focus: cx.focus_handle(),
            acted: 0,
        }
    }
}

static REGISTRY: [Registered; 17] = [
    register::<Stand<0>>(),
    register::<Stand<1>>(),
    register::<Stand<2>>(),
    register::<Stand<3>>(),
    register::<Stand<4>>(),
    register::<Stand<5>>(),
    register::<Stand<6>>(),
    register::<Stand<7>>(),
    register::<Stand<8>>(),
    register::<Stand<9>>(),
    register::<Stand<10>>(),
    register::<Stand<11>>(),
    register::<Stand<12>>(),
    register::<Stand<13>>(),
    register::<Stand<14>>(),
    register::<Stand<15>>(),
    register::<Stand<16>>(),
];

/// A page is described by the registry the app set, at its own place.
#[gpui_kit::test]
fn a_page_is_what_the_registry_says(_: &mut TestAppContext) {
    set_registry(&REGISTRY);
    assert_eq!(Page::Analysis.info().page, Page::Analysis);
    assert_eq!(REGISTRY[6].page(), Page::Analysis);
    assert_eq!(Page::all().count(), 17);
    assert_eq!(Page::named("Watch"), Some(Page::Watch));
    assert!(crate::page::opened_by(Page::Settings).is_none());
}

/// A shell that keeps what it was asked, over the app's made pages.
struct Kept {
    pages: RefCell<Option<crate::page::Pages>>,
    asked: RefCell<Vec<String>>,
}

impl Shell for Kept {
    fn open(&self, page: Page, window: &mut Window, cx: &mut App) {
        self.asked.borrow_mut().push(format!("open {page:?}"));
        if let Some(pages) = self.pages.borrow_mut().as_mut() {
            pages.choose(page, window, cx);
        }
    }

    fn current(&self, _: &App) -> Option<Page> {
        self.pages
            .borrow()
            .as_ref()
            .map(crate::page::Pages::current)
    }

    fn view(&self, page: Page, _: &App) -> Option<AnyView> {
        self.pages.borrow().as_ref()?.view_for(page)
    }

    fn made(&self, page: Page, window: &mut Window, cx: &mut App) -> Option<AnyView> {
        Some(
            self.pages
                .borrow_mut()
                .as_mut()?
                .made_view(page, window, cx),
        )
    }

    fn go(&self, direction: Direction, _: &mut Window, _: &mut App) {
        self.asked.borrow_mut().push(format!("go {direction}"));
    }
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// A page opens, finds and acts on another through the shell the app set, by its type alone.
#[gpui_kit::test]
fn a_page_reaches_others_through_the_shell(cx: &mut TestAppContext) {
    set_registry(&REGISTRY);
    let window = cx.add_window(|_, _| Blank);
    let kept = Rc::new(Kept {
        pages: RefCell::new(None),
        asked: RefCell::new(Vec::new()),
    });
    cx.update_window(window.into(), |_, window, cx| {
        // Before the app sets its shell, nothing is found or opened.
        assert!(shell::current_page(cx).is_none());
        assert!(shell::view::<Stand<6>>(cx).is_none());
        shell::open(Page::Watch, window, cx);
        *kept.pages.borrow_mut() = Some(crate::page::Pages::new(window, cx));
        cx.set_global(Navigator(kept.clone()));
        assert_eq!(shell::current_page(cx), Some(Page::Dashboard));
        assert!(shell::view::<Stand<6>>(cx).is_none(), "not made yet");
        let made = shell::made::<Stand<6>>(window, cx).expect("made now");
        assert_eq!(
            shell::view::<Stand<6>>(cx).map(|view| view.entity_id()),
            Some(made.entity_id())
        );
        shell::show::<Stand<12>>(window, cx, |page, _, _| page.acted += 1);
        assert_eq!(shell::current_page(cx), Some(Page::Watch));
        shell::go(-1, window, cx);
    })
    .unwrap();
    let shown = cx.read(|cx| shell::view::<Stand<12>>(cx).map(|page| page.read(cx).acted));
    assert_eq!(shown, Some(1));
    assert_eq!(*kept.asked.borrow(), ["open Watch", "go -1"]);
}

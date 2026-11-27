//! The registry and the shell as a page reaches them, with stand-in pages and a stand-in shell.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::{
    AnyView, App, AppContext as _, Bounds, Context, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Pixels, Render, Styled as _, TestAppContext, Window, canvas, div, px, size,
};

use crate::page::{
    NavGroup, Page, PageCommand, PageInfo, PageTab, PageView, Registered, TabInfo, register,
    set_registry, tab_places,
};
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PANEL_ZEN, PlayLayout};
use crate::sections::Section;
use crate::shell::{self, Navigator, Shell};
use crate::swipe::Direction;

const ORDER: [Page; 19] = [
    Page::Dashboard,
    Page::Computer,
    Page::Local,
    Page::Puzzles,
    Page::Practice,
    Page::Openings,
    Page::Books,
    Page::Analysis,
    Page::History,
    Page::Studies,
    Page::Database,
    Page::Editor,
    Page::Online,
    Page::Tournaments,
    Page::Watch,
    Page::Insights,
    Page::Friends,
    Page::Players,
    Page::Settings,
];

gpui_kit::actions!(stand, [OpenLast]);

/// A page's two tabs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tab {
    One,
    Two,
}

impl PageTab for Tab {
    const ALL: &'static [Self] = &[Tab::One, Tab::Two];
    const KEY: &'static str = "stand:tab";

    fn info(self) -> TabInfo {
        let key = match self {
            Tab::One => "one",
            Tab::Two => "two",
        };
        TabInfo {
            key,
            label: key,
            title: key,
            icon: "circle",
            keywords: "",
        }
    }
}

/// The page at `N` in the sidebar's order, counting what it was asked to do. The first has tabs
/// Quick Pick goes to; the last is opened by an app-wide action.
struct Stand<const N: usize> {
    focus: FocusHandle,
    acted: usize,
    tab: Option<Tab>,
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
            tab: None,
        }
    }

    fn places(_: &App) -> Vec<PageCommand> {
        if N == 0 {
            tab_places::<Self, Tab>("stand", |page, tab, _, _| page.tab = Some(tab))
        } else {
            Vec::new()
        }
    }

    fn opened_by() -> Option<Box<dyn gpui_kit::Action>> {
        (N == 18).then(|| Box::new(OpenLast) as Box<dyn gpui_kit::Action>)
    }
}

static REGISTRY: [Registered; 19] = [
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
    register::<Stand<17>>(),
    register::<Stand<18>>(),
];

/// A page is described by the registry the app set, at its own place.
#[gpui_kit::test]
fn a_page_is_what_the_registry_says(cx: &mut TestAppContext) {
    set_registry(&REGISTRY);
    assert_eq!(Page::Analysis.info().page, Page::Analysis);
    assert_eq!(REGISTRY[7].page(), Page::Analysis);
    assert_eq!(Page::all().count(), 19);
    assert_eq!(Page::named("Watch"), Some(Page::Watch));
    assert!(crate::page::opened_by(Page::Players).is_none());
    assert!(
        crate::page::opened_by(Page::Settings).is_some_and(|action| action.partial_eq(&OpenLast))
    );
    let places: Vec<_> = cx.read(|cx| {
        crate::page::places(cx)
            .into_iter()
            .map(|place| place.info.id)
            .collect()
    });
    assert_eq!(places, ["stand:one", "stand:two"]);
}

/// A tab chosen is kept, and drawn as one pill each.
#[gpui_kit::test]
fn a_tab_chosen_is_kept(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert_eq!(Tab::stored(cx), Tab::One);
        Tab::Two.store(cx);
        assert_eq!(Tab::stored(cx), Tab::Two);
    });
    let labels: Vec<_> = Tab::pills().into_iter().map(|pill| pill.label).collect();
    assert_eq!(labels, ["one", "two"]);
    assert_eq!(Tab::Two.index(), 1);
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
    fn choose_section(&self, section: Section, row: Option<&'static str>, _: &mut App) {
        self.asked
            .borrow_mut()
            .push(format!("section {section:?} {row:?}"));
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
        assert!(shell::view::<Stand<7>>(cx).is_none());
        shell::open(Page::Watch, window, cx);
        *kept.pages.borrow_mut() = Some(crate::page::Pages::new(window, cx));
        cx.set_global(Navigator(kept.clone()));
        assert_eq!(shell::current_page(cx), Some(Page::Dashboard));
        assert!(shell::view::<Stand<7>>(cx).is_none(), "not made yet");
        let made = shell::made::<Stand<7>>(window, cx).expect("made now");
        assert_eq!(
            shell::view::<Stand<7>>(cx).map(|view| view.entity_id()),
            Some(made.entity_id())
        );
        shell::show::<Stand<14>>(window, cx, |page, _, _| page.acted += 1);
        assert_eq!(shell::current_page(cx), Some(Page::Watch));
        shell::go(-1, window, cx);
        shell::open_settings(Section::Accounts, window, cx);
        shell::open_settings_at(Section::Engine, "engine-threads", window, cx);
        // The page shown has the keys until they go elsewhere.
        let pages = kept.pages.borrow();
        let pages = pages.as_ref().expect("the pages");
        assert!(pages.page_focused(window, cx));
        window.blur(cx);
        assert!(!pages.page_focused(window, cx));
    })
    .unwrap();
    let shown = cx.read(|cx| shell::view::<Stand<14>>(cx).map(|page| page.read(cx).acted));
    assert_eq!(shown, Some(1));
    assert_eq!(
        *kept.asked.borrow(),
        [
            "open Watch",
            "go -1",
            "open Settings",
            "section Accounts None",
            "open Settings",
            "section Engine Some(\"engine-threads\")"
        ]
    );
}

/// A page opened for something asked takes the keys, unless the person gave them elsewhere since
/// they asked: then they stay there.
#[gpui_kit::test]
fn keys_given_elsewhere_since_stay_there(cx: &mut TestAppContext) {
    // A window as the app opens it: the kit's state says whether a dialog has the keys.
    let window = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|_| Blank))
            .expect("a window")
            .0
    });
    cx.update_window(window, |_, window, cx| {
        let (asked, elsewhere, page) = (cx.focus_handle(), cx.focus_handle(), cx.focus_handle());
        window.focus(&asked, cx);
        shell::keeping_keys_since(Some(asked.clone()), window, cx, |window, cx| {
            window.focus(&page, cx)
        });
        assert!(page.is_focused(window), "still where they were asked");
        window.focus(&elsewhere, cx);
        shell::keeping_keys_since(Some(asked), window, cx, |window, cx| {
            window.focus(&page, cx)
        });
        assert!(elsewhere.is_focused(window), "given elsewhere since");
    })
    .unwrap();
}

/// The pages' test shell holds its one page: it finds only that page, keeps what it was asked to
/// open, and Settings' categories chosen.
#[gpui_kit::test]
fn the_test_shell_holds_only_its_page(cx: &mut TestAppContext) {
    set_registry(&REGISTRY);
    let window = cx.add_window(|_, _| Blank);
    let only = cx
        .update_window(window.into(), |_, window, cx| {
            let page = cx.new(|cx| Stand::<14>::create(window, cx));
            let only = Rc::new(crate::testing::OnlyPage::of(page.clone()));
            cx.set_global(Navigator(only.clone()));
            assert_eq!(shell::current_page(cx), Some(Page::Watch));
            assert_eq!(
                shell::view::<Stand<14>>(cx).map(|view| view.entity_id()),
                Some(page.entity_id())
            );
            assert_eq!(
                shell::made::<Stand<14>>(window, cx).map(|view| view.entity_id()),
                Some(page.entity_id())
            );
            assert!(shell::view::<Stand<7>>(cx).is_none());
            shell::open(Page::Puzzles, window, cx);
            shell::open_settings(Section::Accounts, window, cx);
            only
        })
        .unwrap();
    assert_eq!(
        *only.opened.borrow(),
        ["Puzzles", "Settings", "Settings:Accounts"]
    );
}

/// A key a page takes runs its action, under any of its names.
#[test]
fn a_page_key_runs_its_action_by_any_of_its_names() {
    use crate::page::{PageKey, page_key};
    let keys = [
        PageKey {
            action: 1,
            keys: &["space", "enter"],
            shown: "Space",
            does: "Start",
        },
        PageKey {
            action: 2,
            keys: &["p"],
            shown: "P",
            does: "Pause",
        },
    ];
    assert_eq!(page_key(&keys, "enter"), Some(1));
    assert_eq!(page_key(&keys, "p"), Some(2));
    assert_eq!(page_key(&keys, "q"), None);
}

/// `cargo xtask app --watch` starts the app again on the page it was on: the page is noted in
/// `KCHESS_PAGE_FILE` and read back from it.
#[test]
fn a_watched_restart_comes_back_to_the_page() {
    set_registry(&REGISTRY);
    let file = std::env::temp_dir().join(format!("kchess-nav-page-{}", std::process::id()));
    kchess_host::env::set("KCHESS_PAGE_FILE", &file);
    Page::Analysis.remember();
    let noted = std::fs::read_to_string(&file);
    let remembered = Page::remembered();
    kchess_host::env::reset("KCHESS_PAGE_FILE");
    std::fs::remove_file(&file).ok();
    assert_eq!(noted.ok().as_deref(), Some("Analysis"));
    assert_eq!(remembered, Some(Page::Analysis));
}

/// A board page's row: a board column and a panel as the game pages size theirs, with what
/// `panel_width` said of the panel at the last frame and where the panel was drawn.
struct Row {
    layout: PlayLayout,
    max: Pixels,
    said: Rc<Cell<Pixels>>,
    drawn: Rc<Cell<Bounds<Pixels>>>,
}

impl Render for Row {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let board = self.layout.board_size(window);
        self.said
            .set(self.layout.panel_width(board, self.max, window));
        let drawn = Rc::clone(&self.drawn);
        let panel = div()
            .relative()
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(self.max)
            .h(px(40.))
            .child(
                canvas(move |bounds, _, _| drawn.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            );
        let column = div().flex_none().w(board).h(board);
        self.layout.render("row", column, panel, window, cx)
    }
}

/// The width a game page decides its panel's layout by is the width the row gives it, beside
/// the board and under it, with Zen mode's narrower panel too.
#[gpui_kit::test]
fn the_panel_is_as_wide_as_its_width_says(cx: &mut TestAppContext) {
    for max in [PANEL_MAX, PANEL_ZEN] {
        let said = Rc::new(Cell::new(px(0.)));
        let drawn = Rc::new(Cell::new(Bounds::default()));
        let window = cx.add_window(|_, _| Row {
            layout: PlayLayout::default(),
            max,
            said: Rc::clone(&said),
            drawn: Rc::clone(&drawn),
        });
        let sizes = [(500., 700.), (640., 620.), (700., 1000.), (900., 700.)];
        let wider = [(1024., 768.), (1320., 880.), (1600., 900.), (2200., 1200.)];
        for (width, height) in sizes.into_iter().chain(wider) {
            cx.simulate_window_resize(window.into(), size(px(width), px(height)));
            // The row's own measure is a frame late without a shell.
            for _ in 0..3 {
                cx.update_window(window.into(), |_, window, cx| {
                    window.simulate_next_frame(cx);
                    window.draw(cx).clear(cx);
                })
                .expect("the window is open");
            }
            // Layout puts the panel's edges on whole pixels; the side padding is 2.6% of the width.
            let (said, drawn) = (said.get(), drawn.get().size.width);
            assert!(
                (said - drawn).abs() < px(1.),
                "the panel at {width}×{height}, at most {max:?}: said {said:?}, drawn {drawn:?}"
            );
        }
    }
}

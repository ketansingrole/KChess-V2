//! The window's frame, as the Electron app draws it (`AppWorkspace.vue`): a sidebar that collapses
//! to icons, a top bar, and the current page.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarHeader, SidebarItem, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Icon, Selectable as _, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;

use crate::pages::analysis::AnalysisPage;
use crate::pages::history::HistoryPage;
use crate::pages::home::HomePage;
use crate::pages::openings::OpeningsPage;
use crate::pages::practice::PracticePage;
use crate::pages::puzzles::PuzzlesPage;
use crate::pages::settings::{Section, sections};
use crate::pages::{Page, Pages};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::Palette;

/// The window's height above the page; matches `--topbar-height`.
pub const TOPBAR_HEIGHT: Pixels = px(52.);

/// The sidebar's pages, in the Electron app's order (`stores/kchess.ts`, `nav`).
fn nav() -> [(Page, &'static str, Icon); 16] {
    [
        (Page::Dashboard, "Home", icon("house")),
        (Page::Computer, "Play with Computer", icon("monitor")),
        (Page::Local, "Over the board", icon("users-round")),
        (Page::Puzzles, "Puzzles", icon("puzzle")),
        (Page::Practice, "Practice", icon("graduation-cap")),
        (Page::Openings, "Openings", icon("book-open-text")),
        (Page::Analysis, "Analysis board", icon("microscope")),
        (Page::History, "Game history", icon("history")),
        (Page::Studies, "Studies", icon("library-big")),
        (Page::Editor, "Board editor", icon("pencil-ruler")),
        (Page::Online, "Play on Lichess", icon("globe-2")),
        (Page::Tournaments, "Tournaments", icon("trophy")),
        (Page::Watch, "Watch", icon("tv")),
        (Page::Insights, "Lichess insights", icon("chart-pie")),
        (Page::Friends, "Following", icon("users")),
        (Page::Players, "Players", icon("user-search")),
    ]
}
/// What the sidebar lists: menus, with a heading between them while Settings is open.
#[derive(Clone)]
enum NavBlock {
    Menu(Box<SidebarMenu>),
    Heading(&'static str, bool),
}

impl Collapsible for NavBlock {
    fn is_collapsed(&self) -> bool {
        match self {
            NavBlock::Menu(menu) => menu.is_collapsed(),
            NavBlock::Heading(_, collapsed) => *collapsed,
        }
    }

    fn collapsed(self, collapsed: bool) -> Self {
        match self {
            NavBlock::Menu(menu) => NavBlock::Menu(Box::new(menu.collapsed(collapsed))),
            NavBlock::Heading(label, _) => NavBlock::Heading(label, collapsed),
        }
    }
}

impl SidebarItem for NavBlock {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        match self {
            NavBlock::Menu(menu) => menu.render(id, window, cx).into_any_element(),
            // A collapsed sidebar shows icons only.
            NavBlock::Heading(_, true) => div().into_any_element(),
            // The navigation menu's label: `font-semibold text-xs/5 text-highlighted px-2.5 py-1.5`.
            NavBlock::Heading(label, false) => div()
                .px(px(10.))
                .py(px(6.))
                .text_size(px(12.))
                .line_height(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.global::<Palette>().highlighted)
                .child(label)
                .into_any_element(),
        }
    }
}

/// The window's workspace, so a page can open another (`selectPage`).
struct Navigator(WeakEntity<Workspace>);

impl Global for Navigator {}

/// Open `page` as the sidebar would.
pub fn open(page: Page, window: &mut Window, cx: &mut App) {
    let Some(workspace) = cx
        .try_global::<Navigator>()
        .map(|navigator| navigator.0.clone())
    else {
        return;
    };
    let _ = workspace.update(cx, |workspace, cx| workspace.choose(page, window, cx));
}

/// Open Settings at one section (`openSettings`), e.g. the engine setup a page points at.
pub fn open_settings(section: Section, window: &mut Window, cx: &mut App) {
    open(Page::Settings, window, cx);
    let Some(workspace) = cx
        .try_global::<Navigator>()
        .map(|navigator| navigator.0.clone())
    else {
        return;
    };
    let _ = workspace.update(cx, |workspace, cx| workspace.choose_section(section, cx));
}

/// A page's view, once it has been shown.
fn view_of<V: 'static>(page: Page, cx: &App) -> Option<Entity<V>> {
    let workspace = cx.try_global::<Navigator>()?.0.upgrade()?;
    workspace.read(cx).pages.view_of::<V>(page)
}

/// Open a played game in Game history (Home's recent games).
pub fn open_game(id: &str, window: &mut Window, cx: &mut App) {
    open(Page::History, window, cx);
    if let Some(history) = view_of::<HistoryPage>(Page::History, cx) {
        history.update(cx, |history, cx| history.show_game(id, window, cx));
    }
}

/// Analyse a line from the starting position, asking first when the board holds unsaved
/// analysis (`useReplaceAnalysis`).
pub fn open_analysis_line(moves: Vec<String>, window: &mut Window, cx: &mut App) {
    fn load(moves: &[String], window: &mut Window, cx: &mut App) {
        open(Page::Analysis, window, cx);
        if let Some(analysis) = view_of::<AnalysisPage>(Page::Analysis, cx) {
            analysis.update(cx, |analysis, cx| analysis.load_line(moves, cx));
        }
    }
    // Made first, so analysis kept from an earlier run counts.
    let analysis = cx
        .try_global::<Navigator>()
        .and_then(|navigator| navigator.0.upgrade())
        .and_then(|workspace| {
            workspace.update(cx, |workspace, cx| {
                workspace
                    .pages
                    .view_made::<AnalysisPage, _>(Page::Analysis, window, cx)
            })
        });
    let unsaved = analysis.is_some_and(|analysis| analysis.read(cx).unsaved());
    if !unsaved {
        load(&moves, window, cx);
        return;
    }
    window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
        let moves = moves.clone();
        dialog
            .title("Replace the unsaved analysis?")
            .description("The analysis board has moves that are not saved as a study.")
            .ok_text("Replace")
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                load(&moves, window, cx);
                true
            })
    });
}

/// Puzzles from one opening (`startOpening`), by the key Lichess names its puzzles by.
pub fn open_opening_puzzles(name: &str, window: &mut Window, cx: &mut App) {
    let Some(key) = crate::game::rules("openingKey", &[serde_json::json!(name)])
        .ok()
        .and_then(|key| key.as_str().map(str::to_string))
    else {
        return;
    };
    PuzzlesPage::choose_opening(&key, name, cx);
    // A page made now starts with the opening; one made before is told.
    if let Some(puzzles) = view_of::<PuzzlesPage>(Page::Puzzles, cx) {
        puzzles.update(cx, |puzzles, cx| puzzles.opening_chosen(window, cx));
    }
    open(Page::Puzzles, window, cx);
}

pub struct Workspace {
    sidebar_open: bool,
    pages: Pages,
    /// Where Settings' Back returns to: the page open before it.
    back: Page,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Workspace {
        crate::theme::apply(window, cx);
        cx.set_global(Navigator(cx.entity().downgrade()));
        let settings = cx.global::<SettingsHandle>().0.clone();
        let subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| crate::theme::apply(window, cx)),
            cx.subscribe_in(
                &settings,
                window,
                |_, _, _: &SettingsChanged, window, cx| crate::theme::apply(window, cx),
            ),
        ];
        Workspace {
            sidebar_open: true,
            pages: Pages::new(window, cx),
            back: Page::Dashboard,
            _subscriptions: subscriptions,
        }
    }

    fn choose(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.pages.current();
        if page == Page::Settings && current != Page::Settings {
            self.back = current;
        }
        // The analysis board's engine follows the board only while it is shown.
        if current == Page::Analysis
            && page != Page::Analysis
            && let Some(analysis) = self.pages.view_of::<AnalysisPage>(Page::Analysis)
        {
            analysis.update(cx, |analysis, cx| analysis.detach(cx));
        }
        self.pages.show(page, window, cx);
        if page == Page::Analysis
            && let Some(analysis) = self.pages.view_of::<AnalysisPage>(Page::Analysis)
        {
            analysis.update(cx, |analysis, cx| analysis.attach(cx));
        }
        if page == Page::Puzzles
            && let Some(puzzles) = self.pages.view_of::<PuzzlesPage>(Page::Puzzles)
        {
            puzzles.update(cx, |puzzles, cx| puzzles.shown(cx));
        }
        if page == Page::Practice
            && let Some(practice) = self.pages.view_of::<PracticePage>(Page::Practice)
        {
            practice.update(cx, |practice, _| practice.shown());
        }
        // The sidebar's Openings is the browser (`/openings`), not the line shown last.
        if page == Page::Openings
            && let Some(openings) = self.pages.view_of::<OpeningsPage>(Page::Openings)
        {
            openings.update(cx, |openings, cx| openings.show_line(None, cx));
        }
        // Home shows the unfinished game and accounts as they are now.
        if page == Page::Dashboard
            && let Some(home) = self.pages.view_of::<HomePage>(Page::Dashboard)
        {
            home.update(cx, |home, cx| home.refresh(cx));
        }
        cx.notify();
    }

    fn choose_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if let Some(settings) = self.pages.settings() {
            settings.update(cx, |page, cx| page.show(section, cx));
        }
        cx.notify();
    }

    /// The pages, or while Settings is open its categories with a way back (`navItems`).
    fn nav_blocks(&self, cx: &mut Context<Self>) -> Vec<NavBlock> {
        let current = self.pages.current();
        if current != Page::Settings {
            return vec![NavBlock::Menu(Box::new(SidebarMenu::new().children(
                nav().into_iter().map(|(page, label, glyph)| {
                    SidebarMenuItem::new(label)
                        .icon(glyph)
                        .active(current == page)
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.choose(page, window, cx)),
                        )
                }),
            )))];
        }
        let shown = self
            .pages
            .settings()
            .map_or(Section::Appearance, |page| page.read(cx).section());
        let back = self.back;
        vec![
            NavBlock::Menu(Box::new(
                SidebarMenu::new().child(
                    SidebarMenuItem::new("Back")
                        .icon(icon("arrow-left"))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.choose(back, window, cx)),
                        ),
                ),
            )),
            NavBlock::Heading("Settings", false),
            NavBlock::Menu(Box::new(SidebarMenu::new().children(
                sections().into_iter().map(|(section, label, glyph)| {
                    SidebarMenuItem::new(label)
                        .icon(glyph)
                        .active(shown == section)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.choose_section(section, cx)),
                        )
                }),
            ))),
        ]
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.pages.current();
        let open = self.sidebar_open;
        Sidebar::new("sidebar")
            .collapsed(!open)
            .header(
                SidebarHeader::new().h(TOPBAR_HEIGHT).child(
                    // Clear of the macOS traffic lights, as the Electron toggle sits at x84.
                    h_flex()
                        .w_full()
                        .when(cfg!(target_os = "macos"), |row| row.pl(px(72.)))
                        .child(
                            Button::new("sidebar-toggle")
                                .ghost()
                                .small()
                                .icon(icon("panel-left"))
                                .tooltip(if open { "Hide sidebar" } else { "Show sidebar" })
                                .accessibility_label(if open {
                                    "Hide sidebar"
                                } else {
                                    "Show sidebar"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sidebar_open = !this.sidebar_open;
                                    cx.notify();
                                })),
                        ),
                ),
            )
            .children(self.nav_blocks(cx))
            .footer(
                SidebarFooter::new().child(
                    Button::new("settings")
                        .ghost()
                        .w_full()
                        .justify_start()
                        .icon(icon("settings-2"))
                        .when(open, |button| button.label("Settings"))
                        .when(!open, |button| button.tooltip("Settings"))
                        .accessibility_label("Settings")
                        .selected(current == Page::Settings)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.choose(Page::Settings, window, cx)
                        })),
                ),
            )
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("topbar")
            .h(TOPBAR_HEIGHT)
            .flex_none()
            .px_5()
            .border_b_1()
            .border_color(cx.theme().border)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Zen mode on a game page leaves the board and its panel (`.app-shell.zen aside`).
        let zen = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .zen_mode
            && self.pages.current().zen_surface();
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when(!zen, |shell| shell.child(self.render_sidebar(cx)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_topbar(cx))
                    .child(div().flex_1().min_h_0().child(self.pages.view())),
            )
    }
}

//! The window's frame, as the Electron app draws it (`AppWorkspace.vue`): a sidebar that collapses
//! to icons, a top bar, and the current page.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarHeader, SidebarItem, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Disableable as _, Icon, Selectable as _, Sizable as _,
    WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;

use crate::pages::analysis::AnalysisPage;
use crate::pages::computer::ComputerPage;
use crate::pages::editor::EditorPage;
use crate::pages::history::HistoryPage;
use crate::pages::home::HomePage;
use crate::pages::local::LocalPage;
use crate::pages::openings::OpeningsPage;
use crate::pages::practice::PracticePage;
use crate::pages::puzzles::PuzzlesPage;
use crate::pages::settings::{Section, sections};
use crate::pages::studies::StudiesPage;
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

/// Game history's Lichess tab showing one account's games (`historyAccount`).
pub fn open_history_account(account: &str, window: &mut Window, cx: &mut App) {
    open(Page::History, window, cx);
    if let Some(history) = view_of::<crate::pages::history::HistoryPage>(Page::History, cx) {
        let account = account.to_string();
        history.update(cx, |history, cx| {
            history.show_lichess_account(account, window, cx)
        });
    }
}

/// A player's profile on Players (`PlayerLink`).
pub fn open_player(username: &str, window: &mut Window, cx: &mut App) {
    open(Page::Players, window, cx);
    if let Some(players) = view_of::<crate::pages::players::PlayersPage>(Page::Players, cx) {
        let username = username.to_string();
        players.update(cx, |players, cx| players.show_player(&username, window, cx));
    }
}

/// Watch a game being played on Lichess (`watchGame`).
pub fn watch_game(id: &str, window: &mut Window, cx: &mut App) {
    open(Page::Watch, window, cx);
    if let Some(watch) = view_of::<crate::pages::watch::WatchPage>(Page::Watch, cx) {
        let id = id.to_string();
        watch.update(cx, |watch, cx| watch.watch_game(&id, cx));
    }
}

/// A live Lichess game of the account's on Play on Lichess (`openOngoing`).
pub fn open_online_game(id: &str, window: &mut Window, cx: &mut App) {
    open(Page::Online, window, cx);
    let _ = id;
}

/// Challenge a player to a casual game on Lichess (`challengeFriend`).
pub fn challenge(username: &str, window: &mut Window, cx: &mut App) {
    open(Page::Online, window, cx);
    let _ = username;
}

/// Game history's Lichess tab (`/history?source=lichess`).
pub fn open_history_lichess(window: &mut Window, cx: &mut App) {
    open(Page::History, window, cx);
    if let Some(history) = view_of::<crate::pages::history::HistoryPage>(Page::History, cx) {
        history.update(cx, |history, cx| history.show_lichess(cx));
    }
}

/// Open a synced Lichess game in Game history (`openReview`).
pub fn open_lichess_game(game: serde_json::Value, window: &mut Window, cx: &mut App) {
    open(Page::History, window, cx);
    if let Some(history) = view_of::<crate::pages::history::HistoryPage>(Page::History, cx) {
        history.update(cx, |history, cx| {
            history.show_lichess_game(game, window, cx)
        });
    }
}

/// Start every page afresh at Home (after signing out, as Electron reloads its window), so
/// nothing a page kept for an account outlives it.
pub fn restart_pages(window: &mut Window, cx: &mut App) {
    let Some(workspace) = cx
        .try_global::<Navigator>()
        .and_then(|navigator| navigator.0.upgrade())
    else {
        return;
    };
    workspace.update(cx, |workspace, cx| {
        workspace.pages = Pages::new(window, cx);
        workspace.choose(Page::Dashboard, window, cx);
    });
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
pub fn view_of<V: 'static>(page: Page, cx: &App) -> Option<Entity<V>> {
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

/// A page's view, made now (without showing it) if it has not been shown.
pub fn page_view<V: 'static>(page: Page, window: &mut Window, cx: &mut App) -> Option<Entity<V>> {
    let workspace = cx.try_global::<Navigator>()?.0.upgrade()?;
    workspace.update(cx, |workspace, cx| {
        workspace.pages.view_made::<V, _>(page, window, cx)
    })
}

/// The analysis board, made now if it has not been shown (so analysis kept from an earlier run
/// counts).
pub fn analysis_board(window: &mut Window, cx: &mut App) -> Option<Entity<AnalysisPage>> {
    page_view::<AnalysisPage>(Page::Analysis, window, cx)
}

/// Play Stockfish from a position, as the side to move (`startComputerFrom`).
pub fn play_computer_from(fen: String, window: &mut Window, cx: &mut App) {
    let color = if fen.split(' ').nth(1) == Some("b") {
        shakmaty::Color::Black
    } else {
        shakmaty::Color::White
    };
    open(Page::Computer, window, cx);
    // A position Stockfish refuses (one with too many pieces) only opens the page.
    if crate::game::position_problem(&fen).is_some() {
        return;
    }
    if let Some(page) = view_of::<ComputerPage>(Page::Computer, cx) {
        let setup = serde_json::json!({ "variant": "standard", "fen": fen });
        page.update(cx, |page, cx| page.start_from(setup, color, window, cx));
    }
}

/// A game over the board from a position (`/local?fen=`).
pub fn play_local_from(fen: String, window: &mut Window, cx: &mut App) {
    open(Page::Local, window, cx);
    // As the page does with `?fen=`: a position that cannot be played only opens it.
    if crate::game::position_problem(&fen).is_some() {
        return;
    }
    if let Some(page) = view_of::<LocalPage>(Page::Local, cx) {
        page.update(cx, |page, cx| page.start_from(&fen, window, cx));
    }
}

/// The board editor on a position (`editPosition`).
pub fn edit_position(fen: String, orientation: shakmaty::Color, window: &mut Window, cx: &mut App) {
    if let Some(editor) = page_view::<EditorPage>(Page::Editor, window, cx) {
        editor.update(cx, |editor, cx| {
            editor.set_position(&fen, orientation, window, cx)
        });
    }
    open(Page::Editor, window, cx);
}

/// Put something else on the analysis board and show it, asking first while the board holds
/// moves no study keeps (`useReplaceAnalysis`). `load` says whether the board took it.
pub fn replace_analysis(
    window: &mut Window,
    cx: &mut App,
    load: impl Fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>) -> bool + 'static,
) {
    let Some(analysis) = analysis_board(window, cx) else {
        return;
    };
    let load = std::rc::Rc::new(load);
    let run = {
        let analysis = analysis.clone();
        move |load: &dyn Fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>) -> bool,
              window: &mut Window,
              cx: &mut App| {
            if analysis.update(cx, |analysis, cx| load(analysis, window, cx)) {
                open(Page::Analysis, window, cx);
            }
        }
    };
    if !analysis.read(cx).unsaved() {
        run(&*load, window, cx);
        return;
    }
    window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
        let (load, run) = (load.clone(), run.clone());
        dialog
            .title("Replace the unsaved analysis?")
            .description("The analysis board has moves that are not saved as a study.")
            .ok_text("Replace")
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                run(&*load, window, cx);
                true
            })
    });
}

/// Practise a reviewed game's positions on the analysis board (`usePracticeGame`); another game
/// replaces the board's only after asking, when that would lose unsaved analysis.
pub fn practise_game(
    game: serde_json::Value,
    items: Vec<serde_json::Value>,
    window: &mut Window,
    cx: &mut App,
) {
    let shown = analysis_board(window, cx).is_some_and(|analysis| {
        analysis.read(cx).mainline_key() == game["key"].as_str().unwrap_or_default()
    });
    if shown {
        if let Some(analysis) = analysis_board(window, cx) {
            analysis.update(cx, |analysis, cx| analysis.practise_game(&game, items, cx));
        }
        open(Page::Analysis, window, cx);
        return;
    }
    replace_analysis(window, cx, move |analysis, _, cx| {
        analysis.practise_game(&game, items.clone(), cx)
    });
}

/// Analyse a line from the starting position.
pub fn open_analysis_line(moves: Vec<String>, window: &mut Window, cx: &mut App) {
    replace_analysis(window, cx, move |analysis, _, cx| {
        analysis.load_line(&moves, cx);
        true
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
            cx.observe(
                &cx.global::<crate::updates::UpdatesHandle>().0.clone(),
                |_, _, cx| cx.notify(),
            ),
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
        // Watching streams only while Watch is shown.
        if current == Page::Watch
            && page != Page::Watch
            && let Some(watch) = self
                .pages
                .view_of::<crate::pages::watch::WatchPage>(Page::Watch)
        {
            watch.update(cx, |watch, cx| watch.hidden(cx));
        }
        self.pages.show(page, window, cx);
        if page == Page::Watch
            && current != Page::Watch
            && let Some(watch) = self
                .pages
                .view_of::<crate::pages::watch::WatchPage>(Page::Watch)
        {
            watch.update(cx, |watch, cx| watch.shown(window, cx));
        }
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
        if page == Page::Studies
            && let Some(studies) = self.pages.view_of::<StudiesPage>(Page::Studies)
        {
            studies.update(cx, |studies, cx| studies.shown(cx));
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
                    v_flex()
                        .w_full()
                        .gap(px(4.))
                        .child(
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
                        )
                        .child(account_button(open, cx)),
                ),
            )
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("topbar")
            .h(TOPBAR_HEIGHT)
            .flex_none()
            .px_5()
            .gap(px(8.))
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
            .child(div().flex_1())
            .children(update_button(cx))
    }
}

/// The sidebar's account button: Connect Lichess while no account plays online, else the account
/// menu (`userMenuItems`): which account plays, connecting another, Settings and logging out.
fn account_button(expanded: bool, cx: &App) -> AnyElement {
    let handle = crate::accounts::handle(cx);
    let accounts = handle.read(cx);
    let Some(active) = accounts.active(cx) else {
        let target = handle.downgrade();
        return Button::new("connect-lichess")
            .ghost()
            .w_full()
            .justify_start()
            .icon(icon("log-in"))
            .when(expanded, |button| button.label("Connect Lichess"))
            .when(!expanded, |button| button.tooltip("Connect Lichess"))
            .accessibility_label("Connect Lichess")
            .loading(accounts.busy)
            .disabled(accounts.busy)
            .on_click(move |_, window, cx| {
                let _ = target.update(cx, |accounts, cx| accounts.connect(window, cx));
            })
            .into_any_element();
    };
    let connected = accounts.connected();
    let label = format!("@{active}");
    let target = handle.downgrade();
    Button::new("account-menu")
        .ghost()
        .w_full()
        .justify_start()
        .icon(icon("user"))
        .when(expanded, |button| button.label(label.clone()))
        .when(!expanded, |button| button.tooltip(label.clone()))
        .accessibility_label(if expanded {
            format!("{label} account menu")
        } else {
            "Account menu".into()
        })
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
            let act = |run: fn(
                &mut crate::accounts::Accounts,
                &mut Window,
                &mut Context<crate::accounts::Accounts>,
            )| {
                let target = target.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let _ = target.update(cx, |accounts, cx| run(accounts, window, cx));
                }
            };
            let several = connected.len() > 1;
            let mut menu = menu.item(
                PopupMenuItem::label(if several {
                    "Play as".into()
                } else {
                    label.clone()
                })
                .icon(icon("user")),
            );
            if several {
                for name in &connected {
                    let (name, target) = (name.clone(), target.clone());
                    menu = menu.item(
                        PopupMenuItem::new(format!("@{name}"))
                            .checked(name == active)
                            .on_click(move |_, _, cx| {
                                let _ = target
                                    .update(cx, |accounts, cx| accounts.set_active(&name, cx));
                            }),
                    );
                }
            }
            let mut menu = menu
                .separator()
                .item(
                    PopupMenuItem::new(if connected.is_empty() {
                        "Connect Lichess account"
                    } else {
                        "Connect another account"
                    })
                    .icon(icon("link"))
                    .on_click(act(|accounts, window, cx| accounts.connect(window, cx))),
                )
                .item(
                    PopupMenuItem::new("Settings")
                        .icon(icon("settings-2"))
                        .on_click(|_, window, cx| open(Page::Settings, window, cx)),
                )
                .item(
                    PopupMenuItem::new(if several {
                        format!("Log out @{active}")
                    } else {
                        "Log out".into()
                    })
                    .icon(icon("log-out"))
                    .on_click(act(|accounts, window, cx| {
                        accounts.logout(false, window, cx)
                    })),
                );
            if several {
                menu = menu.item(
                    PopupMenuItem::new("Log out all accounts")
                        .icon(icon("log-out"))
                        .on_click(act(|accounts, window, cx| {
                            accounts.logout(true, window, cx)
                        })),
                );
            }
            menu
        })
        .into_any_element()
}

/// `AppUpdateButton.vue`: a newer KChess, its download, or the restart that installs it.
fn update_button(cx: &App) -> Option<impl IntoElement + use<>> {
    use crate::updates::{Phase, UpdatesHandle};
    let handle = cx.global::<UpdatesHandle>().0.clone();
    let updates = handle.read(cx);
    let version = updates.version.clone()?;
    let from = updates.current.clone();
    let (label, tooltip) = match updates.phase {
        Phase::Available => (
            format!("Update to {version}"),
            format!("Download KChess {version} (you have {from})"),
        ),
        Phase::Downloading => (
            match updates.progress {
                Some((received, total)) if total > 0 => {
                    format!("Updating {}%", (100 * received / total).min(100))
                }
                _ => "Updating…".into(),
            },
            format!("Downloading KChess {version} (you have {from})"),
        ),
        Phase::Downloaded => (
            "Restart to update".into(),
            format!("KChess {version} is ready. Restart now, or it installs when you quit."),
        ),
        Phase::Error => (
            "Retry update".into(),
            updates
                .error
                .clone()
                .unwrap_or_else(|| "The update could not finish.".into()),
        ),
        _ => return None,
    };
    let phase = updates.phase;
    let target = handle.downgrade();
    Some(
        Button::new("update-button")
            .small()
            .when(phase == Phase::Downloaded, |button| button.primary())
            .when(phase == Phase::Error, |button| button.danger())
            .when(
                !matches!(phase, Phase::Downloaded | Phase::Error),
                |button| button.outline(),
            )
            .icon(icon(match phase {
                Phase::Downloaded => "rotate-cw",
                Phase::Error => "circle-alert",
                _ => "download",
            }))
            .label(label)
            .tooltip(tooltip)
            .disabled(phase == Phase::Downloading)
            .on_click(move |_, window, cx| {
                if phase == Phase::Downloaded {
                    crate::pages::settings::updates::ask_restart(window, cx);
                } else {
                    let _ = target.update(cx, |updates, cx| updates.download(cx));
                }
            }),
    )
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

/// Study an earlier puzzle on the analysis board, asking first when that would replace unsaved
/// analysis (`usePuzzleAttemptActions().analyse`).
pub fn analyse_puzzle(
    puzzle: serde_json::Value,
    wrong_moves: Vec<serde_json::Value>,
    window: &mut Window,
    cx: &mut App,
) {
    replace_analysis(window, cx, move |analysis, _, cx| {
        analysis.open_puzzle(&puzzle, &wrong_moves, cx)
    });
}

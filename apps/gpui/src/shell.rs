//! The window's frame, as the Electron app draws it: a sidebar that collapses
//! to icons, a top bar, and the current page.

use gpui_kit::component::animation::{EffectTransition, ease_out_cubic};
use gpui_kit::component::button::{
    Button, ButtonCustomVariant, ButtonVariant, ButtonVariants as _,
};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::sidebar::{SidebarItem, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Disableable as _, Icon, Sizable as _, TitleBar, WindowExt as _,
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;

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
use crate::pages::watch::WatchPage;
use crate::pages::{Page, Pages};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::Palette;

/// The window's height above the page; matches `--topbar-height`.
pub const TOPBAR_HEIGHT: Pixels = px(52.);

/// Whether the sidebar is open, kept for both apps (`kchess:sidebar-open`).
const SIDEBAR_OPEN: &str = "kchess:sidebar-open";

/// `USidebar`'s `--sidebar-width`, and `--sidebar-width-icon` collapsed to icons.
const SIDEBAR_WIDTH: Pixels = px(256.);
const SIDEBAR_RAIL: Pixels = px(64.);

fn sidebar_width(open: bool) -> Pixels {
    if open { SIDEBAR_WIDTH } else { SIDEBAR_RAIL }
}

/// The sidebar toggle's left edge: 84px on macOS at any interface size, clear of the traffic
/// lights, else 12px.
fn toggle_left(window: &Window) -> Pixels {
    if cfg!(target_os = "macos") {
        px(84. / window.zoom())
    } else {
        px(12.)
    }
}

/// Part of the window's title bar: dragging moves the window, a double click zooms it.
fn title_bar(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
}

/// The sidebar's pages, in the Electron app's order.
pub(crate) fn nav() -> [(Page, &'static str, Icon); 16] {
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
/// A row of the sidebar's navigation (`UNavigationMenu`'s link, `p-1.5`): 32px tall, a 20px icon
/// 6px from its label.
fn nav_item(label: &'static str, glyph: Icon) -> SidebarMenuItem {
    SidebarMenuItem::new(label)
        .icon(glyph.size(px(20.)))
        .min_h(px(32.))
        .p(px(6.))
        .gap_x(px(6.))
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

/// The page as a cached view fills its slot: the shell redrawing alone (the sidebar, the command
/// palette, a model only the shell shows) reuses the page's last frame. The page redraws when it
/// or a view inside it changes, when the shell forwards a shared model (`shared_changed`), or
/// when the windows refresh.
fn page_style() -> StyleRefinement {
    let mut style = StyleRefinement::default();
    style.size.width = Some(relative(1.).into());
    style.size.height = Some(relative(1.).into());
    style
}

/// The window's workspace, so a page can open another (`selectPage`).
struct Navigator(WeakEntity<Workspace>);

impl Global for Navigator {}

/// Z on a game page turns zen mode on or off (`toggleSetting('zenMode')`); the page calls it when
/// it has the keys itself, so typing a move never does.
pub fn zen_key(key: &str, cx: &mut App) -> bool {
    if key != "z" {
        return false;
    }
    crate::settings::change(cx, |settings| {
        settings.frontend.zen_mode = !settings.frontend.zen_mode
    });
    true
}

/// The page shown, once the window is open.
pub fn current_page(cx: &App) -> Option<Page> {
    let workspace = cx.try_global::<Navigator>()?.0.upgrade()?;
    Some(workspace.read(cx).pages.current())
}

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
    let online = crate::online::handle(cx);
    let (shown, account) = {
        let online = online.read(cx);
        (
            online.id() == id && online.phase == crate::online::Phase::Playing,
            crate::accounts::handle(cx)
                .read(cx)
                .active(cx)
                .unwrap_or_default(),
        )
    };
    // The game already on the board needs no new stream.
    if !shown {
        let id = id.to_string();
        online.update(cx, |online, cx| online.open(&account, &id, cx));
    }
    open(Page::Online, window, cx);
}

/// A tournament's page (`backToTournament`).
pub fn open_tournament(id: &str, window: &mut Window, cx: &mut App) {
    open(Page::Tournaments, window, cx);
    if let Some(page) = view_of::<crate::pages::tournaments::TournamentsPage>(Page::Tournaments, cx)
    {
        let id = id.to_string();
        page.update(cx, |page, cx| page.show_known_event(&id, cx));
    }
}

/// Play Stockfish at a level and side, from the palette (`startComputerGame`).
pub fn play_computer(level: &str, color: &'static str, window: &mut Window, cx: &mut App) {
    open(Page::Computer, window, cx);
    if let Some(page) = view_of::<ComputerPage>(Page::Computer, cx) {
        let level = level.to_string();
        page.update(cx, |page, cx| page.play_level(&level, color, window, cx));
    }
}

/// Look for a real-time game on Lichess, from the palette (`startOnlineGame`).
pub fn play_online(minutes: f64, increment: f64, rated: bool, window: &mut Window, cx: &mut App) {
    open(Page::Online, window, cx);
    let online = crate::online::handle(cx);
    if online.read(cx).busy_with_game() {
        crate::online::notice(cx, "Finish or cancel your game before starting another.");
        return;
    }
    online.update(cx, |online, cx| {
        let form = &mut online.form;
        form.minutes = minutes;
        form.increment = increment;
        form.rated = rated;
        form.color = "random".into();
        form.target.clear();
        form.correspondence = false;
        online.start(None, cx);
    });
}

/// An empty analysis board.
pub fn new_analysis(window: &mut Window, cx: &mut App) {
    replace_analysis(window, cx, |analysis, _, cx| {
        analysis.load(None, cx);
        true
    });
}

/// Whether the sidebar is shown.
pub fn sidebar_open(cx: &App) -> bool {
    cx.try_global::<Navigator>()
        .and_then(|navigator| navigator.0.upgrade())
        .is_none_or(|workspace| workspace.read(cx).sidebar_open)
}

pub fn toggle_sidebar(_: &mut Window, cx: &mut App) {
    if let Some(workspace) = cx.try_global::<Navigator>().and_then(|n| n.0.upgrade()) {
        workspace.update(cx, |workspace, cx| workspace.toggle_sidebar(cx));
    }
}

/// Challenge a player, rated or casual (`challengeFriend(username, rated)`).
pub fn challenge_rated(username: &str, rated: bool, window: &mut Window, cx: &mut App) {
    crate::online::handle(cx).update(cx, |online, cx| {
        online.form.rated = rated;
        cx.notify();
    });
    open(Page::Online, window, cx);
    if let Some(page) = view_of::<crate::pages::online::OnlinePage>(Page::Online, cx) {
        let username = username.to_string();
        page.update(cx, |page, cx| {
            page.challenge_keeping_rated(&username, window, cx)
        });
    }
}

/// Challenge a player to a casual game on Lichess (`challengeFriend`).
pub fn challenge(username: &str, window: &mut Window, cx: &mut App) {
    open(Page::Online, window, cx);
    if let Some(page) = view_of::<crate::pages::online::OnlinePage>(Page::Online, cx) {
        let username = username.to_string();
        page.update(cx, |page, cx| page.challenge(&username, window, cx));
    }
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
/// Redraw the shell, as its network, account and update observers do.
#[cfg(feature = "bench")]
pub fn notify_shell(cx: &mut App) {
    if let Some(workspace) = cx.try_global::<Navigator>().and_then(|nav| nav.0.upgrade()) {
        workspace.update(cx, |_, cx| cx.notify());
    }
}

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

/// The edge arrow while swiping: which way, how far, and whether letting go would go there.
#[derive(Clone, Copy)]
struct SwipeFeedback {
    direction: crate::swipe::Direction,
    progress: f32,
    ready: bool,
    blocked: bool,
}

pub struct Workspace {
    sidebar_open: bool,
    /// The width the sidebar's last open or close started from.
    sidebar_width_from: Pixels,
    history: crate::swipe::History<Page>,
    swipe: crate::swipe::Swipe,
    swipe_feedback: Option<SwipeFeedback>,
    /// How far the page follows the fingers.
    swipe_shift: f32,
    /// Going back or forward, which the history must not record as a visit.
    navigating: bool,
    palette: Entity<crate::palette::CommandPalette>,
    pages: Pages,
    /// Where Settings' Back returns to: the page open before it.
    back: Page,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Workspace {
        crate::theme::apply(window, cx);
        crate::interface_zoom::apply(window, cx);
        cx.set_global(Navigator(cx.entity().downgrade()));
        let settings = cx.global::<SettingsHandle>().0.clone();
        let subscriptions = vec![
            cx.observe(
                &cx.global::<crate::updates::UpdatesHandle>().0.clone(),
                |this, _, cx| this.shared_changed(cx),
            ),
            cx.observe(&crate::online::handle(cx), |this, _, cx| {
                this.shared_changed(cx)
            }),
            cx.observe(&crate::accounts::handle(cx), |this, _, cx| {
                this.shared_changed(cx)
            }),
            cx.observe(
                &cx.global::<crate::network::NetworkHandle>().0.clone(),
                |this, _, cx| this.shared_changed(cx),
            ),
            cx.observe(
                &cx.global::<crate::studies::StudyHandle>().0.clone(),
                |this, _, cx| this.shared_changed(cx),
            ),
            cx.observe(
                &cx.global::<crate::played::PlayedHandle>().0.clone(),
                |this, _, cx| this.shared_changed(cx),
            ),
            cx.observe(
                &cx.global::<crate::reviews::ReviewHandle>().0.clone(),
                |this, _, cx| this.shared_changed(cx),
            ),
            cx.observe(
                &cx.global::<crate::puzzle_db::PuzzleDbHandle>().0.clone(),
                |this, _, cx| this.shared_changed(cx),
            ),
            cx.observe(&crate::voice::handle(cx), |this, _, cx| {
                this.shared_changed(cx)
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                crate::theme::apply(window, cx);
                crate::dock_icon::apply(cx);
            }),
            // The window focused last raises system notifications (`docs/shared-core.md`).
            cx.observe_window_activation(window, |_, window, cx| {
                cx.global::<crate::core::AppCore>()
                    .bridge
                    .focus(window.is_window_active());
            }),
            cx.subscribe_in(
                &settings,
                window,
                |_, _, _: &SettingsChanged, window, cx| {
                    crate::theme::apply(window, cx);
                    // Views read settings while rendering without observing them (the board's
                    // theme and pieces); refreshing redraws cached views too.
                    window.refresh();
                },
            ),
            crate::window_state::track(crate::profile::data_dir().unwrap_or_default(), window, cx),
            // Auto follows the display: a move to another one may call for another size.
            cx.observe_window_bounds(window, |_, window, cx| {
                let wanted = f32::from(crate::interface_zoom::percent(window, cx)) / 100.;
                if (wanted - window.zoom()).abs() > f32::EPSILON {
                    crate::interface_zoom::apply(window, cx);
                }
            }),
        ];
        let sidebar_open = crate::prefs::get(cx, SIDEBAR_OPEN, true);
        Workspace {
            sidebar_open,
            sidebar_width_from: sidebar_width(sidebar_open),
            history: crate::swipe::History::default(),
            swipe: crate::swipe::Swipe::default(),
            swipe_feedback: None,
            swipe_shift: 0.,
            navigating: false,
            palette: crate::palette::init(window, cx),
            pages: Pages::new(window, cx),
            back: Page::Dashboard,
            _subscriptions: subscriptions,
        }
    }

    /// A model the shell and the pages read while rendering changed. The page is a cached view,
    /// which an entity it only reads does not redraw, so it is told as well.
    fn shared_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        let page = self.pages.view().entity_id();
        App::notify(cx, page);
    }

    /// Push to talk: Space held on the page speaks, while voice waits
    /// for it.
    fn talk_key(keystroke: &Keystroke, held: bool, cx: &mut Context<Self>) {
        if keystroke.key != "space" || keystroke.modifiers.modified() {
            return;
        }
        let voice = crate::voice::handle(cx);
        if voice.read(cx).can_talk() {
            voice.update(cx, |voice, cx| voice.talk(held, cx));
            cx.stop_propagation();
        }
    }

    fn choose(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.pages.current();
        if !self.navigating {
            self.history.visit(current, page);
        }
        // Voice belongs to the page it was turned on in; another page releases the microphone.
        if page != current {
            crate::voice::handle(cx).update(cx, |voice, cx| voice.release(cx));
        }
        if page == Page::Settings && current != Page::Settings {
            self.back = current;
        }
        // The microphone test runs only while its section is shown.
        if current == Page::Settings
            && page != Page::Settings
            && let Some(settings) = self.pages.settings()
        {
            settings.update(cx, |settings, cx| settings.stop_mic_test(cx));
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
        if page == Page::Online
            && current != Page::Online
            && let Some(online) = self
                .pages
                .view_of::<crate::pages::online::OnlinePage>(Page::Online)
        {
            online.update(cx, |online, cx| online.shown(cx));
        }
        if page == Page::Insights
            && current != Page::Insights
            && let Some(insights) = self
                .pages
                .view_of::<crate::pages::insights::InsightsPage>(Page::Insights)
        {
            insights.update(cx, |insights, cx| insights.shown(cx));
        }
        if page == Page::Puzzles
            && let Some(puzzles) = self.pages.view_of::<PuzzlesPage>(Page::Puzzles)
        {
            puzzles.update(cx, |puzzles, cx| puzzles.shown(window, cx));
        }
        if page == Page::Practice
            && let Some(practice) = self.pages.view_of::<PracticePage>(Page::Practice)
        {
            practice.update(cx, |practice, cx| {
                practice.shown();
                cx.notify();
            });
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

    /// The open page has a level to step out of going `direction` (`activeSwipeBackStep`): a
    /// game in Game history, a broadcast's game or tournament.
    fn can_step(&self, direction: crate::swipe::Direction, cx: &App) -> bool {
        match self.pages.current() {
            Page::History => self
                .pages
                .view_of::<HistoryPage>(Page::History)
                .is_some_and(|page| page.read(cx).can_step(direction, cx)),
            Page::Watch => self
                .pages
                .view_of::<WatchPage>(Page::Watch)
                .is_some_and(|page| page.read(cx).can_step(direction)),
            _ => false,
        }
    }

    /// Back (-1) or forward (1): out of or into the page's inner level first, then through the
    /// pages visited.
    fn go(
        &mut self,
        direction: crate::swipe::Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.can_step(direction, cx) {
            match self.pages.current() {
                Page::History => {
                    if let Some(page) = self.pages.view_of::<HistoryPage>(Page::History) {
                        page.update(cx, |page, cx| page.step(direction, window, cx));
                    }
                }
                Page::Watch => {
                    if let Some(page) = self.pages.view_of::<WatchPage>(Page::Watch) {
                        page.update(cx, |page, cx| page.step(direction, window, cx));
                    }
                }
                _ => {}
            }
            return;
        }
        let current = self.pages.current();
        if let Some(page) = self.history.go(direction, current) {
            self.navigating = true;
            self.choose(page, window, cx);
            self.navigating = false;
        }
    }

    /// A trackpad scroll: a horizontal two-finger swipe goes back or forward.
    fn scrolled(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let settings = &cx.global::<SettingsHandle>().0.read(cx).settings.frontend;
        let (enabled, feedback) = (settings.swipe_navigation, settings.swipe_indicator);
        // A mouse wheel scrolls in lines; only the trackpad's pixels can swipe.
        if !enabled || !event.delta.precise() {
            return;
        }
        let delta = event.delta.pixel_delta(px(16.));
        let can_go: Vec<bool> = [-1, 1]
            .into_iter()
            .map(|direction| self.can_step(direction, cx) || self.history.can_go(direction))
            .collect();
        let outcome = self.swipe.event(
            event.touch_phase,
            delta.x.as_f32(),
            delta.y.as_f32(),
            |direction| can_go[usize::from(direction > 0)],
        );
        match outcome {
            crate::swipe::Outcome::Nothing => return,
            crate::swipe::Outcome::Feedback {
                direction,
                progress,
                ready,
                blocked,
                shift,
            } => {
                if feedback {
                    self.swipe_feedback = Some(SwipeFeedback {
                        direction,
                        progress,
                        ready,
                        blocked,
                    });
                    self.swipe_shift = shift;
                }
            }
            crate::swipe::Outcome::Clear => {
                self.swipe_feedback = None;
                self.swipe_shift = 0.;
            }
            crate::swipe::Outcome::Go(direction) => {
                self.swipe_feedback = None;
                self.swipe_shift = 0.;
                self.go(direction, window, cx);
            }
        }
        cx.notify();
    }

    /// The arrow at the edge the swipe goes toward, sliding in as it nears the distance.
    fn swipe_edge(&self, cx: &App) -> Option<AnyElement> {
        let feedback = self.swipe_feedback?;
        let theme = cx.theme();
        let back = feedback.direction < 0;
        let size = 44.;
        let offset = -size + (size + 16.) * feedback.progress;
        Some(
            div()
                .absolute()
                .top(relative(0.5))
                .mt(px(-size / 2.))
                .when(back, |edge| edge.left(px(offset)))
                .when(!back, |edge| edge.right(px(offset)))
                .size(px(size))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if feedback.ready {
                    theme.primary
                } else {
                    theme.secondary
                })
                .text_color(if feedback.ready {
                    theme.primary_foreground
                } else {
                    theme.muted_foreground
                })
                .opacity(if feedback.blocked { 0.4 } else { 1. })
                .shadow_md()
                .child(
                    if back {
                        icon("chevron-left")
                    } else {
                        icon("chevron-right")
                    }
                    .size(px(24.)),
                )
                .into_any_element(),
        )
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
            // Electron's rows (`p-1.5`): 32px tall and touching, 20px icons 6px from the label.
            return vec![NavBlock::Menu(Box::new(
                SidebarMenu::new().gap_0().children(nav().into_iter().map(
                    |(page, label, glyph)| {
                        let marked = (page == Page::Online).then(|| game_mark(cx)).flatten();
                        nav_item(label, glyph)
                            .active(current == page)
                            .when_some(marked, |item, mark| {
                                item.suffix(move |_, cx| {
                                    let theme = cx.theme();
                                    let color = if mark == GameMark::Turn {
                                        theme.primary
                                    } else {
                                        theme.muted_foreground
                                    };
                                    div().size(px(8.)).rounded_full().bg(color).when(
                                        mark == GameMark::Turn,
                                        |dot| {
                                            dot.shadow(vec![BoxShadow {
                                                color: color.opacity(0.25),
                                                offset: point(px(0.), px(0.)),
                                                blur_radius: px(0.),
                                                spread_radius: px(3.),
                                                inset: false,
                                            }])
                                        },
                                    )
                                })
                            })
                            .on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.choose(page, window, cx)
                                }),
                            )
                    },
                )),
            ))];
        }
        let shown = self
            .pages
            .settings()
            .map_or(Section::Appearance, |page| page.read(cx).section());
        let back = self.back;
        vec![
            NavBlock::Menu(Box::new(SidebarMenu::new().child(
                nav_item("Back", icon("arrow-left")).on_click(
                    cx.listener(move |this, _, window, cx| this.choose(back, window, cx)),
                ),
            ))),
            NavBlock::Heading("Settings", false),
            NavBlock::Menu(Box::new(SidebarMenu::new().gap_0().children(
                sections().into_iter().map(|(section, label, glyph)| {
                    nav_item(label, glyph).active(shown == section).on_click(
                        cx.listener(move |this, _, _, cx| this.choose_section(section, cx)),
                    )
                }),
            ))),
        ]
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_width_from = sidebar_width(self.sidebar_open);
        self.sidebar_open = !self.sidebar_open;
        crate::prefs::set(cx, SIDEBAR_OPEN, self.sidebar_open);
        cx.notify();
    }

    /// `USidebar` with `collapsible="icon"`: the header row the toggle floats over, the pages,
    /// and the footer, divided by lines (`divide-y`); 256px open, a 64px rail of icons collapsed,
    /// its width easing between the two in 200ms.
    fn render_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.sidebar_open;
        let width = sidebar_width(open);
        let border = cx.theme().sidebar_border;
        // macOS keeps the rail's edge out of the header row, where it would cut through the
        // traffic lights (`.is-mac .kchess-sidebar-container::after`).
        let mac = cfg!(target_os = "macos");
        let blocks: Vec<AnyElement> = self
            .nav_blocks(cx)
            .into_iter()
            .enumerate()
            .map(|(ix, block)| {
                block
                    .collapsed(!open)
                    .render(ix, window, cx)
                    .into_any_element()
            })
            .collect();
        let sidebar = v_flex()
            .w(width)
            .h_full()
            .bg(cx.theme().tokens.sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_color(border)
            .when(!mac, |sidebar| sidebar.border_r_1())
            .child(
                title_bar("sidebar-header")
                    .h(TOPBAR_HEIGHT)
                    .flex_none()
                    .border_b_1()
                    .border_color(border),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .border_color(border)
                    .when(mac, |rest| rest.border_r_1())
                    .child(
                        v_flex()
                            .id("sidebar-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_4()
                            .children(blocks),
                    )
                    .child(
                        v_flex()
                            .flex_none()
                            .p_4()
                            .gap_1()
                            .border_t_1()
                            .border_color(border)
                            .children(dev_build_badge(open, cx))
                            .child(
                                footer_button(
                                    "settings",
                                    icon("settings-2"),
                                    "Settings".into(),
                                    open,
                                    cx,
                                )
                                .accessibility_label("Settings")
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.choose(Page::Settings, window, cx),
                                )),
                            )
                            .child(account_button(open, cx)),
                    ),
            );
        let frame = div()
            .id("sidebar")
            .h_full()
            .flex_none()
            .overflow_hidden()
            .child(sidebar);
        let from = self.sidebar_width_from;
        if from == width {
            return frame.w(width).into_any_element();
        }
        EffectTransition::new(Duration::from_millis(200))
            .ease(ease_out_cubic)
            .width(from, width)
            .apply(
                frame,
                SharedString::from(format!("sidebar-{}-{}", f32::from(from), f32::from(width))),
            )
            .into_any_element()
    }

    /// The sidebar toggle (`.sidebar-toggle-wrap`): one button at the same place in the window
    /// whether the sidebar is open or a rail, beside the macOS traffic lights (which ignore the
    /// interface size, so its offset does too), else at the window's edge.
    fn render_sidebar_toggle(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = if self.sidebar_open {
            "Hide sidebar"
        } else {
            "Show sidebar"
        };
        h_flex()
            .id("sidebar-toggle-wrap")
            .absolute()
            .top_0()
            .left(toggle_left(window))
            .h(TOPBAR_HEIGHT)
            .child(
                Button::new("sidebar-toggle")
                    .custom(ghost_neutral(cx))
                    .icon(icon("panel-left"))
                    .size(px(28.))
                    .tooltip(label)
                    .accessibility_label(label)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
    }

    /// The top bar inside GPUI Kit's title bar, which ends it with the window's minimize,
    /// maximize and close buttons on Windows, and on Linux when the window draws its own
    /// decorations; macOS keeps its traffic lights.
    fn render_topbar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new()
            .h(TOPBAR_HEIGHT)
            .pl_0()
            .bg(cx.theme().background)
            .border_color(cx.theme().border)
            .on_close_window(|_, window, cx| crate::menu::close_window(window.window_handle(), cx))
            .child(self.render_topbar_content(window, cx))
    }

    fn render_topbar_content(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Room for the toggle floating over the topbar beside the macOS rail
        // (`.mac-toggle-spacer`): its 28px and 8px clear of it, less the rail and the padding.
        let spacer = (toggle_left(window) + px(36.) - SIDEBAR_RAIL - px(20.)).max(px(0.));
        title_bar("topbar")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h_full()
            // `.topbar`: `minmax(40px, 1fr) minmax(0, 600px) minmax(40px, 1fr)`, the search
            // centred between equal sides; 12px padding below 560px.
            .px(if window.viewport_size().width < px(560.) {
                px(12.)
            } else {
                px(20.)
            })
            .gap(px(8.))
            .child(
                h_flex()
                    .flex_1()
                    .flex_basis(px(0.))
                    .min_w(px(40.))
                    .when(cfg!(target_os = "macos") && !self.sidebar_open, |left| {
                        left.child(div().flex_none().w(spacer))
                    }),
            )
            .child(
                div()
                    .flex_basis(px(600.))
                    .flex_shrink_1()
                    .min_w_0()
                    .child(self.palette.clone()),
            )
            .child(self.render_topbar_right(cx))
    }

    /// `.topbar-right`: connection, update and work state, at the end.
    fn render_topbar_right(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_1()
            .flex_basis(px(0.))
            .min_w(px(40.))
            .justify_end()
            .gap(px(8.))
            .when(!crate::network::online(cx), |bar| {
                bar.child(
                    h_flex()
                        .id("offline")
                        .gap(px(4.))
                        .text_size(px(14.))
                        .text_color(cx.theme().muted_foreground)
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Local play, analysis and downloaded content are available.",
                            )
                            .build(window, cx)
                        })
                        .child(icon("wifi-off").size(px(16.)))
                        .child("Offline"),
                )
            })
            .children(update_button(cx))
            .when(working(cx), |bar| {
                bar.child(
                    h_flex()
                        .gap(px(6.))
                        .text_size(px(14.))
                        .text_color(cx.theme().muted_foreground)
                        .child(icon("loader-circle").size(px(16.)))
                        .child("Working…"),
                )
            })
    }
}

/// Something is waiting on Lichess (`busy`): signing in, syncing, starting a game.
fn working(cx: &App) -> bool {
    crate::accounts::handle(cx).read(cx).busy || crate::online::handle(cx).read(cx).busy
}

/// The Lichess game the sidebar marks (`lichessGame`): your move (in a live game, or in a
/// correspondence one), or a game going on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GameMark {
    Turn,
    Live,
}

fn game_mark(cx: &App) -> Option<GameMark> {
    let online = crate::online::handle(cx).read(cx);
    let live = online.phase == crate::online::Phase::Playing;
    let my_turn = live && {
        let moves = online.moves();
        let setup = online.setup();
        crate::game::view::turn(
            crate::game::view::position_at(&setup, &moves, moves.len()).as_ref(),
        ) == online.color()
    };
    let correspondence_turn = online.ongoing.iter().any(|game| {
        game["speed"] == serde_json::json!("correspondence")
            && game["isMyTurn"] == serde_json::json!(true)
    });
    if my_turn || correspondence_turn {
        Some(GameMark::Turn)
    } else if live {
        Some(GameMark::Live)
    } else {
        None
    }
}

/// The Electron app's neutral ghost button: the text color, the accented
/// background on hover and while pressed.
fn ghost_neutral(cx: &App) -> ButtonCustomVariant {
    let palette = cx.global::<Palette>();
    ButtonCustomVariant::new(cx)
        .foreground(palette.text)
        .hover(palette.accented)
        .active(palette.accented)
}

/// A footer button as Electron draws it (`UButton` ghost, `block`, `justify-start`): 32px tall,
/// a 20px icon with the label beside it while the sidebar is open. On the rail the icon stands
/// alone, 6px in (`square`), with the label as its tooltip.
fn footer_button(
    id: &'static str,
    glyph: Icon,
    label: SharedString,
    open: bool,
    cx: &App,
) -> Button {
    Button::new(id)
        .custom(ghost_neutral(cx))
        .w_full()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .child(glyph.size(px(20.)).flex_none())
                .when(open, |row| row.child(label.clone())),
        )
        .when(!open, |button| button.px(px(6.)).tooltip(label))
}

/// Marks a debug build (`cargo run -p kchess-gpui`) above Settings, as the Electron app marked
/// its dev builds. Release builds compile none of it; packaging checks the binary for
/// its id.
#[cfg(debug_assertions)]
fn dev_build_badge(open: bool, cx: &App) -> Option<AnyElement> {
    const LABEL: &str = "Development build · GPUI";
    let warning = cx.global::<Palette>().warning;
    Some(
        h_flex()
            .id("kchess-dev-build-badge")
            .h(px(28.))
            .gap(px(6.))
            .rounded(px(6.))
            .overflow_hidden()
            .border_1()
            .border_color(warning.opacity(0.3))
            .bg(warning.opacity(0.12))
            .text_color(warning)
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .map(|badge| {
                if open {
                    badge.px(px(8.))
                } else {
                    badge.justify_center().tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(LABEL).build(window, cx)
                    })
                }
            })
            .child(icon("moon").size(px(16.)).flex_none())
            .when(open, |badge| badge.child("Dev · GPUI"))
            .into_any_element(),
    )
}

#[cfg(not(debug_assertions))]
fn dev_build_badge(_open: bool, _cx: &App) -> Option<AnyElement> {
    None
}

/// The sidebar's account button: Connect Lichess while no account plays online, else the account
/// menu (`userMenuItems`): which account plays, connecting another, Settings and logging out.
fn account_button(expanded: bool, cx: &App) -> AnyElement {
    let handle = crate::accounts::handle(cx);
    let accounts = handle.read(cx);
    let Some(active) = accounts.active(cx) else {
        let target = handle.downgrade();
        return footer_button(
            "connect-lichess",
            icon("log-in"),
            "Connect Lichess".into(),
            expanded,
            cx,
        )
        .accessibility_label("Connect Lichess")
        .disabled(!crate::network::online(cx) || accounts.busy)
        .on_click(move |_, window, cx| {
            let _ = target.update(cx, |accounts, cx| accounts.connect(window, cx));
        })
        .into_any_element();
    };
    let connected = accounts.connected();
    let label = format!("@{active}");
    let target = handle.downgrade();
    // `square`: 6px all round, the label between the icon and a dimmed up-down chevron; elevated
    // while its menu is open.
    let palette = cx.global::<Palette>();
    let dimmed = palette.dimmed;
    Button::new("account-menu")
        .custom(ghost_neutral(cx).active(palette.elevated))
        .w_full()
        .px(px(6.))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .child(icon("user").size(px(20.)).flex_none())
                .when(expanded, |row| {
                    row.child(div().flex_1().min_w_0().child(label.clone()))
                        .child(
                            icon("chevrons-up-down")
                                .size(px(20.))
                                .flex_none()
                                .text_color(dimmed),
                        )
                }),
        )
        .accessibility_label(if expanded {
            format!("{label} account menu")
        } else {
            "Account menu".into()
        })
        // Above the button, centred on it and as wide (at least 192px), 8px clear.
        .dropdown_menu_with_anchor(Anchor::BottomCenter, move |menu, _, _| {
            let menu = menu.min_w(if expanded {
                SIDEBAR_WIDTH - px(33.)
            } else {
                px(192.)
            });
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

/// A newer KChess, its download, or the restart that installs it.
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .relative()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when(!zen, |shell| shell.child(self.render_sidebar(window, cx)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_topbar(window, cx))
                    .child(
                        div()
                            .id("page-area")
                            .flex_1()
                            .min_h_0()
                            .relative()
                            .overflow_hidden()
                            .on_scroll_wheel(cx.listener(Self::scrolled))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if !event.is_held && this.pages.page_focused(window) {
                                    Self::talk_key(&event.keystroke, true, cx);
                                }
                            }))
                            .on_key_up(cx.listener(|_, event: &KeyUpEvent, _, cx| {
                                Self::talk_key(&event.keystroke, false, cx);
                            }))
                            .child(
                                div()
                                    .size_full()
                                    .relative()
                                    .left(px(self.swipe_shift))
                                    .child(self.pages.view().cached(page_style())),
                            )
                            .children(self.swipe_edge(cx)),
                    ),
            )
            .when(!zen, |shell| {
                shell.child(self.render_sidebar_toggle(window, cx))
            })
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

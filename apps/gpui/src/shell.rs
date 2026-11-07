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
    ActiveTheme as _, Collapsible, Disableable as _, Icon, Selectable as _, Sizable as _, TitleBar,
    WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;

use crate::assets::icon;
use crate::part::Part;

use crate::pages::analysis::AnalysisPage;
use crate::pages::computer::ComputerPage;
use crate::pages::editor::EditorPage;
use crate::pages::history::HistoryPage;
use crate::pages::home::HomePage;
use crate::pages::local::LocalPage;
use crate::pages::openings::OpeningsPage;
use crate::pages::practice::PracticePage;
use crate::pages::puzzles::PuzzlesPage;
use crate::pages::settings::{SECTION_GROUPS, Section, sections};
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
/// How long the sidebar's width eases when it opens or closes.
const SIDEBAR_EASE: Duration = Duration::from_millis(200);
const SIDEBAR_RAIL: Pixels = px(64.);
/// Below this window width the sidebar shows as its rail, leaving the page the room; its toggle
/// opens it there for the moment without changing what wider windows show.
const NARROW_WINDOW: Pixels = px(800.);

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

/// A control on a title bar, which takes its own press: on Windows a press on the drag area under
/// it would move the window, and the control would never hear the click.
fn over_title_bar(control: impl IntoElement) -> Div {
    div().occlude().child(control)
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
        (Page::Computer, "Play with computer", icon("bot")),
        (Page::Local, "Over the board", icon("swords")),
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
        (Page::Insights, "Insights", icon("chart-pie")),
        (Page::Friends, "Following", icon("users")),
        (Page::Players, "Players", icon("user-search")),
    ]
}

/// How many of `nav`'s pages each of the sidebar's groups holds: home, playing, training, your
/// own games and boards, Lichess.
pub(crate) const NAV_GROUPS: [usize; 5] = [1, 2, 3, 4, 6];

/// The heading over each of `NAV_GROUPS` in the open sidebar; Home stands alone.
const NAV_HEADINGS: [Option<&str>; 5] = [
    None,
    Some("Play"),
    Some("Train"),
    Some("Review"),
    Some("Lichess"),
];

/// A page's name under its group's heading, where the heading already says it: "Computer" under
/// Play, "Play" under Lichess. The rail's tooltips, the palette and screen readers keep the full name.
fn nav_label(page: Page, label: &'static str) -> &'static str {
    match page {
        Page::Computer => "Computer",
        Page::Online => "Play",
        Page::Insights => "Insights",
        _ => label,
    }
}

#[cfg(test)]
mod nav_tests {
    use super::{NAV_GROUPS, nav, nav_label};

    /// Shortened under their headings, the open sidebar's rows still name one page each.
    #[test]
    fn open_rows_name_one_page_each() {
        let mut shown: Vec<&str> = nav()
            .into_iter()
            .map(|(page, label, _)| nav_label(page, label))
            .collect();
        shown.sort_unstable();
        let count = shown.len();
        shown.dedup();
        assert_eq!(shown.len(), count, "two rows read the same");
        assert_eq!(NAV_GROUPS.iter().sum::<usize>(), nav().len());
    }
}

/// A row of the sidebar's navigation: 32px tall, an 18px icon 8px from its label.
fn nav_item(label: impl Into<SharedString>, glyph: Icon) -> SidebarMenuItem {
    SidebarMenuItem::new(label)
        .icon(glyph.size(px(18.)))
        .min_h(px(32.))
        .p(px(6.))
        .gap_x(px(8.))
}

/// What the sidebar lists: menus, each under its group's heading (on the rail, a short rule
/// instead), and while Settings is open its categories in headed groups.
#[derive(Clone)]
enum NavBlock {
    /// A group: its heading, its rows, and which of its rows carries the Lichess game's mark.
    Menu(
        Option<&'static str>,
        Box<SidebarMenu>,
        Option<(usize, GameMark)>,
    ),
    Group(&'static str, Box<SidebarMenu>),
}

impl Collapsible for NavBlock {
    fn is_collapsed(&self) -> bool {
        match self {
            NavBlock::Menu(_, menu, _) | NavBlock::Group(_, menu) => menu.is_collapsed(),
        }
    }

    fn collapsed(self, collapsed: bool) -> Self {
        match self {
            NavBlock::Menu(heading, menu, mark) => {
                NavBlock::Menu(heading, Box::new(menu.collapsed(collapsed)), mark)
            }
            NavBlock::Group(heading, menu) => {
                NavBlock::Group(heading, Box::new(menu.collapsed(collapsed)))
            }
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
            NavBlock::Menu(None, menu, _) => menu.render(id, window, cx).into_any_element(),
            NavBlock::Menu(Some(heading), menu, mark) => {
                let collapsed = menu.is_collapsed();
                let palette = cx.global::<Palette>();
                let (dimmed, border) = (palette.dimmed, palette.border);
                v_flex()
                    // Midway between the groups: the row above ends 7px below its icon and the
                    // sidebar's 4px gap follows, so the rule keeps 4px to the rows it heads.
                    .when(collapsed, |group| {
                        group.child(div().h(px(1.)).w(px(16.)).mx_auto().mb(px(4.)).bg(border))
                    })
                    .when(!collapsed, |group| {
                        group.child(
                            div()
                                .px(px(6.))
                                .text_size(px(12.))
                                .line_height(px(20.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(dimmed)
                                .child(heading),
                        )
                    })
                    .child(menu.render(id, window, cx))
                    // On the rail the row has no room beside its icon: the mark sits on the
                    // icon's top right corner, cut out of it by a ring of the sidebar's color,
                    // below the rule and 32px down per row before it.
                    .when_some(mark.filter(|_| collapsed), |group, (row, mark)| {
                        group.relative().child(
                            div()
                                .id("sidebar-game-mark")
                                .test_support()
                                .absolute()
                                .top(px(5. + 32. * row as f32 + 3.))
                                .left(px(18.))
                                .size(px(12.))
                                .rounded_full()
                                .border_2()
                                .border_color(cx.theme().tokens.sidebar)
                                .bg(game_color(mark, cx)),
                        )
                    })
                    .into_any_element()
            }
            // The heading on the rows' icon column, dimmed under the rows' labels; on the rail,
            // where there is no room for it, a short rule.
            NavBlock::Group(heading, menu) => {
                let palette = cx.global::<Palette>();
                let (dimmed, border) = (palette.dimmed, palette.border);
                // Named for its heading, which a screen reader announces entering it.
                v_flex()
                    .id(SharedString::from(format!("group-{heading}")))
                    .test_support()
                    .role(Role::Group)
                    .aria_label(heading)
                    .child(if menu.is_collapsed() {
                        div().h(px(1.)).w(px(16.)).mx_auto().my(px(4.)).bg(border)
                    } else {
                        div()
                            .px(px(6.))
                            .text_size(px(12.))
                            .line_height(px(24.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(dimmed)
                            .child(heading)
                    })
                    .child(menu.render(id, window, cx))
                    .into_any_element()
            }
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

/// What the sidebar shows (`render_sidebar`): it is drawn again when one of these changes.
#[derive(PartialEq)]
struct SidebarInputs {
    open: bool,
    width_from: Pixels,
    current: Page,
    section: Option<Section>,
    back: Page,
    game_mark: Option<GameMark>,
    online: bool,
    accounts_busy: bool,
    active: Option<String>,
    connected: Vec<String>,
}

impl SidebarInputs {
    fn of(workspace: &Workspace, cx: &App) -> SidebarInputs {
        let accounts = crate::accounts::handle(cx).read(cx);
        SidebarInputs {
            open: workspace.shown(),
            width_from: workspace.sidebar_width_from,
            current: workspace.pages.current(),
            section: workspace
                .pages
                .settings()
                .map(|page| page.read(cx).section()),
            back: workspace.back,
            game_mark: game_mark(cx),
            online: crate::network::online(cx),
            accounts_busy: accounts.busy,
            active: accounts.active(cx),
            connected: accounts.connected(),
        }
    }
}

/// What the top bar shows besides the command palette, a view of its own
/// (`render_topbar_content`): it is drawn again when one of these changes.
#[derive(PartialEq)]
struct TopbarInputs {
    sidebar_open: bool,
    online: bool,
    working: bool,
    update: UpdateInputs,
}

/// What the update button shows (`update_button`).
#[derive(PartialEq)]
struct UpdateInputs {
    phase: crate::updates::Phase,
    /// A real-time game is on: the button waits until it ends.
    live_game: bool,
    version: Option<String>,
    current: String,
    progress: Option<(u64, u64)>,
    error: Option<String>,
}

impl TopbarInputs {
    fn of(workspace: &Workspace, cx: &App) -> TopbarInputs {
        let updates = cx.global::<crate::updates::UpdatesHandle>().0.read(cx);
        TopbarInputs {
            sidebar_open: workspace.shown(),
            online: crate::network::online(cx),
            working: working(cx),
            update: UpdateInputs {
                phase: updates.phase,
                live_game: crate::online::handle(cx).read(cx).live_game_on(),
                version: updates.version.clone(),
                current: updates.current.clone(),
                progress: updates.progress,
                error: updates.error.clone(),
            },
        }
    }
}

/// The sidebar toggle as its `Part` (`.sidebar-toggle-wrap`): its 28px button, `TOPBAR_HEIGHT`
/// high, over the top bar's left end, and the product's name beside it while the sidebar is open.
fn toggle_style(open: bool, window: &Window) -> StyleRefinement {
    let mut style = StyleRefinement {
        position: Some(Position::Absolute),
        ..Default::default()
    };
    style.inset.top = Some(px(0.).into());
    style.inset.left = Some(toggle_left(window).into());
    style.size.width = Some(px(if open { 100. } else { 28. }).into());
    style.size.height = Some(TOPBAR_HEIGHT.into());
    style
}

/// The top bar as its `Part`: the window's width, `TOPBAR_HEIGHT` high.
fn topbar_style() -> StyleRefinement {
    let mut style = StyleRefinement::default();
    style.size.width = Some(relative(1.).into());
    style.size.height = Some(TOPBAR_HEIGHT.into());
    style.flex_shrink = Some(0.);
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

/// The window's title as the workspace last set it.
#[cfg(test)]
pub fn window_title_shown(cx: &App) -> Option<String> {
    let workspace = cx.try_global::<Navigator>()?.0.upgrade()?;
    Some(workspace.read(cx).title.clone())
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

/// Back (-1) or forward (1): out of the page's inner level, then through the pages visited, as
/// a swipe goes.
pub fn go(direction: crate::swipe::Direction, window: &mut Window, cx: &mut App) {
    let Some(workspace) = cx
        .try_global::<Navigator>()
        .map(|navigator| navigator.0.clone())
    else {
        return;
    };
    let _ = workspace.update(cx, |workspace, cx| workspace.go(direction, window, cx));
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

/// Watch a Lichess TV channel.
pub fn watch_channel(channel: String, window: &mut Window, cx: &mut App) {
    open(Page::Watch, window, cx);
    if let Some(watch) = view_of::<crate::pages::watch::WatchPage>(Page::Watch, cx) {
        watch.update(cx, |watch, cx| watch.open_channel(channel, window, cx));
    }
}

/// Watch at one of its tabs (`tv`, `friends` or `broadcasts`); a Watch page made now opens there.
pub fn watch_tab(tab: &'static str, window: &mut Window, cx: &mut App) {
    crate::prefs::set(cx, crate::pages::watch::TAB_KEY, tab);
    open(Page::Watch, window, cx);
    if let Some(watch) = view_of::<crate::pages::watch::WatchPage>(Page::Watch, cx) {
        watch.update(cx, |watch, cx| watch.show_tab(tab, window, cx));
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

/// A tournament named by its system (`arena` or `swiss`) and id, as a pasted link names it.
pub fn open_event(system: &str, id: &str, window: &mut Window, cx: &mut App) {
    open(Page::Tournaments, window, cx);
    if let Some(page) = view_of::<crate::pages::tournaments::TournamentsPage>(Page::Tournaments, cx)
    {
        page.update(cx, |page, cx| page.show_event(system, id, cx));
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
        crate::online::notice(cx, "Finish or cancel your game before starting another");
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
        .is_none_or(|workspace| workspace.read(cx).shown())
}

/// Show or hide the sidebar from its shortcut, the menu or the command palette: at once, since a
/// command repeated many times a day should not wait on an animation.
pub fn toggle_sidebar(_: &mut Window, cx: &mut App) {
    if let Some(workspace) = cx.try_global::<Navigator>().and_then(|n| n.0.upgrade()) {
        workspace.update(cx, |workspace, cx| workspace.toggle_sidebar(false, cx));
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

/// Studies at its Lichess tab: the connected account's own studies.
pub fn open_studies_lichess(window: &mut Window, cx: &mut App) {
    open(Page::Studies, window, cx);
    if let Some(studies) = view_of::<crate::pages::studies::StudiesPage>(Page::Studies, cx) {
        studies.update(cx, |studies, cx| studies.show_lichess(window, cx));
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

/// Redraw the shell, as its network, account and update observers do.
#[cfg(any(test, feature = "bench"))]
pub fn notify_shell(cx: &mut App) {
    if let Some(workspace) = cx.try_global::<Navigator>().and_then(|nav| nav.0.upgrade()) {
        workspace.update(cx, |_, cx| cx.notify());
    }
}

/// A page's view, once it has been shown.
pub fn view_of<V: 'static>(page: Page, cx: &App) -> Option<Entity<V>> {
    let workspace = cx.try_global::<Navigator>()?.0.upgrade()?;
    workspace.read(cx).pages.view_of::<V>(page)
}

/// Find: the page's own search field, focused with its text selected, when the page shows one
/// (History's, Studies', Openings', Watch's broadcasts', Players' username). It is `false`
/// on a page without one, for Quick Pick to search instead.
pub fn find_on_page(window: &mut Window, cx: &mut App) -> bool {
    use crate::pages::{history, openings, players, studies, watch};
    use gpui_kit::component::input::InputState;
    fn fields_of<V: Focusable>(
        page: Page,
        fields: fn(&V) -> Vec<Entity<InputState>>,
        cx: &App,
    ) -> Option<(FocusHandle, Vec<Entity<InputState>>)> {
        let view = view_of::<V>(page, cx)?;
        let view = view.read(cx);
        Some((view.focus_handle(cx), fields(view)))
    }
    let found = match current_page(cx) {
        Some(Page::History) => fields_of(Page::History, history::HistoryPage::search_fields, cx),
        Some(Page::Studies) => fields_of(Page::Studies, studies::StudiesPage::search_fields, cx),
        Some(Page::Openings) => {
            fields_of(Page::Openings, openings::OpeningsPage::search_fields, cx)
        }
        Some(Page::Watch) => fields_of(Page::Watch, watch::WatchPage::search_fields, cx),
        Some(Page::Players) => fields_of(Page::Players, players::PlayersPage::search_fields, cx),
        _ => None,
    };
    let Some((page, fields)) = found else {
        return false;
    };
    // Only a field the page draws now: History's other tab, or Watch away from Broadcasts,
    // has its field but does not show it.
    let Some(field) = fields
        .into_iter()
        .find(|field| page.contains(&field.read(cx).focus_handle(cx), window))
    else {
        return false;
    };
    field.update(cx, |field, cx| field.focus(window, cx));
    window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    true
}

/// Open a Lichess broadcast link on Watch → Broadcasts.
pub fn open_broadcast(link: crate::game::BroadcastLink, window: &mut Window, cx: &mut App) {
    open(Page::Watch, window, cx);
    if let Some(watch) = view_of::<crate::pages::watch::WatchPage>(Page::Watch, cx) {
        watch.update(cx, |watch, cx| watch.open_broadcast(link, window, cx));
    }
}

/// Open a Lichess puzzle by its id on Puzzles → Daily.
pub fn open_puzzle(id: String, window: &mut Window, cx: &mut App) {
    open(Page::Puzzles, window, cx);
    if let Some(puzzles) = view_of::<crate::pages::puzzles::PuzzlesPage>(Page::Puzzles, cx) {
        puzzles.update(cx, |puzzles, cx| puzzles.open_puzzle(id, window, cx));
    }
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

/// Whether a game can start from `setup` (`{ variant, fen }`): a standard position the engine
/// takes (not one with too many pieces), or a legal one in its variant.
fn playable(setup: &serde_json::Value) -> bool {
    let variant = setup["variant"].as_str().unwrap_or("standard");
    let fen = setup["fen"].as_str().unwrap_or_default();
    if crate::game::standard_rules(variant) {
        crate::game::position_problem(fen).is_none()
    } else {
        kchess_domain::rules::setup_start(variant, fen).is_some()
    }
}

/// Play Stockfish (Fairy-Stockfish in another variant) from a setup's position, as the side to
/// move (`startComputerFrom`).
pub fn play_computer_from(setup: serde_json::Value, window: &mut Window, cx: &mut App) {
    let color = if setup["fen"].as_str().and_then(|fen| fen.split(' ').nth(1)) == Some("b") {
        shakmaty::Color::Black
    } else {
        shakmaty::Color::White
    };
    open(Page::Computer, window, cx);
    // A position that cannot be played only opens the page.
    if !playable(&setup) {
        return;
    }
    if let Some(page) = view_of::<ComputerPage>(Page::Computer, cx) {
        page.update(cx, |page, cx| page.start_from(setup, color, window, cx));
    }
}

/// A game over the board from a setup's position (`/local?fen=`).
pub fn play_local_from(setup: serde_json::Value, window: &mut Window, cx: &mut App) {
    open(Page::Local, window, cx);
    // As the page does with `?fen=`: a position that cannot be played only opens it.
    if !playable(&setup) {
        return;
    }
    if let Some(page) = view_of::<LocalPage>(Page::Local, cx) {
        page.update(cx, |page, cx| page.start_from(setup, window, cx));
    }
}

/// The board editor on a position (`editPosition`): a setup's (`{ variant, fen }`).
pub fn edit_position(
    setup: serde_json::Value,
    orientation: shakmaty::Color,
    window: &mut Window,
    cx: &mut App,
) {
    let variant = setup["variant"].as_str().unwrap_or("standard");
    let fen = setup["fen"].as_str().unwrap_or_default();
    if let Some(editor) = page_view::<EditorPage>(Page::Editor, window, cx) {
        editor.update(cx, |editor, cx| {
            editor.set_position(variant, fen, orientation, window, cx)
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
    crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
        let (load, run) = (load.clone(), run.clone());
        dialog
            .title("Replace the unsaved analysis?")
            .description("The analysis board has moves that are not saved as a study")
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

/// Where a dragged file goes, over the pages: hidden until a file from outside the app is over it,
/// so the drop has somewhere to land before it is let go. It takes no clicks or scrolls; the page
/// area underneath opens the file.
fn drop_zone(palette: &Palette) -> impl IntoElement {
    div()
        .id("drop-zone")
        .test_support()
        .absolute()
        .inset_0()
        .p(px(8.))
        .flex()
        .bg(palette.bg)
        .child(
            v_flex()
                .flex_1()
                .rounded(crate::theme::radii::XL)
                .border_2()
                .border_dashed()
                .border_color(palette.primary)
                .bg(crate::theme::tint(palette.primary, 4.))
                .items_center()
                .justify_center()
                .gap(px(8.))
                .text_color(palette.primary)
                .child(icon("download").size(px(28.)))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Open"),
                ),
        )
        .invisible()
        .drag_over::<ExternalPaths>(|style, _, _, _| style.visible())
}

/// The largest file a drop reads: the import's own limit for a PGN.
const DROPPED_MAX: u64 = 2 * 1024 * 1024;

/// A PGN or FEN file dropped on the window, from any page: one game or a position opens on the
/// analysis board, as if pasted into its import; several games are offered as a study, in the
/// Studies import; anything else is refused with a notice.
pub fn open_dropped(paths: &ExternalPaths, window: &mut Window, cx: &mut App) {
    let Some(path) = paths.paths().first() else {
        return;
    };
    let Some(text) = read_game_file(path) else {
        window.push_notification(
            gpui_kit::component::notification::Notification::error(
                "Drop a PGN or FEN file under 2 MB",
            )
            .title("Could not open that file"),
            cx,
        );
        return;
    };
    let games = crate::game::rules("pgnGames", &[serde_json::json!(text)])
        .ok()
        .and_then(|games| games.as_array().map(Vec::len))
        .unwrap_or(0);
    if games > 1 {
        open(Page::Studies, window, cx);
        StudiesPage::open_import_with(Some(text), window, cx);
        return;
    }
    replace_analysis(window, cx, move |analysis, window, cx| {
        analysis.import_text(text.clone(), window, cx);
        true
    });
}

/// A PGN or FEN file's text (`.pgn`, `.fen` or `.txt`, up to 2 MB), or `None`.
pub fn read_game_file(path: &std::path::Path) -> Option<String> {
    let readable = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["pgn", "fen", "txt"].contains(&extension.to_ascii_lowercase().as_str())
        });
    if !readable {
        return None;
    }
    // The opened file itself must be a regular file (a symlink to `/dev/zero` or a FIFO named
    // `.pgn` reports a length of 0), and the read stops past the limit whatever it reported.
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > DROPPED_MAX {
        return None;
    }
    let mut text = String::new();
    file.take(DROPPED_MAX + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= DROPPED_MAX).then_some(text)
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

/// Guess the move through a game's PGN as `side`, on Practice.
pub fn guess_moves(pgn: &str, side: Option<shakmaty::Color>, window: &mut Window, cx: &mut App) {
    crate::prefs::set(cx, "kchess:practice-tab", "guess");
    open(Page::Practice, window, cx);
    if let Some(practice) = view_of::<crate::pages::practice::PracticePage>(Page::Practice, cx) {
        practice.update(cx, |practice, cx| {
            practice.guess_moves(pgn, side, window, cx)
        });
    }
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
    /// Whether the sidebar is open in a window wider than `NARROW_WINDOW`, as kept.
    sidebar_open: bool,
    /// Whether the window is narrower than `NARROW_WINDOW`.
    narrow: bool,
    /// Whether the sidebar was opened while the window is narrow.
    narrow_open: bool,
    /// The width the sidebar's last open or close started from.
    sidebar_width_from: Pixels,
    /// When the sidebar last opened or closed: it is drawn in the shell, not as its `Part`,
    /// while its width eases.
    sidebar_toggled: Option<std::time::Instant>,
    sidebar: Entity<Part<Workspace>>,
    /// The sidebar's list of pages, scrolled in a short window.
    sidebar_scroll: ScrollHandle,
    /// Settings' categories beside them in its dialog: to the top as it opens, and to the
    /// category it shows.
    sections_scroll: ScrollHandle,
    /// The group to scroll the categories to once the dialog's list is drawn: one asked for as it
    /// opens comes before the list exists.
    sections_pending: std::cell::Cell<Option<usize>>,
    topbar: Entity<Part<Workspace>>,
    /// The sidebar toggle over the top bar (`render_sidebar_toggle`).
    toggle: Entity<Part<Workspace>>,
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
    /// The window's title as last set (`window_title`).
    title: String,
    _subscriptions: Vec<Subscription>,
}

/// The window's title on `page`: its name, so Mission Control, the Window menu and screen
/// readers tell the windows apart; Home is the app alone.
fn window_title(page: Page) -> String {
    let name = match page {
        Page::Dashboard => None,
        Page::Settings => Some("Settings"),
        page => nav()
            .into_iter()
            .find(|(each, ..)| *each == page)
            .map(|(_, label, _)| label),
    };
    match name {
        Some(name) => format!("{name} — {}", crate::profile::APP_NAME),
        None => crate::profile::APP_NAME.to_string(),
    }
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
                if window.is_window_active() {
                    crate::motion::refresh(cx);
                }
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
        let mut workspace = Workspace {
            sidebar_open,
            narrow: false,
            narrow_open: false,
            sidebar_width_from: sidebar_width(sidebar_open),
            sidebar_toggled: None,
            sidebar: Part::keyed(
                |this, window, cx| this.render_sidebar(window, cx).into_any_element(),
                SidebarInputs::of,
                cx,
            ),
            sidebar_scroll: ScrollHandle::new(),
            sections_scroll: ScrollHandle::new(),
            sections_pending: std::cell::Cell::new(None),
            topbar: Part::keyed(
                |this, window, cx| this.render_topbar(window, cx).into_any_element(),
                TopbarInputs::of,
                cx,
            ),
            toggle: Part::keyed(
                |this, _, cx| this.render_sidebar_toggle(cx).into_any_element(),
                |this, _| this.shown(),
                cx,
            ),
            history: crate::swipe::History::default(),
            swipe: crate::swipe::Swipe::default(),
            swipe_feedback: None,
            swipe_shift: 0.,
            navigating: false,
            palette: crate::palette::init(window, cx),
            pages: Pages::new(window, cx),
            back: Page::Dashboard,
            title: crate::profile::APP_NAME.to_string(),
            _subscriptions: subscriptions,
        };
        if let Some(page) = Page::remembered() {
            workspace.choose(page, window, cx);
        }
        // Once the window shows: the notes of an update just installed.
        cx.defer_in(window, |_, window, cx| {
            crate::whats_new::on_start(window, cx)
        });
        workspace
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
        page.remember();
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
        if page == Page::Settings && current != Page::Settings {
            self.sections_scroll.set_offset(point(px(0.), px(0.)));
        }
        if page == Page::Settings
            && let Some(settings) = self.pages.settings()
        {
            self.scroll_to_section(settings.read(cx).section());
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
        // The open tournament refreshes only while Tournaments is shown.
        if current == Page::Tournaments
            && page != Page::Tournaments
            && let Some(tournaments) = self
                .pages
                .view_of::<crate::pages::tournaments::TournamentsPage>(Page::Tournaments)
        {
            tournaments.update(cx, |tournaments, _| tournaments.hidden());
        }
        self.pages.show(page, window, cx);
        if page == Page::Tournaments
            && current != Page::Tournaments
            && let Some(tournaments) = self
                .pages
                .view_of::<crate::pages::tournaments::TournamentsPage>(Page::Tournaments)
        {
            tournaments.update(cx, |tournaments, cx| tournaments.shown(cx));
        }
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
    fn scrolled(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
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
        self.scroll_to_section(section);
        cx.notify();
    }

    /// Scroll Settings' categories to the group of the one it shows, which the palette may have
    /// chosen, or Settings remembered, out of a short window's sight.
    fn scroll_to_section(&self, section: Section) {
        let row = sections()
            .iter()
            .position(|(shown, _, _)| *shown == section);
        let group = row.map_or(0, |row| {
            SECTION_GROUPS
                .iter()
                .scan(0, |end, (_, size)| {
                    *end += size;
                    Some(*end)
                })
                .take_while(|end| *end <= row)
                .count()
        });
        self.sections_scroll.scroll_to_item(group);
        self.sections_pending.set(Some(group));
    }

    /// The page shown in the window: while Settings is open, the one under its dialog.
    fn shown_page(&self) -> Page {
        match self.pages.current() {
            Page::Settings => self.back,
            page => page,
        }
    }

    /// The pages (`navItems`), the one under Settings marked while its dialog is open.
    fn nav_blocks(&self, open: bool, cx: &mut Context<Self>) -> Vec<NavBlock> {
        let current = self.shown_page();
        let mut pages = nav().into_iter();
        NAV_GROUPS
            .into_iter()
            .zip(NAV_HEADINGS)
            .map(|(size, heading)| {
                let group: Vec<_> = pages.by_ref().take(size).collect();
                let mark = group
                    .iter()
                    .position(|(page, _, _)| *page == Page::Online)
                    .and_then(|row| Some((row, game_mark(cx)?)));
                NavBlock::Menu(
                    heading,
                    Box::new(SidebarMenu::new().gap_0().children(group.into_iter().map(
                        |(page, label, glyph)| {
                            let marked = (page == Page::Online).then(|| game_mark(cx)).flatten();
                            let shown = if open { nav_label(page, label) } else { label };
                            nav_item(shown, glyph)
                                .accessibility_label(label)
                                .active(current == page)
                                .when_some(marked, |item, mark| {
                                    item.accessibility_label(match mark {
                                        GameMark::Turn => format!("{label}, your move"),
                                        GameMark::Live => format!("{label}, game in progress"),
                                    })
                                    .suffix(move |_, cx| game_dot(mark, cx))
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.choose(page, window, cx)
                                }))
                        },
                    ))),
                    mark,
                )
            })
            .collect()
    }

    /// Settings' categories in its dialog, in their groups.
    fn section_blocks(&self, cx: &mut Context<Self>) -> Vec<NavBlock> {
        let shown = self
            .pages
            .settings()
            .map_or(Section::Appearance, |page| page.read(cx).section());
        let mut rows = sections().into_iter();
        SECTION_GROUPS
            .into_iter()
            .map(|(heading, size)| {
                NavBlock::Group(
                    heading,
                    Box::new(
                        SidebarMenu::new()
                            .gap_0()
                            .children(rows.by_ref().take(size).map(|(section, label, glyph)| {
                                nav_item(label, glyph).active(shown == section).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.choose_section(section, cx)
                                    }),
                                )
                            })),
                    ),
                )
            })
            .collect()
    }

    /// Settings as a dialog over the page it was opened from, below the top bar (whose search
    /// finds settings, and whose window controls stay in reach): its categories on the left,
    /// the one chosen on the right. The close button, Escape or a press outside it closes it.
    fn settings_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let overlay = cx.theme().overlay;
        let viewport = window.viewport_size();
        let margin = px(24.);
        let width = (viewport.width - margin * 2.).min(px(1040.));
        let height = viewport.height - TOPBAR_HEIGHT - margin * 2.;
        let blocks: Vec<AnyElement> = self
            .section_blocks(cx)
            .into_iter()
            .enumerate()
            .map(|(ix, block)| {
                block
                    .collapsed(false)
                    .render(SharedString::from(format!("s{}", ix + 1)), window, cx)
                    .into_any_element()
            })
            .collect();
        // GPUI drops a scroll asked for before the list's first layout, when it does not know yet
        // that the list scrolls: ask again once it has been laid out.
        if let Some(group) = self.sections_pending.get() {
            if self.sections_scroll.bounds().size.height > px(0.) {
                self.sections_pending.set(None);
                self.sections_scroll.scroll_to_item(group);
            } else {
                window.request_animation_frame();
            }
        }
        let close = cx.listener(|this, _, window, cx| this.choose(this.back, window, cx));
        div()
            .id("settings-backdrop")
            .absolute()
            .top(TOPBAR_HEIGHT)
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .bg(overlay)
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.choose(this.back, window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if crate::widgets::is_escape(event) {
                    cx.stop_propagation();
                    this.choose(this.back, window, cx);
                }
            }))
            .child(
                v_flex()
                    .id("settings-dialog")
                    .test_support()
                    .role(Role::Dialog)
                    .aria_label("Settings")
                    .w(width)
                    .h(height)
                    .rounded(crate::theme::radii::XL)
                    .border_1()
                    .border_color(palette.border)
                    .bg(cx.theme().background)
                    .overflow_hidden()
                    // A press inside is the dialog's own, not one outside it.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .flex_none()
                            .h(px(52.))
                            .px(px(20.))
                            .border_b_1()
                            .border_color(palette.border)
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.highlighted)
                                    .child("Settings"),
                            )
                            .child(
                                Button::new("settings-close")
                                    .ghost()
                                    .small()
                                    .icon(icon("x"))
                                    .accessibility_label("Close settings")
                                    .tooltip("Close (Esc)")
                                    .on_click(close),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .items_stretch()
                            .child(
                                v_flex()
                                    .id("settings-sections")
                                    .test_support()
                                    .w(px(232.))
                                    .flex_none()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.sections_scroll)
                                    .px_3()
                                    .py_2()
                                    .gap_1()
                                    .border_r_1()
                                    .border_color(palette.border)
                                    .bg(cx.theme().tokens.sidebar)
                                    .children(blocks),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(self.pages.view().cached(page_style())),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Whether the sidebar is open as shown: in a narrow window, only while opened there.
    fn shown(&self) -> bool {
        if self.narrow {
            self.narrow_open
        } else {
            self.sidebar_open
        }
    }

    /// Open or close the sidebar; `ease` widens or narrows it over `SIDEBAR_EASE` (a click on its
    /// toggle, where the motion shows where it went) instead of at once.
    fn toggle_sidebar(&mut self, ease: bool, cx: &mut Context<Self>) {
        self.sidebar_width_from = sidebar_width(self.shown());
        if self.narrow {
            self.narrow_open = !self.narrow_open;
        } else {
            self.sidebar_open = !self.sidebar_open;
            crate::prefs::set(cx, SIDEBAR_OPEN, self.sidebar_open);
        }
        if ease {
            self.sidebar_toggled = Some(std::time::Instant::now());
        } else {
            self.sidebar_width_from = sidebar_width(self.shown());
        }
        cx.notify();
    }

    /// Follow the window across `NARROW_WINDOW`: the rail below it, and above it the sidebar as
    /// kept. A resize changes the width at once, without the toggle's easing.
    fn fit_window(&mut self, window: &Window) {
        let narrow = window.viewport_size().width < NARROW_WINDOW;
        if narrow != self.narrow {
            self.narrow = narrow;
            self.narrow_open = false;
            self.sidebar_width_from = sidebar_width(self.shown());
        }
    }

    /// `USidebar` with `collapsible="icon"`: the header row the toggle floats over, the pages,
    /// and the footer, divided by lines (`divide-y`); 256px open, a 64px rail of icons collapsed,
    /// its width easing between the two in 200ms.
    fn render_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.shown();
        let in_settings = self.pages.current() == Page::Settings;
        let width = sidebar_width(open);
        let border = cx.theme().sidebar_border;
        // macOS keeps the rail's edge out of the header row, where it would cut through the
        // traffic lights (`.is-mac .kchess-sidebar-container::after`).
        let mac = cfg!(target_os = "macos");
        let blocks: Vec<AnyElement> = self
            .nav_blocks(open, cx)
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
                            .track_scroll(&self.sidebar_scroll)
                            .px_4()
                            .py_2()
                            .gap_1()
                            .children(blocks),
                    )
                    .child(
                        v_flex()
                            .flex_none()
                            .px_4()
                            .py_2()
                            .border_t_1()
                            .border_color(border)
                            .children(dev_build_badge(open, cx))
                            .child(self.footer_row(open, in_settings, cx)),
                    ),
            );
        let frame = div()
            .id("sidebar")
            .h_full()
            .flex_none()
            .overflow_hidden()
            .child(sidebar)
            .test_support();
        let from = self.sidebar_width_from;
        if from == width || self.sidebar_settled() {
            return frame.w(width).into_any_element();
        }
        EffectTransition::new(SIDEBAR_EASE)
            .ease(ease_out_cubic)
            .width(from, width)
            .apply(
                frame,
                SharedString::from(format!("sidebar-{}-{}", f32::from(from), f32::from(width))),
            )
            .into_any_element()
    }

    /// The account menu with Settings beside it on the right; on the rail, Settings above it.
    fn footer_row(&self, open: bool, in_settings: bool, cx: &mut Context<Self>) -> AnyElement {
        let settings = footer_button("settings", icon("settings-2"), "Settings".into(), false, cx)
            // Open while Settings is, and a second press closes it.
            .when(in_settings, |button| {
                button.custom(open_sidebar(cx)).selected(true).toggled(true)
            })
            .when(open, |button| button.w(px(32.)).flex_none())
            .accessibility_label("Settings")
            .on_click(cx.listener(move |this, _, window, cx| {
                let page = if in_settings {
                    this.back
                } else {
                    Page::Settings
                };
                this.choose(page, window, cx)
            }));
        if open {
            h_flex()
                .gap(px(4.))
                .child(div().flex_1().min_w_0().child(account_button(true, cx)))
                .child(settings)
                .into_any_element()
        } else {
            v_flex()
                .child(settings)
                .child(account_button(false, cx))
                .into_any_element()
        }
    }

    /// Whether the sidebar's width has finished easing since it last opened or closed.
    fn sidebar_settled(&self) -> bool {
        self.sidebar_toggled
            .is_none_or(|toggled| toggled.elapsed() > SIDEBAR_EASE + Duration::from_millis(50))
    }

    /// The sidebar as its `Part` once settled, at its width; while it eases, drawn here.
    fn sidebar_element(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.sidebar_settled() {
            return self.render_sidebar(window, cx).into_any_element();
        }
        let mut style = StyleRefinement::default();
        style.size.width = Some(sidebar_width(self.shown()).into());
        style.size.height = Some(relative(1.).into());
        style.flex_shrink = Some(0.);
        self.sidebar.clone().cached(style).into_any_element()
    }

    /// The sidebar toggle (`.sidebar-toggle-wrap`): one button at the same place in the window
    /// whether the sidebar is open or a rail, beside the macOS traffic lights (which ignore the
    /// interface size, so its offset does too), else at the window's edge.
    fn render_sidebar_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = if self.shown() {
            "Hide sidebar"
        } else {
            "Show sidebar"
        };
        let name = cx.global::<Palette>().highlighted;
        h_flex()
            .id("sidebar-toggle-wrap")
            .size_full()
            .child(over_title_bar(
                Button::new("sidebar-toggle")
                    .custom(ghost_neutral(cx))
                    .icon(icon("panel-left"))
                    .size(px(28.))
                    .tooltip_with_action(label, &crate::menu::ToggleSidebar, None)
                    .accessibility_label(label)
                    .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                        this.toggle_sidebar(!event.is_keyboard(), cx)
                    })),
            ))
            .when(self.shown(), |wrap| {
                wrap.child(
                    title_bar("sidebar-name")
                        .flex()
                        .items_center()
                        .flex_1()
                        .h_full()
                        .pl_2()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(name)
                        .child("KChess"),
                )
            })
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
                    .when(cfg!(target_os = "macos") && !self.shown(), |left| {
                        left.child(div().flex_none().w(spacer))
                    }),
            )
            .child(
                over_title_bar(self.palette.clone())
                    .flex_basis(px(600.))
                    .flex_shrink_1()
                    .min_w_0(),
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
                                "Local play, analysis and downloaded content are available",
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
                        .gap(px(8.))
                        .text_size(px(14.))
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::widgets::spinner(px(16.)))
                        .child("Working…"),
                )
            })
    }
}

/// Something is waiting on Lichess (`busy`): signing in, syncing, starting a game.
fn working(cx: &App) -> bool {
    crate::accounts::handle(cx).read(cx).working() || crate::online::handle(cx).read(cx).busy
}

/// The Lichess game the sidebar marks (`lichessGame`): your move (in a live game, or in a
/// correspondence one), or a game going on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GameMark {
    Turn,
    Live,
}

/// The Lichess game's mark: the primary color on your move, else muted.
fn game_color(mark: GameMark, cx: &App) -> Hsla {
    let theme = cx.theme();
    if mark == GameMark::Turn {
        theme.primary
    } else {
        theme.muted_foreground
    }
}

/// The mark beside Play on Lichess: an 8px dot, in a soft round halo on your move. (A box
/// shadow's spread ignores the corners and drew the halo square.)
fn game_dot(mark: GameMark, cx: &App) -> Div {
    let color = game_color(mark, cx);
    div()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(mark == GameMark::Turn, |halo| halo.bg(color.opacity(0.25)))
        .child(div().size(px(8.)).rounded_full().bg(color))
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

/// The sidebar footer's buttons: muted like the navigation above them.
fn ghost_sidebar(cx: &App) -> ButtonCustomVariant {
    let palette = cx.global::<Palette>();
    ButtonCustomVariant::new(cx)
        .foreground(palette.text_muted)
        .hover(palette.elevated)
        .active(palette.elevated)
}

/// A footer button whose page is open: on the current row's background, in the rows' own text
/// color, so the category open in it stays the one accented row.
fn open_sidebar(cx: &App) -> ButtonCustomVariant {
    let theme = cx.theme();
    ButtonCustomVariant::new(cx)
        .color(theme.sidebar_accent)
        .foreground(theme.sidebar_foreground)
        .hover(theme.sidebar_accent)
        .active(theme.sidebar_accent)
}

/// A footer button drawn as a navigation row: 32px tall, an 18px icon on the rows' column with
/// the label beside it while the sidebar is open. On the rail the icon stands alone, with the
/// label as its tooltip.
fn footer_button(
    id: &'static str,
    glyph: Icon,
    label: SharedString,
    open: bool,
    cx: &App,
) -> Button {
    Button::new(id)
        .custom(ghost_sidebar(cx))
        .w_full()
        .px(px(6.))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(8.))
                .when(!open, |row| row.justify_center())
                .child(glyph.size(px(18.)).flex_none())
                .when(open, |row| row.child(label.clone())),
        )
        .when(!open, |button| button.tooltip(label))
}

/// Marks a debug build (`cargo run -p kchess-gpui`) above Settings, as the Electron app marked
/// its dev builds. Release builds compile none of it; packaging checks the binary for
/// its id.
#[cfg(debug_assertions)]
fn dev_build_badge(open: bool, cx: &App) -> Option<AnyElement> {
    const LABEL: &str = "Development build";
    let warning = cx.global::<Palette>().warning;
    Some(
        h_flex()
            .id("kchess-dev-build-badge")
            .h(px(32.))
            .px(px(6.))
            .gap(px(8.))
            .text_sm()
            .when(!open, |badge| {
                badge.justify_center().tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(LABEL).build(window, cx)
                })
            })
            .child(
                icon("terminal")
                    .size(px(18.))
                    .flex_none()
                    .text_color(warning),
            )
            .when(open, |badge| badge.child(LABEL))
            .into_any_element(),
    )
}

#[cfg(not(debug_assertions))]
fn dev_build_badge(_open: bool, _cx: &App) -> Option<AnyElement> {
    None
}

/// The sidebar's account menu: which account plays (with several), its profile, managing and
/// connecting accounts, What's new, reporting a problem, and logging out. With no account it is
/// named Connect Lichess and opens on connecting one.
fn account_button(expanded: bool, cx: &App) -> AnyElement {
    let handle = crate::accounts::handle(cx);
    let accounts = handle.read(cx);
    let active = accounts.active(cx);
    let connected = accounts.connected();
    let can_connect = crate::network::online(cx) && !accounts.busy;
    let (glyph, label) = match &active {
        Some(active) => (icon("user"), format!("@{active}")),
        None => (icon("log-in"), "Connect Lichess".to_string()),
    };
    let target = handle.downgrade();
    // A navigation row: the label between the icon and a dimmed up-down chevron; elevated while
    // its menu is open.
    let palette = cx.global::<Palette>();
    let dimmed = palette.dimmed;
    Button::new("account-menu")
        .custom(ghost_sidebar(cx))
        .w_full()
        .px(px(6.))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(8.))
                .when(!expanded, |row| row.justify_center())
                .child(glyph.size(px(18.)).flex_none())
                .when(expanded, |row| {
                    row.child(div().flex_1().min_w_0().truncate().child(label.clone()))
                        .child(
                            icon("chevrons-up-down")
                                .size(px(18.))
                                .flex_none()
                                .text_color(dimmed),
                        )
                }),
        )
        .accessibility_label(match &active {
            Some(_) => format!("{label} account menu"),
            None => label.clone(),
        })
        // On the rail the icon stands alone: name the account as the footer's other buttons do.
        .when(!expanded, |button| button.tooltip(label.clone()))
        // Above the button: open, from its left edge across the sidebar; on the rail, centred on
        // it. At least 192px wide, 8px clear.
        .dropdown_menu_with_anchor(
            if expanded {
                Anchor::BottomLeft
            } else {
                Anchor::BottomCenter
            },
            move |menu, _, _| {
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
                let mut menu = menu;
                // Several accounts: which one plays, then the account's pages, then signing out.
                // One account: the trigger already names it, so the menu opens on its pages.
                let several = connected.len() > 1;
                if let Some(active) = &active {
                    if several {
                        menu = menu.label("Play as");
                        for name in &connected {
                            let (name, target) = (name.clone(), target.clone());
                            menu = menu.item(
                                PopupMenuItem::new(format!("@{name}"))
                                    .checked(&name == active)
                                    .on_click(move |_, _, cx| {
                                        let _ = target.update(cx, |accounts, cx| {
                                            accounts.set_active(&name, cx)
                                        });
                                    }),
                            );
                        }
                        menu = menu.separator();
                    }
                    let profile = active.clone();
                    // The Settings button sits beside this one; the menu goes to its accounts.
                    menu = menu
                        .item(
                            PopupMenuItem::new("Profile")
                                .icon(icon("user"))
                                .on_click(move |_, window, cx| open_player(&profile, window, cx)),
                        )
                        .item(
                            PopupMenuItem::new("Manage accounts")
                                .icon(icon("users"))
                                .on_click(|_, window, cx| {
                                    open_settings(Section::Accounts, window, cx)
                                }),
                        );
                }
                menu = menu
                    .item(
                        PopupMenuItem::new(if active.is_none() {
                            "Connect Lichess account"
                        } else {
                            "Connect another account"
                        })
                        .icon(icon("user-plus"))
                        .disabled(!can_connect)
                        .on_click(act(|accounts, window, cx| accounts.connect(window, cx))),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("What's new")
                            .icon(icon("rocket"))
                            .on_click(|_, window, cx| crate::whats_new::open(window, cx)),
                    )
                    .item(
                        PopupMenuItem::new("Report a problem")
                            .icon(icon("message-circle"))
                            .on_click(|_, _, cx| {
                                cx.open_url(crate::pages::settings::updates::ISSUES)
                            }),
                    );
                if let Some(active) = &active {
                    menu = menu.separator().item(
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
                }
                menu
            },
        )
        .into_any_element()
}

/// A newer KChess, its download, or the restart that installs it.
fn update_button(cx: &App) -> Option<impl IntoElement + use<>> {
    use crate::updates::{Phase, UpdatesHandle};
    // Not during a real-time game: the update waits in Settings → Updates until it ends.
    if crate::online::handle(cx).read(cx).live_game_on() {
        return None;
    }
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
                .unwrap_or_else(|| "The update could not finish".into()),
        ),
        _ => return None,
    };
    let phase = updates.phase;
    let target = handle.downgrade();
    Some(
        over_title_bar(
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
        // Its right edge is anchored, so a proportional "Updating N%" moves it with every percent.
        .font_features(crate::widgets::tabular()),
    )
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit_window(window);
        let title = window_title(self.pages.current());
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
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
            .when(!zen, |shell| shell.child(self.sidebar_element(window, cx)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.topbar.clone().cached(topbar_style()))
                    .child(
                        div()
                            .id("page-area")
                            .flex_1()
                            .min_h_0()
                            .relative()
                            .overflow_hidden()
                            .on_scroll(cx.listener(Self::scrolled))
                            // A PGN or FEN file dropped here opens: on the analysis board, or
                            // as a study when it holds several games.
                            .on_drop(|paths: &ExternalPaths, window, cx| {
                                open_dropped(paths, window, cx)
                            })
                            // A mouse's back and forward buttons go as a swipe does.
                            .on_mouse_down(
                                MouseButton::Navigate(NavigationDirection::Back),
                                cx.listener(|this, _, window, cx| this.go(-1, window, cx)),
                            )
                            .on_mouse_down(
                                MouseButton::Navigate(NavigationDirection::Forward),
                                cx.listener(|this, _, window, cx| this.go(1, window, cx)),
                            )
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if !event.is_held && this.pages.page_focused(window, cx) {
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
                                    // Settings opens over the page it was opened from.
                                    .child(
                                        self.pages
                                            .view_for(self.shown_page())
                                            .unwrap_or_else(|| self.pages.view())
                                            .cached(page_style()),
                                    ),
                            )
                            .children(self.swipe_edge(cx))
                            .child(drop_zone(cx.global::<Palette>())),
                    ),
            )
            .when(!zen, |shell| {
                shell.child(
                    self.toggle
                        .clone()
                        .cached(toggle_style(self.shown(), window)),
                )
            })
            .when(self.pages.current() == Page::Settings, |shell| {
                shell.child(self.settings_dialog(window, cx))
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

#[cfg(test)]
mod dropped_file_tests {
    use super::{DROPPED_MAX, read_game_file};

    #[test]
    fn a_game_file_under_the_limit_reads() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("game.pgn");
        std::fs::write(&path, "1. e4 e5 *").unwrap();
        assert_eq!(read_game_file(&path).as_deref(), Some("1. e4 e5 *"));
        std::fs::write(&path, vec![b'a'; DROPPED_MAX as usize + 1]).unwrap();
        assert_eq!(read_game_file(&path), None);
    }

    /// A `.pgn` symlink to `/dev/zero` reported a length of 0 and was read until memory ran out.
    #[cfg(unix)]
    #[test]
    fn a_device_behind_a_game_name_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("game.pgn");
        std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
        assert_eq!(read_game_file(&path), None);
    }
}

//! The window's frame: a sidebar that collapses
//! to icons, a top bar, and the current page.

use gpui_kit::component::animation::{EffectTransition, ease_out_cubic};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::sidebar::{SidebarItem, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Icon, Selectable as _, Sizable as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;

use crate::assets::icon;
use crate::pages::settings::{SECTION_GROUPS, Section, sections};
use crate::pages::{ActionRole, NavGroup, Page, PageCommand, PageView, Pages};
use crate::part::Part;
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::Palette;

/// The window's height above the page; matches `--topbar-height`.
pub const TOPBAR_HEIGHT: Pixels = px(52.);

/// Whether the sidebar is open, kept for both apps (`kchess:sidebar-open`).
const SIDEBAR_OPEN: &str = "kchess:sidebar-open";

/// `USidebar`'s `--sidebar-width`, and `--sidebar-width-icon` collapsed to icons.
pub(crate) const SIDEBAR_WIDTH: Pixels = px(256.);
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
pub(crate) fn over_title_bar(control: impl IntoElement) -> Div {
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

/// A row of the sidebar's navigation: 32px tall, an 18px icon 8px from its label.
fn nav_item(label: impl Into<SharedString>, glyph: Icon) -> SidebarMenuItem {
    SidebarMenuItem::new(label)
        .icon(glyph.size(px(18.)))
        .min_h(px(32.))
        .p(px(6.))
        .gap_x(px(8.))
}

/// The sidebar's pages in their groups, in order.
pub(crate) fn nav_groups() -> Vec<(NavGroup, Vec<Page>)> {
    let mut groups: Vec<(NavGroup, Vec<Page>)> = Vec::new();
    for page in Page::in_sidebar() {
        let group = page.info().group.expect("a sidebar page has a group");
        match groups.last_mut() {
            Some((last, pages)) if *last == group => pages.push(page),
            _ => groups.push((group, vec![page])),
        }
    }
    groups
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
    account: crate::accounts::MenuInputs,
}

impl SidebarInputs {
    fn of(workspace: &Workspace, cx: &App) -> SidebarInputs {
        SidebarInputs {
            open: workspace.shown(),
            width_from: workspace.sidebar_width_from,
            current: workspace.pages.current(),
            section: workspace
                .pages
                .get::<crate::pages::settings::SettingsPage>()
                .map(|page| page.read(cx).section()),
            back: workspace.back,
            game_mark: game_mark(cx),
            account: crate::accounts::MenuInputs::of(cx),
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
    update: crate::pages::settings::updates::ButtonInputs,
}

impl TopbarInputs {
    fn of(workspace: &Workspace, cx: &App) -> TopbarInputs {
        TopbarInputs {
            sidebar_open: workspace.shown(),
            online: crate::network::online(cx),
            working: working(cx),
            update: crate::pages::settings::updates::ButtonInputs::of(cx),
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

/// The window's workspace, so a page can open another.
struct Navigator(WeakEntity<Workspace>);

impl Global for Navigator {}

fn workspace(cx: &App) -> Option<Entity<Workspace>> {
    cx.try_global::<Navigator>()?.0.upgrade()
}

/// The window's title as the workspace last set it.
#[cfg(test)]
pub fn window_title_shown(cx: &App) -> Option<String> {
    Some(workspace(cx)?.read(cx).title.clone())
}

/// The page shown, once the window is open.
pub fn current_page(cx: &App) -> Option<Page> {
    Some(workspace(cx)?.read(cx).pages.current())
}

/// Open `page` as the sidebar would.
pub fn open(page: Page, window: &mut Window, cx: &mut App) {
    if let Some(workspace) = workspace(cx) {
        workspace.update(cx, |workspace, cx| workspace.choose(page, window, cx));
    }
}

/// Open page `V` as the sidebar would, then `act` on it: how one page links into another.
pub fn show<V: PageView>(
    window: &mut Window,
    cx: &mut App,
    act: impl FnOnce(&mut V, &mut Window, &mut Context<V>),
) {
    open(V::INFO.page, window, cx);
    if let Some(page) = view::<V>(cx) {
        page.update(cx, |page, cx| act(page, window, cx));
    }
}

/// Page `V`'s view, once it has been made.
pub fn view<V: PageView>(cx: &App) -> Option<Entity<V>> {
    workspace(cx)?.read(cx).pages.get::<V>()
}

/// Page `V`'s view, made now (without showing it) if it has not been.
pub fn made<V: PageView>(window: &mut Window, cx: &mut App) -> Option<Entity<V>> {
    workspace(cx)?.update(cx, |workspace, cx| {
        Some(workspace.pages.get_made::<V>(window, cx))
    })
}

/// Back (-1) or forward (1): out of the page's inner level, then through the pages visited, as
/// a swipe goes.
pub fn go(direction: crate::swipe::Direction, window: &mut Window, cx: &mut App) {
    if let Some(workspace) = workspace(cx) {
        workspace.update(cx, |workspace, cx| workspace.go(direction, window, cx));
    }
}

/// Whether the sidebar is shown.
pub fn sidebar_open(cx: &App) -> bool {
    workspace(cx).is_none_or(|workspace| workspace.read(cx).shown())
}

/// Show or hide the sidebar from its shortcut, the menu or the command palette: at once, since a
/// command repeated many times a day should not wait on an animation.
pub fn toggle_sidebar(_: &mut Window, cx: &mut App) {
    if let Some(workspace) = workspace(cx) {
        workspace.update(cx, |workspace, cx| workspace.toggle_sidebar(false, cx));
    }
}

/// Start every page afresh at Home (after signing out), so
/// nothing a page kept for an account outlives it.
pub fn restart_pages(window: &mut Window, cx: &mut App) {
    if let Some(workspace) = workspace(cx) {
        workspace.update(cx, |workspace, cx| {
            workspace.pages = Pages::new(window, cx);
            workspace.choose(Page::Dashboard, window, cx);
        });
    }
}

/// Open Settings at one section, e.g. the engine setup a page points at.
pub fn open_settings(section: Section, window: &mut Window, cx: &mut App) {
    open(Page::Settings, window, cx);
    if let Some(workspace) = workspace(cx) {
        workspace.update(cx, |workspace, cx| workspace.choose_section(section, cx));
    }
}

/// Redraw the shell, as its network, account and update observers do.
#[cfg(any(test, feature = "bench"))]
pub fn notify_shell(cx: &mut App) {
    if let Some(workspace) = workspace(cx) {
        workspace.update(cx, |_, cx| cx.notify());
    }
}

/// The current page's actions, as Quick Pick offers them.
pub fn page_commands(cx: &App) -> Vec<PageCommand> {
    workspace(cx).map_or_else(Vec::new, |workspace| workspace.read(cx).pages.commands(cx))
}

/// Zen mode applies to the page shown now.
pub fn zen_page(cx: &App) -> bool {
    workspace(cx).is_some_and(|workspace| workspace.read(cx).pages.zen(cx))
}

/// The current page, to act on.
fn current_handle(cx: &App) -> Option<crate::pages::PageHandle> {
    workspace(cx)?.read(cx).pages.current_handle()
}

/// Copy outside a text field: the page's game as PGN, or its position as FEN where it has no
/// game (the board editor), as its own Copy command would. Under a dialog, or on a page with
/// nothing to copy, nothing happens.
pub fn copy_on_page(window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    if window.has_active_dialog(cx) {
        return;
    }
    if let Some(page) = current_handle(cx) {
        page.run_role(ActionRole::Copy, window, cx);
    }
}

/// New (⌘N): what the page's own New does, asking first where it asks: a game against the
/// computer, over the board or on Lichess, a study, a fresh analysis board. Under a dialog, or
/// on a page with nothing to start, nothing happens.
pub fn new_on_page(window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    if window.has_active_dialog(cx) {
        return;
    }
    if let Some(page) = current_handle(cx) {
        page.new_here(window, cx);
    }
}

/// Find: the page's own search field, focused with its text selected, when the page shows one
/// (History's, Studies', Openings', Watch's broadcasts', Players' username). It is `false`
/// on a page without one, for Quick Pick to search instead.
pub fn find_on_page(window: &mut Window, cx: &mut App) -> bool {
    let Some((page, fields)) = workspace(cx).and_then(|w| w.read(cx).pages.search_fields(cx))
    else {
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

/// Make the pages made ahead (`Workspace::prewarm`) now.
#[cfg(test)]
/// Makes `page` without showing it.
#[cfg(test)]
pub fn prepare(page: Page, window: &mut Window, cx: &mut App) {
    if let Some(workspace) = workspace(cx) {
        workspace.update(cx, |workspace, cx| {
            workspace.pages.prepare(page, window, cx)
        });
    }
}

/// Make the pages made ahead (`Workspace::prewarm`) now.
#[cfg(test)]
pub fn prewarm(window: &mut Window, cx: &mut App) {
    if let Some(workspace) = workspace(cx) {
        workspace.update(cx, |workspace, cx| workspace.prewarm(window, cx));
    }
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
    match page {
        Page::Dashboard => crate::profile::APP_NAME.to_string(),
        page => format!("{} — {}", page.info().label, crate::profile::APP_NAME),
    }
}

/// The pages made ahead, a moment after the window shows: making them is most of their first
/// opening (reading their sessions, the board game's history places), and nothing they do
/// before being shown reaches the engine or the network. Every other page's first opening is
/// its first frame, which making it ahead does not change. 1.8 MiB for every page, measured.
const PREWARMED: [Page; 2] = [Page::Local, Page::Analysis];

/// How long after the window opens the pages are made: past the first frames.
#[cfg(not(test))]
const PREWARM_AFTER: std::time::Duration = std::time::Duration::from_millis(1500);

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
        // A moment after the window shows, the pages whose making is most of their first
        // opening are made. The end-to-end tests make them when a test asks (`prewarm`), so
        // each opens its pages as it did.
        #[cfg(not(test))]
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(PREWARM_AFTER).await;
            let _ = this.update_in(cx, |workspace, window, cx| workspace.prewarm(window, cx));
        })
        .detach();
        workspace
    }

    /// Make the pages of `PREWARMED` not made yet, without showing them.
    pub fn prewarm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for page in PREWARMED {
            self.pages.prepare(page, window, cx);
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
            self.sections_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.pages.choose(page, window, cx);
        if page == Page::Settings
            && let Some(settings) = self.pages.get::<crate::pages::settings::SettingsPage>()
        {
            self.scroll_to_section(settings.read(cx).section());
        }
        cx.notify();
    }

    /// The open page has a level to step out of going `direction`: a game in Game history, a
    /// broadcast's game or tournament.
    fn can_step(&self, direction: crate::swipe::Direction, cx: &App) -> bool {
        self.pages.can_step(direction, cx)
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
            self.pages.step(direction, window, cx);
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
        if let Some(settings) = self.pages.get::<crate::pages::settings::SettingsPage>() {
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

    /// The pages in their groups, the one under Settings marked while its dialog is open.
    fn nav_blocks(&self, open: bool, cx: &mut Context<Self>) -> Vec<NavBlock> {
        let current = self.shown_page();
        nav_groups()
            .into_iter()
            .map(|(group, pages)| {
                let mark = pages
                    .iter()
                    .position(|page| *page == Page::Online)
                    .and_then(|row| Some((row, game_mark(cx)?)));
                NavBlock::Menu(
                    group.heading(),
                    Box::new(
                        SidebarMenu::new()
                            .gap_0()
                            .children(pages.into_iter().map(|page| {
                                let info = page.info();
                                let label = info.label;
                                let marked =
                                    (page == Page::Online).then(|| game_mark(cx)).flatten();
                                let shown = if open { info.short } else { label };
                                nav_item(shown, icon(info.icon))
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
                            })),
                    ),
                    mark,
                )
            })
            .collect()
    }

    /// Settings' categories in its dialog, in their groups.
    fn section_blocks(&self, cx: &mut Context<Self>) -> Vec<NavBlock> {
        let shown = self
            .pages
            .get::<crate::pages::settings::SettingsPage>()
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
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(crate::accounts::menu_button(true, cx)),
                )
                .child(settings)
                .into_any_element()
        } else {
            v_flex()
                .child(settings)
                .child(crate::accounts::menu_button(false, cx))
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
            .children(crate::pages::settings::updates::title_bar_button(cx))
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

/// The neutral ghost button: the text color, the accented
/// background on hover and while pressed.
fn ghost_neutral(cx: &App) -> ButtonCustomVariant {
    let palette = cx.global::<Palette>();
    ButtonCustomVariant::new(cx)
        .foreground(palette.text)
        .hover(palette.accented)
        .active(palette.accented)
}

/// The sidebar footer's buttons: muted like the navigation above them.
pub(crate) fn ghost_sidebar(cx: &App) -> ButtonCustomVariant {
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

/// Marks a debug build (`cargo run -p kchess-gpui`) above Settings.
/// Release builds compile none of it; packaging checks the binary for
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

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit_window(window);
        let title = window_title(self.pages.current());
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
        // Zen mode on a game page leaves the board and its panel.
        let zen = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .zen_mode
            && self.pages.zen(cx);
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
                                crate::links::open_dropped(paths, window, cx)
                            })
                            // Z on a page zen mode applies to, while no text field has the keys.
                            .on_action(cx.listener(
                                |this, _: &crate::menu::ToggleZen, window, cx| {
                                    if this.pages.zen_key(cx) && this.pages.page_focused(window, cx)
                                    {
                                        crate::settings::change(cx, |settings| {
                                            settings.frontend.zen_mode = !settings.frontend.zen_mode
                                        });
                                    } else {
                                        cx.propagate();
                                    }
                                },
                            ))
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

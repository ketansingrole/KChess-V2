//! The window's frame: a sidebar that collapses
//! to icons, a top bar, and the current page.

use gpui_kit::component::animation::{EffectTransition, ease_out_cubic};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::sidebar::{SidebarItem, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Icon, Selectable as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;

use crate::assets::icon;
use crate::pages::settings::Section;
use crate::pages::{ActionRole, NavGroup, Page, PageCommand, Pages};
use crate::part::Part;
use crate::settings::{SettingsChanged, SettingsEdited, SettingsHandle};
use crate::theme::Palette;

mod settings_dialog;
mod sidebar;

pub(crate) use sidebar::ghost_sidebar;
#[cfg(test)]
pub(crate) use sidebar::nav_groups;
use sidebar::*;

const SIDEBAR_OPEN: &str = "kchess:sidebar-open";

/// The sidebar's width while open.
pub(crate) const SIDEBAR_WIDTH: Pixels = px(256.);
/// How long the sidebar's width eases when it opens or closes.
const SIDEBAR_EASE: Duration = Duration::from_millis(200);
const SIDEBAR_RAIL: Pixels = px(64.);
/// The sidebar's padding either side of its rows; on the right, less its 1px edge, which takes
/// room from them, so the rail's rows stay centred on it.
const SIDEBAR_INSET: Pixels = px(16.);
const SIDEBAR_BORDER: Pixels = px(1.);
/// A navigation row's icon, and its padding either side: the rail's rows are as wide as twice
/// the padding and the icon, so an icon sits on the same column open or as the rail.
pub(crate) const NAV_ICON: Pixels = px(18.);
pub(crate) const NAV_INSET: Pixels = px(7.);
/// The sidebar toggle's side.
const TOGGLE_SIDE: Pixels = px(28.);
/// Below this window width the sidebar shows as its rail, leaving the page the room; its toggle
/// opens it there for the moment without changing what wider windows show.
const NARROW_WINDOW: Pixels = px(800.);

fn sidebar_width(open: bool) -> Pixels {
    if open { SIDEBAR_WIDTH } else { SIDEBAR_RAIL }
}

/// The sidebar toggle's left edge: 84px on macOS at any interface size, clear of the traffic
/// lights, else on the column of the navigation's icons below it.
fn toggle_left(window: &Window) -> Pixels {
    if cfg!(target_os = "macos") {
        px(84. / window.zoom())
    } else {
        SIDEBAR_INSET + NAV_INSET + NAV_ICON / 2. - TOGGLE_SIDE / 2.
    }
}

/// A control on a title bar, which takes its own press: on Windows a press on the drag area under
/// it would move the window, and the control would never hear the click.
pub(crate) fn over_title_bar(control: impl IntoElement) -> Div {
    div().occlude().child(control)
}

/// Part of the window's title bar that Windows moves the window from (its caption hit test).
/// It answers no press itself: what lies under it does, once.
fn drag_area(id: &'static str) -> Stateful<Div> {
    div().id(id).window_control_area(WindowControlArea::Drag)
}

/// The sidebar's header, the one part of the title bar outside GPUI Kit's `TitleBar`: dragging
/// moves the window, a double click zooms it. Only one element may answer a press there, or
/// one double click zooms and restores the window again (the top bar and its `TitleBar` both
/// did, and so did the sidebar's name over this header).
fn title_bar(id: &'static str) -> Stateful<Div> {
    drag_area(id).on_mouse_down(MouseButton::Left, |event, window, _| {
        if event.click_count == 2 {
            window.titlebar_double_click();
        } else {
            window.start_window_move();
        }
    })
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

/// The sidebar toggle as its `Part`: its 28px button, `TOPBAR_HEIGHT`
/// high, over the top bar's left end, and the product's name beside it while the sidebar is open.
fn toggle_style(open: bool, window: &Window) -> StyleRefinement {
    let mut style = StyleRefinement {
        position: Some(Position::Absolute),
        ..Default::default()
    };
    style.inset.top = Some(px(0.).into());
    style.inset.left = Some(toggle_left(window).into());
    style.size.width = Some(if open { px(100.) } else { TOGGLE_SIDE }.into());
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

// How a page reaches the shell (`kchess_nav::shell`), answered by the workspace below.
pub use kchess_nav::shell::{
    TOPBAR_HEIGHT, current_page, go, keeping_keys_since, made, open, open_settings,
    open_settings_at, show, view,
};

/// The workspace as pages reach it (`kchess_nav::shell::Shell`).
struct WorkspaceShell;

impl kchess_nav::shell::Shell for WorkspaceShell {
    fn open(&self, page: Page, window: &mut Window, cx: &mut App) {
        if let Some(workspace) = workspace(cx) {
            workspace.update(cx, |workspace, cx| workspace.choose(page, window, cx));
        }
    }

    fn current(&self, cx: &App) -> Option<Page> {
        Some(workspace(cx)?.read(cx).pages.current())
    }

    fn view(&self, page: Page, cx: &App) -> Option<AnyView> {
        workspace(cx)?.read(cx).pages.view_for(page)
    }

    fn made(&self, page: Page, window: &mut Window, cx: &mut App) -> Option<AnyView> {
        workspace(cx)?.update(cx, |workspace, cx| {
            Some(workspace.pages.made_view(page, window, cx))
        })
    }

    fn go(&self, direction: crate::swipe::Direction, window: &mut Window, cx: &mut App) {
        if let Some(workspace) = workspace(cx) {
            workspace.update(cx, |workspace, cx| workspace.go(direction, window, cx));
        }
    }

    fn choose_section(&self, section: Section, row: Option<&'static str>, cx: &mut App) {
        if let Some(workspace) = workspace(cx) {
            workspace.update(cx, |workspace, cx| match row {
                Some(row) => workspace.reveal_row(section, row, cx),
                None => workspace.choose_section(section, cx),
            });
        }
    }
}

fn workspace(cx: &App) -> Option<Entity<Workspace>> {
    cx.try_global::<Navigator>()?.0.upgrade()
}

/// The window's title as the workspace last set it.
#[cfg(test)]
pub fn window_title_shown(cx: &App) -> Option<String> {
    Some(workspace(cx)?.read(cx).title.clone())
}

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
                .rounded(crate::theme::radii::BOX)
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
    /// Settings' categories beside them in its dialog.
    sections: settings_dialog::Sections,
    topbar: Entity<Part<Workspace>>,
    /// The notification centre's bell, at the top bar's right end.
    bell: Entity<crate::notifications::centre::Bell>,
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
        cx.set_global(kchess_nav::shell::Navigator(std::rc::Rc::new(
            WorkspaceShell,
        )));
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
            // A change the person made shows its Undo in the window they made it in.
            cx.subscribe_in(
                &settings,
                window,
                |_, _, edited: &SettingsEdited, window, cx| {
                    if window.is_window_active() {
                        crate::setting_notice::edited(&edited.0, window, cx);
                    }
                },
            ),
            crate::window_state::track(window, cx),
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
            sections: settings_dialog::Sections::default(),
            topbar: Part::keyed(
                |this, window, cx| this.render_topbar(window, cx).into_any_element(),
                TopbarInputs::of,
                cx,
            ),
            bell: cx.new(crate::notifications::centre::Bell::new),
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
            self.sections.to_top();
        }
        self.pages.choose(page, window, cx);
        let title = window_title(page);
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
        if page == Page::Settings
            && let Some(settings) = self.pages.get::<crate::pages::settings::SettingsPage>()
        {
            self.sections.scroll_to(settings.read(cx).section());
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
        let settings = &crate::settings::frontend(cx);
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

    /// The page shown in the window: while Settings is open, the one under its dialog.
    fn shown_page(&self) -> Page {
        match self.pages.current() {
            Page::Settings => self.back,
            page => page,
        }
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
        // Room for the toggle floating over the topbar beside the macOS rail: its 28px and 8px clear of it, less the rail and the padding.
        let spacer =
            (toggle_left(window) + TOGGLE_SIDE + px(8.) - SIDEBAR_RAIL - px(20.)).max(px(0.));
        // GPUI Kit's `TitleBar` around it moves and zooms the window.
        div()
            .id("topbar")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h_full()
            // The search, up to 600px, centred between equal sides of at least 40px; 12px
            // padding below 560px.
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

    /// The top bar's right end: connection, update and work state, then the notification bell,
    /// last so it stays put as the others come and go.
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
            .child(self.bell.clone())
    }
}

/// Something is waiting on Lichess (`busy`): signing in, syncing, starting a game.
fn working(cx: &App) -> bool {
    crate::accounts::handle(cx).read(cx).working() || crate::online::handle(cx).read(cx).busy
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit_window(window);
        // Zen mode on a game page leaves the board and its panel.
        let zen = crate::settings::frontend(cx).zen_mode && self.pages.zen(cx);
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
                            // Measured every frame, shown page or not, so a page opens laid out
                            // for the area it opens in.
                            .child(kchess_nav::play_layout::page_area_probe())
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
                            // F turns the page's board, while no text field has the keys.
                            .on_action(cx.listener(
                                |this, _: &crate::menu::FlipBoard, window, cx| {
                                    let page = this.pages.current_handle();
                                    match page {
                                        Some(page)
                                            if this.pages.page_focused(window, cx)
                                                && page.has_role(ActionRole::Flip, cx) =>
                                        {
                                            cx.defer_in(window, move |_, window, cx| {
                                                page.run_role(ActionRole::Flip, window, cx)
                                            });
                                        }
                                        _ => cx.propagate(),
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

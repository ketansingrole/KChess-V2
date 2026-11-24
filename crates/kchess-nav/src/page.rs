//! The app's pages as the shell drives them, one per sidebar entry, created when first shown and
//! kept while the app runs.
//!
//! Each page describes itself once, as a `PageView`: its name, icon and place in the sidebar, the
//! words Quick Pick finds it by, its tabs and actions, and what it does as it is shown or left.
//! The shell, Quick Pick and the menus reach every page through the registry the app sets
//! (`set_registry`).

use kchess_app::log::ResultExt as _;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use gpui_kit::component::input::InputState;
use gpui_kit::*;

use crate::swipe::Direction;

/// A page, in the sidebar's order (the registry is indexed by it).
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
    Database,
    Editor,
    Online,
    Tournaments,
    Watch,
    Insights,
    Friends,
    Players,
    Settings,
}

/// The sidebar's groups, in order: home, playing, training, your own games and boards, Lichess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavGroup {
    Home,
    Play,
    Train,
    Review,
    Lichess,
}

impl NavGroup {
    /// The heading over the group in the open sidebar; Home stands alone.
    pub fn heading(self) -> Option<&'static str> {
        match self {
            NavGroup::Home => None,
            NavGroup::Play => Some("Play"),
            NavGroup::Train => Some("Train"),
            NavGroup::Review => Some("Review"),
            NavGroup::Lichess => Some("Lichess"),
        }
    }
}

/// How a page names itself.
pub struct PageInfo {
    pub page: Page,
    /// Its full name: the window's title, the rail's tooltip, Quick Pick and screen readers.
    pub label: &'static str,
    /// Its name under its group's heading, where the heading already says the rest ("Computer"
    /// under Play).
    pub short: &'static str,
    pub icon: &'static str,
    /// Its group in the sidebar; Settings, opened from the sidebar's footer, has none.
    pub group: Option<NavGroup>,
    /// Words Quick Pick finds it by that are not shown.
    pub keywords: &'static str,
}

impl Page {
    pub fn info(self) -> &'static PageInfo {
        &registry()[self as usize].info
    }

    /// Every page, in the sidebar's order.
    pub fn all() -> impl Iterator<Item = Page> {
        registry().iter().map(|registered| registered.info.page)
    }

    /// The pages in the sidebar, in its order.
    pub fn in_sidebar() -> impl Iterator<Item = Page> {
        Page::all().filter(|page| page.info().group.is_some())
    }

    /// The page `cargo xtask app --watch` was on before it restarted the app, from
    /// `KCHESS_PAGE_FILE`.
    pub fn remembered() -> Option<Page> {
        let file = kchess_host::env::var_os("KCHESS_PAGE_FILE")?;
        Page::named(std::fs::read_to_string(file).ok()?.trim())
    }

    /// Notes the page in `KCHESS_PAGE_FILE`, when a watch set it. A development aid: a failed
    /// write only means the next restart opens on Home.
    pub fn remember(self) {
        if let Some(file) = kchess_host::env::var_os("KCHESS_PAGE_FILE") {
            std::fs::write(file, format!("{self:?}")).warn_on_err();
        }
    }

    /// The page remembered by its name (`Analysis`).
    pub fn named(name: &str) -> Option<Page> {
        Page::all().find(|page| format!("{page:?}") == name)
    }
}

/// What an action does besides its own entry: the app-wide key that runs it on its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionRole {
    /// Copy outside a text field (⌘C): the page's game or position.
    Copy,
    /// New (⌘N): what the page starts.
    New,
    /// F: turns the page's board.
    Flip,
}

/// How a page's action (or one of its places) is named: its id (kept in Quick Pick's recent
/// entries), label, icon and search words, and the key that also runs it.
#[derive(Clone)]
pub struct ActionInfo {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: &'static str,
    pub keywords: &'static str,
    pub role: Option<ActionRole>,
}

impl ActionInfo {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: &'static str,
        keywords: &'static str,
    ) -> ActionInfo {
        ActionInfo {
            id: id.into(),
            label: label.into(),
            icon,
            keywords,
            role: None,
        }
    }

    /// Every board's Flip board (`id`, kept in Quick Pick's recent entries), which F runs.
    pub fn flip(id: &'static str) -> ActionInfo {
        ActionInfo::new(id, "Flip board", "arrow-up-down", "turn rotate orientation")
            .role(ActionRole::Flip)
    }

    pub fn role(mut self, role: ActionRole) -> ActionInfo {
        self.role = Some(role);
        self
    }
}

/// A page's action, as its buttons, Quick Pick and its key offer it.
pub trait PageAction: Clone + 'static {
    fn info(&self) -> ActionInfo;
}

impl PageAction for std::convert::Infallible {
    fn info(&self) -> ActionInfo {
        match *self {}
    }
}

/// What runs a command, on the window it was chosen in.
pub type Run = Rc<dyn Fn(&mut Window, &mut App)>;

/// Something Quick Pick can run on a page: one of its actions, or one of its places (a tab).
#[derive(Clone)]
pub struct PageCommand {
    pub info: ActionInfo,
    /// Dimmed text after the label: the page a place is on.
    pub detail: Option<SharedString>,
    pub run: Run,
}

/// A page as the shell drives it.
pub trait PageView: Render + Focusable + Sized {
    const INFO: PageInfo;

    /// Its actions' type; `Infallible` for a page without any.
    type Action: PageAction;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self;

    /// The page is shown; `again` when it already was (the sidebar's row pressed again, a link
    /// into it). Called on the entity, so a page without the hook is never borrowed.
    fn shown(_this: &Entity<Self>, _again: bool, _window: &mut Window, _cx: &mut App) {}

    /// Another page is shown in its place.
    fn hidden(_this: &Entity<Self>, _window: &mut Window, _cx: &mut App) {}

    /// The page has a level to step out of (or into) going `direction`, before the pages visited:
    /// a game in Game history, a broadcast's game.
    fn can_step(&self, _direction: Direction, _cx: &App) -> bool {
        false
    }

    fn step(&mut self, _direction: Direction, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// Its own search fields, which Find focuses when one is drawn.
    fn search_fields(&self) -> Vec<Entity<InputState>> {
        Vec::new()
    }

    /// Zen mode applies to what the page shows now: only the game and its panel stay.
    fn zen(&self, _cx: &App) -> bool {
        false
    }

    /// Z turns zen mode on or off here now.
    fn zen_key(&self, cx: &App) -> bool {
        self.zen(cx)
    }

    /// The actions its buttons allow now.
    fn actions(&self, _cx: &App) -> Vec<Self::Action> {
        Vec::new()
    }

    /// Run one of its actions. Called on the entity: an action may reach another view first.
    fn act(_this: &Entity<Self>, _action: Self::Action, _window: &mut Window, _cx: &mut App) {}

    /// New (⌘N) on the page: its action for it, if it has one now.
    fn new_here(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        run_role(this, ActionRole::New, window, cx);
    }

    /// The places inside the page Quick Pick goes to, such as its tabs.
    fn places(_cx: &App) -> Vec<PageCommand> {
        Vec::new()
    }

    /// The app-wide action that opens the page, if one does (Settings' ⌘,).
    fn opened_by() -> Option<Box<dyn gpui_kit::Action>> {
        None
    }
}

/// Run the page's action with `role`, if it has one now.
fn run_role<V: PageView>(this: &Entity<V>, role: ActionRole, window: &mut Window, cx: &mut App) {
    let action = this
        .read(cx)
        .actions(cx)
        .into_iter()
        .find(|action| action.info().role == Some(role));
    if let Some(action) = action {
        V::act(this, action, window, cx);
    }
}

/// A page's tabs, as its row of pills and Quick Pick show them.
pub trait PageTab: Copy + PartialEq + 'static {
    /// Every tab, in order; the first is shown until another is chosen.
    const ALL: &'static [Self];
    /// Where the tab shown is kept.
    const KEY: &'static str;

    /// Its stored name, label and icon on the page, and its name and search words in Quick Pick.
    fn info(self) -> TabInfo;

    /// The tab shown; a tab saved by an earlier release falls back to the first.
    fn stored(cx: &App) -> Self {
        let first = Self::ALL[0];
        let stored = kchess_app::prefs::get::<String>(cx, Self::KEY, first.info().key.into());
        Self::ALL
            .iter()
            .copied()
            .find(|tab| tab.info().key == stored)
            .unwrap_or(first)
    }

    fn store(self, cx: &mut App) {
        kchess_app::prefs::set(cx, Self::KEY, self.info().key);
    }

    /// The tabs as pills, for `widgets::pills`.
    fn pills() -> Vec<kchess_ui::widgets::Pill> {
        Self::ALL
            .iter()
            .map(|tab| {
                let info = tab.info();
                kchess_ui::widgets::Pill {
                    icon: Some(kchess_ui::icon(info.icon)),
                    label: info.label.into(),
                    badge: None,
                    disabled: false,
                }
            })
            .collect()
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

pub struct TabInfo {
    pub key: &'static str,
    pub label: &'static str,
    /// Its name in Quick Pick, beside the page's.
    pub title: &'static str,
    pub icon: &'static str,
    pub keywords: &'static str,
}

/// The tabs of page `V` as Quick Pick's places (`{prefix}:{key}`), each kept, then shown by
/// `show` on the page.
pub fn tab_places<V: PageView, T: PageTab>(
    prefix: &'static str,
    show: fn(&mut V, T, &mut Window, &mut Context<V>),
) -> Vec<PageCommand> {
    T::ALL
        .iter()
        .map(|&tab| {
            let info = tab.info();
            PageCommand {
                info: ActionInfo::new(
                    format!("{prefix}:{}", info.key),
                    info.title,
                    info.icon,
                    info.keywords,
                ),
                detail: Some(V::INFO.label.into()),
                // Kept first, so a page made or shown now opens at it.
                run: Rc::new(move |window, cx| {
                    tab.store(cx);
                    crate::shell::show::<V>(window, cx, move |page, window, cx| {
                        show(page, tab, window, cx)
                    })
                }),
            }
        })
        .collect()
}

/// A page made, as the shell drives it whatever its type.
trait AnyPage {
    fn view(&self) -> AnyView;
    fn focus(&self, cx: &App) -> FocusHandle;
    fn shown(&self, again: bool, window: &mut Window, cx: &mut App);
    fn hidden(&self, window: &mut Window, cx: &mut App);
    fn can_step(&self, direction: Direction, cx: &App) -> bool;
    fn step(&self, direction: Direction, window: &mut Window, cx: &mut App);
    fn search_fields(&self, cx: &App) -> Vec<Entity<InputState>>;
    fn zen(&self, cx: &App) -> bool;
    fn zen_key(&self, cx: &App) -> bool;
    fn commands(&self, cx: &App) -> Vec<PageCommand>;
    fn run_role(&self, role: ActionRole, window: &mut Window, cx: &mut App);
    fn has_role(&self, role: ActionRole, cx: &App) -> bool;
    fn new_here(&self, window: &mut Window, cx: &mut App);
}

impl<V: PageView> AnyPage for Entity<V> {
    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn focus(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn shown(&self, again: bool, window: &mut Window, cx: &mut App) {
        V::shown(self, again, window, cx);
    }

    fn hidden(&self, window: &mut Window, cx: &mut App) {
        V::hidden(self, window, cx);
    }

    fn can_step(&self, direction: Direction, cx: &App) -> bool {
        self.read(cx).can_step(direction, cx)
    }

    fn step(&self, direction: Direction, window: &mut Window, cx: &mut App) {
        self.update(cx, |page, cx| page.step(direction, window, cx));
    }

    fn search_fields(&self, cx: &App) -> Vec<Entity<InputState>> {
        self.read(cx).search_fields()
    }

    fn zen(&self, cx: &App) -> bool {
        self.read(cx).zen(cx)
    }

    fn zen_key(&self, cx: &App) -> bool {
        self.read(cx).zen_key(cx)
    }

    fn commands(&self, cx: &App) -> Vec<PageCommand> {
        let page = self.downgrade();
        self.read(cx)
            .actions(cx)
            .into_iter()
            .map(|action| {
                let page = page.clone();
                PageCommand {
                    info: action.info(),
                    detail: None,
                    run: Rc::new(move |window, cx| {
                        if let Some(page) = page.upgrade() {
                            V::act(&page, action.clone(), window, cx);
                        }
                    }),
                }
            })
            .collect()
    }

    fn run_role(&self, role: ActionRole, window: &mut Window, cx: &mut App) {
        run_role(self, role, window, cx);
    }

    fn has_role(&self, role: ActionRole, cx: &App) -> bool {
        self.read(cx)
            .actions(cx)
            .iter()
            .any(|action| action.info().role == Some(role))
    }

    fn new_here(&self, window: &mut Window, cx: &mut App) {
        V::new_here(self, window, cx);
    }
}

/// A page as the registry makes it (`register`).
pub struct Registered {
    info: PageInfo,
    create: fn(&mut Window, &mut App) -> Rc<dyn AnyPage>,
    places: fn(&App) -> Vec<PageCommand>,
    opened_by: fn() -> Option<Box<dyn gpui_kit::Action>>,
}

impl Registered {
    /// The page it makes.
    pub const fn page(&self) -> Page {
        self.info.page
    }
}

pub const fn register<V: PageView>() -> Registered {
    Registered {
        info: V::INFO,
        create: create::<V>,
        places: V::places,
        opened_by: V::opened_by,
    }
}

fn create<V: PageView>(window: &mut Window, cx: &mut App) -> Rc<dyn AnyPage> {
    Rc::new(cx.new(|cx| V::create(window, cx)))
}

/// The app's pages (`register`), each at its `Page`'s place: set once as the app starts, before
/// anything asks about a page.
static REGISTRY: OnceLock<&'static [Registered]> = OnceLock::new();

/// Register the app's pages, `registry[page as usize]` being `page`'s; later calls change nothing.
pub fn set_registry(registry: &'static [Registered]) {
    let _ = REGISTRY.set(registry);
}

fn registry() -> &'static [Registered] {
    match REGISTRY.get() {
        Some(registry) => registry,
        None => panic!("the app registers its pages as it starts (`page::set_registry`)"),
    }
}

/// Quick Pick's places on every page.
pub fn places(cx: &App) -> Vec<PageCommand> {
    registry()
        .iter()
        .flat_map(|registered| (registered.places)(cx))
        .collect()
}

/// The app-wide action that opens `page`, if one does.
pub fn opened_by(page: Page) -> Option<Box<dyn gpui_kit::Action>> {
    (registry()[page as usize].opened_by)()
}

pub struct Pages {
    current: Page,
    /// The current page, made: what the window draws.
    shown: Rc<dyn AnyPage>,
    views: HashMap<Page, Rc<dyn AnyPage>>,
}

impl Pages {
    pub fn new(window: &mut Window, cx: &mut App) -> Pages {
        let home = (registry()[Page::Dashboard as usize].create)(window, cx);
        window.focus(&home.focus(cx), cx);
        Pages {
            current: Page::Dashboard,
            shown: home.clone(),
            views: HashMap::from([(Page::Dashboard, home)]),
        }
    }

    pub fn current(&self) -> Page {
        self.current
    }

    fn made(&mut self, page: Page, window: &mut Window, cx: &mut App) -> Rc<dyn AnyPage> {
        self.views
            .entry(page)
            .or_insert_with(|| (registry()[page as usize].create)(window, cx))
            .clone()
    }

    /// Show a page, creating it the first time, with its keys going to it.
    fn show(&mut self, page: Page, window: &mut Window, cx: &mut App) {
        self.current = page;
        self.shown = self.made(page, window, cx);
        window.focus(&self.shown.focus(cx), cx);
    }

    /// Show `page` in place of the current one, as the sidebar does: the page left is told, then
    /// the page shown.
    pub fn choose(&mut self, page: Page, window: &mut Window, cx: &mut App) {
        let current = self.current;
        if page != current
            && let Some(left) = self.views.get(&current).cloned()
        {
            left.hidden(window, cx);
        }
        self.show(page, window, cx);
        if let Some(shown) = self.views.get(&page).cloned() {
            shown.shown(page == current, window, cx);
        }
    }

    /// Make the page now, without showing it.
    pub fn prepare(&mut self, page: Page, window: &mut Window, cx: &mut App) {
        self.made(page, window, cx);
    }

    /// The page itself (or its board) has the keys, not one of its text fields or menus.
    pub fn page_focused(&self, window: &Window, cx: &App) -> bool {
        self.views
            .get(&self.current)
            .is_some_and(|shown| kchess_board::board::page_has_keys(&shown.focus(cx), window, cx))
    }

    /// Page `V`'s view, once it has been made.
    pub fn get<V: PageView>(&self) -> Option<Entity<V>> {
        self.views.get(&V::INFO.page)?.view().downcast().ok()
    }

    /// `page`'s view, made now if it has not been.
    pub fn made_view(&mut self, page: Page, window: &mut Window, cx: &mut App) -> AnyView {
        self.made(page, window, cx).view()
    }

    /// A page's view, if it has been made.
    pub fn view_for(&self, page: Page) -> Option<AnyView> {
        self.views.get(&page).map(|shown| shown.view())
    }

    pub fn view(&self) -> AnyView {
        self.shown.view()
    }

    fn current_page(&self) -> Option<&Rc<dyn AnyPage>> {
        self.views.get(&self.current)
    }

    /// The current page has a level to step out of going `direction`.
    pub fn can_step(&self, direction: Direction, cx: &App) -> bool {
        self.current_page()
            .is_some_and(|page| page.can_step(direction, cx))
    }

    pub fn step(&self, direction: Direction, window: &mut Window, cx: &mut App) {
        if let Some(page) = self.current_page().cloned() {
            page.step(direction, window, cx);
        }
    }

    /// The current page's focus and its search fields.
    pub fn search_fields(&self, cx: &App) -> Option<(FocusHandle, Vec<Entity<InputState>>)> {
        let page = self.current_page()?;
        Some((page.focus(cx), page.search_fields(cx)))
    }

    /// Zen mode applies to the current page now.
    pub fn zen(&self, cx: &App) -> bool {
        self.current_page().is_some_and(|page| page.zen(cx))
    }

    /// Z turns zen mode on or off on the current page now.
    pub fn zen_key(&self, cx: &App) -> bool {
        self.current_page().is_some_and(|page| page.zen_key(cx))
    }

    /// The current page's actions, as Quick Pick offers them.
    pub fn commands(&self, cx: &App) -> Vec<PageCommand> {
        self.current_page()
            .map(|page| page.commands(cx))
            .unwrap_or_default()
    }

    /// The current page, to act on outside the workspace's update (its action may open
    /// another page).
    pub fn current_handle(&self) -> Option<PageHandle> {
        self.current_page().cloned().map(PageHandle)
    }
}

/// A page made, held to act on.
pub struct PageHandle(Rc<dyn AnyPage>);

impl PageHandle {
    /// Run the page's action with `role`, if it has one now.
    pub fn run_role(&self, role: ActionRole, window: &mut Window, cx: &mut App) {
        self.0.run_role(role, window, cx);
    }

    /// The page has an action with `role` now.
    pub fn has_role(&self, role: ActionRole, cx: &App) -> bool {
        self.0.has_role(role, cx)
    }

    pub fn new_here(&self, window: &mut Window, cx: &mut App) {
        self.0.new_here(window, cx);
    }
}

/// A key a page takes while no text field has the keys: the key names it matches, how the
/// shortcuts list shows them, and what it does there.
pub struct PageKey<A> {
    pub action: A,
    pub keys: &'static [&'static str],
    pub shown: &'static str,
    pub does: &'static str,
}

/// The action of `key` among a page's `keys`.
pub fn page_key<A: Copy>(keys: &[PageKey<A>], key: &str) -> Option<A> {
    keys.iter()
        .find(|entry| entry.keys.contains(&key))
        .map(|entry| entry.action)
}

gpui_kit::actions!(
    kchess,
    [
        /// A new game on the page shown (⌘N): a page's `ActionRole::New` action.
        New
    ]
);

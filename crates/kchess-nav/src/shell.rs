//! How a page reaches the app's shell without naming it: open another page, look one up, step back
//! or forward. The app's workspace answers (`Shell`), set as a global for its window.

use std::rc::Rc;

use gpui_kit::*;

use crate::page::{Page, PageView};
use crate::swipe::Direction;

/// The top bar's height, which pages lay out under.
pub const TOPBAR_HEIGHT: Pixels = px(52.);

/// The app's shell as pages reach it.
pub trait Shell {
    /// Open `page` as the sidebar would.
    fn open(&self, page: Page, window: &mut Window, cx: &mut App);
    /// The page shown.
    fn current(&self, cx: &App) -> Option<Page>;
    /// `page`'s view, once it has been made.
    fn view(&self, page: Page, cx: &App) -> Option<AnyView>;
    /// `page`'s view, made now (without showing it) if it has not been.
    fn made(&self, page: Page, window: &mut Window, cx: &mut App) -> Option<AnyView>;
    /// Back (-1) or forward (1): out of the page's inner level, then through the pages visited.
    fn go(&self, direction: Direction, window: &mut Window, cx: &mut App);
}

/// The window's shell (`cx.set_global(Navigator(Rc::new(…)))`).
#[derive(Clone)]
pub struct Navigator(pub Rc<dyn Shell>);

impl Global for Navigator {}

fn shell(cx: &App) -> Option<Rc<dyn Shell>> {
    cx.try_global::<Navigator>()
        .map(|Navigator(shell)| shell.clone())
}

/// Open `page` as the sidebar would.
pub fn open(page: Page, window: &mut Window, cx: &mut App) {
    if let Some(shell) = shell(cx) {
        shell.open(page, window, cx);
    }
}

/// The page shown.
pub fn current_page(cx: &App) -> Option<Page> {
    shell(cx)?.current(cx)
}

/// Back (-1) or forward (1): out of the page's inner level, then through the pages visited, as
/// a swipe goes.
pub fn go(direction: Direction, window: &mut Window, cx: &mut App) {
    if let Some(shell) = shell(cx) {
        shell.go(direction, window, cx);
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
    shell(cx)?.view(V::INFO.page, cx)?.downcast().ok()
}

/// Page `V`'s view, made now (without showing it) if it has not been.
pub fn made<V: PageView>(window: &mut Window, cx: &mut App) -> Option<Entity<V>> {
    shell(cx)?.made(V::INFO.page, window, cx)?.downcast().ok()
}

/// Run `show`, which opens a page, for something asked when the keys were at `asked`, before an
/// answer the person need not have waited for. When they have given the keys elsewhere since
/// (Quick Pick opened again, a field clicked), the page opens behind and leaves the keys there:
/// taking them closed Quick Pick under the person's search. A dialog `show` opens keeps them.
pub fn keeping_keys_since(
    asked: Option<FocusHandle>,
    window: &mut Window,
    cx: &mut App,
    show: impl FnOnce(&mut Window, &mut App),
) {
    use gpui_kit::component::WindowExt as _;
    let now = window.focused(cx);
    show(window, cx);
    if let Some(now) = now.filter(|now| asked.as_ref() != Some(now))
        && !window.has_active_dialog(cx)
    {
        window.focus(&now, cx);
    }
}

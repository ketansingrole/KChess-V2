//! What the board's tests and the pages' share: the app as a board page needs it.

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Context, IntoElement, Render, Window};
use kchess_app::tests::TestCore;

/// The app as a board page needs it: a core of the test's own, Settings, the theme and voice,
/// and a window to put the page's views in.
pub struct Started {
    cx: gpui_kit::TestAppContext,
    window: AnyWindowHandle,
    _core: TestCore,
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div()
    }
}

impl Started {
    pub fn new(cx: &mut gpui_kit::TestAppContext) -> Started {
        // The core answers on its Tokio threads; the test waits for them in real time.
        cx.executor().allow_parking();
        let core = cx.update(|cx| {
            gpui_kit::init(cx);
            let core = kchess_app::tests::start_core(cx);
            kchess_app::settings::init_for_tests(cx);
            let colors = kchess_ui::theme::colors("kchess", &[], false);
            cx.set_global(kchess_ui::theme::Palette::of(&colors, false));
            crate::voice::init(cx);
            core
        });
        let window = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|_| Blank))
                .expect("a window")
                .0
        });
        Started {
            cx: cx.clone(),
            window,
            _core: core,
        }
    }

    /// Work in the window, as a page's views are made and changed.
    pub fn with_window<R>(&mut self, work: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx
            .update_window(self.window, |_, window, cx| work(window, cx))
            .expect("the window")
    }

    /// The test's app, to run its tasks or read its state.
    pub fn cx(&mut self) -> &mut gpui_kit::TestAppContext {
        &mut self.cx
    }
}

//! The window's frame, as the Electron app draws it (`AppWorkspace.vue`): a sidebar that collapses
//! to icons, a top bar, and the current page.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;

use crate::pages::{Page, Pages};

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
pub struct Workspace {
    sidebar_open: bool,
    pages: Pages,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Workspace {
        Workspace {
            sidebar_open: true,
            pages: Pages::new(window, cx),
        }
    }

    fn choose(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.pages.show(page, window, cx);
        cx.notify();
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
            .child(
                SidebarMenu::new().children(nav().into_iter().map(|(page, label, glyph)| {
                    SidebarMenuItem::new(label)
                        .icon(glyph)
                        .active(current == page)
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.choose(page, window, cx)),
                        )
                })),
            )
            .footer(
                SidebarFooter::new().child(
                    Button::new("settings")
                        .ghost()
                        .w_full()
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
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_sidebar(cx))
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

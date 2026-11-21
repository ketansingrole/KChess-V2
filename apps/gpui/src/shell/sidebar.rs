//! The sidebar: the pages in their groups, the footer with the account menu and Settings,
//! the toggle that opens it or narrows it to a rail of icons, and the Lichess game's mark.

use super::*;

/// A row of the sidebar's navigation: 32px tall, an 18px icon 8px from its label.
pub(super) fn nav_item(label: impl Into<SharedString>, glyph: Icon) -> SidebarMenuItem {
    SidebarMenuItem::new(label)
        .icon(glyph.size(NAV_ICON))
        .min_h(px(32.))
        .py(px(6.))
        .px(NAV_INSET)
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
pub(super) enum NavBlock {
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

/// What the sidebar shows (`render_sidebar`): it is drawn again when one of these changes.
#[derive(PartialEq)]
pub(super) struct SidebarInputs {
    open: bool,
    width_from: Pixels,
    current: Page,
    section: Option<Section>,
    back: Page,
    game_mark: Option<GameMark>,
    account: crate::account_menu::MenuInputs,
}

impl SidebarInputs {
    pub(super) fn of(workspace: &Workspace, cx: &App) -> SidebarInputs {
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
            account: crate::account_menu::MenuInputs::of(cx),
        }
    }
}

impl Workspace {
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

    /// Whether the sidebar is open as shown: in a narrow window, only while opened there.
    pub(super) fn shown(&self) -> bool {
        if self.narrow {
            self.narrow_open
        } else {
            self.sidebar_open
        }
    }

    /// Open or close the sidebar; `ease` widens or narrows it over `SIDEBAR_EASE` (a click on its
    /// toggle, where the motion shows where it went) instead of at once.
    pub(super) fn toggle_sidebar(&mut self, ease: bool, cx: &mut Context<Self>) {
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
    pub(super) fn fit_window(&mut self, window: &Window) {
        let narrow = window.viewport_size().width < NARROW_WINDOW;
        if narrow != self.narrow {
            self.narrow = narrow;
            self.narrow_open = false;
            self.sidebar_width_from = sidebar_width(self.shown());
        }
    }

    /// The sidebar: the header row the toggle floats over, the pages, and the footer, divided
    /// by lines; 256px open, a 64px rail of icons collapsed,
    /// its width easing between the two in 200ms.
    pub(super) fn render_sidebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            .when(!mac, |sidebar| sidebar.border_r(SIDEBAR_BORDER))
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
                    .when(mac, |rest| rest.border_r(SIDEBAR_BORDER))
                    .child(
                        v_flex()
                            .id("sidebar-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.sidebar_scroll)
                            .pl(SIDEBAR_INSET)
                            .pr(SIDEBAR_INSET - SIDEBAR_BORDER)
                            .py_2()
                            .gap_1()
                            .children(blocks),
                    )
                    .child(
                        v_flex()
                            .flex_none()
                            .pl(SIDEBAR_INSET)
                            .pr(SIDEBAR_INSET - SIDEBAR_BORDER)
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
                        .child(crate::account_menu::menu_button(true, cx)),
                )
                .child(settings)
                .into_any_element()
        } else {
            v_flex()
                .child(settings)
                .child(crate::account_menu::menu_button(false, cx))
                .into_any_element()
        }
    }

    /// Whether the sidebar's width has finished easing since it last opened or closed.
    fn sidebar_settled(&self) -> bool {
        self.sidebar_toggled
            .is_none_or(|toggled| toggled.elapsed() > SIDEBAR_EASE + Duration::from_millis(50))
    }

    /// The sidebar as its `Part` once settled, at its width; while it eases, drawn here.
    pub(super) fn sidebar_element(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.sidebar_settled() {
            return self.render_sidebar(window, cx).into_any_element();
        }
        let mut style = StyleRefinement::default();
        style.size.width = Some(sidebar_width(self.shown()).into());
        style.size.height = Some(relative(1.).into());
        style.flex_shrink = Some(0.);
        self.sidebar.clone().cached(style).into_any_element()
    }

    /// The sidebar toggle: one button at the same place in the window
    /// whether the sidebar is open or a rail, beside the macOS traffic lights (which ignore the
    /// interface size, so its offset does too), else at the window's edge.
    pub(super) fn render_sidebar_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .size(TOGGLE_SIDE)
                    .tooltip_with_action(label, &crate::menu::ToggleSidebar, None)
                    .accessibility_label(label)
                    .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                        this.toggle_sidebar(!event.is_keyboard(), cx)
                    })),
            ))
            .when(self.shown(), |wrap| {
                // Over the sidebar's header, which answers its presses.
                wrap.child(
                    drag_area("sidebar-name")
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
}

/// The Lichess game the sidebar marks: your move (in a live game, or in a
/// correspondence one), or a game going on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GameMark {
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
        crate::game::view::turn(crate::game::view::position_at(setup, &moves, moves.len()).as_ref())
            == online.color()
    };
    let correspondence_turn = online
        .ongoing
        .iter()
        .any(|game| game.correspondence() && game.is_my_turn);
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

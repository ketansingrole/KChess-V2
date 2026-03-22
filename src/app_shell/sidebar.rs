use gpui::{Context, MouseButton, Render, Window, div, prelude::*, px, rgb, rgba, svg};

use crate::pages::{render_play_with_computer_page, render_settings_page};
use crate::ui::top_bar::render_top_bar;

use super::{
    ActivePane, BOARD_CELL_PX, ICON_SEARCH, KChessApp, NavigationState, SEARCH_BOX_HEIGHT,
    SIDEBAR_WIDTH, TAB_OPTIONS, TOP_CHROME_HEIGHT,
};

fn sidebar_item(
    icon_path: &'static str,
    label: &'static str,
    active: bool,
    focused: bool,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(30.0))
        .px_2()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .text_color(rgb(0x2b2f35))
        .hover(|this| this.bg(rgba(0xffffff64)))
        .when(active, |this| {
            this.bg(rgba(0xffffffb2))
                .border_1()
                .border_color(rgba(0xcfd4dcff))
        })
        .when(focused, |this| {
            this.border_1()
                .border_color(rgb(0x3f7fe5))
                .bg(rgba(0xeaf1fdff))
        })
        .child(
            svg()
                .path(icon_path)
                .w(px(14.0))
                .h(px(14.0))
                .text_color(rgb(0x4f545c)),
        )
        .child(div().flex_1().child(label))
}

impl Render for KChessApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let search_query = self.current_search_query(cx);
        if search_query != self.navigation.last_search_query {
            self.navigation.reset_selection_for_query(&search_query);
        }

        let show_play_with_computer = self.navigation.active_pane == ActivePane::PlayWithComputer;
        let window_size = window.bounds().size;
        let window_width = f32::from(window_size.width);
        let window_height = f32::from(window_size.height);
        let board_size = BOARD_CELL_PX * 8.0;
        let right_width = (window_width - SIDEBAR_WIDTH).max(board_size);
        let main_height = (window_height - TOP_CHROME_HEIGHT).max(board_size);
        let board_left = ((right_width - board_size) / 2.0).max(0.0);
        let board_top = ((main_height - board_size) / 2.0).max(0.0);

        let shortcut_hint = if self.cmd_held { "⌘ + F" } else { "" };
        let suggestions = NavigationState::matching_tabs_for_query(&search_query);
        let search_has_focus = self.search_input.read(cx).is_focused(window);
        let show_suggestions = self.navigation.search_active || search_has_focus;
        let play_focused = self.play_focus.is_focused(window);
        let settings_focused = self.settings_focus.is_focused(window);
        let view = cx.entity();

        div()
            .size_full()
            .track_focus(&self.root_focus)
            .on_any_mouse_down(cx.listener(Self::on_root_mouse_down))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_action(cx.listener(Self::focus_search_action))
            .on_action(cx.listener(Self::open_settings_action))
            .on_action(cx.listener(Self::open_play_action))
            .on_action(cx.listener(Self::search_up_action))
            .on_action(cx.listener(Self::search_down_action))
            .on_action(cx.listener(Self::search_confirm_action))
            .on_action(cx.listener(Self::search_cancel_action))
            .on_action(cx.listener(Self::show_help_action))
            .relative()
            .flex()
            .flex_col()
            .bg(rgba(0xf4f5f7e8))
            .child(render_top_bar("KChess", TOP_CHROME_HEIGHT))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .child(
                        div()
                            .w(px(SIDEBAR_WIDTH))
                            .h_full()
                            .p_3()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .bg(rgba(0xf2f3f4d9))
                            .border_r_1()
                            .border_color(rgba(0xd9dce2ff))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .on_any_mouse_down(
                                                cx.listener(Self::on_search_region_mouse_down),
                                            )
                                            .h(px(SEARCH_BOX_HEIGHT))
                                            .w_full()
                                            .px_2()
                                            .rounded_md()
                                            .bg(rgba(0xffffffff))
                                            .border_1()
                                            .border_color(rgba(0xcfd4dcff))
                                            .focus(|style| style.border_color(rgb(0x3f7fe5)))
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(Self::on_search_click),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .flex_1()
                                                    .child(
                                                        svg()
                                                            .path(ICON_SEARCH)
                                                            .w(px(12.0))
                                                            .h(px(12.0))
                                                            .text_color(rgb(0x6b727b)),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .child(self.search_input.clone()),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(0x7a828d))
                                                    .child(shortcut_hint),
                                            ),
                                    )
                                    .when(show_suggestions, |this| {
                                        this.child(
                                            div()
                                                .key_context("SearchOverlay")
                                                .on_any_mouse_down(
                                                    cx.listener(Self::on_search_region_mouse_down),
                                                )
                                                .rounded_md()
                                                .bg(rgba(0xffffffff))
                                                .border_1()
                                                .border_color(rgba(0xd5d8deff))
                                                .shadow_lg()
                                                .p_2()
                                                .when(suggestions.is_empty(), |this| {
                                                    this.child(
                                                        div()
                                                            .px_2()
                                                            .py_2()
                                                            .text_sm()
                                                            .text_color(rgb(0x666b73))
                                                            .child("No related tabs"),
                                                    )
                                                })
                                                .children(suggestions.into_iter().enumerate().map(
                                                    |(ix, tab)| {
                                                        let selected =
                                                            ix == self.navigation.search_selection;
                                                        let pane = tab.pane;
                                                        let view = view.clone();

                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap_1()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded_sm()
                                                            .cursor_pointer()
                                                            .hover(|this| this.bg(rgba(0xf0f2f5ff)))
                                                            .when(selected, |this| {
                                                                this.bg(rgba(0xe7edf7ff))
                                                            })
                                                            .on_mouse_up(
                                                                MouseButton::Left,
                                                                move |_, window, app| {
                                                                    view.update(app, |view, cx| {
                                                                        view.select_tab(
                                                                            pane, true, window, cx,
                                                                        );
                                                                    });
                                                                },
                                                            )
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .items_center()
                                                                    .gap_2()
                                                                    .text_sm()
                                                                    .text_color(rgb(0x23262b))
                                                                    .child(
                                                                        svg()
                                                                            .path(tab.icon_path)
                                                                            .w(px(12.0))
                                                                            .h(px(12.0))
                                                                            .text_color(rgb(
                                                                                0x565c65,
                                                                            )),
                                                                    )
                                                                    .child(tab.label),
                                                            )
                                                    },
                                                )),
                                        )
                                    })
                                    .child(
                                        div().flex().flex_col().gap_1().pt_1().child(
                                            div()
                                                .track_focus(&self.play_focus)
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(Self::show_play_with_computer),
                                                )
                                                .child(sidebar_item(
                                                    TAB_OPTIONS[0].icon_path,
                                                    TAB_OPTIONS[0].label,
                                                    show_play_with_computer,
                                                    play_focused,
                                                )),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .track_focus(&self.settings_focus)
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(Self::show_settings),
                                    )
                                    .child(sidebar_item(
                                        TAB_OPTIONS[1].icon_path,
                                        TAB_OPTIONS[1].label,
                                        !show_play_with_computer,
                                        settings_focused,
                                    )),
                            ),
                    )
                    .child(
                        div().flex_1().relative().child(
                            div()
                                .size_full()
                                .bg(rgba(0xffffffdb))
                                .when(show_play_with_computer, |this| {
                                    this.child(render_play_with_computer_page(
                                        self.board_view.clone(),
                                        board_left,
                                        board_top,
                                    ))
                                })
                                .when(!show_play_with_computer, |this| {
                                    this.child(render_settings_page())
                                }),
                        ),
                    ),
            )
    }
}

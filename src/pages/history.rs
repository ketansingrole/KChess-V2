use gpui::{
    Context, IntoElement, MouseButton, MouseDownEvent, Render, Window, deferred, div, prelude::*,
    px, svg,
};
use time::OffsetDateTime;

use crate::{
    lichess_state::LichessState,
    storage::LichessGame,
    theme::{ThemePalette, ThemeState},
    ui::primitives::{action_button, compact_chip, dropdown_trigger},
};

const PAGE_SIZES: [usize; 5] = [10, 20, 30, 40, 50];
const ICON_CHEVRON_DOWN: &str = "assets/icons/chevron-down.svg";
const ICON_CHECK: &str = "assets/icons/check.svg";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResultFilter {
    All,
    Win,
    Loss,
    Draw,
}

impl ResultFilter {
    fn all() -> [Self; 4] {
        [Self::All, Self::Win, Self::Loss, Self::Draw]
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Win => "Wins",
            Self::Loss => "Losses",
            Self::Draw => "Draws",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RatedFilter {
    All,
    Rated,
    Casual,
}

impl RatedFilter {
    fn all() -> [Self; 3] {
        [Self::All, Self::Rated, Self::Casual]
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Rated => "Rated",
            Self::Casual => "Casual",
        }
    }
}

#[derive(Clone, Debug)]
struct HistoryFiltersState {
    result_filter: ResultFilter,
    rated_filter: RatedFilter,
    account_filters: Vec<String>,
    account_filters_initialized: bool,
    account_dropdown_open: bool,
    ignore_next_outside_account_click: bool,
    page_size: usize,
    current_page: usize,
}

impl Default for HistoryFiltersState {
    fn default() -> Self {
        Self {
            result_filter: ResultFilter::All,
            rated_filter: RatedFilter::All,
            account_filters: Vec::new(),
            account_filters_initialized: false,
            account_dropdown_open: false,
            ignore_next_outside_account_click: false,
            page_size: PAGE_SIZES[1],
            current_page: 0,
        }
    }
}

pub struct HistoryPage {
    lichess_state: gpui::Entity<LichessState>,
    theme_state: gpui::Entity<ThemeState>,
    filters: HistoryFiltersState,
}

impl HistoryPage {
    pub fn new(
        _cx: &mut Context<Self>,
        lichess_state: gpui::Entity<LichessState>,
        theme_state: gpui::Entity<ThemeState>,
    ) -> Self {
        Self {
            lichess_state,
            theme_state,
            filters: HistoryFiltersState::default(),
        }
    }

    fn set_result_filter(&mut self, filter: ResultFilter, cx: &mut Context<Self>) {
        if self.filters.result_filter != filter {
            self.filters.result_filter = filter;
            self.filters.current_page = 0;
            cx.notify();
        }
    }

    fn set_rated_filter(&mut self, filter: RatedFilter, cx: &mut Context<Self>) {
        if self.filters.rated_filter != filter {
            self.filters.rated_filter = filter;
            self.filters.current_page = 0;
            cx.notify();
        }
    }

    fn set_page_size(&mut self, page_size: usize, cx: &mut Context<Self>) {
        if self.filters.page_size != page_size {
            self.filters.page_size = page_size;
            self.filters.current_page = 0;
            cx.notify();
        }
    }

    fn toggle_account_dropdown(&mut self, cx: &mut Context<Self>) {
        self.filters.account_dropdown_open = !self.filters.account_dropdown_open;
        cx.notify();
    }

    fn close_account_dropdown(&mut self, cx: &mut Context<Self>) {
        if self.filters.account_dropdown_open {
            self.filters.account_dropdown_open = false;
            cx.notify();
        }
    }

    fn on_root_mouse_down(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.filters.account_dropdown_open {
            self.filters.ignore_next_outside_account_click = false;
            return;
        }

        if self.filters.ignore_next_outside_account_click {
            self.filters.ignore_next_outside_account_click = false;
            return;
        }

        self.close_account_dropdown(cx);
    }

    fn on_account_region_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.filters.ignore_next_outside_account_click = true;
    }

    fn toggle_all_account_filters(&mut self, account_options: &[String], cx: &mut Context<Self>) {
        let all_selected = !account_options.is_empty()
            && self.filters.account_filters.len() == account_options.len()
            && account_options.iter().all(|username| {
                self.filters
                    .account_filters
                    .iter()
                    .any(|selected| selected.eq_ignore_ascii_case(username))
            });

        if all_selected {
            self.filters.account_filters.clear();
        } else {
            self.filters.account_filters = account_options.to_vec();
        }
        self.filters.current_page = 0;
        cx.notify();
    }

    fn toggle_account_filter(&mut self, username: &str, cx: &mut Context<Self>) {
        if let Some(index) = self
            .filters
            .account_filters
            .iter()
            .position(|selected| selected.eq_ignore_ascii_case(username))
        {
            self.filters.account_filters.remove(index);
        } else {
            self.filters.account_filters.push(username.to_string());
        }
        self.filters.current_page = 0;
        cx.notify();
    }

    fn go_to_previous_page(&mut self, cx: &mut Context<Self>) {
        if self.filters.current_page > 0 {
            self.filters.current_page -= 1;
            cx.notify();
        }
    }

    fn go_to_next_page(&mut self, total_pages: usize, cx: &mut Context<Self>) {
        if self.filters.current_page + 1 < total_pages {
            self.filters.current_page += 1;
            cx.notify();
        }
    }
}

fn pill(label: impl IntoElement, bg: gpui::Rgba, fg: gpui::Rgba) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_full()
        .bg(bg)
        .text_xs()
        .text_color(fg)
        .child(label)
}

fn toolbar_group(
    title: &'static str,
    body: impl IntoElement,
    palette: ThemePalette,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().text_xs().text_color(palette.text_muted).child(title))
        .child(body)
}

fn checkbox_menu_item(
    label: impl IntoElement,
    checked: bool,
    palette: ThemePalette,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|this| this.bg(palette.surface_hover))
        .on_mouse_up(MouseButton::Left, move |_, window, app| {
            on_click(window, app);
        })
        .child(
            div()
                .w(px(14.0))
                .h(px(14.0))
                .rounded_sm()
                .border_1()
                .border_color(if checked {
                    palette.accent
                } else {
                    palette.input_border
                })
                .bg(if checked {
                    palette.accent_bg
                } else {
                    palette.input_bg
                })
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |this| {
                    this.child(
                        svg()
                            .path(ICON_CHECK)
                            .w(px(10.0))
                            .h(px(10.0))
                            .text_color(palette.accent_text),
                    )
                }),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette.text_secondary)
                .child(label),
        )
}

fn game_result(game: &LichessGame) -> ResultFilter {
    match game.winner.as_deref() {
        Some("white") if game.color == "white" => ResultFilter::Win,
        Some("black") if game.color == "black" => ResultFilter::Win,
        Some(_) => ResultFilter::Loss,
        None => ResultFilter::Draw,
    }
}

fn result_badge(game: &LichessGame, palette: ThemePalette) -> impl IntoElement {
    let (label, bg, fg) = match game_result(game) {
        ResultFilter::Win => (
            "Win",
            palette.status_success_bg,
            palette.status_success_text,
        ),
        ResultFilter::Loss => ("Loss", palette.status_error_bg, palette.status_error_text),
        ResultFilter::Draw | ResultFilter::All => {
            ("Draw", palette.surface_alt, palette.text_secondary)
        }
    };

    pill(label, bg, fg)
}

fn rating_diff_text(game: &LichessGame) -> String {
    match game.rating_diff {
        Some(diff) if diff > 0 => format!("+{diff}"),
        Some(diff) if diff < 0 => diff.to_string(),
        Some(_) => "±0".to_string(),
        None => "Unrated".to_string(),
    }
}

fn played_at_text(played_at_ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp(played_at_ms / 1_000)
        .map(|time| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02} UTC",
                time.year(),
                u8::from(time.month()),
                time.day(),
                time.hour(),
                time.minute()
            )
        })
        .unwrap_or_else(|_| "Unknown time".to_string())
}

fn opening_text(game: &LichessGame) -> String {
    game.opening_name
        .clone()
        .unwrap_or_else(|| "Opening unavailable".to_string())
}

fn history_row(game: &LichessGame, palette: ThemePalette) -> impl IntoElement {
    let row_bg = if game.color == "white" {
        palette.surface_alt
    } else {
        palette.surface_hover
    };

    div()
        .w_full()
        .px_4()
        .py_3()
        .rounded_lg()
        .bg(row_bg)
        .border_1()
        .border_color(palette.border_muted)
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(result_badge(game, palette))
                        .child(
                            div()
                                .text_sm()
                                .text_color(palette.text_primary)
                                .child(format!(
                                    "@{} vs {}",
                                    game.account_username, game.opponent_name
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(pill(
                            format!(
                                "{} {}",
                                game.speed,
                                if game.rated { "rated" } else { "casual" }
                            ),
                            palette.status_info_bg,
                            palette.status_info_text,
                        ))
                        .child(pill(
                            rating_diff_text(game),
                            palette.surface_alt,
                            palette.text_secondary,
                        )),
                ),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette.text_secondary)
                .child(opening_text(game)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.text_muted)
                        .child(format!(
                            "{} | {} | {}",
                            game.variant,
                            game.status,
                            played_at_text(game.played_at)
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.text_muted)
                        .child(format!(
                            "{} {}",
                            if game.color == "white" {
                                "White"
                            } else {
                                "Black"
                            },
                            game.player_rating
                                .map(|rating| rating.to_string())
                                .unwrap_or_else(|| "rating ?".to_string())
                        )),
                ),
        )
}

fn empty_state(message: &'static str, palette: ThemePalette) -> impl IntoElement {
    div()
        .h_full()
        .w_full()
        .min_h(px(240.0))
        .flex()
        .items_center()
        .justify_center()
        .px_6()
        .text_sm()
        .text_center()
        .text_color(palette.text_muted)
        .child(message)
}

impl Render for HistoryPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme_state.read(cx).palette();
        let (accounts, games, syncing, status_message, error_message) = {
            let state = self.lichess_state.read(cx);
            (
                state.accounts().to_vec(),
                state.games().to_vec(),
                state.syncing(),
                state.status_message().map(ToOwned::to_owned),
                state.error_message().map(ToOwned::to_owned),
            )
        };

        let account_options = accounts
            .iter()
            .map(|account| account.username.clone())
            .collect::<Vec<_>>();

        if !self.filters.account_filters_initialized && !account_options.is_empty() {
            self.filters.account_filters = account_options.clone();
            self.filters.account_filters_initialized = true;
        }

        self.filters.account_filters.retain(|selected| {
            account_options
                .iter()
                .any(|option| option.eq_ignore_ascii_case(selected))
        });
        let all_accounts_selected = !account_options.is_empty()
            && self.filters.account_filters.len() == account_options.len()
            && account_options.iter().all(|username| {
                self.filters
                    .account_filters
                    .iter()
                    .any(|selected| selected.eq_ignore_ascii_case(username))
            });

        let selected_account_label =
            if account_options.is_empty() || self.filters.account_filters.is_empty() {
                "No accounts".to_string()
            } else if all_accounts_selected {
                "All accounts".to_string()
            } else if self.filters.account_filters.len() == 1 {
                format!("@{}", self.filters.account_filters[0])
            } else {
                format!("{} selected", self.filters.account_filters.len())
            };

        let filtered_games = games
            .iter()
            .filter(|game| match self.filters.result_filter {
                ResultFilter::All => true,
                filter => game_result(game) == filter,
            })
            .filter(|game| match self.filters.rated_filter {
                RatedFilter::All => true,
                RatedFilter::Rated => game.rated,
                RatedFilter::Casual => !game.rated,
            })
            .filter(|game| {
                self.filters
                    .account_filters
                    .iter()
                    .any(|selected| game.account_username.eq_ignore_ascii_case(selected))
            })
            .collect::<Vec<_>>();

        let total_filtered = filtered_games.len();
        let total_pages = total_filtered.max(1).div_ceil(self.filters.page_size);
        if self.filters.current_page >= total_pages {
            self.filters.current_page = total_pages.saturating_sub(1);
        }
        let start_index = self.filters.current_page * self.filters.page_size;
        let end_index = (start_index + self.filters.page_size).min(total_filtered);
        let visible_games = if start_index < end_index {
            filtered_games[start_index..end_index].to_vec()
        } else {
            Vec::new()
        };

        let history_view = cx.entity();

        div()
            .size_full()
            .min_h(px(0.0))
            .on_any_mouse_down(cx.listener(Self::on_root_mouse_down))
            .child(
            div()
                .size_full()
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .gap_3()
                .p_5()
                .child(
                    div()
                        .rounded_xl()
                        .border_1()
                        .border_color(palette.border)
                        .bg(palette.surface)
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_lg().text_color(palette.text_primary).child("Game History"))
                                .child(pill(
                                    format!("{} account(s)", accounts.len()),
                                    palette.accent_bg,
                                    palette.accent_text,
                                ))
                                .child(pill(
                                    format!("{} filtered", total_filtered),
                                    palette.surface_alt,
                                    palette.text_secondary,
                                ))
                                .when(syncing, |this| {
                                    this.child(pill("Syncing", palette.accent_bg, palette.accent_text))
                                }),
                        )
                        .child(action_button("Sync now", syncing, palette, move |_, app| {
                            history_view.update(app, |view, cx| {
                                view.lichess_state.update(cx, |state, cx| state.sync_all(cx));
                            });
                        })),
                )
                .when(status_message.is_some(), |this| {
                    this.child(
                        div()
                            .px_1()
                            .text_sm()
                            .text_color(palette.status_info_text)
                            .child(status_message.unwrap_or_default()),
                    )
                })
                .when(error_message.is_some(), |this| {
                    this.child(
                        div()
                            .px_1()
                            .text_sm()
                            .text_color(palette.status_error_text)
                            .child(error_message.unwrap_or_default()),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .rounded_xl()
                        .border_1()
                        .border_color(palette.border)
                        .bg(palette.surface)
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex_none()
                                .px_4()
                                .py_2()
                                .border_b_1()
                                .border_color(palette.border_muted)
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_4()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(palette.text_secondary)
                                        .child("Recent games"),
                                )
                                .child(
                                    div().flex().items_center().gap_3().child(
                                        div()
                                            .text_xs()
                                            .text_color(palette.text_muted)
                                            .child(if total_filtered == 0 {
                                                "Showing 0".to_string()
                                            } else {
                                                format!(
                                                    "Showing {}-{} of {}",
                                                    start_index + 1,
                                                    end_index,
                                                    total_filtered
                                                )
                                            }),
                                    )
                                    .child(toolbar_group(
                                        "Per page",
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .children(PAGE_SIZES.into_iter().map(|page_size| {
                                                let view = cx.entity();
                                                compact_chip(
                                                    format!("{page_size}"),
                                                    self.filters.page_size == page_size,
                                                    true,
                                                    palette,
                                                    move |_, app| {
                                                        view.update(app, |view, cx| {
                                                            view.set_page_size(page_size, cx);
                                                        });
                                                    },
                                                )
                                            })),
                                        palette,
                                    ))
                                    .child(toolbar_group(
                                        "Page",
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child({
                                                let view = cx.entity();
                                                compact_chip(
                                                    "Prev",
                                                    false,
                                                    self.filters.current_page > 0,
                                                    palette,
                                                    move |_, app| {
                                                        view.update(app, |view, cx| {
                                                            view.go_to_previous_page(cx);
                                                        });
                                                    },
                                                )
                                            })
                                            .child(
                                                div()
                                                    .px_1()
                                                    .text_xs()
                                                    .text_color(palette.text_muted)
                                                    .child(format!(
                                                        "{} / {}",
                                                        self.filters.current_page + 1,
                                                        total_pages
                                                    )),
                                            )
                                            .child({
                                                let view = cx.entity();
                                                compact_chip(
                                                    "Next",
                                                    false,
                                                    self.filters.current_page + 1 < total_pages,
                                                    palette,
                                                    move |_, app| {
                                                        view.update(app, |view, cx| {
                                                            view.go_to_next_page(total_pages, cx);
                                                        });
                                                    },
                                                )
                                            }),
                                        palette,
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px_4()
                                .py_2()
                                .border_b_1()
                                .border_color(palette.border_muted)
                                .flex()
                                .items_center()
                                .gap_4()
                                .child(toolbar_group(
                                    "Result",
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .children(ResultFilter::all().into_iter().map(|filter| {
                                            let view = cx.entity();
                                            compact_chip(
                                                filter.label(),
                                                self.filters.result_filter == filter,
                                                true,
                                                palette,
                                                move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.set_result_filter(filter, cx);
                                                    });
                                                },
                                            )
                                        })),
                                    palette,
                                ))
                                .child(toolbar_group(
                                    "Type",
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .children(RatedFilter::all().into_iter().map(|filter| {
                                            let view = cx.entity();
                                            compact_chip(
                                                filter.label(),
                                                self.filters.rated_filter == filter,
                                                true,
                                                palette,
                                                move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.set_rated_filter(filter, cx);
                                                    });
                                                },
                                            )
                                        })),
                                    palette,
                                ))
                                .child(toolbar_group(
                                    "Account",
                                    div()
                                        .relative()
                                        .w(px(220.0))
                                        .on_any_mouse_down(
                                            cx.listener(Self::on_account_region_mouse_down),
                                        )
                                        .child({
                                            let view = cx.entity();
                                            dropdown_trigger(
                                                selected_account_label.clone(),
                                                palette,
                                                svg()
                                                    .path(ICON_CHEVRON_DOWN)
                                                    .w(px(12.0))
                                                    .h(px(12.0))
                                                    .text_color(palette.text_muted),
                                                move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.toggle_account_dropdown(cx);
                                                    });
                                                },
                                            )
                                        })
                                        .when(self.filters.account_dropdown_open, |this| {
                                            this.child(
                                                deferred(
                                                    div()
                                                        .id("history-account-filter-dropdown")
                                                        .on_any_mouse_down(
                                                            cx.listener(
                                                                Self::on_account_region_mouse_down,
                                                            ),
                                                        )
                                                        .absolute()
                                                        .left(px(0.0))
                                                        .top(px(38.0))
                                                        .w(px(220.0))
                                                        .max_h(px(240.0))
                                                        .overflow_y_scroll()
                                                        .scrollbar_width(px(8.0))
                                                        .rounded_lg()
                                                        .border_1()
                                                        .border_color(palette.border)
                                                        .bg(palette.overlay_bg)
                                                        .shadow_lg()
                                                        .p_2()
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .when(account_options.is_empty(), |this| {
                                                            this.child(
                                                                div()
                                                                    .px_2()
                                                                    .py_2()
                                                                    .text_sm()
                                                                    .text_color(palette.text_muted)
                                                                    .child("No accounts"),
                                                            )
                                                        })
                                                        .when(!account_options.is_empty(), |this| {
                                                            this.child({
                                                                let view = cx.entity();
                                                                let account_options =
                                                                    account_options.clone();
                                                                checkbox_menu_item(
                                                                    "All accounts",
                                                                    all_accounts_selected,
                                                                    palette,
                                                                    move |_, app| {
                                                                        view.update(
                                                                            app,
                                                                            |view, cx| {
                                                                                view.toggle_all_account_filters(
                                                                                    &account_options,
                                                                                    cx,
                                                                                );
                                                                            },
                                                                        );
                                                                    },
                                                                )
                                                            })
                                                            .child(
                                                                div()
                                                                    .h(px(1.0))
                                                                    .bg(palette.border_muted)
                                                                    .my_1(),
                                                            )
                                                            .children(account_options.iter().map(
                                                                |username| {
                                                                    let checked = self
                                                                        .filters
                                                                        .account_filters
                                                                        .iter()
                                                                        .any(|selected| {
                                                                            selected.eq_ignore_ascii_case(username)
                                                                        });
                                                                    let username = username.clone();
                                                                    let view = cx.entity();
                                                                    checkbox_menu_item(
                                                                        format!("@{username}"),
                                                                        checked,
                                                                        palette,
                                                                        move |_, app| {
                                                                            view.update(
                                                                                app,
                                                                                |view, cx| {
                                                                                    view.toggle_account_filter(
                                                                                        &username,
                                                                                        cx,
                                                                                    );
                                                                                },
                                                                            );
                                                                        },
                                                                    )
                                                                },
                                                            ))
                                                        }),
                                                )
                                                .with_priority(10_000),
                                            )
                                        }),
                                    palette,
                                )),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.0))
                                .id("history-games-list")
                                .overflow_y_scroll()
                                .scrollbar_width(px(10.0))
                                .px_3()
                                .py_3()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .when(accounts.is_empty(), |this| {
                                    this.child(empty_state(
                                        "No linked Lichess accounts yet. Add or connect one in Settings to build local history.",
                                        palette,
                                    ))
                                })
                                .when(!accounts.is_empty() && total_filtered == 0, |this| {
                                    this.child(empty_state(
                                        "No games match the current filters. Adjust the filters or sync more games.",
                                        palette,
                                    ))
                                })
                                .children(visible_games.into_iter().map(|game| history_row(game, palette))),
                        ),
                ),
            )
    }
}

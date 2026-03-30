use gpui::{
    Context, IntoElement, MouseButton, MouseDownEvent, Render, Window, deferred, div, prelude::*,
    px, rgb, rgba, svg,
};
use time::OffsetDateTime;

use crate::{lichess_state::LichessState, storage::LichessGame};

const PAGE_SIZES: [usize; 5] = [10, 20, 30, 40, 50];
const ICON_CHEVRON_DOWN: &str = "assets/icons/chevron-down.svg";
const ICON_CHECK: &str = "assets/icons/check.svg";

#[derive(Clone, Copy, PartialEq, Eq)]
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

#[derive(Clone, Copy, PartialEq, Eq)]
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

pub struct HistoryPage {
    lichess_state: gpui::Entity<LichessState>,
    result_filter: ResultFilter,
    rated_filter: RatedFilter,
    account_filters: Vec<String>,
    account_filters_initialized: bool,
    account_dropdown_open: bool,
    ignore_next_outside_account_click: bool,
    page_size: usize,
    current_page: usize,
}

impl HistoryPage {
    pub fn new(_cx: &mut Context<Self>, lichess_state: gpui::Entity<LichessState>) -> Self {
        Self {
            lichess_state,
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

    fn set_result_filter(&mut self, filter: ResultFilter, cx: &mut Context<Self>) {
        if self.result_filter != filter {
            self.result_filter = filter;
            self.current_page = 0;
            cx.notify();
        }
    }

    fn set_rated_filter(&mut self, filter: RatedFilter, cx: &mut Context<Self>) {
        if self.rated_filter != filter {
            self.rated_filter = filter;
            self.current_page = 0;
            cx.notify();
        }
    }

    fn set_page_size(&mut self, page_size: usize, cx: &mut Context<Self>) {
        if self.page_size != page_size {
            self.page_size = page_size;
            self.current_page = 0;
            cx.notify();
        }
    }

    fn toggle_account_dropdown(&mut self, cx: &mut Context<Self>) {
        self.account_dropdown_open = !self.account_dropdown_open;
        cx.notify();
    }

    fn close_account_dropdown(&mut self, cx: &mut Context<Self>) {
        if self.account_dropdown_open {
            self.account_dropdown_open = false;
            cx.notify();
        }
    }

    fn on_root_mouse_down(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.account_dropdown_open {
            self.ignore_next_outside_account_click = false;
            return;
        }

        if self.ignore_next_outside_account_click {
            self.ignore_next_outside_account_click = false;
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
        self.ignore_next_outside_account_click = true;
    }

    fn toggle_all_account_filters(&mut self, account_options: &[String], cx: &mut Context<Self>) {
        let all_selected = !account_options.is_empty()
            && self.account_filters.len() == account_options.len()
            && account_options.iter().all(|username| {
                self.account_filters
                    .iter()
                    .any(|selected| selected.eq_ignore_ascii_case(username))
            });

        if all_selected {
            self.account_filters.clear();
        } else {
            self.account_filters = account_options.to_vec();
        }
        self.current_page = 0;
        cx.notify();
    }

    fn toggle_account_filter(&mut self, username: &str, cx: &mut Context<Self>) {
        if let Some(index) = self
            .account_filters
            .iter()
            .position(|selected| selected.eq_ignore_ascii_case(username))
        {
            self.account_filters.remove(index);
        } else {
            self.account_filters.push(username.to_string());
        }
        self.current_page = 0;
        cx.notify();
    }

    fn go_to_previous_page(&mut self, cx: &mut Context<Self>) {
        if self.current_page > 0 {
            self.current_page -= 1;
            cx.notify();
        }
    }

    fn go_to_next_page(&mut self, total_pages: usize, cx: &mut Context<Self>) {
        if self.current_page + 1 < total_pages {
            self.current_page += 1;
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

fn action_button(
    label: impl IntoElement,
    disabled: bool,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(if disabled {
            rgba(0xcbd5e166)
        } else {
            rgba(0x3f7fe544)
        })
        .bg(if disabled {
            rgba(0xf8fafcff)
        } else {
            rgba(0xe8f0feff)
        })
        .text_sm()
        .text_color(if disabled {
            rgb(0x94a3b8)
        } else {
            rgb(0x1d4ed8)
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgba(0xdbeafeff)))
                .on_mouse_up(MouseButton::Left, move |_, window, app| {
                    on_click(window, app);
                })
        })
        .child(label)
}

fn compact_chip(
    label: impl IntoElement,
    active: bool,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_full()
        .border_1()
        .border_color(if active {
            rgba(0x3f7fe588)
        } else {
            rgba(0xcbd5e1ff)
        })
        .bg(if active {
            rgba(0xe8f0feff)
        } else {
            rgba(0xffffffff)
        })
        .text_xs()
        .text_color(if enabled {
            if active { rgb(0x1d4ed8) } else { rgb(0x475569) }
        } else {
            rgb(0x94a3b8)
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgba(0xf8fafcff)))
                .on_mouse_up(MouseButton::Left, move |_, window, app| {
                    on_click(window, app);
                })
        })
        .when(!enabled, |this| this.opacity(0.45))
        .child(label)
}

fn toolbar_group(title: &'static str, body: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().text_xs().text_color(rgb(0x64748b)).child(title))
        .child(body)
}

fn dropdown_trigger(
    label: impl IntoElement,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .w_full()
        .h(px(34.0))
        .rounded_full()
        .border_1()
        .border_color(rgba(0xcbd5e1ff))
        .bg(rgba(0xf8fafcff))
        .cursor_pointer()
        .hover(|this| this.bg(rgba(0xf1f5f9ff)))
        .on_mouse_up(MouseButton::Left, move |_, window, app| {
            on_click(window, app);
        })
        .child(
            div()
                .h_full()
                .w_full()
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(div().text_sm().text_color(rgb(0x334155)).child(label))
                .child(
                    svg()
                        .path(ICON_CHEVRON_DOWN)
                        .w(px(12.0))
                        .h(px(12.0))
                        .text_color(rgb(0x64748b)),
                ),
        )
}

fn checkbox_menu_item(
    label: impl IntoElement,
    checked: bool,
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
        .hover(|this| this.bg(rgba(0xf1f5f9ff)))
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
                    rgba(0x3f7fe5ff)
                } else {
                    rgba(0xcbd5e1ff)
                })
                .bg(if checked {
                    rgba(0xe8f0feff)
                } else {
                    rgba(0xffffffff)
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
                            .text_color(rgb(0x1d4ed8)),
                    )
                }),
        )
        .child(div().text_sm().text_color(rgb(0x334155)).child(label))
}

fn game_result(game: &LichessGame) -> ResultFilter {
    match game.winner.as_deref() {
        Some("white") if game.color == "white" => ResultFilter::Win,
        Some("black") if game.color == "black" => ResultFilter::Win,
        Some(_) => ResultFilter::Loss,
        None => ResultFilter::Draw,
    }
}

fn result_badge(game: &LichessGame) -> impl IntoElement {
    let (label, bg, fg) = match game_result(game) {
        ResultFilter::Win => ("Win", rgba(0xdcfce7ff), rgb(0x166534)),
        ResultFilter::Loss => ("Loss", rgba(0xfee2e2ff), rgb(0xb91c1c)),
        ResultFilter::Draw | ResultFilter::All => ("Draw", rgba(0xe2e8f0ff), rgb(0x475569)),
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

fn history_row(game: &LichessGame) -> impl IntoElement {
    let row_bg = if game.color == "white" {
        rgba(0xf8fafcff)
    } else {
        rgba(0xf1f5f9ff)
    };

    div()
        .w_full()
        .px_4()
        .py_3()
        .rounded_lg()
        .bg(row_bg)
        .border_1()
        .border_color(rgba(0xe2e8f0ff))
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
                        .child(result_badge(game))
                        .child(div().text_sm().text_color(rgb(0x0f172a)).child(format!(
                            "@{} vs {}",
                            game.account_username, game.opponent_name
                        ))),
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
                            rgba(0xe0f2feff),
                            rgb(0x075985),
                        ))
                        .child(pill(
                            rating_diff_text(game),
                            rgba(0xf8fafcff),
                            rgb(0x475569),
                        )),
                ),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x475569))
                .child(opening_text(game)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(div().text_xs().text_color(rgb(0x64748b)).child(format!(
                    "{} | {} | {}",
                    game.variant,
                    game.status,
                    played_at_text(game.played_at)
                )))
                .child(div().text_xs().text_color(rgb(0x64748b)).child(format!(
                            "{} {}",
                            if game.color == "white" { "White" } else { "Black" },
                            game.player_rating
                                .map(|rating| rating.to_string())
                                .unwrap_or_else(|| "rating ?".to_string())
                        ))),
        )
}

fn empty_state(message: &'static str) -> impl IntoElement {
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
        .text_color(rgb(0x64748b))
        .child(message)
}

impl Render for HistoryPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        if !self.account_filters_initialized && !account_options.is_empty() {
            self.account_filters = account_options.clone();
            self.account_filters_initialized = true;
        }

        self.account_filters.retain(|selected| {
            account_options
                .iter()
                .any(|option| option.eq_ignore_ascii_case(selected))
        });
        let all_accounts_selected = !account_options.is_empty()
            && self.account_filters.len() == account_options.len()
            && account_options.iter().all(|username| {
                self.account_filters
                    .iter()
                    .any(|selected| selected.eq_ignore_ascii_case(username))
            });

        let selected_account_label =
            if account_options.is_empty() || self.account_filters.is_empty() {
                "No accounts".to_string()
            } else if all_accounts_selected {
                "All accounts".to_string()
            } else if self.account_filters.len() == 1 {
                format!("@{}", self.account_filters[0])
            } else {
                format!("{} selected", self.account_filters.len())
            };

        let filtered_games = games
            .iter()
            .filter(|game| match self.result_filter {
                ResultFilter::All => true,
                filter => game_result(game) == filter,
            })
            .filter(|game| match self.rated_filter {
                RatedFilter::All => true,
                RatedFilter::Rated => game.rated,
                RatedFilter::Casual => !game.rated,
            })
            .filter(|game| {
                self.account_filters
                    .iter()
                    .any(|selected| game.account_username.eq_ignore_ascii_case(selected))
            })
            .collect::<Vec<_>>();

        let total_filtered = filtered_games.len();
        let total_pages = total_filtered.max(1).div_ceil(self.page_size);
        if self.current_page >= total_pages {
            self.current_page = total_pages.saturating_sub(1);
        }
        let start_index = self.current_page * self.page_size;
        let end_index = (start_index + self.page_size).min(total_filtered);
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
                        .border_color(rgba(0xdbe1e8ff))
                        .bg(rgba(0xffffffed))
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
                                .child(div().text_lg().text_color(rgb(0x0f172a)).child("Game History"))
                                .child(pill(
                                    format!("{} account(s)", accounts.len()),
                                    rgba(0xe0e7ffff),
                                    rgb(0x3730a3),
                                ))
                                .child(pill(
                                    format!("{} filtered", total_filtered),
                                    rgba(0xfef3c7ff),
                                    rgb(0x92400e),
                                ))
                                .when(syncing, |this| {
                                    this.child(pill("Syncing", rgba(0xdbeafeff), rgb(0x1d4ed8)))
                                }),
                        )
                        .child(action_button("Sync now", syncing, move |_, app| {
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
                            .text_color(rgb(0x0369a1))
                            .child(status_message.unwrap_or_default()),
                    )
                })
                .when(error_message.is_some(), |this| {
                    this.child(
                        div()
                            .px_1()
                            .text_sm()
                            .text_color(rgb(0xb91c1c))
                            .child(error_message.unwrap_or_default()),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .rounded_xl()
                        .border_1()
                        .border_color(rgba(0xdbe1e8ff))
                        .bg(rgba(0xffffffed))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex_none()
                                .px_4()
                                .py_2()
                                .border_b_1()
                                .border_color(rgba(0xe2e8f088))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_4()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(0x334155))
                                        .child("Recent games"),
                                )
                                .child(
                                    div().flex().items_center().gap_3().child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x64748b))
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
                                                    self.page_size == page_size,
                                                    true,
                                                    move |_, app| {
                                                        view.update(app, |view, cx| {
                                                            view.set_page_size(page_size, cx);
                                                        });
                                                    },
                                                )
                                            })),
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
                                                    self.current_page > 0,
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
                                                    .text_color(rgb(0x64748b))
                                                    .child(format!(
                                                        "{} / {}",
                                                        self.current_page + 1,
                                                        total_pages
                                                    )),
                                            )
                                            .child({
                                                let view = cx.entity();
                                                compact_chip(
                                                    "Next",
                                                    false,
                                                    self.current_page + 1 < total_pages,
                                                    move |_, app| {
                                                        view.update(app, |view, cx| {
                                                            view.go_to_next_page(total_pages, cx);
                                                        });
                                                    },
                                                )
                                            }),
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px_4()
                                .py_2()
                                .border_b_1()
                                .border_color(rgba(0xe2e8f088))
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
                                                self.result_filter == filter,
                                                true,
                                                move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.set_result_filter(filter, cx);
                                                    });
                                                },
                                            )
                                        })),
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
                                                self.rated_filter == filter,
                                                true,
                                                move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.set_rated_filter(filter, cx);
                                                    });
                                                },
                                            )
                                        })),
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
                                            dropdown_trigger(selected_account_label, move |_, app| {
                                                view.update(app, |view, cx| {
                                                    view.toggle_account_dropdown(cx);
                                                });
                                            })
                                        })
                                        .when(self.account_dropdown_open, |this| {
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
                                                        .border_color(rgba(0xdbe1e8ff))
                                                        .bg(rgba(0xffffffff))
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
                                                                    .text_color(rgb(0x94a3b8))
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
                                                                    .bg(rgba(0xe2e8f0ff))
                                                                    .my_1(),
                                                            )
                                                            .children(account_options.iter().map(
                                                                |username| {
                                                                    let checked = self
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
                                    ))
                                })
                                .when(!accounts.is_empty() && total_filtered == 0, |this| {
                                    this.child(empty_state(
                                        "No games match the current filters. Adjust the filters or sync more games.",
                                    ))
                                })
                                .children(visible_games.into_iter().map(history_row)),
                        ),
                ),
            )
    }
}

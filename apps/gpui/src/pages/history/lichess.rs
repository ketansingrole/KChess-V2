//! Game history → Lichess (`pages/history.vue`'s Lichess tab, `stores/kchess/gameHistory.ts`,
//! `GameRow.vue`): the synced games of the connected and followed accounts, a page at a time,
//! filtered by account, result and rating; each with its review's accuracy; opened to go through
//! move by move or to review on the analysis board.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Map, Value, json};
use shakmaty::Color;

use super::HistoryPage;
use crate::assets::icon;
use crate::core::AppCore;
use crate::game::tree::TreeNode;
use crate::pages::Page;
use crate::replay::GameReplay;
use crate::theme::Palette;
use crate::widgets::{self, Choice, KeyboardFocus as _, Pill};

const RESULTS: [(&str, &str); 4] = [
    ("all", "All"),
    ("win", "Wins"),
    ("loss", "Losses"),
    ("draw", "Draws"),
];
const RATED: [(&str, &str); 3] = [
    ("all", "Rated & casual"),
    ("rated", "Rated"),
    ("casual", "Casual"),
];
const PAGE_SIZES: [usize; 5] = [10, 20, 30, 40, 50];

/// The Lichess tab's filters, page and games.
pub struct LichessTab {
    pub account: String,
    pub result: &'static str,
    pub rated: &'static str,
    pub page: usize,
    pub page_size: usize,
    pub games: Vec<Value>,
    pub total: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// `ReviewSummary` by game id.
    pub summaries: Map<String, Value>,
    epoch: u64,
    pub account_select: Entity<SelectState<Vec<Choice>>>,
    pub rated_select: Entity<SelectState<Vec<Choice>>>,
    pub size_select: Entity<SelectState<Vec<Choice>>>,
    /// The game open, its review's side, and its board.
    pub open: Option<(Value, Entity<GameReplay>)>,
}

/// `gameResult`: the game from the account's side.
fn result_of(game: &Value) -> &'static str {
    match game["winner"].as_str() {
        None => "draw",
        Some(winner) if game["color"].as_str() == Some(winner) => "win",
        Some(_) => "loss",
    }
}

/// The PGN's setup and main line, for the replay.
fn line_of(pgn: &str) -> Option<(Value, Vec<String>)> {
    let tree = TreeNode::from_pgn(pgn)?;
    let mut moves = Vec::new();
    let mut node = &tree;
    while let Some(next) = node.children.first() {
        moves.push(next.uci.clone());
        node = next;
    }
    Some((json!({ "variant": "standard", "fen": tree.fen }), moves))
}

pub fn new_tab(
    window: &mut Window,
    cx: &mut Context<HistoryPage>,
) -> (LichessTab, Vec<Subscription>) {
    let choices = |items: &[(&str, &str)]| -> Vec<Choice> {
        items
            .iter()
            .map(|(value, label)| Choice::new(*value, *label))
            .collect()
    };
    let account_select =
        cx.new(|cx| SelectState::new(vec![Choice::new("all", "All accounts")], None, window, cx));
    let rated_select = cx.new(|cx| SelectState::new(choices(&RATED), None, window, cx));
    let sizes: Vec<Choice> = PAGE_SIZES
        .iter()
        .map(|size| Choice::new(size.to_string(), format!("{size} per page")))
        .collect();
    let size_select = cx.new(|cx| SelectState::new(sizes, None, window, cx));
    for (select, value) in [
        (&account_select, "all".to_string()),
        (&rated_select, "all".to_string()),
        (&size_select, "20".to_string()),
    ] {
        select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(value), window, cx)
        });
    }
    let subscriptions = vec![
        cx.subscribe(
            &account_select,
            |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.lichess.account = value.to_string();
                    this.lichess.page = 0;
                    this.load_lichess(cx);
                }
            },
        ),
        cx.subscribe(
            &rated_select,
            |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.lichess.rated = RATED
                        .iter()
                        .find(|(key, _)| *key == value.as_ref())
                        .map_or("all", |(key, _)| *key);
                    this.lichess.page = 0;
                    this.load_lichess(cx);
                }
            },
        ),
        cx.subscribe(
            &size_select,
            |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.lichess.page_size = value.parse().unwrap_or(20);
                    this.lichess.page = 0;
                    this.load_lichess(cx);
                }
            },
        ),
        // A sync or sign-out changes the games.
        cx.observe(&crate::accounts::handle(cx), |this, _, cx| {
            this.load_lichess(cx)
        }),
    ];
    (
        LichessTab {
            account: "all".into(),
            result: "all",
            rated: "all",
            page: 0,
            page_size: 20,
            games: Vec::new(),
            total: 0,
            loading: false,
            error: None,
            summaries: Map::new(),
            epoch: 0,
            account_select,
            rated_select,
            size_select,
            open: None,
        },
        subscriptions,
    )
}

impl HistoryPage {
    /// `gamePage`: the page of games the filters ask for, and their reviews.
    pub(super) fn load_lichess(&mut self, cx: &mut Context<Self>) {
        let tab = &mut self.lichess;
        tab.epoch += 1;
        let epoch = tab.epoch;
        tab.loading = true;
        tab.error = None;
        let mut query = json!({
            "offset": tab.page * tab.page_size,
            "limit": tab.page_size,
        });
        // Filters left at "all" are left out, as Electron sends them.
        if tab.account != "all" {
            query["account"] = json!(tab.account);
        }
        if tab.result != "all" {
            query["result"] = json!(tab.result);
        }
        if tab.rated != "all" {
            query["rated"] = json!(tab.rated == "rated");
        }
        let core = cx.global::<AppCore>().clone();
        let page = core.call("gamePage", vec![query]);
        cx.spawn(async move |this, cx| {
            let page = page.await;
            let ids: Vec<Value> = page
                .as_ref()
                .ok()
                .and_then(|page| page["games"].as_array())
                .map(|games| games.iter().map(|game| game["id"].clone()).collect())
                .unwrap_or_default();
            let summaries = if ids.is_empty() {
                Ok(Value::Null)
            } else {
                core.call("reviewSummaries", vec![json!(ids)]).await
            };
            let _ = this.update(cx, |this, cx| {
                let tab = &mut this.lichess;
                if epoch != tab.epoch {
                    return;
                }
                tab.loading = false;
                match page {
                    Ok(page) => {
                        tab.total = page["total"].as_f64().unwrap_or_default() as usize;
                        tab.games = page["games"].as_array().cloned().unwrap_or_default();
                        let pages = tab.total.div_ceil(tab.page_size).max(1);
                        if tab.page >= pages {
                            tab.page = pages - 1;
                            this.load_lichess(cx);
                            return;
                        }
                    }
                    Err(cause) => tab.error = Some(cause.message),
                }
                if let Ok(Value::Object(summaries)) = summaries {
                    tab.summaries = summaries;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_lichess_account(
        &mut self,
        account: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lichess.account = account.clone();
        self.lichess.page = 0;
        self.lichess.account_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(account), window, cx)
        });
        self.load_lichess(cx);
    }

    fn choose_result(&mut self, result: &'static str, cx: &mut Context<Self>) {
        self.lichess.result = result;
        self.lichess.page = 0;
        self.load_lichess(cx);
    }

    fn clear_lichess_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.lichess;
        tab.account = "all".into();
        tab.result = "all";
        tab.rated = "all";
        tab.page = 0;
        for select in [&tab.account_select, &tab.rated_select] {
            select.update(cx, |state, cx| {
                state.set_selected_value(&SharedString::from("all"), window, cx)
            });
        }
        self.load_lichess(cx);
    }

    /// `openReview`: read the game's PGN (list rows carry none) and go through it.
    pub fn open_lichess(&mut self, game: Value, window: &mut Window, cx: &mut Context<Self>) {
        let read = if game["pgn"].is_string() {
            None
        } else {
            Some(
                cx.global::<AppCore>()
                    .call("gamePgn", vec![game["account"].clone(), game["id"].clone()]),
            )
        };
        cx.spawn_in(window, async move |this, cx| {
            let pgn = match read {
                Some(read) => read
                    .await
                    .ok()
                    .and_then(|pgn| pgn.as_str().map(str::to_string)),
                None => game["pgn"].as_str().map(str::to_string),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some((setup, moves)) = pgn.as_deref().and_then(line_of) else {
                    this.message = Some("That game could not be read.".into());
                    cx.notify();
                    return;
                };
                let color = if game["color"] == json!("black") {
                    Color::Black
                } else {
                    Color::White
                };
                let account = game["account"].as_str().unwrap_or_default().to_string();
                let opponent = game["opponent"].as_str().unwrap_or_default().to_string();
                let replay =
                    cx.new(|cx| GameReplay::new(setup, moves, color, account, opponent, cx));
                window.focus(&replay.read(cx).focus_handle(cx), cx);
                let mut game = game.clone();
                game["pgn"] = json!(pgn);
                this.lichess.open = Some((game, replay));
                this.message = None;
                this.redo = None;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn close_lichess(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lichess.open = None;
        self.redo = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The review of a game, from its account's side (`ReviewSide`).
    fn side_of(&self, game: &Value) -> Option<Value> {
        let id = game["id"].as_str()?;
        let color = game["color"].as_str()?;
        self.lichess
            .summaries
            .get(id)
            .map(|summary| summary[color].clone())
            .filter(Value::is_object)
    }

    /// The accounts the filter offers, kept up to date with the account store.
    fn sync_account_choices(&self, window: &mut Window, cx: &mut Context<Self>) {
        let accounts: Vec<String> = crate::accounts::handle(cx)
            .read(cx)
            .all()
            .iter()
            .filter_map(|account| account["username"].as_str().map(str::to_string))
            .collect();
        let mut items = vec![Choice::new("all", "All accounts")];
        items.extend(
            accounts
                .iter()
                .map(|name| Choice::new(name.clone(), name.clone())),
        );
        let chosen = self.lichess.account.clone();
        self.lichess.account_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_value(&SharedString::from(chosen), window, cx);
        });
    }

    pub(super) fn render_lichess_open(
        &self,
        game: &Value,
        replay: &Entity<GameReplay>,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let outcome = result_of(game);
        let account = game["account"].as_str().unwrap_or_default().to_string();
        let opponent = game["opponent"].as_str().unwrap_or_default().to_string();
        let date = crate::game::game_date(game["createdAt"].as_f64().unwrap_or_default());
        let opening = game["opening"].as_str().map(str::to_string);
        let side = self.side_of(game);
        let review_line = side.map(|side| {
            let mut parts = Vec::new();
            if let Some(accuracy) = side["accuracy"].as_f64() {
                parts.push(format!("{accuracy}% accuracy"));
            }
            parts.extend(
                crate::reviews::judgment_counts(&side)
                    .into_iter()
                    .map(|(_, text)| text),
            );
            parts.join(" · ")
        });
        let reviewed = game.clone();
        let replay_handle = replay.clone();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .px(crate::play_layout::side_padding(window))
                    .pt(crate::play_layout::PAGE_TOP)
                    .child(
                        Button::new("lichess-all-games")
                            .ghost()
                            .small()
                            .icon(icon("arrow-left"))
                            .label("All games")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_lichess(window, cx)),
                            ),
                    )
                    .child(super::badge(outcome, None, palette))
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .text_size(px(14.))
                            .child(div().font_weight(FontWeight::BOLD).child(account))
                            .child("vs")
                            .child(div().font_weight(FontWeight::BOLD).child(opponent))
                            .child(div().text_color(palette.text_muted).child(match opening {
                                Some(opening) => format!("· {date} · {opening}"),
                                None => format!("· {date}"),
                            })),
                    )
                    .children(review_line.filter(|line| !line.is_empty()).map(|line| {
                        div()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(line)
                    }))
                    .child(
                        Button::new("lichess-review")
                            .outline()
                            .small()
                            .icon(icon("sparkles"))
                            .label("Review")
                            .on_click(move |_, window, cx| {
                                let ply = replay_handle.read(cx).ply();
                                open_review(reviewed.clone(), ply, window, cx);
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().child(replay.clone()))
            .into_any_element()
    }

    pub(super) fn render_lichess(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        self.sync_account_choices(window, cx);
        let accounts = crate::accounts::handle(cx);
        let (count, has_accounts, busy) = {
            let accounts = accounts.read(cx);
            (
                accounts.data["gameCount"].as_f64().unwrap_or_default() as usize,
                !accounts.all().is_empty(),
                accounts.busy,
            )
        };
        let sync = Button::new("lichess-sync")
            .outline()
            .icon(icon("refresh-cw"))
            .label("Sync games")
            .loading(busy)
            .disabled(busy)
            .on_click(cx.listener(|this, _, window, cx| {
                let account = (this.lichess.account != "all").then(|| this.lichess.account.clone());
                crate::accounts::handle(cx)
                    .update(cx, |accounts, cx| accounts.sync(account, window, cx));
            }));
        if count == 0 {
            let body = if has_accounts {
                widgets::empty_state(
                    icon("history"),
                    "No saved Lichess games",
                    Some("Sync games to get started.".into()),
                    palette,
                )
                .child(h_flex().mt(px(12.)).child(sync))
            } else {
                widgets::empty_state(
                    icon("history"),
                    "No saved Lichess games",
                    Some("Follow a player or connect Lichess.".into()),
                    palette,
                )
                .child(
                    h_flex()
                        .mt(px(12.))
                        .gap(px(8.))
                        .child(
                            Button::new("lichess-follow")
                                .primary()
                                .icon(icon("user-plus"))
                                .label("Follow a player")
                                .on_click(|_, window, cx| {
                                    crate::shell::open(Page::Friends, window, cx)
                                }),
                        )
                        .child(
                            Button::new("lichess-analyse")
                                .outline()
                                .label("Analyze a PGN")
                                .on_click(|_, window, cx| {
                                    crate::shell::open(Page::Analysis, window, cx)
                                }),
                        ),
                )
            };
            return widgets::card(palette)
                .py(px(30.))
                .child(body)
                .into_any_element();
        }
        let tab = &self.lichess;
        let filtered = tab.account != "all" || tab.result != "all" || tab.rated != "all";
        let chosen = RESULTS
            .iter()
            .position(|(key, _)| *key == tab.result)
            .unwrap_or_default();
        let results = widgets::pills(
            "lichess-result",
            RESULTS
                .iter()
                .map(|(_, label)| Pill {
                    icon: None,
                    label: (*label).into(),
                    badge: None,
                    disabled: false,
                })
                .collect(),
            chosen,
            {
                let page = cx.entity().downgrade();
                move |index, _, cx| {
                    let _ = page.update(cx, |this, cx| this.choose_result(RESULTS[index].0, cx));
                }
            },
            true,
            palette,
        );
        let several = accounts.read(cx).all().len() > 1;
        let toolbar = h_flex()
            .flex_wrap()
            .gap(px(10.))
            .when(several, |row| {
                row.child(
                    div()
                        .w(px(180.))
                        .child(Select::new(&tab.account_select).small()),
                )
            })
            .child(results)
            .child(
                div()
                    .w(px(160.))
                    .child(Select::new(&tab.rated_select).small()),
            )
            .when(filtered, |row| {
                row.child(
                    Button::new("lichess-clear")
                        .ghost()
                        .small()
                        .icon(icon("x"))
                        .label("Clear filters")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.clear_lichess_filters(window, cx)
                        })),
                )
            })
            .child(div().flex_1())
            .child(sync);
        let body = if tab.loading && tab.games.is_empty() {
            div()
                .p(px(24.))
                .text_color(palette.text_muted)
                .child("Loading games…")
                .into_any_element()
        } else if let Some(error) = &tab.error {
            div()
                .p(px(24.))
                .text_color(palette.error)
                .child(error.clone())
                .into_any_element()
        } else if tab.games.is_empty() {
            widgets::empty_state(
                icon("search-x"),
                "No games match these filters",
                None,
                palette,
            )
            .child(
                h_flex().mt(px(10.)).child(
                    Button::new("lichess-clear-empty")
                        .outline()
                        .label("Clear filters")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.clear_lichess_filters(window, cx)
                        })),
                ),
            )
            .into_any_element()
        } else {
            let head = h_flex()
                .px(px(14.))
                .pb(px(8.))
                .gap(px(12.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.text_muted)
                .child(div().w(px(64.)).child("RESULT"))
                .child(div().flex_1().child("OPPONENT"))
                .child(div().w(px(90.)).child("MODE"))
                .child(div().w(px(60.)).child("RATING"))
                .child(div().w(px(110.)).child("ACCURACY"))
                .child(div().w(px(100.)).child("DATE"));
            let rows: Vec<AnyElement> = tab
                .games
                .iter()
                .enumerate()
                .map(|(index, game)| {
                    let chosen = game.clone();
                    let page = cx.entity().downgrade();
                    game_row(
                        ("lichess-game", index),
                        game,
                        self.side_of(game),
                        RowDetail::Full,
                        move |window, cx| {
                            let _ = page.update(cx, |this, cx| {
                                this.open_lichess(chosen.clone(), window, cx)
                            });
                        },
                        palette,
                    )
                })
                .collect();
            v_flex().child(head).children(rows).into_any_element()
        };
        let total = tab.total;
        let from = tab.page * tab.page_size + 1;
        let range = if total == 0 {
            "0 games".to_string()
        } else {
            format!(
                "{from}–{} of {total} games",
                (from + tab.page_size - 1).min(total)
            )
        };
        let pages = total.div_ceil(tab.page_size).max(1);
        let pager = h_flex()
            .pt(px(12.))
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(range),
            )
            .child(div().flex_1())
            .child(
                div()
                    .w(px(130.))
                    .child(Select::new(&tab.size_select).small()),
            )
            .child(self.lichess_pager(tab.page, pages, cx));
        v_flex()
            .gap(px(16.))
            .child(toolbar)
            .child(widgets::card(palette).p(px(10.)).child(body).child(pager))
            .into_any_element()
    }

    /// `GameRow.vue detailed`: result, opponent and opening, mode, rating change, accuracy, date.
    fn lichess_pager(
        &self,
        current: usize,
        pages: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let go =
            |label: String, target: usize, disabled: bool, chosen: bool, cx: &mut Context<Self>| {
                Button::new(SharedString::from(format!("lichess-page-{label}")))
                    .small()
                    .when(chosen, |button| button.primary())
                    .when(!chosen, |button| button.outline())
                    .label(label)
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.lichess.page = target;
                        this.load_lichess(cx);
                    }))
            };
        // The first, the last, and the pages around the current one (`sibling-count="1"`).
        let shown: Vec<usize> = (0..pages)
            .filter(|page| *page == 0 || *page + 1 == pages || page.abs_diff(current) <= 1)
            .collect();
        let mut row = h_flex().gap(px(4.)).child(go(
            "‹".into(),
            current.saturating_sub(1),
            current == 0,
            false,
            cx,
        ));
        let mut last = None;
        for page in shown {
            if last.is_some_and(|last: usize| page > last + 1) {
                row = row.child(div().px(px(4.)).child("…"));
            }
            row = row.child(go((page + 1).to_string(), page, false, page == current, cx));
            last = Some(page);
        }
        row.child(go(
            "›".into(),
            (current + 1).min(pages - 1),
            current + 1 >= pages,
            false,
            cx,
        ))
    }
}

/// How much a game row says (`GameRow.vue`'s `detailed` and `hide-accuracy`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowDetail {
    /// Result, opponent, mode and date.
    Brief,
    /// Also the opening and rating, the rating change, and the review's accuracy and worst
    /// mistakes (`side`).
    Full,
    /// `Full` without the review.
    NoReview,
}

/// `GameRow.vue`: a game from its account's side, in as much detail as asked for.
pub fn game_row(
    id: impl Into<ElementId>,
    game: &Value,
    side: Option<Value>,
    detail: RowDetail,
    select: impl Fn(&mut Window, &mut App) + 'static,
    palette: &Palette,
) -> AnyElement {
    let detailed = detail != RowDetail::Brief;
    let outcome = result_of(game);
    let opponent = game["opponent"].as_str().unwrap_or("?").to_string();
    let mut sub = game["opening"]
        .as_str()
        .or(game["account"].as_str())
        .unwrap_or_default()
        .to_string();
    if let Some(rating) = game["opponentRating"].as_f64() {
        sub.push_str(&format!(" · {rating}"));
    }
    let speed = game["speed"].as_str().unwrap_or_default();
    let speed = {
        let mut chars = speed.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect::<String>())
            .unwrap_or_default()
    };
    let diff = game["ratingDiff"].as_f64().filter(|diff| *diff != 0.);
    let accuracy = match &side {
        Some(side) => side["accuracy"]
            .as_f64()
            .map_or_else(|| "Reviewed".to_string(), |accuracy| format!("{accuracy}%")),
        None => "—".into(),
    };
    let worst = side
        .as_ref()
        .and_then(|side| crate::reviews::judgment_counts(side).into_iter().next());
    let muted = palette.text_muted;
    let line = |text: String| {
        div()
            .text_size(px(12.))
            .text_color(muted)
            .truncate()
            .child(text)
    };
    // `.game-row`: 11px by 12px, 13px text.
    h_flex()
        .id(id)
        .role(Role::Button)
        .keyboard_focus(palette.primary)
        .aria_label(format!("Game against {opponent}"))
        .px(px(12.))
        .py(px(11.))
        .gap(px(12.))
        .rounded(px(8.))
        .text_size(px(13.))
        .cursor_pointer()
        .hover(|row| row.bg(palette.accented))
        .on_click(move |_, window, cx| select(window, cx))
        .child(
            div()
                .w(px(64.))
                .child(super::badge(outcome, Some(px(52.)), palette)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .truncate()
                        .child(opponent),
                )
                .when(detailed, |column| column.child(line(sub))),
        )
        .child(v_flex().w(px(90.)).child(speed).when(detailed, |column| {
            column.child(line(if game["rated"] == json!(true) {
                "Rated".into()
            } else {
                "Casual".into()
            }))
        }))
        .when(detailed, |row| {
            row.child(
                div()
                    .w(px(60.))
                    .text_color(match diff {
                        Some(diff) if diff > 0. => palette.win,
                        Some(_) => palette.loss,
                        None => muted,
                    })
                    .child(match diff {
                        Some(diff) if diff > 0. => format!("+{diff}"),
                        Some(diff) => format!("{diff}"),
                        None => "—".into(),
                    }),
            )
        })
        .when(detail == RowDetail::Full, |row| {
            row.child(
                v_flex()
                    .w(px(110.))
                    .child(
                        div()
                            .when(side.is_none(), |text| text.text_color(muted))
                            .child(accuracy),
                    )
                    .children(worst.map(|(judgment, text)| {
                        let (_, _, color) =
                            crate::pages::analysis::classification_look(judgment, palette.dark);
                        div().text_size(px(12.)).text_color(color).child(text)
                    })),
            )
        })
        .child(
            div()
                .w(px(100.))
                .text_color(muted)
                .child(crate::game::game_date(
                    game["createdAt"].as_f64().unwrap_or_default(),
                )),
        )
        .into_any_element()
}

/// Review a synced game on the analysis board, opened at the move being looked at
/// (`openGameReview`).
fn open_review(game: Value, ply: usize, window: &mut Window, cx: &mut App) {
    let Some(pgn) = game["pgn"].as_str().map(str::to_string) else {
        return;
    };
    let color = if game["color"] == json!("black") {
        Color::Black
    } else {
        Color::White
    };
    let (account, opponent) = (game["account"].clone(), game["opponent"].clone());
    let origin = json!({
        "gameId": game["id"],
        "account": account,
        "white": if color == Color::White { &account } else { &opponent },
        "black": if color == Color::Black { &account } else { &opponent },
        "player": game["color"],
    });
    crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
        analysis.open_game(&pgn, ply, Some(color), Some(origin.clone()), true, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::{line_of, result_of};
    use serde_json::json;

    #[test]
    fn a_game_reads_from_its_accounts_side() {
        assert_eq!(
            result_of(&json!({ "color": "white", "winner": "white" })),
            "win"
        );
        assert_eq!(
            result_of(&json!({ "color": "black", "winner": "white" })),
            "loss"
        );
        assert_eq!(result_of(&json!({ "color": "black" })), "draw");
    }

    #[test]
    fn a_pgn_gives_the_replay_its_line() {
        let (setup, moves) = line_of("1. e4 e5 2. Nf3 (2. Bc4) Nc6 *").unwrap();
        assert_eq!(moves, vec!["e2e4", "e7e5", "g1f3", "b8c6"]);
        assert_eq!(setup["variant"], json!("standard"));
    }
}

//! Players (`pages/players.vue`): look up any Lichess player — their profile, whether they are
//! online, your record against them, their rating history, their record in a speed, and their
//! recent games — and follow, challenge, message or watch them.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::core::AppCore;
use crate::pages::history::lichess::{RowDetail, game_row};
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::rating_history::{RatingHistory, Retry, has_points};
use crate::theme::Palette;
use crate::widgets::{self, Choice, Look};

/// Recent lookups kept (`kchess:players-recent`), and names offered under the search.
const RECENT_KEY: &str = "kchess:players-recent";
const RECENT_KEPT: usize = 8;
const SUGGESTED: usize = 12;

/// `PERF_LABELS`, in `PERF_TYPES` order.
const PERFS: [(&str, &str); 14] = [
    ("ultraBullet", "UltraBullet"),
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("correspondence", "Correspondence"),
    ("chess960", "Chess960"),
    ("kingOfTheHill", "King of the Hill"),
    ("threeCheck", "Three-check"),
    ("antichess", "Antichess"),
    ("atomic", "Atomic"),
    ("horde", "Horde"),
    ("racingKings", "Racing Kings"),
    ("crazyhouse", "Crazyhouse"),
];

/// `USERNAME`: what Lichess accepts as a name.
fn is_username(name: &str) -> bool {
    (2..=30).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `playedPerfs`: the speeds a player has games in, most played first.
fn played_perfs(profile: &Value) -> Vec<(&'static str, &'static str)> {
    let games = |key: &str| profile["perfs"][key]["games"].as_f64().unwrap_or_default();
    let mut perfs: Vec<_> = PERFS
        .iter()
        .copied()
        .filter(|(key, _)| games(key) > 0.)
        .collect();
    perfs.sort_by(|a, b| games(b.0).total_cmp(&games(a.0)));
    perfs
}

/// Recent lookups first, then followed players, each name once.
fn suggestions(recent: &[String], followed: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    recent
        .iter()
        .chain(followed)
        .filter(|name| seen.insert(name.to_lowercase()))
        .take(SUGGESTED)
        .cloned()
        .collect()
}

/// "12 h" or "40 min" of play.
fn hours(seconds: f64) -> String {
    let hours = seconds / 3600.;
    if hours >= 1. {
        format!("{} h", hours.round())
    } else {
        format!("{} min", (seconds / 60.).round())
    }
}

fn local_date(millis: f64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .map(|date| date.format("%-d %b %Y").to_string())
        .unwrap_or_else(|| "?".into())
}

fn message_of(cause: &kchess_core::CoreError) -> SharedString {
    cause.message.clone().into()
}

pub struct PlayersPage {
    input: Entity<InputState>,
    profile: Option<Value>,
    /// `UserPresence`, once known.
    status: Option<Value>,
    perf: String,
    perf_select: Entity<SelectState<Vec<Choice>>>,
    stats: Option<Value>,
    stats_error: Option<SharedString>,
    /// `Crosstable` between the active account and the player.
    record: Option<Value>,
    /// What could not be read beside the profile, so "none" is not implied.
    extras_failed: Vec<&'static str>,
    games: Option<Vec<Value>>,
    games_error: bool,
    history: Entity<RatingHistory>,
    error: Option<SharedString>,
    loading: bool,
    /// Each lookup; answers to an older one are dropped.
    epoch: u64,
    stats_epoch: u64,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for PlayersPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PlayersPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> PlayersPage {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Lichess username"));
        let perf_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let history =
            cx.new(|cx| RatingHistory::new("Built from the last 200 rated games.", window, cx));
        let accounts = crate::accounts::handle(cx);
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter { .. } => this.lookup(None, window, cx),
                        InputEvent::Change => cx.notify(),
                        _ => {}
                    }
                }),
                cx.subscribe(
                    &perf_select,
                    |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                        if let SelectEvent::Confirm(Some(value)) = event
                            && *value != this.perf.as_str()
                        {
                            this.perf = value.to_string();
                            this.load_stats(cx);
                        }
                    },
                ),
                cx.observe(&accounts, |_, _, cx| cx.notify()),
            ];
        PlayersPage {
            input,
            profile: None,
            status: None,
            perf: "blitz".into(),
            perf_select,
            stats: None,
            stats_error: None,
            record: None,
            extras_failed: Vec::new(),
            games: None,
            games_error: false,
            history,
            error: None,
            loading: false,
            epoch: 0,
            stats_epoch: 0,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// A profile link opened elsewhere (`/players?name=`): look the player up, unless shown.
    pub fn show_player(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.profile.as_ref().and_then(|p| p["username"].as_str());
        if shown.is_some_and(|shown| shown.eq_ignore_ascii_case(name)) {
            return;
        }
        let name = name.to_string();
        self.input
            .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
        self.lookup(Some(name), window, cx);
    }

    fn active_account(cx: &App) -> Option<String> {
        crate::accounts::handle(cx).read(cx).active(cx)
    }

    /// `lookup`: the profile first, then everything else about the player at once.
    fn lookup(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let name = name.unwrap_or_else(|| self.input.read(cx).value().trim().to_string());
        if !is_username(&name) {
            self.error = Some("Enter a Lichess username.".into());
            cx.notify();
            return;
        }
        self.epoch += 1;
        self.stats_epoch += 1;
        let epoch = self.epoch;
        self.loading = true;
        self.error = None;
        self.stats = None;
        self.stats_error = None;
        self.record = None;
        self.status = None;
        self.extras_failed.clear();
        self.games = None;
        self.games_error = false;
        self.history.update(cx, |history, cx| {
            history.set(Value::Null, Value::Null, cx);
            history.set_withheld(None, cx);
        });
        cx.notify();
        let profile = cx.global::<AppCore>().call("profile", vec![json!(name)]);
        cx.spawn_in(window, async move |this, cx| {
            let found = profile.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.epoch != epoch {
                    return;
                }
                this.loading = false;
                match found {
                    Ok(found) => this.found(found, window, cx),
                    Err(cause) => {
                        crate::log::write(
                            kchess_core::Level::Warn,
                            "players",
                            &format!("Profile lookup failed: {}", cause.message),
                        );
                        this.profile = None;
                        this.error = Some(message_of(&cause));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn found(&mut self, found: Value, window: &mut Window, cx: &mut Context<Self>) {
        let username = found["username"].as_str().unwrap_or_default().to_string();
        self.input.update(cx, |input, cx| {
            input.set_value(username.clone(), window, cx)
        });
        let mut recent: Vec<String> = crate::prefs::get(cx, RECENT_KEY, Vec::new());
        recent.retain(|name| !name.eq_ignore_ascii_case(&username));
        recent.insert(0, username.clone());
        recent.truncate(RECENT_KEPT);
        crate::prefs::set(cx, RECENT_KEY, recent);
        let perfs = played_perfs(&found);
        self.perf = perfs.first().map_or("blitz", |(key, _)| key).to_string();
        let items: Vec<Choice> = perfs
            .iter()
            .map(|(key, label)| Choice::new(*key, *label))
            .collect();
        let perf = SharedString::from(self.perf.clone());
        self.perf_select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_value(&perf, window, cx);
        });
        self.profile = Some(found);
        self.load_stats(cx);
        self.load_extras(&username, cx);
        self.load_activity(cx);
    }

    /// Your record against them, and whether they are online.
    fn load_extras(&mut self, username: &str, cx: &mut Context<Self>) {
        let epoch = self.epoch;
        let core = cx.global::<AppCore>().clone();
        if let Some(me) = Self::active_account(cx)
            && !me.eq_ignore_ascii_case(username)
        {
            let crosstable = core.call("crosstable", vec![json!(me), json!(username)]);
            cx.spawn(async move |this, cx| {
                let answer = crosstable.await;
                let _ = this.update(cx, |this, cx| {
                    if this.epoch != epoch {
                        return;
                    }
                    match answer {
                        Ok(record) => this.record = Some(record),
                        Err(_) => this.extras_failed.push("your record"),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        let presence = core.call("presence", vec![json!([username])]);
        let key = username.to_lowercase();
        cx.spawn(async move |this, cx| {
            let answer = presence.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                match answer {
                    Ok(report) => {
                        this.status = Some(report["users"][&key].clone()).filter(Value::is_object)
                    }
                    Err(_) => this.extras_failed.push("online status"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `loadStats`: their record in the chosen speed.
    fn load_stats(&mut self, cx: &mut Context<Self>) {
        let Some(username) = self.username() else {
            return;
        };
        self.stats_epoch += 1;
        let stats_epoch = self.stats_epoch;
        self.stats = None;
        self.stats_error = None;
        cx.notify();
        let stats = cx
            .global::<AppCore>()
            .call("playerPerf", vec![json!(username), json!(self.perf)]);
        cx.spawn(async move |this, cx| {
            let answer = stats.await;
            let _ = this.update(cx, |this, cx| {
                if this.stats_epoch != stats_epoch {
                    return;
                }
                match answer {
                    Ok(stats) => this.stats = Some(stats),
                    Err(cause) => this.stats_error = Some(message_of(&cause)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `loadHistory` and `loadGames`: Lichess's rating history (or one rebuilt from their recent
    /// rated games when it sends none) and their recent games.
    fn load_activity(&mut self, cx: &mut Context<Self>) {
        let Some(username) = self.username() else {
            return;
        };
        let epoch = self.epoch;
        self.games = None;
        self.games_error = false;
        let core = cx.global::<AppCore>().clone();
        let games = core.call("recentGames", vec![json!(username)]);
        cx.spawn(async move |this, cx| {
            let answer = games.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                match answer {
                    Ok(games) => this.games = Some(games.as_array().cloned().unwrap_or_default()),
                    // Lichess sometimes answers "not found" for every player's games.
                    Err(_) => this.games_error = true,
                }
                cx.notify();
            });
        })
        .detach();
        let official = core.call("ratingHistory", vec![json!(username)]);
        cx.spawn(async move |this, cx| {
            let official = official.await.unwrap_or(json!([]));
            let rebuilt = if has_points(&official) {
                None
            } else {
                let rated = core.call("recentGames", vec![json!(username), json!(true)]);
                let rated = rated.await.unwrap_or(json!([]));
                Some(crate::game::rules("ratingHistoryFromGames", &[rated]).unwrap_or(json!([])))
            };
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                let withheld = this.withheld(cx);
                this.history.update(cx, |history, cx| {
                    history.set(official, rebuilt.unwrap_or(json!([])), cx);
                    history.set_withheld(withheld, cx);
                });
            });
        })
        .detach();
    }

    /// Rated games on the profile but nothing to chart: Lichess withheld the data.
    fn withheld(&self, cx: &mut Context<Self>) -> Option<(SharedString, Retry)> {
        let played = self
            .profile
            .as_ref()
            .is_some_and(|profile| !played_perfs(profile).is_empty());
        let page = cx.entity().downgrade();
        played.then(|| {
            let retry: Retry = Rc::new(move |_, cx| {
                let _ = page.update(cx, |page, cx| page.load_activity(cx));
            });
            (
                SharedString::from("Lichess isn’t sending this player’s rating history right now."),
                retry,
            )
        })
    }

    fn username(&self) -> Option<String> {
        self.profile
            .as_ref()
            .and_then(|profile| profile["username"].as_str().map(str::to_string))
    }

    /// Open one of their games on the analysis board, from their side.
    fn open_game(game: Value, window: &mut Window, cx: &mut App) {
        let white = game["color"] != json!("black");
        let (account, opponent) = (game["account"].clone(), game["opponent"].clone());
        let (white_name, black_name) = if white {
            (account, opponent)
        } else {
            (opponent, account)
        };
        let color = if white {
            shakmaty::Color::White
        } else {
            shakmaty::Color::Black
        };
        open_exported(
            game["id"].as_str().unwrap_or_default(),
            white_name,
            black_name,
            color,
            window,
            cx,
        );
    }

    fn open_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(username), Some(account)) = (self.username(), Self::active_account(cx)) else {
            return;
        };
        let dialog = cx.new(|cx| MessageDialog::new(account, username.clone(), window, cx));
        let field = dialog.read(cx).text.clone();
        window.open_dialog(cx, move |modal, _, _| {
            modal
                .title(format!("Message {username}"))
                .w(px(480.))
                .child(dialog.clone())
        });
        window.defer(cx, move |window, cx| {
            field.update(cx, |field, cx| field.focus(window, cx));
        });
    }

    fn search(&self, followed: &[String], cx: &mut Context<Self>, palette: &Palette) -> Div {
        let recent: Vec<String> = crate::prefs::get(cx, RECENT_KEY, Vec::new());
        let names = suggestions(&recent, followed);
        let empty = self.profile.is_none();
        let typed = !self.input.read(cx).value().trim().is_empty();
        widgets::card(palette)
            .gap(px(10.))
            .when(empty, |card| {
                card.py(px(28.)).items_center().child(
                    v_flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            icon("user-search")
                                .size(px(22.))
                                .text_color(palette.text_muted),
                        )
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Look up a player"),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap(px(8.))
                    .when(empty, |row| row.w(px(480.)))
                    .when(!empty, |row| row.w_full())
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.input).aria_label("Lichess username")),
                    )
                    .child(
                        Button::new("players-lookup")
                            .primary()
                            .label("Look up")
                            .loading(self.loading)
                            .disabled(!typed)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.lookup(None, window, cx)),
                            ),
                    ),
            )
            .when(!names.is_empty(), |card| {
                card.child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(4.))
                        .when(empty, |row| row.justify_center())
                        .children(names.into_iter().enumerate().map(|(index, name)| {
                            Button::new(("players-suggestion", index))
                                .xsmall()
                                .ghost()
                                .label(name.clone())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.show_player(&name, window, cx)
                                }))
                        })),
                )
            })
    }

    fn profile_card(&self, profile: &Value, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let username = profile["username"].as_str().unwrap_or_default().to_string();
        let accounts = crate::accounts::handle(cx).read(cx);
        let me = accounts.active(cx);
        let connected = !accounts.connected().is_empty();
        let followed = accounts.all().iter().any(|account| {
            account["username"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(&username))
        });
        let busy = accounts.busy;
        let not_me = me
            .as_deref()
            .is_none_or(|me| !me.eq_ignore_ascii_case(&username));
        let status = self.status.clone().unwrap_or(Value::Null);
        let playing_id = status["playingId"].as_str().map(str::to_string);
        let title = profile["title"].as_str();
        let mut line = format!(
            "{} games · joined {}",
            widgets::format_count(profile["count"]["all"].as_f64().unwrap_or_default()),
            profile["createdAt"].as_f64().map_or("?".into(), local_date)
        );
        if let Some(seen) = profile["seenAt"].as_f64() {
            line.push_str(&format!(" · seen {}", local_date(seen)));
        }
        let score = self
            .record
            .as_ref()
            .zip(me.as_ref())
            .and_then(|(record, me)| {
                let games = record["nbGames"].as_f64().filter(|games| *games > 0.)?;
                let of = |value: &Value, name: &str| {
                    value["users"][name.to_lowercase()]
                        .as_f64()
                        .unwrap_or_default()
                };
                let mut text = format!(
                    "You (@{me}) {} – {} {username} over {games} games",
                    of(record, me),
                    of(record, &username)
                );
                if record["matchup"]["nbGames"]
                    .as_f64()
                    .is_some_and(|games| games > 0.)
                {
                    text.push_str(&format!(
                        " · today {} – {}",
                        of(&record["matchup"], me),
                        of(&record["matchup"], &username)
                    ));
                }
                Some(text)
            });
        let (follow_name, challenge_name) = (username.clone(), username.clone());
        widgets::card(palette)
            .gap(px(12.))
            .child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .flex_wrap()
                    .gap(px(10.))
                    .child(
                        v_flex()
                            .gap(px(2.))
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::BOLD)
                                    .children(title.map(|title| {
                                        widgets::badge(title.to_string(), Look::Primary, palette)
                                    }))
                                    .child(username.clone())
                                    .when(status["online"] == json!(true), |row| {
                                        row.child(widgets::badge(
                                            if status["playing"] == json!(true) {
                                                "Playing"
                                            } else {
                                                "Online"
                                            },
                                            Look::Success,
                                            palette,
                                        ))
                                    }),
                            )
                            .child(widgets::hint(line, palette)),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(px(6.))
                            .when(!followed, |row| {
                                row.child(
                                    Button::new("players-follow")
                                        .small()
                                        .outline()
                                        .icon(icon("user-plus"))
                                        .label("Follow player")
                                        .disabled(busy)
                                        .on_click(move |_, window, cx| {
                                            let name = follow_name.clone();
                                            crate::accounts::handle(cx).update(
                                                cx,
                                                |accounts, cx| {
                                                    accounts.follow(name, window, cx, |_, _, _| {})
                                                },
                                            )
                                        }),
                                )
                            })
                            .when(connected && not_me, |row| {
                                row.child(
                                    Button::new("players-challenge")
                                        .small()
                                        .primary()
                                        .icon(icon("swords"))
                                        .label("Challenge")
                                        .on_click(move |_, window, cx| {
                                            crate::shell::challenge(&challenge_name, window, cx)
                                        }),
                                )
                                .child(
                                    Button::new("players-message")
                                        .small()
                                        .outline()
                                        .icon(icon("message-circle"))
                                        .label("Message")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_message(window, cx)
                                        })),
                                )
                            })
                            .children(playing_id.map(|id| {
                                Button::new("players-watch")
                                    .small()
                                    .outline()
                                    .icon(icon("tv"))
                                    .label("Watch")
                                    .on_click(move |_, window, cx| {
                                        crate::shell::watch_game(&id, window, cx)
                                    })
                            })),
                    ),
            )
            .when(!self.extras_failed.is_empty(), |card| {
                card.child(widgets::hint(
                    format!("Could not load {}.", self.extras_failed.join(" or ")),
                    palette,
                ))
            })
            .children(
                profile["profile"]["bio"]
                    .as_str()
                    .map(|bio| div().text_size(px(13.)).child(bio.to_string())),
            )
            .children(score.map(|score| {
                h_flex()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .bg(palette.elevated)
                    .text_size(px(13.))
                    .child(icon("scale").size(px(16.)))
                    .child(score)
            }))
            .child(self.history.clone())
    }

    fn record_card(&self, palette: &Palette) -> Div {
        let mut card = widgets::card(palette).gap(px(10.)).child(
            h_flex()
                .justify_between()
                .items_start()
                .gap(px(10.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(widgets::heading("Record", palette))
                        .children(self.stats.as_ref().map(|stats| {
                            let mut parts = Vec::new();
                            if let Some(deviation) = stats["deviation"].as_f64() {
                                parts.push(format!("± {}", deviation.round()));
                            }
                            if let Some(rank) = stats["rank"].as_f64() {
                                parts.push(format!("rank #{rank}"));
                            }
                            if let Some(percentile) = stats["percentile"].as_f64() {
                                parts.push(format!("better than {percentile}% of players"));
                            }
                            h_flex()
                                .flex_wrap()
                                .gap(px(6.))
                                .items_baseline()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(
                                    div()
                                        .text_size(px(16.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(palette.highlighted)
                                        .child(
                                            stats["rating"]
                                                .as_f64()
                                                .map_or("–".into(), |r| r.round().to_string()),
                                        ),
                                )
                                .child(parts.join(" · "))
                        })),
                )
                .child(
                    div()
                        .w(px(150.))
                        .flex_none()
                        .child(Select::new(&self.perf_select).small()),
                ),
        );
        if let Some(error) = &self.stats_error {
            return card.child(
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error.clone()),
            );
        }
        let Some(stats) = &self.stats else {
            return card.child(widgets::hint("Loading…", palette));
        };
        let count = |key: &str| stats["count"][key].as_f64().unwrap_or_default();
        let mark = |key: &str| match stats[key]["rating"].as_f64() {
            Some(rating) => format!(
                "{rating} · {}",
                stats[key]["at"]
                    .as_str()
                    .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                    .map(|at| at.format("%-d %b %Y").to_string())
                    .or_else(|| stats[key]["at"].as_f64().map(local_date))
                    .unwrap_or_default()
            ),
            None => "–".into(),
        };
        let streak = |key: &str, word: &str| {
            format!(
                "{} now · {word} {}",
                stats[key]["current"].as_f64().unwrap_or_default(),
                stats[key]["best"].as_f64().unwrap_or_default()
            )
        };
        let rows = [
            (
                "Games",
                format!("{} · {} rated", count("all"), count("rated")),
            ),
            ("Opponents' average", count("opAvg").round().to_string()),
            ("Time played", hours(count("seconds"))),
            (
                "Tournament games",
                format!("{} · berserked {}", count("tour"), count("berserk")),
            ),
            ("Highest", mark("highest")),
            ("Lowest", mark("lowest")),
            ("Winning streak", streak("winStreak", "best")),
            ("Losing streak", streak("lossStreak", "worst")),
        ];
        card = card
            .child(widgets::result_row(
                "Results",
                None,
                (count("win"), count("draw"), count("loss")),
                palette,
            ))
            .child(v_flex().children(rows.into_iter().map(|(label, value)| {
                h_flex()
                    .justify_between()
                    .gap(px(16.))
                    .py(px(7.))
                    .border_t_1()
                    .border_color(palette.border)
                    .text_size(px(13.))
                    .child(div().text_color(palette.text_muted).child(label))
                    .child(value)
            })));
        let opponents =
            |title: &'static str, list: &Value, none: &'static str, id: &'static str| {
                let entries = list.as_array().cloned().unwrap_or_default();
                v_flex()
                    .flex_1()
                    .min_w(px(160.))
                    .gap(px(4.))
                    .text_size(px(13.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .when(entries.is_empty(), |list| {
                        list.child(div().text_color(palette.text_muted).child(none))
                    })
                    .children(entries.into_iter().enumerate().map(move |(index, entry)| {
                        let name = entry["opponent"].as_str().unwrap_or("?").to_string();
                        h_flex()
                            .justify_between()
                            .gap(px(12.))
                            .child(player_link((id, index), name))
                            .child(
                                div().text_color(palette.text_muted).child(
                                    entry["opponentRating"]
                                        .as_f64()
                                        .map_or(String::new(), |r| r.to_string()),
                                ),
                            )
                    }))
            };
        card.child(
            h_flex()
                .flex_wrap()
                .items_start()
                .gap(px(16.))
                .mt(px(8.))
                .child(opponents(
                    "Best rated wins",
                    &stats["bestWins"],
                    "None yet.",
                    "best-win",
                ))
                .child(opponents(
                    "Worst rated losses",
                    &stats["worstLosses"],
                    "None.",
                    "worst-loss",
                )),
        )
    }

    fn games_card(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let card = widgets::card(palette)
            .gap(px(8.))
            .child(widgets::heading("Recent games", palette));
        if self.games_error {
            return card.child(
                h_flex()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .text_color(palette.text_muted)
                    .child("Lichess isn’t sending this player’s games right now.")
                    .child(
                        Button::new("players-games-retry")
                            .text()
                            .xsmall()
                            .icon(icon("rotate-cw"))
                            .label("Retry")
                            .on_click(cx.listener(|this, _, _, cx| this.load_activity(cx))),
                    ),
            );
        }
        match &self.games {
            None => card.child(widgets::hint("Loading…", palette)),
            Some(games) if games.is_empty() => card.child(widgets::hint("No games yet.", palette)),
            Some(games) => card.children(games.iter().enumerate().map(|(index, game)| {
                let chosen = game.clone();
                game_row(
                    ("players-game", index),
                    game,
                    None,
                    RowDetail::NoReview,
                    move |window, cx| Self::open_game(chosen.clone(), window, cx),
                    palette,
                )
            })),
        }
    }
}

/// A finished Lichess game, fetched as PGN (`exportGame`), on the analysis board facing
/// `orientation`; asks first when that replaces unsaved analysis.
pub fn open_exported(
    id: &str,
    white: Value,
    black: Value,
    orientation: shakmaty::Color,
    window: &mut Window,
    cx: &mut App,
) {
    let export = cx.global::<AppCore>().call("exportGame", vec![json!(id)]);
    let origin = json!({ "white": white, "black": black, "gameId": id });
    window
        .spawn(cx, async move |cx| {
            let pgn = export.await;
            let _ = cx.update(|window, cx| match pgn {
                Ok(Value::String(pgn)) => {
                    crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
                        analysis.open_game(
                            &pgn,
                            0,
                            Some(orientation),
                            Some(origin.clone()),
                            false,
                            cx,
                        )
                    });
                }
                Ok(_) => window.push_notification("That game could not be read.", cx),
                Err(cause) => window
                    .push_notification(format!("Could not open the game: {}", cause.message), cx),
            });
        })
        .detach();
}

/// `PlayerLink.vue`: a name that opens the player's profile.
pub fn player_link(id: impl Into<ElementId>, name: String) -> Stateful<Div> {
    div()
        .id(id)
        .truncate()
        .cursor_pointer()
        .hover(|link| link.underline())
        .child(name.clone())
        .on_click(move |_, window, cx| crate::shell::open_player(&name, window, cx))
}

impl Render for PlayersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let followed: Vec<String> = crate::accounts::handle(cx)
            .read(cx)
            .tracked()
            .iter()
            .filter_map(|friend| friend["username"].as_str().map(str::to_string))
            .collect();
        let search = self.search(&followed, cx, &palette);
        let narrow = window.viewport_size().width < px(1060.);
        let body = self.profile.clone().map(|profile| {
            let profile_card = self.profile_card(&profile, cx, &palette);
            let record = self.record_card(&palette);
            let games = self.games_card(cx, &palette);
            if narrow {
                v_flex()
                    .gap(px(16.))
                    .child(profile_card)
                    .child(record)
                    .child(games)
            } else {
                // Record is a sidebar beside the profile and the games.
                h_flex()
                    .items_start()
                    .gap(px(16.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(16.))
                            .child(profile_card)
                            .child(games),
                    )
                    .child(div().w(px(380.)).flex_none().child(record))
            }
        });
        div()
            .id("players-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap(px(16.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .child(search)
                    .children(self.error.clone().map(|error| {
                        div()
                            .text_size(px(13.))
                            .text_color(palette.error)
                            .child(error)
                    }))
                    .children(body),
            )
    }
}

/// The message dialog: a private message from the active account.
pub struct MessageDialog {
    account: String,
    username: String,
    text: Entity<TextareaState>,
    busy: bool,
    error: Option<SharedString>,
    /// The account was connected before messaging was asked for, and must connect again.
    reconnect: bool,
    _subscriptions: Vec<Subscription>,
}

impl MessageDialog {
    fn new(
        account: String,
        username: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MessageDialog {
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(5)
                .placeholder("Write a message")
        });
        let subscriptions = vec![cx.observe(&text, |_, _, cx| cx.notify())];
        MessageDialog {
            account,
            username,
            text,
            busy: false,
            error: None,
            reconnect: false,
            _subscriptions: subscriptions,
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.read(cx).value().trim().to_string();
        if text.is_empty() || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let send = cx.global::<AppCore>().call(
            "sendMessage",
            vec![json!(self.account), json!(self.username), json!(text)],
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = send.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match answer {
                    Ok(result) if result.get("needsReconnect").is_some() => this.reconnect = true,
                    Ok(_) => {
                        window.close_dialog(cx);
                        window.push_notification(format!("Message sent to {}", this.username), cx);
                    }
                    Err(cause) => this.error = Some(message_of(&cause)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for MessageDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let empty = self.text.read(cx).value().trim().is_empty();
        v_flex()
            .gap(px(12.))
            .child(widgets::hint(
                format!("Sent from @{} as a Lichess private message.", self.account),
                &palette,
            ))
            .when(self.reconnect, |dialog| {
                dialog.child(
                    h_flex()
                        .gap(px(8.))
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(crate::theme::tint(palette.warning, 10.))
                        .text_size(px(13.))
                        .child(icon("key-round").size(px(16.)).text_color(palette.warning))
                        .child(div().flex_1().child(format!(
                            "Lichess needs a new permission. Connect @{} again to allow sending messages.",
                            self.account
                        )))
                        .child(
                            Button::new("message-reconnect")
                                .small()
                                .outline()
                                .icon(icon("link"))
                                .label("Reconnect")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                    crate::accounts::handle(cx)
                                        .update(cx, |accounts, cx| accounts.connect(window, cx));
                                }),
                        ),
                )
            })
            .child(Textarea::new(&self.text).aria_label("Message"))
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(
                h_flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("message-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("message-send")
                            .primary()
                            .icon(icon("send"))
                            .label("Send")
                            .loading(self.busy)
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{hours, is_username, played_perfs, suggestions};
    use serde_json::json;

    #[test]
    fn usernames_are_checked_before_asking() {
        assert!(is_username("Magnus_C-1"));
        assert!(!is_username("a"));
        assert!(!is_username("two words"));
        assert!(!is_username(&"x".repeat(31)));
    }

    #[test]
    fn played_speeds_come_most_played_first() {
        let profile = json!({ "perfs": {
            "blitz": { "games": 10 },
            "rapid": { "games": 40 },
            "bullet": { "games": 0 },
            "puzzle": { "games": 900 },
        }});
        let keys: Vec<_> = played_perfs(&profile)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, ["rapid", "blitz"]);
    }

    #[test]
    fn suggestions_name_each_player_once() {
        let recent = vec!["Alice".to_string(), "bob".to_string()];
        let followed = vec!["Bob".to_string(), "Carol".to_string()];
        assert_eq!(suggestions(&recent, &followed), ["Alice", "bob", "Carol"]);
    }

    #[test]
    fn play_time_reads_in_hours_or_minutes() {
        assert_eq!(hours(7200.), "2 h");
        assert_eq!(hours(1500.), "25 min");
    }
}

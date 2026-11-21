//! Players: look up any Lichess player — their profile, whether they are
//! online, your record against them, their rating history, their record in a speed, and their
//! recent games — and follow, challenge, message or watch them.

mod cards;
mod message;
mod rating_speeds;

use crate::loading::{Epoch, Loading, Turn};
use crate::pages::PageView;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{MessageText, PresenceUsers, Username};
use kchess_contracts::types::{
    ActivityDay, Crosstable, LeaderboardEntry, LichessGame, LichessRelation, PerfResult, PerfStats,
    PerfType, RelationAction, Reply, UserPresence,
};
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::LichessUser;
use kchess_domain::perf::{Perf, Speed};

use crate::assets::icon;
use crate::core::AppCore;
use crate::pages::history::lichess::{RowDetail, game_row};
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::player_search::{self, NameMatches, SUGGESTED, chip_row, name_chip};
use crate::rating_history::{Retry, has_points};
use crate::theme::{Palette, radii};
use crate::widgets::{self, Choice, Look};
use message::MessageDialog;
use rating_speeds::RatingSpeeds;

const BOARD_PERF_KEY: &str = "kchess:leaderboard-perf";
/// Which players the leaderboard shows: `all`, `titled` or one title.
const BOARD_TITLE_KEY: &str = "kchess:leaderboard-title";
/// Lichess's titles, in the order offered.
const TITLES: [&str; 11] = [
    "GM", "IM", "FM", "CM", "NM", "WGM", "WIM", "WFM", "WCM", "LM", "BOT",
];
/// Rating categories with a Lichess leaderboard: every one but correspondence.
fn board_perfs() -> impl Iterator<Item = Perf> {
    Perf::ALL
        .into_iter()
        .filter(|perf| *perf != Perf::Speed(Speed::Correspondence))
}

/// The rating categories with their names, in `PERF_TYPES` order.
fn perfs() -> impl Iterator<Item = (&'static str, &'static str)> {
    use kchess_domain::misc::constants::PERF_TYPES;
    use kchess_domain::records::ratings::perf_label;
    PERF_TYPES
        .iter()
        .map(|key| (*key, perf_label(key).unwrap_or(key)))
}

/// The rating category a key of `PERF_TYPES` (or of the leaderboard) names; blitz for any other.
fn perf_type(key: &str) -> PerfType {
    PerfType::ALL
        .iter()
        .copied()
        .find(|perf| perf.as_str() == key)
        .unwrap_or(PerfType::Blitz)
}

/// The speeds a player has games in, most played first.
fn played_perfs(profile: &LichessUser) -> Vec<(&'static str, &'static str)> {
    profile.played(perfs())
}

/// Recent lookups, then followed players not among them, each name once and at most
/// `SUGGESTED` in all.
fn suggestions(recent: &[String], followed: &[String]) -> (Vec<String>, Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut once = |name: &&String| seen.insert(name.to_lowercase());
    let recent: Vec<String> = recent
        .iter()
        .filter(&mut once)
        .take(SUGGESTED)
        .cloned()
        .collect();
    let followed = followed
        .iter()
        .filter(&mut once)
        .take(SUGGESTED - recent.len())
        .cloned()
        .collect();
    (recent, followed)
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
    crate::dates::local(millis)
        .map(|date| date.strftime("%-d %b %Y").to_string())
        .unwrap_or_else(|| "?".into())
}

/// The open player's actions, as their buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Follow,
    Challenge,
    Message,
    Watch,
}

/// Follow a player with the active account.
fn follow(name: String, window: &mut Window, cx: &mut App) {
    crate::accounts::handle(cx).update(cx, |accounts, cx| {
        accounts.follow(name, window, cx, |_, _, _| {})
    });
}

pub struct PlayersPage {
    input: Entity<InputState>,
    profile: Option<LichessUser>,
    /// `UserPresence`, once known.
    status: Option<UserPresence>,
    perf: String,
    perf_select: Entity<SelectState<Vec<Choice>>>,
    stats: Option<PerfStats>,
    /// `stats`' load, in the chosen speed.
    stats_loading: Loading,
    /// `Crosstable` between the active account and the player.
    record: Option<Crosstable>,
    /// What could not be read beside the profile, so "none" is not implied.
    extras_failed: Vec<&'static str>,
    games: Option<Vec<LichessGame>>,
    games_error: bool,
    history: Entity<RatingSpeeds>,
    /// `ActivityDay`s, once Lichess sent them.
    days: Option<Vec<ActivityDay>>,
    days_error: bool,
    /// `LichessRelation` of the online account with the player; None without one to ask.
    relation: Option<LichessRelation>,
    /// The online account `record` and `relation` were read as, so switching accounts reads them
    /// again rather than showing another account's.
    extras_account: Option<String>,
    /// Each read of `record` and `relation`; answers to an older one are dropped.
    account_epoch: Epoch,
    /// The leaderboard shown while no player is open.
    board: Option<Vec<LeaderboardEntry>>,
    /// `board`'s load.
    board_loading: Loading,
    board_perf: String,
    board_select: Entity<SelectState<Vec<Choice>>>,
    board_title: String,
    title_select: Entity<SelectState<Vec<Choice>>>,
    /// The `board_loading` request whose titles `title_select` offers.
    title_choices_for: Option<Turn>,
    /// `PlayerSuggestion`s for what is typed.
    names: NameMatches,
    /// Each lookup of a player: the profile, and the requests that follow from it, which count
    /// only while it is the latest.
    profile_loading: Loading,
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
        let history = cx.new(|cx| RatingSpeeds::new("Built from the last 200 rated games", cx));
        let accounts = crate::accounts::handle(cx);
        let board_perf: String = crate::prefs::get(cx, BOARD_PERF_KEY, "blitz".to_string());
        let board_items: Vec<Choice> = board_perfs()
            .map(|perf| Choice::new(perf.key(), perf.label()))
            .collect();
        let chosen = SharedString::from(board_perf.clone());
        let board_select = cx.new(|cx| {
            let mut select = SelectState::new(board_items, None, window, cx);
            select.set_selected_value(&chosen, window, cx);
            select
        });
        let board_title: String = crate::prefs::get(cx, BOARD_TITLE_KEY, "all".to_string());
        let title_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                match event {
                    InputEvent::PressEnter { .. } => this.lookup(None, window, cx),
                    InputEvent::Change => {
                        // A message about what was typed before goes with it.
                        this.profile_loading.clear_error();
                        this.search_names(cx)
                    }
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
            cx.subscribe(
                &board_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event
                        && *value != this.board_perf.as_str()
                    {
                        this.board_perf = value.to_string();
                        crate::prefs::set(cx, BOARD_PERF_KEY, this.board_perf.clone());
                        this.load_board(cx);
                    }
                },
            ),
            cx.subscribe(
                &title_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.show_titles(value.to_string(), cx);
                    }
                },
            ),
            cx.observe(&accounts, |this, _, cx| {
                this.account_changed(cx);
                cx.notify();
            }),
        ];
        PlayersPage {
            input,
            profile: None,
            status: None,
            perf: "blitz".into(),
            perf_select,
            stats: None,
            stats_loading: Loading::default(),
            record: None,
            extras_failed: Vec::new(),
            games: None,
            games_error: false,
            history,
            days: None,
            days_error: false,
            relation: None,
            extras_account: None,
            account_epoch: Epoch::default(),
            board: None,
            board_loading: Loading::default(),
            board_perf,
            board_select,
            board_title,
            title_select,
            title_choices_for: None,
            names: NameMatches::default(),
            profile_loading: Loading::default(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
        .with_board(cx)
    }

    fn with_board(mut self, cx: &mut Context<Self>) -> Self {
        self.load_board(cx);
        self
    }

    /// The leaderboard of the chosen category, while no player is open.
    fn load_board(&mut self, cx: &mut Context<Self>) {
        self.board = None;
        let board = cx.global::<AppCore>().request(calls::Leaderboard {
            perf: perf_type(&self.board_perf),
        });
        self.board_loading.load(
            cx,
            "players",
            |this: &mut Self| &mut this.board_loading,
            board,
            |this, answer, _| this.board = answer.ok(),
        );
        cx.notify();
    }

    /// Show only the leaderboard's players with `filter` (`all`, `titled` or one title).
    fn show_titles(&mut self, filter: String, cx: &mut Context<Self>) {
        if filter != self.board_title {
            self.board_title = filter;
            crate::prefs::set(cx, BOARD_TITLE_KEY, self.board_title.clone());
            // The chosen filter is offered even when the board has none of its players.
            self.title_choices_for = None;
            cx.notify();
        }
    }

    /// Offer the titles the loaded leaderboard has, each with how many of its players hold it.
    fn sync_title_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(board) = &self.board else {
            return;
        };
        if self.title_choices_for == Some(self.board_loading.current()) {
            return;
        }
        self.title_choices_for = Some(self.board_loading.current());
        let items = title_choices(board, &self.board_title);
        let chosen = SharedString::from(self.board_title.clone());
        self.title_select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_value(&chosen, window, cx);
        });
    }

    /// Players whose names start with what is typed, once typing settles.
    fn search_names(&mut self, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value().to_string();
        let shown = self.username();
        self.names.search(
            &typed,
            shown.as_deref(),
            |page: &mut Self| &mut page.names,
            cx,
        );
    }

    /// A profile link opened elsewhere (`/players?name=`): look the player up, unless shown.
    pub fn show_player(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.profile.as_ref().map(|p| p.username.as_str());
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
        if !crate::game::is_username(&name) {
            // This search replaces one still on its way: its answer would show that profile
            // under this error and put its name back in the box over what was typed.
            self.profile_loading.refuse(if name.is_empty() {
                "Enter a Lichess username"
            } else {
                "Lichess usernames are 2–30 letters, digits, - or _"
            });
            cx.notify();
            return;
        }
        let turn = self.profile_loading.start();
        self.stats_loading.stop();
        self.stats = None;
        self.record = None;
        self.status = None;
        self.extras_failed.clear();
        self.games = None;
        self.games_error = false;
        self.days = None;
        self.days_error = false;
        self.relation = None;
        self.account_epoch.invalidate();
        self.names.clear();
        self.history.update(cx, |history, cx| {
            history.set(Vec::new(), Vec::new(), cx);
            history.set_withheld(None, cx);
        });
        cx.notify();
        let profile = cx.global::<AppCore>().request(calls::Profile {
            username: Username(name.clone()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let found = profile.await;
            this.update_in(cx, |this, window, cx| {
                if !this.profile_loading.settle(turn, "players", &found) {
                    return;
                }
                match found {
                    Ok(found) => this.found(found, window, cx),
                    Err(_) => this.profile = None,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn found(&mut self, found: LichessUser, window: &mut Window, cx: &mut Context<Self>) {
        let username = found.username.clone();
        self.input.update(cx, |input, cx| {
            input.set_value(username.clone(), window, cx)
        });
        player_search::remember(&username, cx);
        let current = found.perfs.clone();
        self.history
            .update(cx, |history, cx| history.set_current(current, cx));
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
        self.load_days(&username, cx);
    }

    /// What the player did lately on Lichess.
    fn load_days(&mut self, username: &str, cx: &mut Context<Self>) {
        let epoch = self.profile_loading.current();
        let days = cx.global::<AppCore>().request(calls::Activity {
            username: Username(username.to_string()),
        });
        cx.spawn(async move |this, cx| {
            let answer = days.await;
            let _ = this.update(cx, |this, cx| {
                if !this.profile_loading.is_current(epoch) {
                    return;
                }
                match answer {
                    Ok(days) => this.days = Some(days),
                    Err(cause) => {
                        crate::loading::failure("players", &cause);
                        this.days_error = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Block or unblock the player as the online account; blocking asks first.
    fn toggle_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(username), Some(relation)) = (self.username(), self.relation) else {
            return;
        };
        let blocking = !relation.blocking;
        let account_epoch = self.account_epoch.current();
        let page = cx.entity().downgrade();
        let apply = move |window: &mut Window, cx: &mut App| {
            let page = page.clone();
            crate::accounts::handle(cx).update(cx, |accounts, cx| {
                accounts.set_relation(
                    username.clone(),
                    if blocking {
                        RelationAction::Block
                    } else {
                        RelationAction::Unblock
                    },
                    window,
                    cx,
                    move |took, _, cx| {
                        if took {
                            let _ = page.update(cx, |page, cx| {
                                // Another account or player since: its relation is read anew.
                                if !page.account_epoch.is_current(account_epoch) {
                                    return;
                                }
                                let following = !blocking && relation.following;
                                page.relation = Some(LichessRelation {
                                    following,
                                    blocking,
                                });
                                cx.notify();
                            });
                        }
                    },
                )
            });
        };
        if !blocking {
            apply(window, cx);
            return;
        }
        let name = self.username().unwrap_or_default();
        let account = Self::active_account(cx).unwrap_or_default();
        let apply = std::rc::Rc::new(apply);
        crate::widgets::open_alert_dialog(
            window,
            cx,
            move |dialog: gpui_kit::component::dialog::AlertDialog, _, _| {
                let apply = apply.clone();
                dialog
                    .title(format!("Block {name}?"))
                    .description(format!(
                        "@{account} stops seeing their messages, challenges and chat on Lichess"
                    ))
                    .ok_text("Block")
                    .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                    .cancel_text("Cancel")
                    .show_cancel(true)
                    .on_ok(move |_, window, cx| {
                        apply(window, cx);
                        true
                    })
            },
        );
    }

    /// Your record against them, and whether they are online.
    fn load_extras(&mut self, username: &str, cx: &mut Context<Self>) {
        let epoch = self.profile_loading.current();
        let core = cx.global::<AppCore>().clone();
        self.load_account_extras(username, cx);
        let presence = core.request(calls::Presence {
            usernames: PresenceUsers(vec![username.to_string()]),
        });
        let key = username.to_lowercase();
        cx.spawn(async move |this, cx| {
            let answer = presence.await;
            let _ = this.update(cx, |this, cx| {
                if !this.profile_loading.is_current(epoch) {
                    return;
                }
                match answer {
                    Ok(report) => this.status = report.users.get(&key).cloned(),
                    Err(cause) => {
                        crate::loading::failure("players", &cause);
                        this.extras_failed.push("online status");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The online account changed while a player is open: their record and relation were the
    /// other account's, so read them again as this one.
    fn account_changed(&mut self, cx: &mut Context<Self>) {
        let Some(username) = self.username() else {
            return;
        };
        let now = Self::active_account(cx);
        let same = match (&now, &self.extras_account) {
            (Some(now), Some(before)) => now.eq_ignore_ascii_case(before),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.load_account_extras(&username, cx);
        }
    }

    /// The online account's record against the player and how it stands with them on Lichess.
    fn load_account_extras(&mut self, username: &str, cx: &mut Context<Self>) {
        let account_epoch = self.account_epoch.start();
        self.record = None;
        self.relation = None;
        self.extras_failed.retain(|failed| *failed != "your record");
        self.extras_account = Self::active_account(cx);
        let Some(me) = self
            .extras_account
            .clone()
            .filter(|me| !me.eq_ignore_ascii_case(username))
        else {
            return;
        };
        let core = cx.global::<AppCore>().clone();
        let relation = core.request(calls::Relation {
            account: Username(me.clone()),
            username: Username(username.to_string()),
        });
        cx.spawn(async move |this, cx| {
            let answer = relation.await.warn_on_err();
            let _ = this.update(cx, |this, cx| {
                if this.account_epoch.is_current(account_epoch)
                    && let Some(Reply::Done(relation)) = answer
                {
                    this.relation = Some(relation);
                    cx.notify();
                }
            });
        })
        .detach();
        let crosstable = core.request(calls::Crosstable {
            a: Username(me),
            b: Username(username.to_string()),
        });
        cx.spawn(async move |this, cx| {
            let answer = crosstable.await;
            let _ = this.update(cx, |this, cx| {
                if !this.account_epoch.is_current(account_epoch) {
                    return;
                }
                match answer {
                    Ok(record) => this.record = Some(record),
                    Err(cause) => {
                        crate::loading::failure("players", &cause);
                        this.extras_failed.push("your record");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Their record in the chosen speed.
    fn load_stats(&mut self, cx: &mut Context<Self>) {
        let Some(username) = self.username() else {
            return;
        };
        self.stats = None;
        let stats = cx.global::<AppCore>().request(calls::PlayerPerf {
            username: Username(username),
            perf: perf_type(&self.perf),
        });
        self.stats_loading.load(
            cx,
            "players",
            |this: &mut Self| &mut this.stats_loading,
            stats,
            |this, answer, _| this.stats = answer.ok(),
        );
        cx.notify();
    }

    /// Lichess's rating history (or one rebuilt from their recent
    /// rated games when it sends none) and their recent games.
    fn load_activity(&mut self, cx: &mut Context<Self>) {
        let Some(username) = self.username() else {
            return;
        };
        let epoch = self.profile_loading.current();
        self.games = None;
        self.games_error = false;
        let core = cx.global::<AppCore>().clone();
        let games = core.request(calls::RecentGames {
            username: Username(username.clone()),
            rated: None,
        });
        cx.spawn(async move |this, cx| {
            let answer = games.await;
            let _ = this.update(cx, |this, cx| {
                if !this.profile_loading.is_current(epoch) {
                    return;
                }
                match answer {
                    Ok(games) => this.games = Some(games),
                    // Lichess sometimes answers "not found" for every player's games.
                    Err(cause) => {
                        crate::loading::failure("players", &cause);
                        this.games_error = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        let official = core.request(calls::RatingHistory {
            username: Username(username.clone()),
        });
        cx.spawn(async move |this, cx| {
            let official = official.await.warn_on_err().unwrap_or_default();
            let rebuilt = if has_points(&official) {
                None
            } else {
                let rated = core.request(calls::RecentGames {
                    username: Username(username),
                    rated: Some(true),
                });
                let rated = rated.await.warn_on_err().unwrap_or_default();
                Some(kchess_domain::records::ratings::rating_series(
                    rated.iter().filter_map(LichessGame::rated_game),
                ))
            };
            let _ = this.update(cx, |this, cx| {
                if !this.profile_loading.is_current(epoch) {
                    return;
                }
                let withheld = this.withheld(cx);
                this.history.update(cx, |history, cx| {
                    history.set(official, rebuilt.unwrap_or_default(), cx);
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
                SharedString::from("Lichess isn’t sending this player’s rating history right now"),
                retry,
            )
        })
    }

    pub(crate) fn username(&self) -> Option<String> {
        self.profile
            .as_ref()
            .map(|profile| profile.username.clone())
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        let Some(username) = self.username() else {
            return;
        };
        match action {
            Action::Follow => follow(username, window, cx),
            Action::Challenge => crate::pages::online::challenge(&username, window, cx),
            Action::Message => self.open_message(window, cx),
            Action::Watch => {
                let playing = self
                    .status
                    .as_ref()
                    .and_then(|status| status.playing_id.clone());
                if let Some(id) = playing {
                    crate::pages::watch::watch_game(&id, window, cx);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn show_profile(&mut self, profile: LichessUser, cx: &mut Context<Self>) {
        self.profile = Some(profile);
        cx.notify();
    }

    fn open_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(username), Some(account)) = (self.username(), Self::active_account(cx)) else {
            return;
        };
        let dialog = cx.new(|cx| MessageDialog::new(account, username.clone(), window, cx));
        let field = dialog.read(cx).text.clone();
        crate::widgets::open_dialog(window, cx, move |modal, _, _| {
            modal
                .title(format!("Message {username}"))
                .w(px(480.))
                .child(dialog.clone())
        });
        window.defer(cx, move |window, cx| {
            field.update(cx, |field, cx| field.focus(window, cx));
        });
    }
}

/// Whether a player with `title` passes the leaderboard's `filter`.
fn title_shown(filter: &str, title: Option<&str>) -> bool {
    match filter {
        "all" => true,
        "titled" => title.is_some_and(|title| title != "BOT"),
        filter => title == Some(filter),
    }
}

/// The leaderboard's title filters: everyone, the titled, and each title its players hold, with
/// how many; `chosen` is kept even when none hold it.
fn title_choices(board: &[LeaderboardEntry], chosen: &str) -> Vec<Choice> {
    let held = |filter: &str| {
        board
            .iter()
            .filter(|entry| title_shown(filter, entry.title.as_deref()))
            .count()
    };
    let titled = held("titled");
    let mut choices = vec![Choice::new("all", "All players")];
    if titled > 0 || chosen == "titled" {
        choices.push(Choice::new("titled", format!("Titled ({titled})")));
    }
    choices.extend(TITLES.iter().filter_map(|title| {
        let count = held(title);
        (count > 0 || chosen == *title).then(|| Choice::new(*title, format!("{title} ({count})")))
    }));
    choices
}

// A name that opens the player's profile, on any page (`kchess_board::player_search`).
pub use crate::player_search::player_link;

impl Render for PlayersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let followed: Vec<String> = crate::accounts::handle(cx)
            .read(cx)
            .tracked()
            .into_iter()
            .map(|friend| friend.username)
            .collect();
        let search = self.search(&followed, cx, &palette);
        let narrow = window.viewport_size().width < px(1060.);
        let body = self.profile.clone().map(|profile| {
            let profile_card = self.profile_card(&profile, cx, &palette);
            let record = self.record_card(&palette, cx);
            let games = self.games_card(cx, &palette);
            let days = self.days_card(narrow, &palette);
            if narrow {
                v_flex()
                    .gap(px(16.))
                    .child(profile_card)
                    .child(record)
                    .child(games)
                    .child(days)
            } else {
                // Record is a sidebar beside the profile and the games; activity runs under both.
                v_flex()
                    .gap(px(16.))
                    .child(
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
                            .child(div().w(px(380.)).flex_none().child(record)),
                    )
                    .child(days)
            }
        });
        let board = (self.profile.is_none() && crate::network::online(cx)).then(|| {
            self.sync_title_choices(window, cx);
            self.board_card(&palette, cx)
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
                    .children(board)
                    .children(body),
            )
    }
}

impl crate::pages::PageAction for Action {
    fn info(&self) -> crate::pages::ActionInfo {
        use crate::pages::ActionInfo;
        match self {
            Action::Follow => {
                ActionInfo::new("players:follow", "Follow player", "user-plus", "friend add")
            }
            Action::Challenge => ActionInfo::new(
                "players:challenge",
                "Challenge",
                "swords",
                "play game invite",
            ),
            Action::Message => {
                ActionInfo::new("players:message", "Message", "message-circle", "send write")
            }
            Action::Watch => ActionInfo::new("players:watch", "Watch", "tv", "spectate game"),
        }
    }
}

impl PageView for PlayersPage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: crate::pages::Page::Players,
        label: "Players",
        short: "Players",
        icon: "user-search",
        group: Some(crate::pages::NavGroup::Lichess),
        keywords: "lookup profile user",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        PlayersPage::new(window, cx)
    }

    fn search_fields(&self) -> Vec<Entity<InputState>> {
        vec![self.input.clone()]
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        let Some(username) = self.username() else {
            return Vec::new();
        };
        let accounts = crate::accounts::handle(cx).read(cx);
        let followed = accounts
            .all()
            .iter()
            .any(|account| account.username.eq_ignore_ascii_case(&username));
        let not_me = accounts
            .active(cx)
            .as_deref()
            .is_none_or(|me| !me.eq_ignore_ascii_case(&username));
        let mut actions = Vec::new();
        if !followed && !accounts.busy {
            actions.push(Action::Follow);
        }
        if !accounts.connected().is_empty() && not_me {
            actions.extend([Action::Challenge, Action::Message]);
        }
        if self
            .status
            .as_ref()
            .is_some_and(|status| status.playing_id.is_some())
        {
            actions.push(Action::Watch);
        }
        actions
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.act(action, window, cx));
    }
}

/// A player's profile on Players.
pub fn open_player(username: &str, window: &mut Window, cx: &mut App) {
    let username = username.to_string();
    crate::shell::show::<PlayersPage>(window, cx, move |players, window, cx| {
        players.show_player(&username, window, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::{hours, played_perfs, suggestions, title_choices, title_shown};
    use crate::game::is_username;
    use serde_json::json;

    /// An invalid name searched while a lookup was on its way left that lookup current: its
    /// answer showed the old player under the error and put their name back over the typing.
    #[gpui_kit::test]
    fn an_invalid_search_replaces_the_lookup_on_its_way(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = crate::e2e::Harness::start(cx);
        let (loading, stale) = app.with_window(|window, cx| {
            let page = cx.new(|cx| super::PlayersPage::new(window, cx));
            page.update(cx, |page, cx| {
                page.lookup(Some("Alice".into()), window, cx);
                let asked = page.profile_loading.current();
                page.lookup(Some("a".into()), window, cx);
                (
                    page.profile_loading.busy(),
                    page.profile_loading.is_current(asked),
                )
            })
        });
        assert!(!loading && !stale);
    }

    #[test]
    fn usernames_are_checked_before_asking() {
        assert!(is_username("Magnus_C-1"));
        assert!(!is_username("a"));
        assert!(!is_username("two words"));
        assert!(!is_username(&"x".repeat(31)));
    }

    #[test]
    fn played_speeds_come_most_played_first() {
        let profile: kchess_contracts::types::LichessUser =
            serde_json::from_value(json!({ "perfs": {
                "blitz": { "games": 10 },
                "rapid": { "games": 40 },
                "bullet": { "games": 0 },
                "puzzle": { "games": 900 },
            }}))
            .expect("a user");
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
        let (recent, followed) = suggestions(&recent, &followed);
        assert_eq!(recent, ["Alice", "bob"]);
        assert_eq!(followed, ["Carol"]);
    }

    #[test]
    fn play_time_reads_in_hours_or_minutes() {
        assert_eq!(hours(7200.), "2 h");
        assert_eq!(hours(1500.), "25 min");
    }

    #[test]
    fn titles_are_offered_as_the_board_holds_them() {
        let entry = |title: Option<&str>| kchess_contracts::types::LeaderboardEntry {
            username: "p".into(),
            title: title.map(str::to_string),
            rating: 2500.,
            progress: 0.,
            online: None,
        };
        let board = [
            entry(Some("GM")),
            entry(None),
            entry(Some("IM")),
            entry(Some("GM")),
        ];
        let labels = |chosen: &str| {
            title_choices(&board, chosen)
                .into_iter()
                .map(|choice| choice.label.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels("all"),
            ["All players", "Titled (3)", "GM (2)", "IM (1)"]
        );
        // A saved title the board lacks stays choosable, at none.
        assert_eq!(
            labels("WGM"),
            ["All players", "Titled (3)", "GM (2)", "IM (1)", "WGM (0)"]
        );
        assert!(!title_shown("titled", Some("BOT")));
        assert!(!title_shown("GM", None));
    }
}

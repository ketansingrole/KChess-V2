//! Following:
//! the players followed in KChess — their ratings, whether they are online or playing, how the
//! connected accounts have fared against them, and their synced games — with follow, import
//! from Lichess, sync, clear and remove.

use crate::loading::Epoch;
use kchess_contracts::calls;
use kchess_contracts::check::{FriendList, PresenceUsers, Username};
use kchess_contracts::types::{
    Account, FollowedUser, FollowingProblem, FollowingReport, GameRecord, TimelineEntry,
    UserPresence,
};
use std::collections::BTreeMap;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use crate::accounts::{Accounts, AccountsHandle};
use crate::assets::icon;
use crate::core::AppCore;
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, KeyboardFocus as _, Look};

/// Presence is asked for this many players at a time, and again this often.
const PRESENCE_CHUNK: usize = 50;
/// The most players `profiles` takes at once.
const PROFILES_CHUNK: usize = 1000;
const PRESENCE_EVERY: Duration = Duration::from_secs(20);
/// A saved profile this recent is not asked for again, except by the page's own round of everyone.
const PROFILE_FRESH_MS: f64 = 3_600_000.;
/// How often the page asks for every profile afresh, which tells whose games are new.
const PROFILES_EVERY: Duration = Duration::from_secs(300);

/// Timeline lines shown at first, and how many more each "Show more" adds.
const TIMELINE_FIRST: usize = 12;
const TIMELINE_MORE: usize = 20;

/// The header's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    Follow,
    Import,
    Sync,
}

/// Download every followed player's new games.
fn sync_all(window: &mut Window, cx: &mut App) {
    crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.sync(None, window, cx));
}

pub struct FriendsPage {
    input: Entity<InputState>,
    /// Profiles by lowercase name, and presence (`UserPresence`).
    profiles: HashMap<String, Value>,
    /// When every profile was last asked for.
    profiles_asked: Option<std::time::Instant>,
    status: HashMap<String, UserPresence>,
    loading: bool,
    /// `gameLibraryOverview().versus`: the connected accounts' record against each player.
    versus: BTreeMap<String, GameRecord>,
    /// How many players were followed at the last look.
    followed: usize,
    importer: Option<Entity<ImportDialog>>,
    /// What the players on the list did lately (`TimelineEntry`s), once read.
    timeline: Option<Vec<TimelineEntry>>,
    timeline_error: Option<SharedString>,
    timeline_epoch: Epoch,
    /// How many of its lines the timeline shows.
    timeline_shown: usize,
    focus: FocusHandle,
    _poll: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for FriendsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Whether Lichess has games of the player that the last sync did not download: their play time
/// grows with every finished game, so a profile with more than the sync saved has new ones.
fn has_new_games(friend: &Account, profile: &Value) -> bool {
    match (
        friend.synced_play_time,
        profile["playTime"]["total"].as_f64(),
    ) {
        (Some(synced), Some(now)) => now > synced as f64,
        _ => false,
    }
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

/// "5 minutes ago", for last seen and last synced.
fn ago(millis: f64) -> String {
    crate::studies::time_ago(millis, crate::studies::now())
}

impl FriendsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> FriendsPage {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Lichess username"));
        let accounts = crate::accounts::handle(cx);
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter { .. } => this.follow(window, cx),
                        InputEvent::Change => cx.notify(),
                        _ => {}
                    }
                }),
                cx.observe(&accounts, |this, accounts, cx| {
                    // Someone new (followed or imported): their profile and presence.
                    let count = accounts.read(cx).tracked().len();
                    if count != this.followed {
                        this.followed = count;
                        this.load_profiles(cx);
                        this.refresh_status(cx);
                        this.load_timeline(cx);
                    }
                    this.load_versus(cx);
                    cx.notify();
                }),
            ];
        // Cheap (one small request for everyone), and stops with the page.
        let poll = crate::timer::every(PRESENCE_EVERY, cx, |this, cx| {
            this.refresh_status(cx);
            true
        });
        let mut page = FriendsPage {
            input,
            profiles: HashMap::new(),
            profiles_asked: None,
            status: HashMap::new(),
            loading: false,
            versus: BTreeMap::new(),
            followed: 0,
            importer: None,
            timeline: None,
            timeline_error: None,
            timeline_epoch: Epoch::default(),
            timeline_shown: TIMELINE_FIRST,
            focus: cx.focus_handle(),
            _poll: poll,
            _subscriptions: subscriptions,
        };
        page.shown(cx);
        page
    }

    fn friends(cx: &App) -> Vec<Account> {
        crate::accounts::handle(cx).read(cx).tracked()
    }

    /// The page was shown: usage, profiles and presence afresh.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
        self.load_profiles(cx);
        self.refresh_status(cx);
        self.load_versus(cx);
        self.load_timeline(cx);
    }

    /// What the players on the list did lately, from their Lichess activity.
    fn load_timeline(&mut self, cx: &mut Context<Self>) {
        let epoch = self.timeline_epoch.next();
        self.timeline_error = None;
        if Self::friends(cx).is_empty() || !crate::network::online(cx) {
            self.timeline = None;
            return;
        }
        let timeline = cx.global::<AppCore>().request(calls::Timeline {});
        cx.spawn(async move |this, cx| {
            let answer = timeline.await;
            let _ = this.update(cx, |this, cx| {
                if !this.timeline_epoch.is_current(epoch) {
                    return;
                }
                match answer {
                    Ok(entries) => this.timeline = Some(entries),
                    Err(cause) => this.timeline_error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_versus(&mut self, cx: &mut Context<Self>) {
        let overview = cx
            .global::<AppCore>()
            .request(calls::GameLibraryOverview {});
        crate::loading::fetch(cx, "friends", overview, |this, overview, _| {
            this.versus = overview.versus;
        });
    }

    /// Saved profiles at once, the rest in one request per thousand players; every
    /// few minutes the rest is everyone, so a play time that grew shows as new games.
    fn load_profiles(&mut self, cx: &mut Context<Self>) {
        let names: Vec<String> = Self::friends(cx)
            .into_iter()
            .map(|friend| friend.username)
            .collect();
        if names.is_empty() {
            return;
        }
        let everyone = self
            .profiles_asked
            .is_none_or(|asked| asked.elapsed() >= PROFILES_EVERY);
        if everyone {
            self.profiles_asked = Some(std::time::Instant::now());
        }
        self.loading = true;
        cx.notify();
        let core = cx.global::<AppCore>().clone();
        cx.spawn(async move |this, cx| {
            let mut fresh = HashSet::new();
            for name in &names {
                if let Ok(saved) = core
                    .request(calls::CachedProfile {
                        username: Username(name.clone()),
                    })
                    .await
                    && let Some(profile) = saved.profile.filter(Value::is_object)
                {
                    let recent = saved
                        .profile_fetched_at
                        .is_some_and(|at| crate::studies::now() - (at as f64) < PROFILE_FRESH_MS);
                    if recent && !everyone {
                        fresh.insert(key(name));
                    }
                    let _ = this.update(cx, |this, cx| {
                        this.profiles.entry(key(name)).or_insert(profile);
                        cx.notify();
                    });
                }
            }
            let stale: Vec<String> = names
                .iter()
                .filter(|name| !fresh.contains(&key(name)))
                .cloned()
                .collect();
            for chunk in stale.chunks(PROFILES_CHUNK) {
                if let Ok(users) = core
                    .request(calls::Profiles {
                        usernames: FriendList(chunk.to_vec()),
                    })
                    .await
                {
                    let _ = this.update(cx, |this, cx| {
                        for user in users {
                            if let Some(name) = user["username"].as_str() {
                                this.profiles.insert(key(name), user.clone());
                            }
                        }
                        cx.notify();
                    });
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
                cx.notify();
            });
        })
        .detach();
    }

    /// Who is online or playing, fifty names a request.
    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let names: Vec<String> = Self::friends(cx)
            .into_iter()
            .map(|friend| friend.username)
            .collect();
        let core = cx.global::<AppCore>().clone();
        for chunk in names.chunks(PRESENCE_CHUNK) {
            let presence = core.request(calls::Presence {
                usernames: PresenceUsers(chunk.to_vec()),
            });
            crate::loading::fetch(cx, "friends", presence, |this, report, _| {
                for (name, presence) in report.users {
                    this.status.insert(key(&name), presence);
                }
            });
        }
    }

    /// Follow the player typed and download their games.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let input = self.input.clone();
        let page = cx.entity().downgrade();
        crate::accounts::handle(cx).update(cx, |accounts, cx| {
            accounts.follow(name, window, cx, move |added, window, cx| {
                if added {
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                    let _ = page.update(cx, |page, cx| {
                        page.load_profiles(cx);
                        page.refresh_status(cx);
                    });
                }
            })
        });
    }

    fn ask_remove(name: String, games: f64, stored: f64, window: &mut Window, cx: &mut App) {
        let account = crate::accounts::handle(cx).read(cx).active(cx);
        crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
            let name = name.clone();
            dialog
                .title("Stop following this player?")
                .description(format!(
                    "@{name} and their {} stored games ({}) will be deleted from this device. They stay removed until you add them again.{}",
                    widgets::format_count(games),
                    widgets::format_bytes(stored),
                    account
                        .as_ref()
                        .map_or(String::new(), |account| format!(
                            " @{account} stops following them on Lichess."
                        ))
                ))
                .ok_text("Remove")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let name = name.clone();
                    crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.remove(name, window, cx));
                    true
                })
        });
    }

    fn ask_clear(name: String, games: f64, stored: f64, window: &mut Window, cx: &mut App) {
        crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
            let name = name.clone();
            dialog
                .title("Clear this player's synced data?")
                .description(format!(
                    "Deletes @{name}'s {} stored games and cached profile ({}) from this device. They stay in Following, and syncing again downloads everything afresh.",
                    widgets::format_count(games),
                    widgets::format_bytes(stored)
                ))
                .ok_text("Clear data")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let name = name.clone();
                    crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.clear_data(name, window, cx));
                    true
                })
        });
    }

    /// What the header's buttons can do now.
    pub(crate) fn actions(&self, cx: &App) -> Vec<Action> {
        let accounts = crate::accounts::handle(cx).read(cx);
        let mut actions = Vec::new();
        if !self.input.read(cx).value().trim().is_empty() && !accounts.busy {
            actions.push(Action::Follow);
        }
        if !accounts.connected().is_empty() {
            actions.push(Action::Import);
        }
        if !accounts.tracked().is_empty() && !accounts.syncing() {
            actions.push(Action::Sync);
        }
        actions
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        match action {
            Action::Follow => self.follow(window, cx),
            Action::Import => self.open_import(window, cx),
            Action::Sync => sync_all(window, cx),
        }
    }

    fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| ImportDialog::new(window, cx));
        self.importer = Some(dialog.clone());
        crate::widgets::open_dialog(window, cx, move |modal, _, _| {
            modal
                .title("Import followed players from Lichess")
                .w(px(560.))
                .child(dialog.clone())
        });
    }

    /// One followed player's row.
    fn row(
        &self,
        friend: &Account,
        accounts: &Accounts,
        index: usize,
        palette: &Palette,
    ) -> AnyElement {
        let name = friend.username.clone();
        let profile = self
            .profiles
            .get(&key(&name))
            .cloned()
            .unwrap_or(Value::Null);
        let presence = self.status.get(&key(&name)).cloned();
        let (games, stored) = accounts.stored(&name);
        let (label, dot) = match &presence {
            None => ("Status unknown", palette.text_muted),
            Some(presence) if presence.playing => ("Playing now", palette.success),
            Some(presence) if presence.online => ("Online", palette.success),
            Some(_) => ("Offline", palette.border_accented),
        };
        let seen = presence
            .as_ref()
            .filter(|presence| !presence.online)
            .and(profile["seenAt"].as_f64())
            .map(|seen| format!(" · seen {}", ago(seen)));
        let rating = |perf: &str| {
            profile["perfs"][perf]["rating"]
                .as_f64()
                .map_or_else(|| "—".to_string(), |rating| rating.to_string())
        };
        let versus = self
            .versus
            .get(&key(&name))
            .map(|versus| format!("{}W {}L {}D", versus.win, versus.loss, versus.draw));
        let playing_id = presence
            .as_ref()
            .and_then(|presence| presence.playing_id.clone());
        let synced = friend.last_synced_at.map(|at| at as f64);
        // Lichess has games the last sync did not download.
        let mut new_badge = has_new_games(friend, &profile).then(|| {
            div()
                .id(("friend-new-games", index))
                .test_support()
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("New games to sync")
                        .build(window, cx)
                })
                .child(widgets::badge("New", Look::Primary, palette))
        });
        let cell = |width: f32| div().w(px(width)).text_right();
        let syncing = accounts.syncing();
        let (sync_name, clear_name, remove_name, games_name, challenge_name) = (
            name.clone(),
            name.clone(),
            name.clone(),
            name.clone(),
            name.clone(),
        );
        h_flex()
            .id(("friend", index))
            .gap(px(12.))
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(palette.border)
            .text_size(px(13.))
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(180.))
                    .gap(px(8.))
                    .child(widgets::initials(&name, 30., palette))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .id(("friend-name", index))
                                            .role(Role::Link)
                                            .keyboard_focus(palette.primary)
                                            .aria_label(format!("@{name}"))
                                            .font_weight(FontWeight::MEDIUM)
                                            .truncate()
                                            .cursor_pointer()
                                            .pressable_link()
                                            .on_click({
                                                let name = name.clone();
                                                move |_, window, cx| {
                                                    crate::pages::players::open_player(
                                                        &name, window, cx,
                                                    )
                                                }
                                            })
                                            .child(format!("@{name}")),
                                    )
                                    .children(profile["title"].as_str().map(|title| {
                                        widgets::badge(title.to_string(), Look::Primary, palette)
                                    })),
                            )
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .child(div().size(px(7.)).rounded_full().bg(dot))
                                    .child(label)
                                    .children(seen),
                            ),
                    ),
            )
            .child(
                cell(60.)
                    .text_color(palette.text_muted)
                    .child(rating("bullet")),
            )
            .child(cell(60.).child(rating("blitz")))
            .child(cell(60.).child(rating("rapid")))
            .child(
                cell(70.)
                    .text_color(palette.text_muted)
                    .child(rating("classical")),
            )
            .child(cell(100.).child(versus.unwrap_or_else(|| "—".into())))
            .child(
                v_flex()
                    .w(px(130.))
                    .when(games > 0., |column| {
                        column
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .child(widgets::format_counted(games, "game", "games"))
                                    .children(new_badge.take()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .child(match synced {
                                        Some(at) => format!(
                                            "{} · {}",
                                            widgets::format_bytes(stored),
                                            ago(at)
                                        ),
                                        None => widgets::format_bytes(stored),
                                    }),
                            )
                    })
                    .when(games <= 0., |column| match synced {
                        // Synced, with nothing to keep: when, as for a player with games.
                        Some(at) => column
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .child("No games")
                                    .children(new_badge.take()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .child(ago(at)),
                            ),
                        None => column.child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child("Not synced"),
                        ),
                    }),
            )
            .child(
                h_flex()
                    .w(px(190.))
                    .justify_end()
                    .gap(px(4.))
                    .children(playing_id.map(|id| {
                        Button::new(("friend-watch", index))
                            .xsmall()
                            .outline()
                            .icon(icon("tv"))
                            .label("Watch")
                            .on_click(move |_, window, cx| {
                                crate::pages::watch::watch_game(&id, window, cx)
                            })
                    }))
                    .child(
                        Button::new(("friend-challenge", index))
                            .xsmall()
                            .ghost()
                            .icon(icon("swords"))
                            .label("Challenge")
                            .on_click(move |_, window, cx| {
                                crate::pages::online::challenge(&challenge_name, window, cx)
                            }),
                    )
                    .child(
                        Button::new(("friend-games", index))
                            .xsmall()
                            .ghost()
                            .icon(icon("history"))
                            .label("Games")
                            .disabled(games <= 0.)
                            .on_click(move |_, window, cx| {
                                crate::pages::history::open_account(&games_name, window, cx)
                            }),
                    )
                    .child(
                        Button::new(("friend-more", index))
                            .xsmall()
                            .ghost()
                            .icon(icon("ellipsis"))
                            .accessibility_label("More")
                            .dropdown_menu(move |menu, _, _| {
                                let (sync_name, clear_name, remove_name) =
                                    (sync_name.clone(), clear_name.clone(), remove_name.clone());
                                menu.item(
                                    PopupMenuItem::new("Sync")
                                        .icon(icon("refresh-cw"))
                                        .disabled(syncing)
                                        .on_click(move |_, window, cx| {
                                            let name = sync_name.clone();
                                            crate::accounts::handle(cx).update(
                                                cx,
                                                |accounts, cx| {
                                                    accounts.sync(Some(name), window, cx)
                                                },
                                            )
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("Clear data")
                                        .icon(icon("eraser"))
                                        .disabled(games <= 0. && stored <= 0.)
                                        .on_click(move |_, window, cx| {
                                            Self::ask_clear(
                                                clear_name.clone(),
                                                games,
                                                stored,
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new("Remove")
                                        .icon(icon("user-minus"))
                                        .on_click(move |_, window, cx| {
                                            Self::ask_remove(
                                                remove_name.clone(),
                                                games,
                                                stored,
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for FriendsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let handle = cx.global::<AccountsHandle>().0.clone();
        let accounts = handle.read(cx);
        let mut friends = accounts.tracked();
        let rank = |friend: &Account, status: &HashMap<String, UserPresence>| {
            let presence = status.get(&key(&friend.username));
            match presence {
                Some(presence) if presence.playing => 0,
                Some(presence) if presence.online => 1,
                _ => 2,
            }
        };
        // Who can be watched or challenged right now comes first.
        friends.sort_by_key(|friend| rank(friend, &self.status));
        let online_count = friends
            .iter()
            .filter(|friend| rank(friend, &self.status) < 2)
            .count();
        let (stored, downloaded) = friends
            .iter()
            .fold((0., 0.), |(stored, downloaded), friend| {
                let name = friend.username.as_str();
                (
                    stored + accounts.stored(name).1,
                    downloaded + accounts.downloaded(name),
                )
            });
        let typed = !self.input.read(cx).value().trim().is_empty();
        let (busy, syncing) = (accounts.busy, accounts.syncing());
        let new_games = friends
            .iter()
            .filter(|friend| {
                self.profiles
                    .get(&key(&friend.username))
                    .is_some_and(|profile| has_new_games(friend, profile))
            })
            .count();
        let has_connected = !accounts.connected().is_empty();
        let header = h_flex()
            .flex_wrap()
            .gap(px(8.))
            // 10rem to 22rem, with its user-plus icon.
            .child(
                div().flex_1().min_w(px(160.)).max_w(px(352.)).child(
                    Input::new(&self.input)
                        .prefix(icon("user-plus").size(px(16.)))
                        .aria_label("Lichess username to follow"),
                ),
            )
            .child(
                Button::new("friends-follow")
                    .primary()
                    .icon(icon("plus"))
                    .label("Follow player")
                    .tooltip(match accounts.active(cx) {
                        Some(account) if has_connected => {
                            format!("Follows on Lichess as @{account} and downloads public games")
                        }
                        _ => "Follows in KChess and downloads public games".to_string(),
                    })
                    .disabled(!typed || busy)
                    .on_click(cx.listener(|this, _, window, cx| this.follow(window, cx))),
            )
            .when(has_connected, |row| {
                row.child(
                    Button::new("friends-import")
                        .outline()
                        .icon(icon("download"))
                        .label("Import from Lichess")
                        .tooltip("Adds players without downloading their games")
                        .on_click(cx.listener(|this, _, window, cx| this.open_import(window, cx))),
                )
            })
            .child(div().flex_1())
            .when(!friends.is_empty(), |row| {
                row.child(
                    Button::new("friends-sync")
                        .outline()
                        .icon(icon("refresh-cw"))
                        .label(match new_games {
                            0 => "Sync all".to_string(),
                            count => format!("Sync all · {count} new"),
                        })
                        .when(new_games > 0, |button| {
                            button.tooltip(match new_games {
                                1 => "1 player has new games".to_string(),
                                count => format!("{count} players have new games"),
                            })
                        })
                        .loading(syncing)
                        .disabled(syncing)
                        .on_click(|_, window, cx| sync_all(window, cx)),
                )
            });
        let body =
            if friends.is_empty() {
                widgets::card(&palette)
                    .child(widgets::empty_state(
                        icon("users"),
                        "Follow your first player",
                        None,
                        &palette,
                    ))
                    .into_any_element()
            } else {
                let head_cell =
                    |text: &'static str, width: f32| div().w(px(width)).text_right().child(text);
                v_flex()
                .gap(px(8.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .text_color(palette.text_muted)
                        .child(format!(
                            "{} {} · {online_count} online · {} on this computer · {} downloaded",
                            widgets::format_count(friends.len() as f64),
                            if friends.len() == 1 { "player" } else { "players" },
                            widgets::format_bytes(stored),
                            widgets::format_bytes(downloaded)
                        ))
                        .when(self.loading, |row| row.child(crate::widgets::spinner(px(14.)))),
                )
                .child(
                    widgets::card(&palette)
                        .p(px(0.))
                        .child(
                            h_flex()
                                .gap(px(12.))
                                .px(px(12.))
                                .py(px(8.))
                                // 12px, medium, as written.
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_muted)
                                .border_b_1()
                                .border_color(palette.border)
                                .child(div().flex_1().min_w(px(180.)).child("Player"))
                                .child(head_cell("Bullet", 60.))
                                .child(head_cell("Blitz", 60.))
                                .child(head_cell("Rapid", 60.))
                                .child(head_cell("Classical", 70.))
                                .child(head_cell("You vs them", 100.))
                                .child(div().w(px(130.)).child("Synced"))
                                .child(div().w(px(190.))),
                        )
                        .children(
                            friends
                                .iter()
                                .enumerate()
                                .map(|(index, friend)| self.row(friend, accounts, index, &palette)),
                        ),
                )
                .into_any_element()
            };
        div()
            .id("friends-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap(px(20.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .child(header)
                    .child(body)
                    .children(self.timeline_card(window, cx, &palette)),
            )
    }
}

impl FriendsPage {
    fn timeline_card(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Option<Div> {
        if Self::friends(cx).is_empty() || !crate::network::online(cx) {
            return None;
        }
        let card = widgets::card(palette)
            .gap(px(8.))
            .child(widgets::heading("Timeline", palette));
        Some(match (&self.timeline_error, &self.timeline) {
            (Some(error), _) => {
                let page = cx.entity().downgrade();
                card.child(widgets::retry_line(
                    "friends-timeline-retry",
                    error.clone(),
                    palette.error,
                    move |_, _, cx| {
                        let _ = page.update(cx, |page, cx| page.load_timeline(cx));
                    },
                ))
            }
            (None, None) => card.child(widgets::loading("Loading…", palette)),
            (None, Some(entries)) if entries.is_empty() => {
                card.child(widgets::hint("Nothing new", palette))
            }
            (None, Some(entries)) => card
                .child(crate::activity::activity_list(
                    "friends-timeline",
                    &crate::activity::timeline_groups(
                        &entries[..entries.len().min(self.timeline_shown)],
                    ),
                    window.viewport_size().width < px(900.),
                    palette,
                ))
                .when(entries.len() > self.timeline_shown, |card| {
                    card.child(
                        h_flex().child(
                            widgets::text_button("friends-timeline-more")
                                .small()
                                .label("Show more")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.timeline_shown += TIMELINE_MORE;
                                    cx.notify();
                                })),
                        ),
                    )
                }),
        })
    }
}

/// The players the connected accounts follow on Lichess, to add.
pub struct ImportDialog {
    report: Option<FollowingReport>,
    loading: bool,
    error: Option<String>,
    selected: HashSet<String>,
    show_removed: bool,
    filter: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ImportDialog {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> ImportDialog {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by name"));
        let subscriptions = vec![cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        let mut dialog = ImportDialog {
            report: None,
            loading: false,
            error: None,
            selected: HashSet::new(),
            show_removed: false,
            filter,
            _subscriptions: subscriptions,
        };
        dialog.load(cx);
        dialog
    }

    /// Ask Lichess afresh each time.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        cx.notify();
        let following = cx.global::<AppCore>().request(calls::Following {});
        cx.spawn(async move |this, cx| {
            let result = following.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(report) => this.report = Some(report),
                    Err(cause) => this.error = Some(cause.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn candidates(&self, cx: &App) -> Vec<FollowedUser> {
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        self.report
            .iter()
            .flat_map(|report| &report.users)
            .filter(|user| !user.already_added)
            .filter(|user| self.show_removed || !user.dismissed)
            .filter(|user| needle.is_empty() || user.username.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }
}

impl Render for ImportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let users: Vec<FollowedUser> = self
            .report
            .as_ref()
            .map(|report| report.users.clone())
            .unwrap_or_default();
        let candidates = self.candidates(cx);
        let already = users.iter().filter(|user| user.already_added).count();
        let removed = users
            .iter()
            .filter(|user| user.dismissed && !user.already_added)
            .count();
        let all_selected = !candidates.is_empty()
            && candidates
                .iter()
                .all(|user| self.selected.contains(&user.username));
        let several = crate::accounts::handle(cx).read(cx).connected().len() > 1;
        let problems: Vec<FollowingProblem> = self
            .report
            .as_ref()
            .map(|report| report.problems.clone())
            .unwrap_or_default();
        let count = self.selected.len();
        let body: AnyElement =
            if self.loading {
                h_flex()
                    .gap(px(8.))
                    .text_color(palette.text_muted)
                    .child(crate::widgets::spinner(px(14.)))
                    .child("Asking Lichess…")
                    .into_any_element()
            } else if self.report.is_some() {
                v_flex()
                    .gap(px(8.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(div().flex_1().child(
                                Input::new(&self.filter).aria_label("Filter followed players"),
                            ))
                            .child(
                                Button::new("import-all")
                                    .outline()
                                    .label(if all_selected { "Clear" } else { "Select all" })
                                    .disabled(candidates.is_empty())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let names: Vec<String> = this
                                            .candidates(cx)
                                            .into_iter()
                                            .map(|user| user.username)
                                            .collect();
                                        if all_selected {
                                            this.selected.clear();
                                        } else {
                                            this.selected.extend(names);
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(if candidates.is_empty() {
                        widgets::empty_state(
                            icon("users"),
                            if users.is_empty() {
                                "No followed players found"
                            } else {
                                "Nobody new to add"
                            },
                            Some(
                                if users.is_empty() {
                                    "Follow players on lichess.org and they will show up here"
                                } else {
                                    "Everyone they follow is already in Following"
                                }
                                .into(),
                            ),
                            &palette,
                        )
                        .into_any_element()
                    } else {
                        v_flex()
                            .id("import-list")
                            .max_h(px(320.))
                            .overflow_y_scroll()
                            .children(candidates.iter().enumerate().map(|(index, user)| {
                                let name = user.username.clone();
                                let checked = self.selected.contains(&name);
                                let mut ratings = Vec::new();
                                if let Some(blitz) = user.ratings.blitz {
                                    ratings.push(format!("Blitz {blitz}"));
                                }
                                if let Some(rapid) = user.ratings.rapid {
                                    ratings.push(format!("Rapid {rapid}"));
                                }
                                let via: Vec<String> = user
                                    .followed_by
                                    .iter()
                                    .map(|name| format!("@{name}"))
                                    .collect();
                                let toggled = name.clone();
                                h_flex()
                                    .gap(px(8.))
                                    .py(px(6.))
                                    .child(
                                        Checkbox::new(("import-user", index))
                                            .checked(checked)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.selected.remove(&toggled) {
                                                    this.selected.insert(toggled.clone());
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(format!("@{name}")),
                                    )
                                    .children(user.title.clone().map(|title| {
                                        widgets::badge(title, Look::Primary, &palette)
                                    }))
                                    .when(user.dismissed, |row| {
                                        row.child(widgets::badge(
                                            "Removed before",
                                            Look::NeutralSoft,
                                            &palette,
                                        ))
                                    })
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette.text_muted)
                                            .child(ratings.join(" · ")),
                                    )
                                    .when(several, |row| {
                                        row.child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(palette.text_muted)
                                                .child(format!("via {}", via.join(", "))),
                                        )
                                    })
                            }))
                            .into_any_element()
                    })
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!("{already} already added"))
                            .when(removed > 0, |row| {
                                row.child(
                                    div()
                                        .id("import-removed")
                                        .role(Role::Button)
                                        .keyboard_focus(palette.primary)
                                        .aria_label(format!(
                                            "{} {removed} you removed earlier",
                                            if self.show_removed { "Hide" } else { "Show" }
                                        ))
                                        .underline()
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.show_removed = !this.show_removed;
                                            cx.notify();
                                        }))
                                        .child(format!(
                                            "{} {removed} you removed earlier",
                                            if self.show_removed { "Hide" } else { "Show" }
                                        )),
                                )
                            }),
                    )
                    .into_any_element()
            } else {
                div().into_any_element()
            };
        v_flex()
            .gap(px(12.))
            .child(widgets::hint(
                "Players your connected accounts follow on Lichess",
                &palette,
            ))
            .children(problems.iter().map(|problem| {
                h_flex()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(radii::MD)
                    .bg(crate::theme::tint(palette.warning, 12.))
                    .text_size(px(13.))
                    .child(icon("triangle-alert").size(px(14.)))
                    .child(format!(
                        "@{}: couldn't read who it follows. {}",
                        problem.account, problem.message
                    ))
            }))
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(format!("Couldn't reach Lichess: {error}"))
            }))
            .child(body)
            .child(
                h_flex()
                    .justify_between()
                    .pt(px(8.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!("{count} selected")),
                    )
                    .child(
                        Button::new("import-add")
                            .primary()
                            .icon(icon("user-plus"))
                            .label(format!(
                                "Add {} {}",
                                if count > 0 {
                                    count.to_string()
                                } else {
                                    String::new()
                                },
                                if count == 1 { "player" } else { "players" }
                            ))
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let names: Vec<String> = this.selected.iter().cloned().collect();
                                crate::accounts::handle(cx)
                                    .update(cx, |accounts, cx| accounts.import(names, window, cx));
                                window.close_dialog(cx);
                            })),
                    ),
            )
    }
}

impl crate::pages::PageAction for Action {
    fn info(&self) -> crate::pages::ActionInfo {
        use crate::pages::ActionInfo;
        match self {
            Action::Follow => {
                ActionInfo::new("following:follow", "Follow player", "plus", "friend add")
            }
            Action::Import => ActionInfo::new(
                "following:import",
                "Import from Lichess",
                "download",
                "followed players",
            ),
            Action::Sync => {
                ActionInfo::new("following:sync", "Sync all", "refresh-cw", "games download")
            }
        }
    }
}

impl crate::pages::PageView for FriendsPage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: crate::pages::Page::Friends,
        label: "Following",
        short: "Following",
        icon: "users",
        group: Some(crate::pages::NavGroup::Lichess),
        keywords: "following players",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        FriendsPage::new(window, cx)
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        FriendsPage::actions(self, cx)
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.act(action, window, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::has_new_games;
    use kchess_contracts::types::Account;
    use serde_json::json;

    #[test]
    fn a_longer_play_time_than_at_the_sync_means_new_games() {
        let friend = |synced_play_time| Account {
            username: "dana".into(),
            connected: false,
            last_synced_at: None,
            synced_play_time,
        };
        let synced = friend(Some(1000));
        assert!(has_new_games(
            &synced,
            &json!({ "playTime": { "total": 1300 } })
        ));
        assert!(!has_new_games(
            &synced,
            &json!({ "playTime": { "total": 1000 } })
        ));
        // Without a play time saved at a sync, or in the profile, nothing is claimed.
        assert!(!has_new_games(
            &friend(None),
            &json!({ "playTime": { "total": 1300 } })
        ));
        assert!(!has_new_games(&synced, &json!({})));
    }
}

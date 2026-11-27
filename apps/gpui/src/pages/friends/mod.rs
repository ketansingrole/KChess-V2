//! Following:
//! the players followed in KChess — their ratings, whether they are online or playing, how the
//! connected accounts have fared against them, and their synced games — with follow, import
//! from Lichess, sync, clear and remove.

use crate::loading::Loading;
use crate::pages::PageView;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{FriendList, PresenceUsers, Username};
use kchess_contracts::types::{
    Account, FollowedUser, FollowingProblem, FollowingReport, GameRecord, LichessUser,
    TimelineEntry, UserPresence,
};
use std::collections::BTreeMap;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::accounts::{Accounts, AccountsHandle};
use crate::assets::icon;
use crate::core::AppCore;
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, KeyboardFocus as _, Look};

mod import;
use import::ImportDialog;

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
pub enum Action {
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
    /// Lichess players whose names start with what is typed, listed under the field.
    names: crate::player_search::NameMatches,
    /// Profiles by lowercase name, and presence (`UserPresence`).
    profiles: HashMap<String, LichessUser>,
    /// When every profile was last asked for.
    profiles_asked: Option<std::time::Instant>,
    status: HashMap<String, UserPresence>,
    /// The list's order, by presence as it was when the page opened (or when presence first
    /// came): later presence shows in place. Resorted on every poll, a friend starting a game
    /// swapped rows with the one under the pointer, Challenge and all. None until presence is
    /// known.
    order: Option<Vec<String>>,
    loading: bool,
    /// `gameLibraryOverview().versus`: the connected accounts' record against each player.
    versus: BTreeMap<String, GameRecord>,
    /// How many players were followed at the last look.
    followed: usize,
    importer: Option<Entity<ImportDialog>>,
    /// What the players on the list did lately (`TimelineEntry`s), once read.
    timeline: Option<Vec<TimelineEntry>>,
    timeline_loading: Loading,
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

fn key(name: &str) -> String {
    name.to_lowercase()
}

/// Who can be watched (0) or challenged (1) right now, before everyone else (2).
fn rank(name: &str, status: &HashMap<String, UserPresence>) -> u8 {
    match status.get(&key(name)) {
        Some(presence) if presence.playing => 0,
        Some(presence) if presence.online => 1,
        _ => 2,
    }
}

/// "5 min ago", for last seen and last synced.
fn ago(millis: f64) -> String {
    crate::widgets::time_ago(millis, kchess_games::now_ms())
}

impl FriendsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> FriendsPage {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Lichess username"));
        let accounts = crate::accounts::handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(
                &input,
                window,
                |this, input, event: &InputEvent, window, cx| {
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    this.names.follow(event, focused);
                    match event {
                        // The row chosen in the list under the field, or else the name typed, is
                        // followed.
                        InputEvent::PressEnter { .. } => {
                            let typed = this.input.read(cx).value().to_string();
                            if let Some(name) = this.names.entered(&typed) {
                                this.pick(name, window, cx);
                            }
                        }
                        InputEvent::Change => this.search_names(cx),
                        _ => {}
                    }
                    cx.notify();
                },
            ),
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
        FriendsPage {
            input,
            names: Default::default(),
            profiles: HashMap::new(),
            profiles_asked: None,
            status: HashMap::new(),
            order: None,
            loading: false,
            versus: BTreeMap::new(),
            followed: 0,
            importer: None,
            timeline: None,
            timeline_loading: Loading::default(),
            timeline_shown: TIMELINE_FIRST,
            focus: cx.focus_handle(),
            _poll: poll,
            _subscriptions: subscriptions,
        }
    }

    fn friends(cx: &App) -> Vec<Account> {
        crate::accounts::handle(cx).read(cx).tracked()
    }

    /// The page was shown: usage, profiles and presence afresh.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        // Opening the page sorts it again, by the presence known now.
        self.order = None;
        if !self.status.is_empty() {
            self.sort(cx);
        }
        crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
        self.load_profiles(cx);
        self.refresh_status(cx);
        self.load_versus(cx);
        self.load_timeline(cx);
    }

    /// What the players on the list did lately, from their Lichess activity.
    fn load_timeline(&mut self, cx: &mut Context<Self>) {
        if Self::friends(cx).is_empty() || !crate::network::online(cx) {
            self.timeline_loading.stop();
            self.timeline = None;
            return;
        }
        let timeline = cx.global::<AppCore>().request(calls::Timeline {});
        self.timeline_loading.load(
            cx,
            "friends",
            |this: &mut Self| &mut this.timeline_loading,
            timeline,
            |this, answer, _| {
                if let Ok(entries) = answer {
                    this.timeline = Some(entries);
                }
            },
        );
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
                if let Some(saved) = core
                    .request(calls::CachedProfile {
                        username: Username(name.clone()),
                    })
                    .await
                    .warn_on_err()
                    && let Some(profile) = saved.profile
                {
                    let recent = saved
                        .profile_fetched_at
                        .is_some_and(|at| kchess_games::now_ms() - (at as f64) < PROFILE_FRESH_MS);
                    if recent && !everyone {
                        fresh.insert(key(name));
                    }
                    this.update(cx, |this, cx| {
                        this.profiles.entry(key(name)).or_insert(profile);
                        cx.notify();
                    })
                    .ok();
                }
            }
            let stale: Vec<String> = names
                .iter()
                .filter(|name| !fresh.contains(&key(name)))
                .cloned()
                .collect();
            for chunk in stale.chunks(PROFILES_CHUNK) {
                if let Some(users) = core
                    .request(calls::Profiles {
                        usernames: FriendList(chunk.to_vec()),
                    })
                    .await
                    .warn_on_err()
                {
                    this.update(cx, |this, cx| {
                        for user in users.into_iter().filter(|user| !user.username.is_empty()) {
                            this.profiles.insert(key(&user.username), user);
                        }
                        cx.notify();
                    })
                    .ok();
                }
            }
            this.update(cx, |this, cx| {
                this.loading = false;
                crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
                cx.notify();
            })
            .ok();
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
            crate::loading::fetch(cx, "friends", presence, |this, report, cx| {
                this.presence_arrived(report.users, cx);
            });
        }
    }

    /// Who is online and playing, as Lichess says: the list is sorted by it the first time.
    pub(crate) fn presence_arrived(
        &mut self,
        users: impl IntoIterator<Item = (String, UserPresence)>,
        cx: &mut Context<Self>,
    ) {
        for (name, presence) in users {
            self.status.insert(key(&name), presence);
        }
        if self.order.is_none() {
            self.sort(cx);
        }
    }

    /// Set the list's order: who can be watched or challenged now first.
    fn sort(&mut self, cx: &App) {
        let mut friends = Self::friends(cx);
        friends.sort_by_key(|friend| rank(&friend.username, &self.status));
        self.order = Some(friends.iter().map(|friend| key(&friend.username)).collect());
    }

    /// Lichess players whose names start with what is typed, once typing settles.
    fn search_names(&mut self, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value().to_string();
        self.names
            .search(&typed, None, |page: &mut Self| &mut page.names, cx);
    }

    /// Follow a player picked from the list under the field, their name in it until they are
    /// added.
    fn pick(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.names.clear();
        self.input
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.follow(window, cx);
        cx.notify();
    }

    /// Follow the player typed and download their games.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().trim().to_string();
        // One change to the accounts at a time: the field spins until the last one is done.
        if name.is_empty() || crate::accounts::handle(cx).read(cx).busy {
            return;
        }
        let input = self.input.clone();
        let page = cx.entity().downgrade();
        crate::accounts::handle(cx).update(cx, |accounts, cx| {
            accounts.follow(name, window, cx, move |added, window, cx| {
                if added {
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                    page.update(cx, |page, cx| {
                        page.load_profiles(cx);
                        page.refresh_status(cx);
                    })
                    .ok();
                }
            })
        });
    }

    fn ask_remove(name: String, games: f64, stored: f64, window: &mut Window, cx: &mut App) {
        let account = crate::accounts::handle(cx).read(cx).active(cx);
        let description = format!(
            "@{name} and their {} stored games ({}) will be deleted from this device. They stay removed until you add them again.{}",
            widgets::format_count(games),
            widgets::format_bytes(stored),
            account.as_ref().map_or(String::new(), |account| format!(
                " @{account} stops following them on Lichess."
            ))
        );
        crate::widgets::confirm_danger(
            window,
            cx,
            "Stop following this player?",
            description,
            "Remove",
            move |window, cx| {
                let name = name.clone();
                crate::accounts::handle(cx)
                    .update(cx, |accounts, cx| accounts.remove(name, window, cx));
            },
        );
    }

    fn ask_clear(name: String, games: f64, stored: f64, window: &mut Window, cx: &mut App) {
        let description = format!(
            "Deletes @{name}'s {} stored games and cached profile ({}) from this device. They stay in Following, and syncing again downloads everything afresh.",
            widgets::format_count(games),
            widgets::format_bytes(stored)
        );
        crate::widgets::confirm_danger(
            window,
            cx,
            "Clear this player's synced data?",
            description,
            "Clear data",
            move |window, cx| {
                let name = name.clone();
                crate::accounts::handle(cx)
                    .update(cx, |accounts, cx| accounts.clear_data(name, window, cx));
            },
        );
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
        let profile = self.profiles.get(&key(&name)).cloned().unwrap_or_default();
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
            .and(profile.seen_at)
            .map(|seen| format!(" · seen {}", ago(seen as f64)));
        let rating = |perf: &str| {
            profile
                .perfs
                .get(perf)
                .and_then(|perf| perf.rating)
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
        let mut new_badge = profile.has_games_since(friend.synced_play_time).then(|| {
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
                                            .test_support()
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
                                    .children(profile.title.clone().map(|title| {
                                        widgets::badge(title, Look::Primary, palette)
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
                            .small()
                            .outline()
                            .icon(icon("tv"))
                            .label("Watch")
                            .on_click(move |_, window, cx| {
                                crate::pages::watch::watch_game(&id, window, cx)
                            })
                    }))
                    .child(
                        Button::new(("friend-challenge", index))
                            .small()
                            .ghost()
                            .icon(icon("swords"))
                            .label("Challenge")
                            .on_click(move |_, window, cx| {
                                crate::pages::online::challenge(&challenge_name, window, cx)
                            }),
                    )
                    .child(
                        Button::new(("friend-games", index))
                            .small()
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
                            .small()
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
        // The field to follow a player by, and the Lichess players its name starts.
        let busy = crate::accounts::handle(cx).read(cx).busy;
        let typed_name = self.input.read(cx).value().to_string();
        let search = crate::player_search::name_search(
            crate::player_search::NameSearch {
                id: "friends-match",
                // As Players' search: the players a name starts, picked to follow.
                field: Input::new(&self.input)
                    .prefix(icon("search").size(px(16.)))
                    .cleanable(true)
                    .when(busy, |field| field.suffix(widgets::spinner(px(14.))))
                    .aria_label("Lichess username to follow")
                    .into_any_element(),
                typed: &typed_name,
                typed_hint: Some("Follow"),
                // The players followed are listed on the page already.
                known: vec![crate::player_search::NameGroup::new(
                    "Recent",
                    "friends-recent",
                    crate::player_search::recent(kchess_contracts::types::GameSite::Lichess, cx),
                )],
                shown: None,
            },
            &self.names,
            |page: &mut Self| &mut page.names,
            |page, name, window, cx| page.pick(name, window, cx),
            &palette,
            cx,
        );
        let handle = cx.global::<AccountsHandle>().0.clone();
        let accounts = handle.read(cx);
        let mut friends = accounts.tracked();
        // In the order set (`order`); someone followed since, after, by presence.
        friends.sort_by_key(|friend| {
            let name = key(&friend.username);
            let at = self
                .order
                .as_ref()
                .and_then(|order| order.iter().position(|kept| *kept == name));
            (
                at.unwrap_or(usize::MAX),
                rank(&friend.username, &self.status),
            )
        });
        let online_count = friends
            .iter()
            .filter(|friend| rank(&friend.username, &self.status) < 2)
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
        let syncing = accounts.syncing();
        let new_games = friends
            .iter()
            .filter(|friend| {
                self.profiles
                    .get(&key(&friend.username))
                    .is_some_and(|profile| profile.has_games_since(friend.synced_play_time))
            })
            .count();
        let has_connected = !accounts.connected().is_empty();
        // As Players' search: the field centred; the page's actions above the players, at the right.
        let mut actions = Vec::new();
        if has_connected {
            actions.push(
                Button::new("friends-import")
                    .outline()
                    .icon(icon("download"))
                    .label("Import from Lichess")
                    .tooltip("Adds players without downloading their games")
                    .on_click(cx.listener(|this, _, window, cx| this.open_import(window, cx)))
                    .into_any_element(),
            );
        }
        if !friends.is_empty() {
            actions.push(
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
                    .on_click(|_, window, cx| sync_all(window, cx))
                    .into_any_element(),
            );
        }
        let header = v_flex()
            .gap(px(12.))
            .child(crate::player_search::centred_search(search, None))
            .when(!actions.is_empty(), |header| {
                header.child(h_flex().justify_end().gap(px(8.)).children(actions))
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
        Some(match (self.timeline_loading.error(), &self.timeline) {
            (Some(error), _) => {
                let page = cx.entity().downgrade();
                card.child(widgets::retry_line(
                    "friends-timeline-retry",
                    error.clone(),
                    palette.error,
                    move |_, _, cx| {
                        page.update(cx, |page, cx| page.load_timeline(cx)).ok();
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

impl PageView for FriendsPage {
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

    /// Each visit, not only the first: the list is sorted again and its presence asked afresh.
    fn shown(this: &Entity<Self>, again: bool, _window: &mut Window, cx: &mut App) {
        if !again {
            this.update(cx, |friends, cx| friends.shown(cx));
        }
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
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

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.act(action, window, cx));
    }
}

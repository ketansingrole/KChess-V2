//! Following (`pages/friends.vue`, `stores/friends.ts`, `FriendRow.vue`, `FriendImportDialog.vue`):
//! the players followed in KChess — their ratings, whether they are online or playing, how the
//! connected accounts have fared against them, and their synced games — with follow, import
//! from Lichess, sync, clear and remove.

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
use serde_json::{Value, json};

use crate::accounts::{Accounts, AccountsHandle};
use crate::assets::icon;
use crate::core::AppCore;
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::theme::Palette;
use crate::widgets::{self, Look};

/// Presence is asked for this many players at a time, and again this often.
const PRESENCE_CHUNK: usize = 50;
const PRESENCE_EVERY: Duration = Duration::from_secs(20);
/// A saved profile this recent is not asked for again.
const PROFILE_FRESH_MS: f64 = 3_600_000.;

pub struct FriendsPage {
    input: Entity<InputState>,
    /// Profiles by lowercase name (`LichessUser`), and presence (`UserPresence`).
    profiles: HashMap<String, Value>,
    status: HashMap<String, Value>,
    loading: bool,
    /// `gameLibraryOverview().versus`: the connected accounts' record against each player.
    versus: Value,
    /// How many players were followed at the last look.
    followed: usize,
    importer: Option<Entity<ImportDialog>>,
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
                    }
                    this.load_versus(cx);
                    cx.notify();
                }),
            ];
        // Cheap (one small request for everyone), and stops with the page.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PRESENCE_EVERY).await;
                if this.update(cx, |this, cx| this.refresh_status(cx)).is_err() {
                    return;
                }
            }
        });
        let mut page = FriendsPage {
            input,
            profiles: HashMap::new(),
            status: HashMap::new(),
            loading: false,
            versus: Value::Null,
            followed: 0,
            importer: None,
            focus: cx.focus_handle(),
            _poll: poll,
            _subscriptions: subscriptions,
        };
        page.shown(cx);
        page
    }

    fn friends(cx: &App) -> Vec<Value> {
        crate::accounts::handle(cx).read(cx).tracked()
    }

    /// The page was shown: usage, profiles and presence afresh.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
        self.load_profiles(cx);
        self.refresh_status(cx);
        self.load_versus(cx);
    }

    fn load_versus(&mut self, cx: &mut Context<Self>) {
        let overview = cx
            .global::<AppCore>()
            .call("gameLibraryOverview", Vec::new());
        cx.spawn(async move |this, cx| {
            if let Ok(overview) = overview.await {
                let _ = this.update(cx, |this, cx| {
                    this.versus = overview["versus"].clone();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// `loadProfiles`: saved profiles at once, fresh ones one at a time (Lichess asks for no
    /// parallel requests).
    fn load_profiles(&mut self, cx: &mut Context<Self>) {
        let names: Vec<String> = Self::friends(cx)
            .iter()
            .filter_map(|friend| friend["username"].as_str().map(str::to_string))
            .collect();
        if names.is_empty() {
            return;
        }
        self.loading = true;
        cx.notify();
        let core = cx.global::<AppCore>().clone();
        cx.spawn(async move |this, cx| {
            let mut fresh = HashSet::new();
            for name in &names {
                if let Ok(saved) = core.call("cachedProfile", vec![json!(name)]).await
                    && saved["profile"].is_object()
                {
                    let recent = saved["profileFetchedAt"]
                        .as_f64()
                        .is_some_and(|at| crate::studies::now() - at < PROFILE_FRESH_MS);
                    if recent {
                        fresh.insert(key(name));
                    }
                    let _ = this.update(cx, |this, cx| {
                        this.profiles
                            .entry(key(name))
                            .or_insert(saved["profile"].clone());
                        cx.notify();
                    });
                }
            }
            for name in &names {
                if fresh.contains(&key(name)) {
                    continue;
                }
                if let Ok(profile) = core.call("profile", vec![json!(name)]).await {
                    let _ = this.update(cx, |this, cx| {
                        this.profiles.insert(key(name), profile);
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

    /// `refreshStatus`: who is online or playing, fifty names a request.
    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let names: Vec<String> = Self::friends(cx)
            .iter()
            .filter_map(|friend| friend["username"].as_str().map(str::to_string))
            .collect();
        let core = cx.global::<AppCore>().clone();
        for chunk in names.chunks(PRESENCE_CHUNK) {
            let presence = core.call("presence", vec![json!(chunk)]);
            cx.spawn(async move |this, cx| {
                if let Ok(report) = presence.await
                    && let Some(users) = report["users"].as_object()
                {
                    let users = users.clone();
                    let _ = this.update(cx, |this, cx| {
                        for (name, presence) in users {
                            this.status.insert(key(&name), presence);
                        }
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    /// `addAccount`: follow the player typed and download their games.
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
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
            let name = name.clone();
            dialog
                .title("Stop following this player?")
                .description(format!(
                    "@{name} and their {} stored games ({}) will be deleted from this device. They stay removed until you add them again. Nothing changes on Lichess.",
                    widgets::format_count(games),
                    widgets::format_bytes(stored)
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
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
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

    fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| ImportDialog::new(window, cx));
        self.importer = Some(dialog.clone());
        window.open_dialog(cx, move |modal, _, _| {
            modal
                .title("Import followed players from Lichess")
                .w(px(560.))
                .child(dialog.clone())
        });
    }

    /// `FriendRow.vue`.
    fn row(
        &self,
        friend: &Value,
        accounts: &Accounts,
        index: usize,
        palette: &Palette,
    ) -> AnyElement {
        let name = friend["username"].as_str().unwrap_or_default().to_string();
        let profile = self
            .profiles
            .get(&key(&name))
            .cloned()
            .unwrap_or(Value::Null);
        let presence = self.status.get(&key(&name)).cloned();
        let (games, stored) = accounts.stored(&name);
        let (label, dot) = match &presence {
            None => ("Status unknown", palette.text_muted),
            Some(presence) if presence["playing"] == json!(true) => {
                ("Playing now", palette.success)
            }
            Some(presence) if presence["online"] == json!(true) => ("Online", palette.success),
            Some(_) => ("Offline", palette.border_accented),
        };
        let seen = presence
            .as_ref()
            .filter(|presence| presence["online"] != json!(true))
            .and(profile["seenAt"].as_f64())
            .map(|seen| format!(" · seen {}", ago(seen)));
        let rating = |perf: &str| {
            profile["perfs"][perf]["rating"]
                .as_f64()
                .map_or_else(|| "—".to_string(), |rating| rating.to_string())
        };
        let versus = &self.versus[key(&name)];
        let versus = versus["win"].as_f64().map(|win| {
            format!(
                "{win}W {}L {}D",
                versus["loss"].as_f64().unwrap_or_default(),
                versus["draw"].as_f64().unwrap_or_default()
            )
        });
        let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
        let playing_id = presence
            .as_ref()
            .and_then(|presence| presence["playingId"].as_str().map(str::to_string));
        let synced = friend["lastSyncedAt"].as_f64();
        let cell = |width: f32| div().w(px(width)).text_right();
        let busy = accounts.busy;
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
            .py(px(10.))
            .border_t_1()
            .border_color(palette.border)
            .text_size(px(13.))
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(180.))
                    .gap(px(10.))
                    .child(
                        div()
                            .size(px(30.))
                            .flex_none()
                            .rounded_full()
                            .bg(palette.accented)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(initials),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .id(("friend-name", index))
                                            .font_weight(FontWeight::BOLD)
                                            .truncate()
                                            .cursor_pointer()
                                            .hover(|name| name.underline())
                                            .on_click({
                                                let name = name.clone();
                                                move |_, window, cx| {
                                                    crate::shell::open_player(&name, window, cx)
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
                                    .gap(px(6.))
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
                            .child(format!("{} games", widgets::format_count(games)))
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
                    .when(games <= 0., |column| {
                        column.child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child("Not synced"),
                        )
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
                                crate::shell::watch_game(&id, window, cx)
                            })
                    }))
                    .child(
                        Button::new(("friend-challenge", index))
                            .xsmall()
                            .ghost()
                            .icon(icon("swords"))
                            .label("Challenge")
                            .on_click(move |_, window, cx| {
                                crate::shell::challenge(&challenge_name, window, cx)
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
                                crate::shell::open_history_account(&games_name, window, cx)
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
                                        .disabled(busy)
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
        let rank = |friend: &Value, status: &HashMap<String, Value>| {
            let presence = friend["username"]
                .as_str()
                .and_then(|name| status.get(&key(name)));
            match presence {
                Some(presence) if presence["playing"] == json!(true) => 0,
                Some(presence) if presence["online"] == json!(true) => 1,
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
                let name = friend["username"].as_str().unwrap_or_default();
                (
                    stored + accounts.stored(name).1,
                    downloaded + accounts.downloaded(name),
                )
            });
        let typed = !self.input.read(cx).value().trim().is_empty();
        let busy = accounts.busy;
        let has_connected = !accounts.connected().is_empty();
        let header = h_flex()
            .flex_wrap()
            .gap(px(8.))
            // `.friends-add-input`: 10rem to 22rem, with its user-plus icon.
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
                    .tooltip("Follows in KChess and downloads public games; does not change Lichess following.")
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
                        .label("Sync all")
                        .loading(busy)
                        .disabled(busy)
                        .on_click(|_, window, cx| {
                            crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.sync(None, window, cx))
                        }),
                )
            });
        let body =
            if friends.is_empty() {
                widgets::empty_state(icon("users"), "Follow your first player", None, &palette)
                    .into_any_element()
            } else {
                let head_cell =
                    |text: &'static str, width: f32| div().w(px(width)).text_right().child(text);
                v_flex()
                .gap(px(10.))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .text_size(px(13.))
                        .text_color(palette.text_muted)
                        .child(format!(
                            "{} {} · {online_count} online · {} on this computer · {} downloaded",
                            widgets::format_count(friends.len() as f64),
                            if friends.len() == 1 { "player" } else { "players" },
                            widgets::format_bytes(stored),
                            widgets::format_bytes(downloaded)
                        ))
                        .when(self.loading, |row| row.child(icon("loader-circle").size(px(14.)))),
                )
                .child(
                    widgets::card(&palette)
                        .p(px(0.))
                        .child(
                            h_flex()
                                .gap(px(12.))
                                .px(px(12.))
                                .py(px(8.))
                                // `.friends-table thead th`: 12px, medium, as written.
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
                    .child(body),
            )
    }
}

/// `FriendImportDialog.vue`: the players the connected accounts follow on Lichess, to add.
pub struct ImportDialog {
    report: Option<Value>,
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

    /// `loadFollowing`: ask Lichess afresh each time.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        cx.notify();
        let following = cx.global::<AppCore>().call("following", Vec::new());
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

    fn candidates(&self, cx: &App) -> Vec<Value> {
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        self.report
            .as_ref()
            .and_then(|report| report["users"].as_array())
            .into_iter()
            .flatten()
            .filter(|user| user["alreadyAdded"] != json!(true))
            .filter(|user| self.show_removed || user["dismissed"] != json!(true))
            .filter(|user| {
                needle.is_empty()
                    || user["username"]
                        .as_str()
                        .is_some_and(|name| name.to_lowercase().contains(&needle))
            })
            .cloned()
            .collect()
    }
}

impl Render for ImportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let users: Vec<Value> = self
            .report
            .as_ref()
            .and_then(|report| report["users"].as_array().cloned())
            .unwrap_or_default();
        let candidates = self.candidates(cx);
        let already = users
            .iter()
            .filter(|user| user["alreadyAdded"] == json!(true))
            .count();
        let removed = users
            .iter()
            .filter(|user| user["dismissed"] == json!(true) && user["alreadyAdded"] != json!(true))
            .count();
        let all_selected = !candidates.is_empty()
            && candidates.iter().all(|user| {
                user["username"]
                    .as_str()
                    .is_some_and(|name| self.selected.contains(name))
            });
        let several = crate::accounts::handle(cx).read(cx).connected().len() > 1;
        let problems: Vec<Value> = self
            .report
            .as_ref()
            .and_then(|report| report["problems"].as_array().cloned())
            .unwrap_or_default();
        let count = self.selected.len();
        let body: AnyElement = if self.loading {
            h_flex()
                .gap(px(8.))
                .text_color(palette.text_muted)
                .child(icon("loader-circle").size(px(14.)))
                .child("Asking Lichess…")
                .into_any_element()
        } else if self.report.is_some() {
            v_flex()
                .gap(px(10.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .child(
                            div().flex_1().child(
                                Input::new(&self.filter).aria_label("Filter followed players"),
                            ),
                        )
                        .child(
                            Button::new("import-all")
                                .outline()
                                .label(if all_selected { "Clear" } else { "Select all" })
                                .disabled(candidates.is_empty())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let names: Vec<String> = this
                                        .candidates(cx)
                                        .iter()
                                        .filter_map(|user| {
                                            user["username"].as_str().map(str::to_string)
                                        })
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
                                "Follow players on lichess.org and they will show up here."
                            } else {
                                "Everyone they follow is already in Following."
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
                            let name = user["username"].as_str().unwrap_or_default().to_string();
                            let checked = self.selected.contains(&name);
                            let mut ratings = Vec::new();
                            if let Some(blitz) = user["ratings"]["blitz"].as_f64() {
                                ratings.push(format!("Blitz {blitz}"));
                            }
                            if let Some(rapid) = user["ratings"]["rapid"].as_f64() {
                                ratings.push(format!("Rapid {rapid}"));
                            }
                            let via: Vec<String> = user["followedBy"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|name| name.as_str().map(|name| format!("@{name}")))
                                .collect();
                            let toggled = name.clone();
                            h_flex()
                                .gap(px(10.))
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
                                        .font_weight(FontWeight::BOLD)
                                        .child(format!("@{name}")),
                                )
                                .children(user["title"].as_str().map(|title| {
                                    widgets::badge(title.to_string(), Look::Primary, &palette)
                                }))
                                .when(user["dismissed"] == json!(true), |row| {
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
                        .child(format!("{already} already added."))
                        .when(removed > 0, |row| {
                            row.child(
                                div()
                                    .id("import-removed")
                                    .underline()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_removed = !this.show_removed;
                                        cx.notify();
                                    }))
                                    .child(format!(
                                        "{} {removed} you removed earlier.",
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
                "Players your connected accounts follow on Lichess.",
                &palette,
            ))
            .children(problems.iter().map(|problem| {
                h_flex()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(crate::theme::tint(palette.warning, 12.))
                    .text_size(px(13.))
                    .child(icon("triangle-alert").size(px(14.)))
                    .child(format!(
                        "@{}: couldn't read who it follows. {}",
                        problem["account"].as_str().unwrap_or_default(),
                        problem["message"].as_str().unwrap_or_default()
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

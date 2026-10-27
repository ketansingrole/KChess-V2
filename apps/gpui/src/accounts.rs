//! The Lichess accounts (`stores/kchess.ts`'s accounts, `stores/usage.ts`): who is connected,
//! which account plays online, connecting, syncing, logging out and removing, and what each
//! account downloaded and keeps. Pages read it; the sidebar's account menu and Settings → My
//! accounts act on it.

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, SharedString, Window};
use kchess_core::Level;
use serde_json::{Value, json};

use crate::core::{Answer, AppCore, Reads};

/// The account chosen for online play (`onlineAccountPreference`).
pub const ONLINE_ACCOUNT: &str = "kchess:online-account";

pub struct Accounts {
    /// `loadData`'s answer: the accounts and their sync state.
    pub data: Value,
    /// `usage`: bytes downloaded per account, and what each keeps (`UsageReport`).
    pub usage: Value,
    pub busy: bool,
    reads: Reads,
}

/// How views reach the accounts: `cx.global::<AccountsHandle>().0`.
#[derive(Clone)]
pub struct AccountsHandle(pub Entity<Accounts>);

impl Global for AccountsHandle {}

pub fn init(cx: &mut App) {
    let accounts = cx.new(|_| Accounts {
        data: Value::Null,
        usage: Value::Null,
        busy: false,
        reads: Reads::default(),
    });
    cx.set_global(AccountsHandle(accounts.clone()));
    accounts.update(cx, |accounts, cx| accounts.refresh(cx));
    // Another window signed in, synced or removed an account.
    crate::core::on_profile_change(cx, &["accounts", "usage"], {
        let accounts = accounts.downgrade();
        move |cx| {
            let _ = accounts.update(cx, |accounts, cx| accounts.refresh(cx));
        }
    });
}

/// The accounts, from anywhere.
pub fn handle(cx: &App) -> Entity<Accounts> {
    cx.global::<AccountsHandle>().0.clone()
}

/// Notifications from code that has no window at hand.
fn say(window: &mut Window, cx: &mut App, message: impl Into<SharedString>) {
    window.push_notification(message.into(), cx);
}

impl Accounts {
    /// Every account KChess keeps (connected or only tracked).
    pub fn all(&self) -> Vec<Value> {
        self.data["accounts"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// The usernames signed in through OAuth (`connectedAccounts`).
    pub fn connected(&self) -> Vec<String> {
        self.all()
            .iter()
            .filter(|account| account["connected"] == json!(true))
            .filter_map(|account| account["username"].as_str().map(str::to_string))
            .collect()
    }

    /// The players followed (`trackedAccounts`): accounts kept here that are not signed in.
    pub fn tracked(&self) -> Vec<Value> {
        self.all()
            .into_iter()
            .filter(|account| account["connected"] != json!(true))
            .collect()
    }

    /// The account that plays online (`activeOnlineAccount`).
    pub fn active(&self, cx: &App) -> Option<String> {
        crate::online::active_account(&self.all(), cx)
    }

    /// `usageOf(username).total.bytesIn`.
    pub fn downloaded(&self, username: &str) -> f64 {
        self.usage["accounts"][username.to_lowercase()]["total"]["bytesIn"]
            .as_f64()
            .unwrap_or_default()
    }

    /// `storageOf(username)`: games kept, and their bytes with the account's cache.
    pub fn stored(&self, username: &str) -> (f64, f64) {
        let storage = &self.usage["storage"][username.to_lowercase()];
        (
            storage["games"].as_f64().unwrap_or_default(),
            storage["bytes"].as_f64().unwrap_or_default()
                + storage["cacheBytes"].as_f64().unwrap_or_default(),
        )
    }

    /// Read the accounts and their usage again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let read = self.reads.start();
        let core = cx.global::<AppCore>().clone();
        let (data, usage) = (
            core.call("loadData", Vec::new()),
            core.call("usage", Vec::new()),
        );
        cx.spawn(async move |this, cx| {
            let (data, usage) = (data.await, usage.await);
            let _ = this.update(cx, |this, cx| {
                // Only the newest read shows: an older answer may predate a sign-in or sync.
                if this.reads.answer(read) != Answer::Show {
                    return;
                }
                if let Ok(data) = data {
                    this.data = data;
                }
                if let Ok(usage) = usage {
                    this.usage = usage;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_active(&mut self, username: &str, cx: &mut Context<Self>) {
        crate::prefs::set(cx, ONLINE_ACCOUNT, json!(username));
        cx.notify();
    }

    /// `connect`: sign in on Lichess in the browser; a new account plays online and is synced.
    pub fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let known: Vec<String> = self
            .connected()
            .iter()
            .map(|name| name.to_lowercase())
            .collect();
        let connected = cx.global::<AppCore>().call("connectLichess", Vec::new());
        cx.spawn_in(window, async move |this, cx| {
            let result = connected.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                cx.notify();
                let result = match result {
                    Ok(result) => result,
                    Err(cause) => return say(window, cx, cause.message),
                };
                if result["data"].is_object() {
                    this.data = result["data"].clone();
                }
                let Some(username) = result["username"].as_str().map(str::to_string) else {
                    return;
                };
                if known.contains(&username.to_lowercase()) {
                    // Lichess authorises whoever is signed in to the browser: the same account.
                    return say(
                        window,
                        cx,
                        format!(
                            "@{username} was already connected, so its login was refreshed. To add another account, sign in to it on lichess.org first."
                        ),
                    );
                }
                this.set_active(&username, cx);
                say(window, cx, format!("Connected @{username}."));
                this.sync(Some(username), window, cx);
            });
        })
        .detach();
    }

    /// `sync`: download one account's games (all when `None`), and say what that cost.
    pub fn sync(&mut self, username: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let core = cx.global::<AppCore>().clone();
        let before: f64 = self.total_downloaded();
        let args = username.map(|name| vec![json!(name)]).unwrap_or_default();
        let synced = core.call("syncGames", args);
        cx.spawn_in(window, async move |this, cx| {
            let result = synced.await;
            let usage = core.call("usage", Vec::new()).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if let Ok(usage) = usage {
                    this.usage = usage;
                }
                match result {
                    Ok(data) => {
                        this.data = data;
                        let downloaded = this.total_downloaded() - before;
                        let message = if downloaded > 0. {
                            format!(
                                "Lichess games synced · {} downloaded",
                                crate::widgets::format_bytes(downloaded)
                            )
                        } else {
                            "Lichess games synced.".into()
                        };
                        say(window, cx, message);
                    }
                    Err(cause) => say(window, cx, cause.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `addAccount`: follow a player and download their games; `done` hears whether it worked.
    pub fn follow(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(bool, &mut Window, &mut App) + 'static,
    ) {
        let typed = name.trim().trim_start_matches('@').to_lowercase();
        self.busy = true;
        cx.notify();
        let added = cx.global::<AppCore>().call("addAccount", vec![json!(name)]);
        cx.spawn_in(window, async move |this, cx| {
            let result = added.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(data) => {
                        this.data = data;
                        let username = this
                            .all()
                            .iter()
                            .filter_map(|account| account["username"].as_str())
                            .find(|username| username.to_lowercase() == typed)
                            .map(str::to_string);
                        // After this update: the caller reads the accounts again.
                        window.defer(cx, move |window, cx| done(true, window, cx));
                        this.sync(username, window, cx);
                    }
                    Err(cause) => {
                        say(window, cx, cause.message);
                        window.defer(cx, move |window, cx| done(false, window, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `clearSyncedData`: free a followed player's games and profile, still following them.
    pub fn clear_data(&mut self, username: String, window: &mut Window, cx: &mut Context<Self>) {
        let cleared = cx
            .global::<AppCore>()
            .call("clearAccountData", vec![json!(username)]);
        cx.spawn_in(window, async move |this, cx| {
            let result = cleared.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(data) => {
                        this.data = data;
                        this.refresh(cx);
                        say(
                            window,
                            cx,
                            format!("Cleared @{username}'s synced data. You still follow them; sync to download again."),
                        );
                    }
                    Err(cause) => say(window, cx, cause.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `importFriends`: follow players without downloading their games.
    pub fn import(&mut self, names: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let count = names.len();
        self.busy = true;
        cx.notify();
        let added = cx
            .global::<AppCore>()
            .call("addFriends", vec![json!(names)]);
        cx.spawn_in(window, async move |this, cx| {
            let result = added.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(data) => {
                        this.data = data;
                        this.refresh(cx);
                        say(
                            window,
                            cx,
                            format!(
                                "Added {count} {} to Following. Their games aren't downloaded until you sync them.",
                                if count == 1 { "player" } else { "players" }
                            ),
                        );
                    }
                    Err(cause) => say(window, cx, cause.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn total_downloaded(&self) -> f64 {
        self.usage["accounts"]
            .as_object()
            .map(|accounts| {
                accounts
                    .values()
                    .filter_map(|account| account["total"]["bytesIn"].as_f64())
                    .sum()
            })
            .unwrap_or_default()
    }

    /// `logout`: sign the online account out (every account when `all`).
    pub fn logout(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.active(cx);
        if !all && account.is_none() {
            return;
        }
        let core = cx.global::<AppCore>().clone();
        let call = if all {
            core.call("logoutAll", Vec::new())
        } else {
            core.call("logout", vec![json!(account)])
        };
        self.after_sign_out(call, None, window, cx);
    }

    /// `removeAccount`: disconnect one's own account (signing it out), or stop tracking another.
    pub fn remove(&mut self, username: String, window: &mut Window, cx: &mut Context<Self>) {
        let connected = self
            .connected()
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&username));
        let call = cx
            .global::<AppCore>()
            .call("removeAccount", vec![json!(username)]);
        let note = (!connected).then(|| {
            format!("@{username} removed. Their stored games were deleted from this device.")
        });
        self.after_sign_out(call, note, window, cx);
    }

    /// `signedOut`: take the answer in, forget the account's choices, and start the pages afresh
    /// so nothing kept for the account outlives it (Electron reloads its window).
    fn after_sign_out(
        &mut self,
        call: impl std::future::Future<Output = Result<Value, kchess_core::CoreError>> + 'static,
        note: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                let data = match result {
                    Ok(data) => data,
                    Err(cause) => {
                        say(window, cx, cause.message);
                        cx.notify();
                        return;
                    }
                };
                this.data = data;
                let first = this.connected().into_iter().next().unwrap_or_default();
                crate::prefs::set(cx, ONLINE_ACCOUNT, json!(first));
                if let Some(note) = note {
                    say(window, cx, note);
                    this.refresh(cx);
                    return;
                }
                crate::prefs::forget(cx, "kchess:puzzle-account");
                if first.is_empty() {
                    crate::prefs::forget(cx, "kchess:puzzle-mode");
                    crate::prefs::forget(cx, "kchess:puzzle-tab");
                }
                crate::log::write(
                    Level::Info,
                    "accounts",
                    "signed out; starting the pages afresh",
                );
                this.refresh(cx);
                // After this update: the new pages read and refresh the accounts themselves.
                window.defer(cx, crate::shell::restart_pages);
            });
        })
        .detach();
    }
}

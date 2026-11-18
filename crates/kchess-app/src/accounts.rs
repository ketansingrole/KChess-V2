//! The Lichess accounts: who is connected,
//! which account plays online, connecting, syncing, logging out and removing, and what each
//! account downloaded and keeps; and the Chess.com players whose games are synced. Pages read it; the sidebar's account menu and Settings → My
//! accounts act on it.

use futures::FutureExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, SharedString, Window};
use kchess_contracts::Level;
use kchess_contracts::calls;
use kchess_contracts::check::{ChesscomUsername, FriendList, Username};
use kchess_contracts::types::{
    Account, AppData, ChesscomAccount, ProfileArea, RelationAction, Reply, UsageReport,
};
use kchess_domain::records::accounts::{ListedAccount, connected_account_index};
use serde_json::json;

use crate::core::{Answer, AppCore, Reads};

/// The account chosen for online play.
pub const ONLINE_ACCOUNT: &str = "kchess:online-account";

pub struct Accounts {
    /// `loadData`'s answer: the accounts and their sync state; `None` until it is read.
    pub data: Option<AppData>,
    /// `usage`: bytes downloaded per account, and what each keeps (`UsageReport`).
    pub usage: UsageReport,
    /// `chesscomAccounts`: the Chess.com players synced, with when each last synced.
    pub chesscom: Vec<ChesscomAccount>,
    /// An account action runs: connecting, following, importing, signing out or removing.
    pub busy: bool,
    /// Game syncs running. They take minutes on a big account and run beside the actions: the
    /// core joins a second sync of the same account to the running one.
    syncs: usize,
    reads: Reads,
}

/// How views reach the accounts: `cx.global::<AccountsHandle>().0`.
#[derive(Clone)]
pub struct AccountsHandle(pub Entity<Accounts>);

impl Global for AccountsHandle {}

pub fn init(cx: &mut App) {
    let accounts = cx.new(|_| Accounts {
        data: None,
        usage: UsageReport::default(),
        chesscom: Vec::new(),
        busy: false,
        syncs: 0,
        reads: Reads::default(),
    });
    cx.set_global(AccountsHandle(accounts.clone()));
    accounts.update(cx, |accounts, cx| accounts.refresh(cx));
    // Another window signed in, synced or removed an account.
    crate::core::on_profile_change(cx, &[ProfileArea::Accounts, ProfileArea::Usage], {
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
    /// A game sync runs.
    pub fn syncing(&self) -> bool {
        self.syncs > 0
    }

    /// Anything runs: an account action or a sync.
    pub fn working(&self) -> bool {
        self.busy || self.syncing()
    }

    /// Every account KChess keeps (connected or only tracked).
    pub fn all(&self) -> &[Account] {
        self.data.as_ref().map_or(&[], |data| &data.accounts)
    }

    /// The games synced, of every account.
    pub fn game_count(&self) -> i64 {
        self.data.as_ref().map_or(0, |data| data.game_count)
    }

    /// The Chess.com players whose games are synced.
    pub fn chesscom(&self) -> &[ChesscomAccount] {
        &self.chesscom
    }

    /// The usernames signed in through OAuth.
    pub fn connected(&self) -> Vec<String> {
        self.all()
            .iter()
            .filter(|account| account.connected)
            .map(|account| account.username.clone())
            .collect()
    }

    /// The players followed: accounts kept here that are not signed in.
    pub fn tracked(&self) -> Vec<Account> {
        self.all()
            .iter()
            .filter(|account| !account.connected)
            .cloned()
            .collect()
    }

    /// The account that plays online.
    pub fn active(&self, cx: &App) -> Option<String> {
        active_account(self.all(), cx)
    }

    /// The bytes downloaded for `username`.
    pub fn downloaded(&self, username: &str) -> f64 {
        self.usage
            .accounts
            .get(&username.to_lowercase())
            .map_or(0.0, |account| account.total.bytes_in as f64)
    }

    /// Games kept, and their bytes with the account's cache.
    pub fn stored(&self, username: &str) -> (f64, f64) {
        let storage = self
            .usage
            .storage
            .get(&username.to_lowercase())
            .copied()
            .unwrap_or_default();
        (
            storage.games as f64,
            (storage.bytes + storage.cache_bytes) as f64,
        )
    }

    /// Show the accounts a change made here answered with: a read started before it is read again.
    fn adopt(&mut self, data: AppData) {
        self.data = Some(data);
        self.reads.edited();
    }

    /// Read the accounts and their usage again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let read = self.reads.start();
        let core = cx.global::<AppCore>().clone();
        let (data, usage, chesscom) = (
            core.request(calls::LoadData {}),
            core.request(calls::Usage {}),
            core.request(calls::ChesscomAccounts {}),
        );
        cx.spawn(async move |this, cx| {
            let (data, usage, chesscom) = (data.await, usage.await, chesscom.await);
            let _ = this.update(cx, |this, cx| {
                // Only the newest read shows: an older answer may predate a sign-in or sync, so one
                // that started before this window's own change reads again.
                match this.reads.answer(read) {
                    Answer::Show => {}
                    Answer::Again => return this.refresh(cx),
                    Answer::Superseded => return,
                }
                if let Ok(data) = data {
                    this.data = Some(data);
                }
                if let Ok(usage) = usage {
                    this.usage = usage;
                }
                if let Ok(chesscom) = chesscom {
                    this.chesscom = chesscom;
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
        let connected = cx.global::<AppCore>().request(calls::ConnectLichess {});
        cx.spawn_in(window, async move |this, cx| {
            let result = connected.await;
            let connected = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                cx.notify();
                let result = match result {
                    Ok(result) => result,
                    Err(cause) => {
                        say(window, cx, cause.message);
                        return false;
                    }
                };
                this.adopt(result.data);
                let username = result.username;
                if known.contains(&username.to_lowercase()) {
                    // Lichess authorises whoever is signed in to the browser: the same account.
                    say(
                        window,
                        cx,
                        format!(
                            "@{username} was already connected, so its login was refreshed. To add another account, sign in to it on lichess.org first."
                        ),
                    );
                    return true;
                }
                this.set_active(&username, cx);
                say(window, cx, format!("Connected @{username}"));
                this.sync(Some(username), window, cx);
                true
            });
            // Once the accounts hold the new one: the online state reads them.
            if matches!(connected, Ok(true)) {
                let _ = cx.update(|_, cx| hooks(cx).connected.map(|connected| connected(cx)));
            }
        })
        .detach();
    }

    /// `sync`: download one account's games (all when `None`), and say what that cost.
    /// Syncing everyone waits for no other sync but starts only when none runs; one account's sync
    /// always starts (a player just followed is not in a sync of everyone already running).
    pub fn sync(&mut self, username: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if username.is_none() && self.syncing() {
            return;
        }
        // Everyone's games: the Chess.com players' too.
        if username.is_none() && !self.chesscom().is_empty() {
            self.sync_chesscom(None, window, cx);
        }
        self.syncs += 1;
        cx.notify();
        let core = cx.global::<AppCore>().clone();
        let before: f64 = self.total_downloaded();
        let synced = core.request(calls::SyncGames {
            username: username.map(Username),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = synced.await;
            let usage = core.request(calls::Usage {}).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.syncs -= 1;
                if let Ok(usage) = usage {
                    this.usage = usage;
                }
                match result {
                    Ok(data) => {
                        this.adopt(data);
                        let downloaded = this.total_downloaded() - before;
                        let message = if downloaded > 0. {
                            format!(
                                "Lichess games synced · {} downloaded",
                                kchess_ui::widgets::format_bytes(downloaded)
                            )
                        } else {
                            "Lichess games synced".into()
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

    /// `addChesscomAccount`: sync a Chess.com player's games; `done` hears whether the player was
    /// added (they stay added when only their games could not be read yet).
    pub fn add_chesscom(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(bool, &mut Window, &mut App) + 'static,
    ) {
        self.syncs += 1;
        cx.notify();
        let core = cx.global::<AppCore>().clone();
        let added = core.request(calls::AddChesscomAccount {
            username: ChesscomUsername(name.trim().to_string()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = added.await;
            let players = core.request(calls::ChesscomAccounts {}).await;
            let usage = core.request(calls::Usage {}).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.syncs -= 1;
                if let Ok(usage) = usage {
                    this.usage = usage;
                }
                let before = this.chesscom().len();
                if let Ok(players) = players {
                    this.chesscom = players;
                }
                let added = this.chesscom().len() > before;
                if let Err(cause) = result {
                    say(window, cx, cause.message);
                }
                // The game count (`loadData`) changed with the games synced.
                this.refresh(cx);
                window.defer(cx, move |window, cx| done(added, window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    /// `syncChesscomGames`: download one Chess.com player's new games (every player's when `None`).
    pub fn sync_chesscom(
        &mut self,
        username: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syncs += 1;
        cx.notify();
        let core = cx.global::<AppCore>().clone();
        let synced = core.request(calls::SyncChesscomGames {
            username: username.map(ChesscomUsername),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = synced.await;
            let usage = core.request(calls::Usage {}).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.syncs -= 1;
                if let Ok(usage) = usage {
                    this.usage = usage;
                }
                match result {
                    Ok(players) => {
                        this.chesscom = players;
                        say(window, cx, "Chess.com games synced");
                    }
                    Err(cause) => say(window, cx, cause.message),
                }
                this.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// `removeChesscomAccount`: stop syncing a Chess.com player and delete their games here.
    pub fn remove_chesscom(
        &mut self,
        username: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let removed = cx
            .global::<AppCore>()
            .request(calls::RemoveChesscomAccount {
                username: ChesscomUsername(username.clone()),
            });
        cx.spawn_in(window, async move |this, cx| {
            let result = removed.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(players) => {
                        this.chesscom = players;
                        this.refresh(cx);
                        say(window, cx, format!("{username} removed. Their Chess.com games were deleted from this device."));
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
        let added = cx.global::<AppCore>().request(calls::AddAccount {
            username: Username(name),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = added.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(data) => {
                        this.adopt(data);
                        let username = this
                            .all()
                            .iter()
                            .map(|account| account.username.as_str())
                            .find(|username| username.to_lowercase() == typed)
                            .map(str::to_string);
                        // After this update: the caller reads the accounts again.
                        window.defer(cx, move |window, cx| done(true, window, cx));
                        // A connected account follows them on Lichess too.
                        if let Some(name) = username.clone().filter(|name| !this.is_connected(name))
                        {
                            this.set_relation(
                                name,
                                RelationAction::Follow,
                                window,
                                cx,
                                |_, _, _| {},
                            );
                        }
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

    fn is_connected(&self, username: &str) -> bool {
        self.connected()
            .iter()
            .any(|name| name.eq_ignore_ascii_case(username))
    }

    /// Follow, unfollow, block or unblock `username` on Lichess as the
    /// online account. Without one there is nothing to do; a login from before the permission is
    /// asked to reconnect. `done` hears whether Lichess took it.
    pub fn set_relation(
        &mut self,
        username: String,
        action: RelationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(bool, &mut Window, &mut App) + 'static,
    ) {
        let Some(account) = self.active(cx) else {
            return;
        };
        let call = cx.global::<AppCore>().request(calls::SetRelation {
            account: Username(account.clone()),
            username: Username(username),
            action,
        });
        let action = action.as_str();
        cx.spawn_in(window, async move |this, cx| {
            let answer = call.await;
            let _ = this.update_in(cx, |_, window, cx| {
                let took = match answer {
                    Ok(Reply::Done(took)) => took,
                    Ok(Reply::NeedsReconnect) => {
                        let note = gpui_kit::component::notification::Notification::warning(
                            format!("Connect @{account} again to {action} players on Lichess"),
                        )
                        .title("Lichess needs a new permission");
                        let reconnect = kchess_ui::widgets::action_notification(
                            note,
                            "relation-reconnect",
                            "Reconnect",
                            |window, cx| {
                                handle(cx).update(cx, |accounts, cx| accounts.connect(window, cx))
                            },
                        );
                        window.push_notification(reconnect, cx);
                        false
                    }
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "accounts",
                            &format!("Lichess {action} failed: {}", cause.message),
                        );
                        say(window, cx, cause.message);
                        false
                    }
                };
                window.defer(cx, move |window, cx| done(took, window, cx));
            });
        })
        .detach();
    }

    /// Free a followed player's games and profile, still following them.
    pub fn clear_data(&mut self, username: String, window: &mut Window, cx: &mut Context<Self>) {
        let cleared = cx.global::<AppCore>().request(calls::ClearAccountData {
            username: Username(username.clone()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = cleared.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(data) => {
                        this.adopt(data);
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

    /// Follow players without downloading their games.
    pub fn import(&mut self, names: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let count = names.len();
        self.busy = true;
        cx.notify();
        let added = cx.global::<AppCore>().request(calls::AddFriends {
            usernames: FriendList(names),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = added.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(data) => {
                        this.adopt(data);
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
        self.usage
            .accounts
            .iter()
            .map(|(_, account)| account.total.bytes_in as f64)
            .sum()
    }

    /// `logout`: sign the online account out (every account when `all`).
    pub fn logout(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.active(cx);
        if !all && account.is_none() {
            return;
        }
        let core = cx.global::<AppCore>().clone();
        // One future type for either call.
        let call = if all {
            core.request(calls::LogoutAll {}).boxed_local()
        } else {
            core.request(calls::Logout {
                username: Username(account.unwrap_or_default()),
            })
            .boxed_local()
        };
        self.after_sign_out(call, None, window, cx);
    }

    /// `removeAccount`: disconnect one's own account (signing it out), or stop tracking another.
    pub fn remove(&mut self, username: String, window: &mut Window, cx: &mut Context<Self>) {
        let connected = self
            .connected()
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&username));
        let call = cx.global::<AppCore>().request(calls::RemoveAccount {
            username: Username(username.clone()),
        });
        let note = (!connected).then(|| {
            format!("@{username} removed. Their stored games were deleted from this device.")
        });
        // A friend leaves Following on Lichess too.
        if !connected {
            self.set_relation(
                username.clone(),
                RelationAction::Unfollow,
                window,
                cx,
                |_, _, _| {},
            );
        }
        self.after_sign_out(call, note, window, cx);
    }

    /// Take the answer in, forget the account's choices, and start the pages afresh
    /// so nothing kept for the account outlives it.
    fn after_sign_out(
        &mut self,
        call: impl std::future::Future<Output = Result<AppData, kchess_contracts::CoreError>> + 'static,
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
                this.adopt(data);
                let first = this.connected().into_iter().next().unwrap_or_default();
                crate::prefs::set(cx, ONLINE_ACCOUNT, json!(first));
                if let Some(note) = note {
                    say(window, cx, note);
                    this.refresh(cx);
                    return;
                }
                if let Some(signed_out) = hooks(cx).signed_out {
                    signed_out(!first.is_empty(), cx);
                }
                crate::log::write(
                    Level::Info,
                    "accounts",
                    "signed out; starting the pages afresh",
                );
                this.refresh(cx);
                // After this update: the new pages read and refresh the accounts themselves.
                if let Some(restart) = hooks(cx).restart_pages {
                    window.defer(cx, restart);
                }
            });
        })
        .detach();
    }
}

/// The account Lichess actions use: the one chosen for online play when it
/// is connected, else the first connected one.
pub fn active_account(accounts: &[kchess_contracts::types::Account], cx: &App) -> Option<String> {
    let preferred = crate::prefs::get::<String>(cx, ONLINE_ACCOUNT, String::new());
    let listed: Vec<ListedAccount> = accounts
        .iter()
        .map(|account| ListedAccount {
            username: Some(account.username.as_str()),
            connected: account.connected,
        })
        .collect();
    let index = connected_account_index(&listed, Some(&preferred))?;
    listed[index].username.map(str::to_string)
}

/// Which connected account plays.
pub fn set_active_account(account: &str, cx: &mut App) {
    crate::prefs::set(cx, ONLINE_ACCOUNT, account.to_string());
}

/// What the app does when accounts change, which this crate cannot do itself: tell the online
/// state an account connected, let the pages forget a signed-out account (the puzzles' chosen
/// account), and start the pages afresh. Set by the app (`cx.set_global(AccountHooks { … })`);
/// until then nothing else happens.
#[derive(Clone, Copy, Default)]
pub struct AccountHooks {
    pub connected: Option<fn(&mut App)>,
    /// Whether any account is left.
    pub signed_out: Option<fn(bool, &mut App)>,
    pub restart_pages: Option<fn(&mut Window, &mut App)>,
}

impl Global for AccountHooks {}

fn hooks(cx: &App) -> AccountHooks {
    cx.try_global::<AccountHooks>().copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(username: &str, connected: bool) -> kchess_contracts::types::Account {
        kchess_contracts::types::Account {
            username: username.into(),
            connected,
            last_synced_at: None,
            synced_play_time: None,
        }
    }

    /// Lichess actions use the account chosen for online play while it is connected, else the
    /// first connected one; a player only followed never plays.
    #[gpui_kit::test]
    fn the_active_account_is_the_one_chosen_while_it_is_connected(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            let all = [
                account("first", true),
                account("second", true),
                account("followed", false),
            ];
            assert_eq!(super::active_account(&all, cx), Some("first".into()));
            super::set_active_account("second", cx);
            assert_eq!(super::active_account(&all, cx), Some("second".into()));
            super::set_active_account("followed", cx);
            assert_eq!(super::active_account(&all, cx), Some("first".into()));
            assert_eq!(super::active_account(&all[2..], cx), None);
        });
    }

    /// A window that shows the accounts, as the app's sidebar does: the window an account
    /// action's answer is shown in.
    struct Shown;

    impl gpui_kit::Render for Shown {
        fn render(
            &mut self,
            _: &mut Window,
            cx: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            let busy = handle(cx).read(cx).busy;
            use gpui_kit::ParentElement as _;
            gpui_kit::div().child(if busy { "Busy" } else { "Ready" })
        }
    }

    /// The accounts over a core of the test's own, holding `accounts`, in a window.
    fn accounts_holding(
        accounts: Vec<kchess_contracts::types::Account>,
        cx: &mut gpui_kit::TestAppContext,
    ) -> (crate::tests::TestCore, gpui_kit::AnyWindowHandle) {
        cx.executor().allow_parking();
        let core = cx.update(|cx| {
            gpui_kit::init(cx);
            let core = crate::tests::start_core(cx);
            super::init(cx);
            core
        });
        let data = cx.update(|cx| cx.global::<AppCore>().request(calls::LoadData {}));
        let mut data = futures::executor::block_on(data).expect("the core's data");
        data.accounts = accounts;
        cx.run_until_parked();
        cx.update(|cx| super::handle(cx).update(cx, |this, _| this.adopt(data)));
        let window = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|_| Shown))
                .expect("a window")
                .0
        });
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .expect("the window");
        (core, window)
    }

    /// Connecting and syncing show at once that they run, before the core answers; the account
    /// that plays is the one the accounts hold.
    #[gpui_kit::test]
    fn actions_show_at_once_that_they_run(cx: &mut gpui_kit::TestAppContext) {
        let (_core, window) = accounts_holding(vec![account("player", true)], cx);
        cx.update_window(window, |_, window, cx| {
            let accounts = super::handle(cx);
            assert_eq!(accounts.read(cx).active(cx), Some("player".into()));
            accounts.update(cx, |this, cx| this.sync(None, window, cx));
            assert!(accounts.read(cx).syncing());
            accounts.update(cx, |this, cx| this.connect(window, cx));
            assert!(accounts.read(cx).busy);
        })
        .unwrap();
    }

    /// Following a player tells the page whether Lichess took it: without a Lichess login kept,
    /// the core refuses, and the page hears so.
    #[gpui_kit::test]
    fn a_refused_relation_is_heard(cx: &mut gpui_kit::TestAppContext) {
        use std::sync::atomic::{AtomicU8, Ordering};
        static HEARD: AtomicU8 = AtomicU8::new(0);
        let (_core, window) = accounts_holding(vec![account("player", true)], cx);
        cx.update_window(window, |_, window, cx| {
            super::handle(cx).update(cx, |this, cx| {
                this.set_relation(
                    "friend".into(),
                    RelationAction::Follow,
                    window,
                    cx,
                    |took, _, _| HEARD.store(if took { 2 } else { 1 }, Ordering::SeqCst),
                );
            });
        })
        .unwrap();
        for _ in 0..500 {
            cx.run_until_parked();
            if HEARD.load(Ordering::SeqCst) != 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(HEARD.load(Ordering::SeqCst), 1, "refused");
    }

    /// After signing out the pages forget the account, told whether one is left, and start
    /// afresh.
    #[gpui_kit::test]
    fn signing_out_starts_the_pages_afresh(cx: &mut gpui_kit::TestAppContext) {
        use std::sync::atomic::{AtomicU8, Ordering};
        static LEFT: AtomicU8 = AtomicU8::new(0);
        static RESTARTED: AtomicU8 = AtomicU8::new(0);
        let (_core, window) = accounts_holding(Vec::new(), cx);
        cx.update(|cx| {
            cx.set_global(super::AccountHooks {
                connected: None,
                signed_out: Some(|left, _| LEFT.store(if left { 2 } else { 1 }, Ordering::SeqCst)),
                restart_pages: Some(|_, _| {
                    RESTARTED.fetch_add(1, Ordering::SeqCst);
                }),
            })
        });
        let data = cx.update(|cx| cx.global::<AppCore>().request(calls::LoadData {}));
        let mut data = futures::executor::block_on(data).expect("the core's data");
        data.accounts = vec![account("still", true)];
        // The answer arrives after the call, as the core's does.
        let (answer, answered) = futures::channel::oneshot::channel();
        cx.update_window(window, |_, window, cx| {
            super::handle(cx).update(cx, |this, cx| {
                let call = async move { answered.await.expect("the answer") };
                this.after_sign_out(call, None, window, cx);
            });
        })
        .unwrap();
        let _ = answer.send(Ok(data));
        for _ in 0..500 {
            cx.run_until_parked();
            if RESTARTED.load(Ordering::SeqCst) != 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(LEFT.load(Ordering::SeqCst), 2, "an account is left");
        assert_eq!(RESTARTED.load(Ordering::SeqCst), 1);
    }

    /// Until the app sets its hooks nothing else happens; then the one it set runs.
    #[gpui_kit::test]
    fn the_hooks_the_app_set_run(cx: &mut gpui_kit::TestAppContext) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static LEFT: AtomicUsize = AtomicUsize::new(0);
        fn signed_out(left: bool, _: &mut gpui_kit::App) {
            LEFT.fetch_add(if left { 10 } else { 1 }, Ordering::SeqCst);
        }
        cx.update(|cx| {
            let none = super::hooks(cx);
            assert!(none.connected.is_none() && none.signed_out.is_none());
            assert!(none.restart_pages.is_none());
            cx.set_global(super::AccountHooks {
                signed_out: Some(signed_out),
                ..Default::default()
            });
            if let Some(hook) = super::hooks(cx).signed_out {
                hook(true, cx);
            }
        });
        assert_eq!(LEFT.load(Ordering::SeqCst), 10);
    }
    use crate::core::{Answer, Reads};
    use kchess_contracts::types::{Account, AppData};

    /// A refresh started before a sign-in (or sync, or removal) here answers with the accounts
    /// as they were: it reads again rather than replacing the newer ones.
    #[test]
    fn a_read_from_before_a_local_change_is_read_again() {
        let settings = kchess_contracts::defaults::default_settings()
            .expect("the default settings")
            .core;
        let mut accounts = Accounts {
            data: None,
            usage: Default::default(),
            chesscom: Vec::new(),
            busy: false,
            syncs: 0,
            reads: Reads::default(),
        };
        let read = accounts.reads.start();
        accounts.adopt(AppData {
            settings,
            accounts: vec![Account {
                username: "new".into(),
                connected: true,
                last_synced_at: None,
                synced_play_time: None,
            }],
            game_count: 0,
        });
        assert_eq!(accounts.reads.answer(read), Answer::Again);
        assert_eq!(accounts.connected(), ["new"]);
    }
}

use std::collections::HashSet;

use gpui::Context;

use crate::{
    lichess::{self, SyncSummary},
    lichess_auth,
    storage::{self, LICHESS_AUTH_KIND_OAUTH, LichessAccount, LichessGame},
};

const HISTORY_GAMES_PER_ACCOUNT_LIMIT: usize = 200;

pub struct LichessState {
    accounts: Vec<LichessAccount>,
    games: Vec<LichessGame>,
    connecting: bool,
    syncing: bool,
    removing_accounts: HashSet<String>,
    status_message: Option<String>,
    error_message: Option<String>,
}

impl LichessState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            accounts: Vec::new(),
            games: Vec::new(),
            connecting: false,
            syncing: false,
            removing_accounts: HashSet::new(),
            status_message: None,
            error_message: None,
        };

        state.reload_from_storage();
        if !state.accounts.is_empty() {
            state.sync_all(cx);
        }

        state
    }

    pub fn accounts(&self) -> &[LichessAccount] {
        &self.accounts
    }

    pub fn tracked_accounts(&self) -> Vec<LichessAccount> {
        self.accounts
            .iter()
            .filter(|account| account.auth_kind != LICHESS_AUTH_KIND_OAUTH)
            .cloned()
            .collect()
    }

    pub fn own_accounts(&self) -> Vec<LichessAccount> {
        self.accounts
            .iter()
            .filter(|account| account.auth_kind == LICHESS_AUTH_KIND_OAUTH)
            .cloned()
            .collect()
    }

    pub fn games(&self) -> &[LichessGame] {
        &self.games
    }

    pub fn syncing(&self) -> bool {
        self.syncing
    }

    pub fn connecting(&self) -> bool {
        self.connecting
    }

    pub fn removing(&self, username: &str) -> bool {
        self.removing_accounts
            .contains(&username.to_ascii_lowercase())
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    pub fn add_account(
        &mut self,
        username: &str,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let normalized =
            storage::normalize_lichess_username(username).map_err(|err| err.to_string())?;
        storage::add_lichess_account(&normalized).map_err(|err| err.to_string())?;

        self.reload_from_storage();
        self.error_message = None;
        self.status_message = Some(format!("Added @{normalized}. Syncing games now."));
        self.sync_all(cx);

        Ok(normalized)
    }

    pub fn remove_account(&mut self, username: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let normalized =
            storage::normalize_lichess_username(username).map_err(|err| err.to_string())?;
        let Some(account) = self
            .accounts
            .iter()
            .find(|account| account.username.eq_ignore_ascii_case(&normalized))
            .cloned()
        else {
            return Ok(());
        };

        if !self
            .removing_accounts
            .insert(normalized.to_ascii_lowercase())
        {
            return Ok(());
        }

        self.error_message = None;
        self.status_message = Some(format!("Removing @{}…", account.username));
        cx.notify();

        cx.spawn(async move |this, cx| {
            let username = account.username.clone();
            let result = async {
                if account.auth_kind == LICHESS_AUTH_KIND_OAUTH {
                    let delete_task = cx
                        .update(|app| {
                            app.delete_credentials(&lichess_auth::credential_url(&account.username))
                        })
                        .map_err(|err| err.to_string())?;
                    delete_task.await.map_err(|err| err.to_string())?;
                }

                storage::remove_lichess_account(&account.username)
                    .map_err(|err| err.to_string())?;
                Ok(username.clone())
            }
            .await;

            let _ = this.update(cx, |state, cx| {
                state.finish_remove_account(&username, result, cx);
            });
        })
        .detach();

        Ok(())
    }

    pub fn start_connect_flow(&mut self, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }

        let (auth_url, pending_login) = match lichess_auth::begin_oauth_login() {
            Ok(flow) => flow,
            Err(err) => {
                self.error_message = Some(err.to_string());
                self.status_message = None;
                self.connecting = false;
                cx.notify();
                return;
            }
        };

        self.connecting = true;
        self.error_message = None;
        self.status_message = Some("Opening Lichess login in your browser…".to_string());
        cx.notify();

        cx.open_url(&auth_url);

        cx.spawn(async move |this, cx| {
            let result = async {
                let connected = cx
                    .background_executor()
                    .spawn(async move { lichess_auth::complete_oauth_login(pending_login) })
                    .await
                    .map_err(|err| err.to_string())?;
                let credential_task = cx
                    .update(|app| {
                        app.write_credentials(
                            &lichess_auth::credential_url(&connected.username),
                            &connected.username,
                            connected.access_token.as_bytes(),
                        )
                    })
                    .map_err(|err| err.to_string())?;
                credential_task.await.map_err(|err| err.to_string())?;

                storage::upsert_connected_lichess_account(&connected.username)
                    .map_err(|err| err.to_string())?;

                Ok(connected.username)
            }
            .await;

            let _ = this.update(cx, |state, cx| {
                state.finish_connect_flow(result, cx);
            });
        })
        .detach();
    }

    pub fn sync_all(&mut self, cx: &mut Context<Self>) {
        if self.syncing || self.accounts.is_empty() {
            return;
        }

        self.syncing = true;
        self.error_message = None;
        self.status_message = Some(format!("Syncing {} account(s)…", self.accounts.len()));
        cx.notify();

        let accounts = self.accounts.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { lichess::sync_accounts(&accounts) })
                .await;

            let _ = this.update(cx, |state, cx| {
                state.finish_sync(result, cx);
            });
        })
        .detach();
    }

    fn finish_sync(
        &mut self,
        result: Result<SyncSummary, lichess::SyncError>,
        cx: &mut Context<Self>,
    ) {
        self.syncing = false;

        match result {
            Ok(summary) => {
                self.reload_from_storage();
                self.error_message = None;
                self.status_message = Some(sync_summary_message(&summary));
            }
            Err(err) => {
                self.reload_from_storage();
                self.error_message = Some(err.to_string());
                self.status_message = None;
            }
        }

        cx.notify();
    }

    fn finish_connect_flow(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        self.connecting = false;

        match result {
            Ok(username) => {
                self.reload_from_storage();
                self.error_message = None;
                self.status_message = Some(format!("Connected @{username}. Syncing games now."));
                self.sync_all(cx);
            }
            Err(err) => {
                self.reload_from_storage();
                self.error_message = Some(err);
                self.status_message = None;
            }
        }

        cx.notify();
    }

    fn finish_remove_account(
        &mut self,
        username: &str,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        self.removing_accounts
            .remove(&username.to_ascii_lowercase());

        match result {
            Ok(username) => {
                self.reload_from_storage();
                self.error_message = None;
                self.status_message = Some(format!("Removed @{username}."));
            }
            Err(err) => {
                self.reload_from_storage();
                self.error_message = Some(err);
                self.status_message = None;
            }
        }

        cx.notify();
    }

    fn reload_from_storage(&mut self) {
        match storage::load_lichess_accounts() {
            Ok(accounts) => self.accounts = accounts,
            Err(err) => self.error_message = Some(err.to_string()),
        }

        match storage::load_lichess_games(HISTORY_GAMES_PER_ACCOUNT_LIMIT) {
            Ok(games) => self.games = games,
            Err(err) => self.error_message = Some(err.to_string()),
        }
    }
}

fn sync_summary_message(summary: &SyncSummary) -> String {
    if summary.checked_accounts == 0 {
        return "Add a Lichess account to start syncing games.".to_string();
    }

    if summary.new_games == 0 && summary.failed_accounts == 0 {
        return format!(
            "Checked {} account(s). No new games found.",
            summary.checked_accounts
        );
    }

    if summary.failed_accounts == 0 {
        return format!(
            "Checked {} account(s). Added {} new game(s).",
            summary.checked_accounts, summary.new_games
        );
    }

    if summary.new_games == 0 {
        return format!(
            "Checked {} account(s). No new games found. {} account(s) failed to sync.",
            summary.checked_accounts, summary.failed_accounts
        );
    }

    format!(
        "Checked {} account(s). Added {} new game(s). {} account(s) failed to sync.",
        summary.checked_accounts, summary.new_games, summary.failed_accounts
    )
}

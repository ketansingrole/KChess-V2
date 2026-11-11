//! Starting online games and answering challenges.

use std::sync::Arc;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::error::{CoreError, ErrorKind, Result as CoreResult};
use crate::host::Level;
use crate::lichess::accounts::path_segment;
use crate::lichess::challenges::{ChallengeInfo, TimeControl};
use crate::lichess::client::Auth;

use crate::lichess::online::events::{perf_for, rule_bool};
use crate::lichess::online::requests::{Emitter, StreamRequest, game_body};
use crate::lichess::online::state::{
    Handle, Lane, OnlineOptions, OnlineSession, SEEK_REJECTED, StartResult, js_number,
};

impl OnlineSession {
    /// `start(options)`: a challenge to a player, a public seek, or a correspondence game.
    pub async fn start(&self, options: &OnlineOptions) -> CoreResult<StartResult> {
        {
            let st = self.st();
            // A correspondence game on the board may be left for a real-time one.
            let correspondence = st.live.as_ref().is_some_and(|game| game.correspondence);
            if !correspondence && (st.live.is_some() || !st.protected_game.is_empty()) {
                return Err(CoreError::new(
                    "Reconnect to your current game before finding another.",
                ));
            }
        }
        let target = options.target.as_deref().unwrap_or("").trim().to_string();
        let correspondence = options.days.is_some();
        let perf = perf_for(options.minutes, options.increment);
        let label = format!(
            "{}+{}",
            js_number(options.minutes),
            js_number(options.increment)
        );
        // Validate before any stream opens, so a rejected request leaves nothing running.
        let has_fen = options.fen.as_deref().is_some_and(|fen| !fen.is_empty());
        let rejection = if has_fen && (target.is_empty() || options.rated) {
            Some(
                "A game from a set-up position must be a casual challenge to a player.".to_string(),
            )
        } else if !correspondence
            && !target.is_empty()
            && !rule_bool("canDirectChallenge", options.minutes, options.increment)
        {
            Some(format!(
                "Lichess blocks {perf} ({label}) even for direct challenges. The Board API allows Blitz and slower — pick at least 3 minutes of estimated play (time + 40 × increment)."
            ))
        } else if !correspondence
            && target.is_empty()
            && !rule_bool("canBoardSeek", options.minutes, options.increment)
        {
            Some(format!(
                "Lichess blocks {perf} ({label}) for public seeks. The Board API allows Rapid and slower for matchmaking — add a username to challenge directly (Blitz allowed) or pick a longer control."
            ))
        } else {
            None
        };
        if let Some(message) = rejection {
            // A refused request leaves the seek or challenge already running alone.
            return Err(CoreError::new(message));
        }
        self.cancel_inner();
        let handle = {
            let mut st = self.st();
            let handle = self.next_handle(&mut st);
            st.controller = Some(handle.clone());
            handle
        };
        match self
            .start_inner(options, &target, correspondence, &perf, &label, &handle)
            .await
        {
            Ok(result) => Ok(result),
            Err(cause) => {
                // Don't leave the event stream running for a game the UI never started.
                handle.token.cancel();
                let released = {
                    let mut st = self.st();
                    if st.controller.as_ref().is_some_and(|c| c.id == handle.id) {
                        st.controller = None;
                        true
                    } else {
                        false
                    }
                };
                if released {
                    self.start_lobby();
                }
                Err(cause)
            }
        }
    }

    async fn start_inner(
        &self,
        options: &OnlineOptions,
        target: &str,
        correspondence: bool,
        perf: &str,
        label: &str,
        handle: &Handle,
    ) -> CoreResult<StartResult> {
        let connected: Vec<String> = self
            .inner
            .lichess
            .store()
            .accounts()
            .await?
            .into_iter()
            .filter(|row| row.connected)
            .map(|row| row.username)
            .collect();
        if handle.token.is_cancelled() {
            return Err(crate::lichess::aborted());
        }
        if connected.is_empty() {
            return Err(CoreError::new(
                "Connect your Lichess account in Settings first.",
            ));
        }
        // An explicit account must match exactly: never play as a different one.
        let account = match options.account.as_deref().filter(|name| !name.is_empty()) {
            Some(wanted) => connected
                .iter()
                .find(|name| name.eq_ignore_ascii_case(wanted))
                .cloned()
                .ok_or_else(|| {
                    CoreError::new(format!(
                        "@{wanted} is not connected. Connect it in Settings."
                    ))
                })?,
            None => connected[0].clone(),
        };
        self.st().current_account = account.clone();
        let auth = self.login(&account).await?;
        if handle.token.is_cancelled() {
            return Err(crate::lichess::aborted());
        }
        let Some(auth) = auth else {
            return Err(crate::lichess::accounts::login_unavailable(&account));
        };
        if correspondence {
            // Correspondence games run for days: nothing waits here; the idle stream reports acceptance.
            let created = self
                .create_correspondence(options, &auth, &handle.token, target)
                .await?;
            handle.token.cancel();
            {
                let mut st = self.st();
                if st.controller.as_ref().is_some_and(|c| c.id == handle.id) {
                    st.controller = None;
                }
            }
            self.start_lobby();
            return Ok(StartResult {
                id: created.id,
                url: created.url,
                seeking: None,
                correspondence: Some(true),
            });
        }
        self.start_event_stream(auth.clone(), handle.clone());
        if !target.is_empty() {
            let mut form: Vec<(&'static str, String)> = vec![
                ("clock.limit", js_number(options.minutes * 60.0)),
                ("clock.increment", js_number(options.increment)),
            ];
            form.extend(game_body(options));
            let path = format!("/api/challenge/{}", path_segment(target));
            // Cancellation does not cut the creation short: Lichess may already have created the
            // challenge, and its id is cancelled below once the answer arrives.
            let creation = CancellationToken::new();
            let reply = self.post_form(&path, &form, &auth, &creation).await;
            let challenge = match reply {
                Ok(challenge) => challenge,
                Err(_) if handle.token.is_cancelled() => {
                    return Err(crate::lichess::aborted());
                }
                Err(cause) => {
                    if cause.kind == Some(ErrorKind::Refused) {
                        return Err(CoreError::new(format!(
                            "Lichess rejected this {perf} ({label}) challenge: {}. The Board API allows Blitz and slower for direct challenges.",
                            cause.message
                        )));
                    }
                    return Err(cause);
                }
            };
            let id = challenge["id"].as_str().unwrap_or("").to_string();
            // A server may finish creating the challenge just as cancellation arrives.
            if handle.token.is_cancelled() {
                self.cancel_challenge(&id, &auth).await;
                return Err(crate::lichess::aborted());
            }
            if self.st().live.is_none() {
                self.st().pending_challenge = Some(id.clone()).filter(|id| !id.is_empty());
            }
            return Ok(StartResult {
                id: Some(id),
                url: challenge["url"].as_str().map(str::to_string),
                seeking: None,
                correspondence: None,
            });
        }
        // A public seek: its stream is the game controller's, and it does not reconnect.
        let seek = {
            let mut st = self.st();
            let seek = self.next_handle(&mut st);
            st.game_controller = Some(seek.clone());
            seek
        };
        let mut form: Vec<(&'static str, String)> = vec![
            ("time", js_number(options.minutes)),
            ("increment", js_number(options.increment)),
        ];
        form.extend(game_body(options));
        let request = StreamRequest {
            method: reqwest::Method::POST,
            path: "/api/board/seek".into(),
            query: Vec::new(),
            auth,
            form: Some(form),
        };
        let emit_self = self.clone();
        let emit: Emitter = Arc::new(move |event| emit_self.emit("online:event", event));
        let mut options_listen = self.listen_options(Lane::Seek, false, None, None);
        options_listen.refused = Some(SEEK_REJECTED);
        let listen_self = self.clone();
        tokio::spawn(async move {
            listen_self
                .listen(request, seek.token, emit, options_listen)
                .await;
        });
        Ok(StartResult {
            id: None,
            url: None,
            seeking: Some(true),
            correspondence: None,
        })
    }

    async fn create_correspondence(
        &self,
        options: &OnlineOptions,
        auth: &Auth,
        cancel: &CancellationToken,
        target: &str,
    ) -> CoreResult<StartResult> {
        let mut form: Vec<(&'static str, String)> =
            vec![("days", js_number(options.days.unwrap_or_default()))];
        form.extend(game_body(options));
        let path = if target.is_empty() {
            "/api/board/seek".to_string()
        } else {
            format!("/api/challenge/{}", path_segment(target))
        };
        let created = self.post_form(&path, &form, auth, cancel).await?;
        Ok(StartResult {
            id: created["id"].as_str().map(str::to_string),
            url: created["url"].as_str().map(str::to_string),
            seeking: None,
            correspondence: None,
        })
    }

    /// A form POST as `auth`, under `cancel`. The answer is JSON.
    async fn post_form(
        &self,
        path: &str,
        form: &[(&str, String)],
        auth: &Auth,
        cancel: &CancellationToken,
    ) -> CoreResult<Value> {
        self.inner
            .lichess
            .client()
            .post(path, auth)
            .form(form)
            .cancel(cancel)
            .read_json()
            .await
    }

    /// A failed cancel is logged, not raised.
    pub(super) async fn cancel_challenge(&self, id: &str, auth: &Auth) {
        let path = format!("/api/challenge/{}/cancel", path_segment(id));
        if let Err(cause) = self.challenge_post(auth, &path, &[]).await {
            self.log(
                Level::Debug,
                format!("Challenge cancel failed: {id} {}", cause.message),
            );
        }
    }

    async fn challenge_post(
        &self,
        auth: &Auth,
        path: &str,
        form: &[(&str, &str)],
    ) -> CoreResult<()> {
        self.inner
            .lichess
            .client()
            .post(path, auth)
            .form(form)
            .read_json::<Value>()
            .await
            .map(|_| ())
    }

    /// Accept an incoming challenge as the account it was sent to; a real-time game opens at once.
    pub async fn accept_challenge(&self, challenge: &ChallengeInfo) -> CoreResult<()> {
        let live = matches!(challenge.time_control, TimeControl::Clock { .. });
        if live {
            let st = self.st();
            if st.realtime_live() {
                return Err(CoreError::new(
                    "Finish the game on the board before accepting another.",
                ));
            }
        }
        let auth = self.required_login(&challenge.account).await?;
        let path = format!("/api/challenge/{}/accept", path_segment(&challenge.id));
        self.challenge_post(&auth, &path, &[]).await?;
        // The game has the challenge's id. Open it now rather than wait for the event stream,
        // which may belong to another connected account.
        if live {
            self.attach(&challenge.account, &challenge.id).await
        } else {
            self.ongoing_changed();
            Ok(())
        }
    }

    /// Decline an incoming challenge with a reason Lichess shows the challenger.
    pub async fn decline_challenge(
        &self,
        challenge: &ChallengeInfo,
        reason: &str,
    ) -> CoreResult<()> {
        let auth = self.required_login(&challenge.account).await?;
        let path = format!("/api/challenge/{}/decline", path_segment(&challenge.id));
        self.challenge_post(&auth, &path, &[("reason", reason)])
            .await
    }

    /// Withdraw an outgoing challenge.
    pub async fn withdraw_challenge(&self, challenge: &ChallengeInfo) -> CoreResult<()> {
        let auth = self.required_login(&challenge.account).await?;
        {
            let mut st = self.st();
            if st.pending_challenge.as_deref() == Some(challenge.id.as_str()) {
                st.pending_challenge = None;
            }
        }
        let path = format!("/api/challenge/{}/cancel", path_segment(&challenge.id));
        self.challenge_post(&auth, &path, &[]).await
    }
}

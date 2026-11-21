//! Challenges and games in progress: the account's challenges, the event stream kept open for
//! them, and the games waiting on a move.

use super::*;

impl OnlineState {
    pub fn refresh_ongoing(&mut self, cx: &mut Context<Self>) {
        // A newer request either way: an answer still on its way (a poll overtaken by a change,
        // or from before signing out) must not bring back older games, or their notifications.
        if crate::accounts::handle(cx).read(cx).connected().is_empty() {
            self.ongoing_loading.stop();
            self.ongoing.clear();
            cx.notify();
            return;
        }
        let turn = self.ongoing_loading.start();
        let call = cx.global::<AppCore>().request(calls::OngoingGames {});
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                if !this.ongoing_loading.settle(turn, "online-game", &answer) {
                    return;
                }
                if let Ok(games) = answer {
                    for game in &games {
                        let id = game.game_id.clone();
                        let mine = game.is_my_turn;
                        let before = this.turns.get(&id).copied();
                        if is_your_move_now(game, before, this.id()) {
                            crate::notifications::your_move(game, cx);
                        }
                        this.turns.insert(id, mine);
                    }
                    this.ongoing = games;
                }
                this.ongoing_loaded = true;
                cx.notify();
            });
        })
        .detach();
    }

    /// The account whose event stream stays open while idle: one taking part in a tournament,
    /// else the active one when challenges are wanted.
    pub fn stay_connected(&mut self, cx: &mut Context<Self>) {
        let wanted = cx
            .try_global::<crate::settings::SettingsHandle>()
            .is_none_or(|settings| settings.0.read(cx).settings.core.receive_challenges);
        let joined = cx.global::<AppCore>().request(calls::JoinedTournaments {});
        let active = crate::accounts::handle(cx)
            .read(cx)
            .active(cx)
            .unwrap_or_default();
        let core = cx.global::<AppCore>().clone();
        cx.spawn(async move |this, cx| {
            // Unknown tournaments leave the stream as it is: read as none, a player's tournament
            // stream would close.
            let Some(joined) = joined.await.warn_on_err() else {
                return;
            };
            let now = kchess_games::now_ms();
            let current: Vec<_> = joined.iter().filter(|entry| entry.until > now).collect();
            let participant = current
                .iter()
                .find(|entry| entry.account == active)
                .or(current.last())
                .map(|entry| entry.account.clone());
            let account = participant.unwrap_or(if wanted { active } else { String::new() });
            if let Err(cause) = core
                .request(calls::StayConnected {
                    account: OptionalAccount(account.clone()),
                })
                .await
            {
                log(&format!("staying connected failed: {}", cause.message));
            }
            if account.is_empty() {
                let _ = this.update(cx, |this, cx| {
                    this.lobby = None;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn load_challenges(&mut self, cx: &mut Context<Self>) {
        let call = cx.global::<AppCore>().request(calls::Challenges {});
        cx.spawn(async move |this, cx| {
            if let Some(list) = call.await.warn_on_err() {
                let _ = this.update(cx, |this, cx| {
                    this.challenges = list;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn challenge_act(
        &mut self,
        id: String,
        call: impl Future<Output = Result<(), CoreError>> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        self.busy_challenge = id;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            this.update(cx, |this, cx| {
                this.busy_challenge.clear();
                cx.notify();
                match answer {
                    Ok(_) => true,
                    Err(cause) => {
                        notice(cx, kchess_app::loading::failure("online-game", &cause));
                        false
                    }
                }
            })
            .unwrap_or(false)
        })
    }

    /// Accept a challenge: a live one opens on the board; a correspondence one joins the games.
    pub fn accept(&mut self, challenge: &ChallengeInfo, cx: &mut Context<Self>) -> Task<bool> {
        let id = challenge.id.clone();
        let live = matches!(challenge.time_control, ChallengeTimeControl::Clock { .. });
        if live {
            self.prepare_for_game(&challenge.account, &id);
        }
        let opponent = challenge.opponent.name.clone();
        let call = cx.global::<AppCore>().request(calls::AcceptChallenge {
            id: GameId(id.clone()),
        });
        let done = self.challenge_act(id, call, cx);
        cx.spawn(async move |this, cx| {
            let done = done.await;
            if done && !live {
                let _ = this.update(cx, |this, cx| {
                    notice(
                        cx,
                        format!("Accepted. Your game against {opponent} is under Correspondence."),
                    );
                    this.refresh_ongoing(cx);
                });
            }
            done && live
        })
    }

    pub fn decline(
        &mut self,
        challenge: &ChallengeInfo,
        reason: DeclineReason,
        cx: &mut Context<Self>,
    ) {
        let id = challenge.id.clone();
        let call = cx.global::<AppCore>().request(calls::DeclineChallenge {
            id: GameId(id.clone()),
            reason,
        });
        self.challenge_act(id, call, cx).detach();
    }

    pub fn withdraw(&mut self, challenge: &ChallengeInfo, cx: &mut Context<Self>) {
        let id = challenge.id.clone();
        let call = cx.global::<AppCore>().request(calls::CancelChallenge {
            id: GameId(id.clone()),
        });
        self.challenge_act(id, call, cx).detach();
    }

    /// The opponent's rematch offer for the game just finished.
    pub fn rematch_offer(&self) -> Option<ChallengeInfo> {
        let last = self.model.last_game.as_ref()?.id.as_str();
        self.challenges
            .iter()
            .find(|c| {
                c.direction == kchess_contracts::types::ChallengeDirection::In
                    && c.rematch_of.as_deref() == Some(last)
            })
            .cloned()
    }

    pub(super) fn start_polling_ongoing(&mut self, cx: &mut Context<Self>) {
        let minutes = cx
            .try_global::<crate::settings::SettingsHandle>()
            .map_or(5., |settings| {
                settings.0.read(cx).settings.core.correspondence_poll
            });
        if minutes <= 0. {
            self._ongoing_poll = None;
            return;
        }
        let every = Duration::from_secs_f64(minutes.max(1.) * 60.);
        self._ongoing_poll = Some(crate::timer::every(every, cx, |this, cx| {
            this.refresh_ongoing(cx);
            true
        }));
    }
}

/// A challenge's clock as its card and notice name it: "5+3", "2 days per move", "No clock".
pub fn describe_control(control: &ChallengeTimeControl) -> String {
    match control {
        ChallengeTimeControl::Clock { limit, increment } => crate::widgets::time_control(
            limit.as_f64().unwrap_or_default() / 60.,
            increment.as_f64().unwrap_or_default(),
        ),
        ChallengeTimeControl::Correspondence { days } => {
            let days = days.as_f64().unwrap_or_default();
            format!(
                "{days} {} per move",
                if days == 1. { "day" } else { "days" }
            )
        }
        ChallengeTimeControl::Unlimited => "No clock".into(),
    }
}

/// A challenge's notification: who sent it, then its clock, variant and whether it is rated
/// ("Alice (1800) challenges you", "5+3 · Standard · Rated").
pub fn challenge_alert(challenge: &ChallengeInfo) -> (String, String) {
    let opponent = &challenge.opponent;
    let name = &opponent.name;
    let title = match opponent
        .rating
        .as_ref()
        .and_then(serde_json::Number::as_f64)
    {
        Some(rating) => format!("{name} ({rating}) challenges you"),
        None => format!("{name} challenges you"),
    };
    let variant = &challenge.variant_name;
    let rated = if challenge.rated { "Rated" } else { "Casual" };
    let control = describe_control(&challenge.time_control);
    (title, format!("{control} · {variant} · {rated}"))
}

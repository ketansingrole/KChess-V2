//! Engine help beside a live game: held back while a game is played, looked for, or not yet
//! known to be over, and the checks for a game in progress that let it back.

use super::*;

/// Why engine help is held back now (`OnlineState::pause`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause {
    /// Lichess is being asked for a game in progress.
    Checking,
    /// Lichess could not be asked, or did not answer: offered again (`check_again`).
    Unverified,
    /// A live game is being played, looked for, or not yet seen to end.
    LiveGame,
}

impl Pause {
    pub fn text(self) -> &'static str {
        match self {
            Pause::Checking => "Checking Lichess for a game in progress…",
            Pause::Unverified => "Couldn't check Lichess for a game in progress",
            Pause::LiveGame => "Paused during your Lichess game",
        }
    }
}

/// The first wait before a failed check for a game in progress is tried again, and the longest.
pub(super) const RECHECK_FIRST: Duration = Duration::from_secs(15);
pub(super) const RECHECK_MAX: Duration = Duration::from_secs(5 * 60);

impl OnlineState {
    /// The engine may help: no live game, none still being looked for, and the core lends it.
    /// (The app's phase alone took a failed check for a game as all clear, while the core kept
    /// refusing every engine call.)
    pub fn assistance_allowed(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Finished)
            && self.assistance == Assistance::Allowed
    }

    /// Why engine help is held back, if it is.
    pub fn pause(&self) -> Option<Pause> {
        if self.assistance_allowed() {
            return None;
        }
        let checking = self.phase == Phase::Checking
            || self
                .model
                .online_connection
                .as_ref()
                .is_some_and(OnlineConnection::checking);
        Some(if checking {
            Pause::Checking
        } else if matches!(self.phase, Phase::Idle | Phase::Finished)
            && self.assistance == Assistance::Unverified
        {
            Pause::Unverified
        } else {
            Pause::LiveGame
        })
    }

    /// Ask the core what help it lends now, unless it reports a change first.
    pub(super) fn read_assistance(&mut self, cx: &mut Context<Self>) {
        let seen = self.assistance_reports;
        let status = cx.global::<AppCore>().request(calls::AssistanceStatus {});
        cx.spawn(async move |this, cx| match status.await {
            Ok(assistance) => {
                this.update(cx, |state, cx| {
                    if state.assistance_reports == seen {
                        state.assistance = assistance;
                        cx.notify();
                    }
                })
                .ok();
            }
            Err(cause) => log(&format!("reading engine help failed: {}", cause.message)),
        })
        .detach();
    }

    /// A check for a game in progress failed: try again later, longer each time. A refused login
    /// waits for the account to be connected again, which checks again itself.
    pub(super) fn schedule_recheck(&mut self, cx: &mut Context<Self>) {
        let refused = self
            .model
            .online_connection
            .as_ref()
            .is_some_and(|connection| connection.phase == OnlinePhase::AuthRequired);
        if refused {
            self._recheck = None;
            return;
        }
        let wait = self.recheck_after;
        self.recheck_after = (wait * 2).min(RECHECK_MAX);
        self._recheck = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(wait).await;
            cx.update(check_again);
        }));
    }
}

/// Ask Lichess again for a game in progress after a check that failed (its Check again button,
/// the retry timer, the network's return): the core lends engine help only once one answers.
pub fn check_again(cx: &mut App) {
    let Some(state) = cx
        .try_global::<OnlineHandle>()
        .map(|handle| handle.0.clone())
    else {
        return;
    };
    let failed = state.update(cx, |state, cx| {
        if state.pause() != Some(Pause::Unverified) {
            return false;
        }
        state._recheck = None;
        state.transition(Model::checking, None, cx);
        true
    });
    if failed {
        resume(state, cx);
    }
}

/// The Check again link shown where a failed check holds engine help back.
pub fn check_again_button(id: &'static str) -> gpui_kit::component::button::Button {
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    Button::new(id)
        .link()
        .xsmall()
        .label("Check again")
        .on_click(|_, _, cx| check_again(cx))
}

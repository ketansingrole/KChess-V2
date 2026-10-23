//! Whether a live Lichess game is on, as far as engine help is concerned. At startup the core
//! recovers any live game (`resumeOnline`, which it requires before engine assistance); until then,
//! and while a game is live, the engine stays off (`assistanceAllowed` in `stores/kchess.ts`).

use gpui_kit::{App, AppContext as _, Entity, Global};
use kchess_core::Level;
use serde_json::{Value, json};

use crate::core::AppCore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Recovering a live game after startup.
    Checking,
    Idle,
    /// A live game was found; it plays on the Lichess page.
    Playing,
}

pub struct OnlineState {
    pub phase: Phase,
}

impl OnlineState {
    /// The engine may help: no live game, and none still being looked for.
    pub fn assistance_allowed(&self) -> bool {
        self.phase == Phase::Idle
    }
}

/// The account Lichess actions use (`activeOnlineAccount`): the one chosen for online play when it
/// is connected, else the first connected one.
pub fn active_account(accounts: &[Value], cx: &App) -> Option<String> {
    let preferred = crate::prefs::get::<String>(cx, "kchess:online-account", String::new());
    let listed: Vec<Value> = accounts
        .iter()
        .map(
            |account| json!({ "username": account["username"], "connected": account["connected"] }),
        )
        .collect();
    let index = crate::game::rules("connectedAccountIndex", &[json!(listed), json!(preferred)])
        .ok()
        .filter(|index| !index.is_null())?;
    accounts.get(crate::game::whole(&index) as usize)?["username"]
        .as_str()
        .map(str::to_string)
}

#[derive(Clone)]
pub struct OnlineHandle(pub Entity<OnlineState>);

impl Global for OnlineHandle {}

pub fn init(cx: &mut App) {
    let state = cx.new(|_| OnlineState {
        phase: Phase::Checking,
    });
    cx.set_global(OnlineHandle(state.clone()));
    let resume = cx.global::<AppCore>().call("resumeOnline", Vec::new());
    cx.spawn(async move |cx| {
        let resumed = resume.await;
        state.update(cx, |state, cx| {
            state.phase = match resumed {
                Ok(game) if !game.is_null() => Phase::Playing,
                Ok(_) => Phase::Idle,
                Err(cause) => {
                    crate::log::write(
                        Level::Warn,
                        "kchess",
                        &format!("resuming online game failed: {}", cause.message),
                    );
                    Phase::Idle
                }
            };
            cx.notify();
        });
    })
    .detach();
}

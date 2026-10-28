//! `ComputerGame` : a game against the engine. The rules decide everything
//! (`crates/kchess-domain/src/games/computer.rs`); the page answers the reads (`now`, `allowed`,
//! `ready`) and carries out the effects, in order.

use serde_json::{Value, json};

use crate::{assign_written, rules, transition};

/// What a transition asks the page to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    StopEngine,
    Flagged,
    Moved {
        san: String,
        computer: bool,
    },
    /// It is the engine's turn: start a search (`search_start`).
    ComputerTurn,
    /// The last engine operation failed.
    Failed,
}

/// An engine search to send; its reply must still match the epoch and move count.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchRequest {
    pub epoch: u64,
    pub count: u64,
    pub moves: Vec<String>,
    pub level: String,
    pub options: Value,
}

/// What the page reports when a transition reads the host.
pub struct Host {
    /// Monotonic milliseconds, as `performance.now()`.
    pub now: f64,
    pub allowed: bool,
    pub ready: bool,
}

pub struct ComputerGame {
    /// The `ComputerState` document (the saved session plus `thinking` and `epoch`).
    pub state: Value,
    runtime: Value,
}

struct Done {
    value: Value,
    effects: Vec<Effect>,
}

fn effects(done: &Value) -> Vec<Effect> {
    done["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|effect| match effect["type"].as_str()? {
            "stopEngine" => Some(Effect::StopEngine),
            "flagged" => Some(Effect::Flagged),
            "moved" => Some(Effect::Moved {
                san: effect["san"].as_str()?.to_string(),
                computer: effect["computer"].as_bool()?,
            }),
            "computerTurn" => Some(Effect::ComputerTurn),
            "failed" => Some(Effect::Failed),
            // `debug`: an expected cancellation, silent by design.
            _ => None,
        })
        .collect()
}

impl ComputerGame {
    /// A game from a saved session (`computerState(saved)`), or a new one.
    pub fn new(saved: Option<&Value>, host: &Host) -> Result<ComputerGame, String> {
        let state = rules("computerState", &[saved.cloned().unwrap_or(Value::Null)])?;
        let init = transition("computerInit", &[], |kind| read(host, kind))?;
        let runtime = init
            .get("runtime")
            .cloned()
            .ok_or("A computer game could not start.")?;
        Ok(ComputerGame { state, runtime })
    }

    fn apply(&mut self, method: &str, input: &[Value], host: &Host) -> Result<Done, String> {
        let mut args = vec![self.state.clone(), self.runtime.clone()];
        args.extend_from_slice(input);
        let done = transition(method, &args, |kind| read(host, kind))?;
        if let Some(next) = done.get("state") {
            assign_written(&mut self.state, next, &done["written"]);
        }
        if let Some(runtime) = done.get("runtime") {
            self.runtime = runtime.clone();
        }
        Ok(Done {
            value: done.get("value").cloned().unwrap_or(Value::Null),
            effects: effects(&done),
        })
    }

    /// A getter of the game; nothing changes.
    fn view(&self, query: Value, host: &Host) -> Value {
        transition(
            "computerView",
            &[self.state.clone(), self.runtime.clone(), query],
            |kind| read(host, kind),
        )
        .ok()
        .and_then(|done| done.get("value").cloned())
        .unwrap_or(Value::Null)
    }

    pub fn over(&self, host: &Host) -> bool {
        self.view(json!({ "view": "over" }), host).as_bool() == Some(true)
    }

    /// `{ kind, title, detail }` once the game has ended.
    pub fn result(&self, host: &Host) -> Option<Value> {
        Some(self.view(json!({ "view": "result" }), host)).filter(|value| !value.is_null())
    }

    /// A side's clock in milliseconds.
    pub fn remaining(&self, color: &str, host: &Host) -> f64 {
        self.view(
            json!({ "view": "remaining", "color": color, "at": host.now }),
            host,
        )
        .as_f64()
        .unwrap_or(0.)
    }

    /// The session to save (`saveSession('computer', …)`).
    pub fn snapshot(&self, host: &Host) -> Value {
        self.view(json!({ "view": "snapshot" }), host)
    }

    /// The finished game for the history (`saveArchivedGame`).
    pub fn archive_snapshot(&self, host: &Host) -> Value {
        self.view(json!({ "view": "archiveSnapshot" }), host)
    }

    pub fn availability_changed(&mut self, host: &Host) -> Result<Vec<Effect>, String> {
        Ok(self.apply("computerAvailability", &[], host)?.effects)
    }

    pub fn expire(&mut self, host: &Host) -> Result<Vec<Effect>, String> {
        Ok(self
            .apply("computerExpire", &[json!(host.now)], host)?
            .effects)
    }

    /// Play the player's move; false when it is not legal or not their turn.
    pub fn play(&mut self, uci: &str, host: &Host) -> Result<(bool, Vec<Effect>), String> {
        let done = self.apply("computerMove", &[json!(uci)], host)?;
        Ok((done.value.as_bool() == Some(true), done.effects))
    }

    /// The engine search to send now, if it is the engine's turn and nothing is searching.
    pub fn search_start(&mut self, host: &Host) -> Result<Option<SearchRequest>, String> {
        if self.state["thinking"].as_bool() == Some(true) {
            return Ok(None);
        }
        let request = self.apply("computerSearchStart", &[], host)?.value;
        if request.is_null() {
            return Ok(None);
        }
        Ok(Some(SearchRequest {
            epoch: crate::whole(&request["epoch"]),
            count: crate::whole(&request["count"]),
            moves: serde_json::from_value(request["moves"].clone()).unwrap_or_default(),
            level: request["level"].as_str().unwrap_or("beginner").to_string(),
            options: request["options"].clone(),
        }))
    }

    pub fn search_reply(
        &mut self,
        request: &SearchRequest,
        reply: &str,
        host: &Host,
    ) -> Result<Vec<Effect>, String> {
        let input = json!({ "epoch": request.epoch, "count": request.count, "move": reply });
        let effects = self.apply("computerSearchReply", &[input], host)?.effects;
        self.apply(
            "computerSearchEnd",
            &[json!({ "epoch": request.epoch })],
            host,
        )?;
        Ok(effects)
    }

    pub fn search_failed(
        &mut self,
        request: &SearchRequest,
        host: &Host,
    ) -> Result<Vec<Effect>, String> {
        let effects = self
            .apply(
                "computerSearchFailed",
                &[json!({ "epoch": request.epoch })],
                host,
            )?
            .effects;
        self.apply(
            "computerSearchEnd",
            &[json!({ "epoch": request.epoch })],
            host,
        )?;
        Ok(effects)
    }

    /// A new game; `setup` and `clock` default to the current game's.
    pub fn start(
        &mut self,
        setup: Option<Value>,
        clock: Option<Value>,
        host: &Host,
    ) -> Result<Vec<Effect>, String> {
        let setup = setup.unwrap_or_else(|| self.state["setup"].clone());
        let clock = clock.unwrap_or_else(|| self.state["clock"].clone());
        Ok(self.apply("computerStart", &[setup, clock], host)?.effects)
    }

    pub fn takeback(&mut self, host: &Host) -> Result<Vec<Effect>, String> {
        Ok(self.apply("computerTakeback", &[], host)?.effects)
    }

    pub fn resign(&mut self, host: &Host) -> Result<Vec<Effect>, String> {
        Ok(self.apply("computerResign", &[], host)?.effects)
    }
}

fn read(host: &Host, kind: &str) -> Result<Value, String> {
    match kind {
        "now" => Ok(json!(host.now)),
        "allowed" => Ok(json!(host.allowed)),
        "ready" => Ok(json!(host.ready)),
        other => Err(format!("A computer game has no host read {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(now: f64) -> Host {
        Host {
            now,
            allowed: true,
            ready: true,
        }
    }

    #[test]
    fn a_move_hands_the_turn_to_the_engine_and_its_reply_is_played() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        let (played, effects) = game.play("e2e4", &host(10.)).unwrap();
        assert!(played);
        assert!(effects.contains(&Effect::ComputerTurn), "{effects:?}");
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Moved { san, computer: false } if san == "e4"))
        );
        let request = game.search_start(&host(20.)).unwrap().expect("a search");
        assert_eq!(request.moves, vec!["e2e4"]);
        assert_eq!(game.state["thinking"], true);
        // A second start while it searches sends nothing.
        assert!(game.search_start(&host(21.)).unwrap().is_none());
        let effects = game.search_reply(&request, "e7e5", &host(30.)).unwrap();
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Moved { san, computer: true } if san == "e5"))
        );
        assert_eq!(game.state["moves"], json!(["e2e4", "e7e5"]));
        assert_eq!(game.state["thinking"], false);
    }

    #[test]
    fn a_reply_to_a_game_taken_back_is_ignored() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.)).unwrap();
        let request = game.search_start(&host(2.)).unwrap().unwrap();
        game.takeback(&host(3.)).unwrap();
        game.search_reply(&request, "e7e5", &host(4.)).unwrap();
        assert_eq!(game.state["moves"], json!([]));
    }

    #[test]
    fn an_illegal_move_is_refused() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        let (played, _) = game.play("e2e5", &host(1.)).unwrap();
        assert!(!played);
        assert!(!game.over(&host(1.)));
    }
}

//! `LocalGame` : two people at one board. The rules run it
//! (`crates/kchess-domain/src/games/local.rs`); the page answers `now` and carries out the one
//! effect, a flag falling.

use serde_json::{Value, json};

use crate::{assign_written, rules, transition};

pub struct LocalGame {
    /// The `LocalState` document: the saved session and its pause flag.
    pub state: Value,
    runtime: Value,
}

/// Whether a transition let a flag fall (`flagged`).
pub struct Flagged(pub bool);

fn read(now: f64, kind: &str) -> Result<Value, String> {
    match kind {
        "now" => Ok(json!(now)),
        other => Err(format!("A local game has no host read {other}")),
    }
}

impl LocalGame {
    /// A game from a saved session (`localState(saved)`), or a new one.
    pub fn new(saved: Option<&Value>, now: f64) -> Result<LocalGame, String> {
        let state = rules("localState", &[saved.cloned().unwrap_or(Value::Null)])?;
        let init = transition("localInit", &[], |kind| read(now, kind))?;
        let runtime = init
            .get("runtime")
            .cloned()
            .ok_or("A local game could not start.")?;
        Ok(LocalGame { state, runtime })
    }

    fn apply(
        &mut self,
        method: &str,
        input: &[Value],
        now: f64,
    ) -> Result<(Value, Flagged), String> {
        let mut args = vec![self.state.clone(), self.runtime.clone()];
        args.extend_from_slice(input);
        let done = transition(method, &args, |kind| read(now, kind))?;
        if let Some(next) = done.get("state") {
            assign_written(&mut self.state, next, &done["written"]);
        }
        if let Some(runtime) = done.get("runtime") {
            self.runtime = runtime.clone();
        }
        let flagged = done["effects"]
            .as_array()
            .is_some_and(|effects| effects.iter().any(|effect| effect["type"] == "flagged"));
        Ok((
            done.get("value").cloned().unwrap_or(Value::Null),
            Flagged(flagged),
        ))
    }

    fn view(&self, query: Value, now: f64) -> Value {
        transition(
            "localView",
            &[self.state.clone(), self.runtime.clone(), query],
            |kind| read(now, kind),
        )
        .ok()
        .and_then(|done| done.get("value").cloned())
        .unwrap_or(Value::Null)
    }

    /// `{ winner?, reason }` once the game has ended.
    pub fn result(&self, now: f64) -> Option<Value> {
        Some(self.view(json!({ "view": "result" }), now)).filter(|value| !value.is_null())
    }

    pub fn over(&self, now: f64) -> bool {
        self.view(json!({ "view": "over" }), now).as_bool() == Some(true)
    }

    /// A side's clock in milliseconds.
    pub fn remaining(&self, color: &str, now: f64) -> f64 {
        self.view(
            json!({ "view": "remaining", "color": color, "at": now }),
            now,
        )
        .as_f64()
        .unwrap_or(0.)
    }

    pub fn snapshot(&self, now: f64) -> Value {
        self.view(json!({ "view": "snapshot" }), now)
    }

    pub fn archive_snapshot(&self, now: f64) -> Value {
        self.view(json!({ "view": "archiveSnapshot" }), now)
    }

    pub fn expire(&mut self, now: f64) -> Result<Flagged, String> {
        Ok(self.apply("localExpire", &[json!(now)], now)?.1)
    }

    /// Play a move; its SAN, or `None` when it is not legal.
    pub fn play(&mut self, uci: &str, now: f64) -> Result<(Option<String>, Flagged), String> {
        let (san, flagged) = self.apply("localMove", &[json!(uci)], now)?;
        Ok((san.as_str().map(str::to_string), flagged))
    }

    pub fn takeback(&mut self, now: f64) -> Result<Flagged, String> {
        Ok(self.apply("localTakeback", &[], now)?.1)
    }

    /// A new game from `setup` with `clock` (`{ white: {minutes, increment}, black: … }` or null).
    pub fn start(&mut self, setup: Value, clock: Value, now: f64) -> Result<Flagged, String> {
        Ok(self.apply("localStart", &[setup, clock], now)?.1)
    }

    pub fn resign(&mut self, color: &str, now: f64) -> Result<Flagged, String> {
        Ok(self.apply("localResign", &[json!(color)], now)?.1)
    }

    pub fn agree_draw(&mut self, now: f64) -> Result<Flagged, String> {
        Ok(self.apply("localAgreeDraw", &[], now)?.1)
    }

    pub fn toggle_pause(&mut self, now: f64) -> Result<Flagged, String> {
        Ok(self.apply("localTogglePause", &[], now)?.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_move_and_a_resignation_ends_the_game() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        assert_eq!(game.play("e2e4", 1.).unwrap().0.as_deref(), Some("e4"));
        assert_eq!(game.play("e7e5", 2.).unwrap().0.as_deref(), Some("e5"));
        assert_eq!(game.play("e2e4", 3.).unwrap().0, None);
        game.resign("white", 4.).unwrap();
        assert!(game.over(5.));
        assert_eq!(game.result(5.).unwrap()["winner"], "black");
    }

    #[test]
    fn a_clock_runs_from_the_first_move_and_pauses() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        let setup = game.state["setup"].clone();
        let clock = json!({ "white": { "minutes": 1, "increment": 0 }, "black": { "minutes": 1, "increment": 0 } });
        game.start(setup, clock, 0.).unwrap();
        assert_eq!(game.remaining("white", 10_000.), 60_000.);
        game.play("e2e4", 1_000.).unwrap();
        assert!(game.remaining("black", 11_000.) < 60_000.);
        game.toggle_pause(11_000.).unwrap();
        let paused = game.remaining("black", 11_000.);
        assert_eq!(game.remaining("black", 40_000.), paused);
    }
}

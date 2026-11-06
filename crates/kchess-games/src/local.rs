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

    fn one_minute() -> Value {
        json!({ "white": { "minutes": 1, "increment": 0 }, "black": { "minutes": 1, "increment": 0 } })
    }

    #[test]
    fn a_flag_falls_once_the_side_to_move_runs_out() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        let setup = game.state["setup"].clone();
        game.start(setup, one_minute(), 0.).unwrap();
        game.play("e2e4", 1_000.).unwrap();
        assert!(!game.expire(30_000.).unwrap().0);
        assert!(!game.over(30_000.));
        assert!(game.expire(62_000.).unwrap().0);
        assert!(game.over(62_000.));
        assert_eq!(game.result(62_000.).unwrap()["winner"], "white");
    }

    #[test]
    fn an_agreed_draw_ends_the_game_without_a_winner() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.play("e2e4", 1.).unwrap();
        game.agree_draw(2.).unwrap();
        assert!(game.over(3.));
        assert!(
            game.result(3.)
                .unwrap()
                .get("winner")
                .is_none_or(Value::is_null)
        );
    }

    #[test]
    fn a_takeback_undoes_the_last_move() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.play("e2e4", 1.).unwrap();
        game.takeback(2.).unwrap();
        // White is to move again, so e4 is legal once more and e5 is not.
        assert_eq!(game.play("e7e5", 3.).unwrap().0, None);
        assert_eq!(game.play("e2e4", 4.).unwrap().0.as_deref(), Some("e4"));
    }

    #[test]
    fn checkmate_ends_the_game() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        for (ply, uci) in ["f2f3", "e7e5", "g2g4", "d8h4"].iter().enumerate() {
            assert!(!game.over(ply as f64));
            game.play(uci, ply as f64).unwrap();
        }
        assert!(game.over(10.));
        assert_eq!(game.result(10.).unwrap()["winner"], "black");
    }

    #[test]
    fn a_saved_game_resumes_where_it_was_left() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.play("e2e4", 1.).unwrap();
        game.play("e7e5", 2.).unwrap();
        let mut resumed = LocalGame::new(Some(&game.state), 3.).unwrap();
        assert_eq!(resumed.play("e2e4", 4.).unwrap().0, None);
        assert_eq!(resumed.play("g1f3", 5.).unwrap().0.as_deref(), Some("Nf3"));
    }

    /// `1e306` minutes made both times infinite, stored as `null`: every later move failed to
    /// read the state, and the game could only be reset. Negative minutes flagged at once.
    #[test]
    fn a_clock_no_game_can_have_is_refused_at_the_start() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        let setup = game.state["setup"].clone();
        for (minutes, increment) in [(1e306, 0.), (-5., 0.), (5., -1.), (181., 0.), (5., 1e9)] {
            let side = json!({ "minutes": minutes, "increment": increment });
            let clock = json!({ "white": side, "black": { "minutes": 5, "increment": 0 } });
            assert!(
                game.start(setup.clone(), clock, 0.).is_err(),
                "{minutes} {increment}"
            );
            assert_eq!(game.play("e2e4", 1.).unwrap().0.as_deref(), Some("e4"));
            game.takeback(2.).unwrap();
        }
        let clock = json!({ "white": { "minutes": 180, "increment": 180 }, "black": { "minutes": 0, "increment": 0 } });
        game.start(setup, clock, 0.)
            .expect("the longest and shortest clocks");
    }
}

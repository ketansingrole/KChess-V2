//! Spoken moves on a board: a clear move is played (unless Settings ask to
//! confirm moves), an unsure or ambiguous one waits for “confirm”, its number, or the move said
//! again. A page's own commands (“take back”, “back”) are tried first. Each phrase goes to the
//! voice history with what came of it, and once a move is made where it was said, the move that was
//! meant: that pairing of “heard” and “meant” shows where recognition goes wrong.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use serde_json::{Value, json};

use super::control::Control;
use super::{Entry, Heard, VoiceEvent};
use crate::assets::icon;

/// What a page made of a phrase it took as a command.
pub struct Reply {
    pub said: String,
    pub feedback: String,
    /// False when it was a command but not one that applies now.
    pub understood: bool,
}

/// A move the phrase could mean.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub uci: String,
    pub san: String,
}

/// When the board takes a move: `taking` is the player's turn at the live position, `idle` says
/// why not otherwise, and `immediate` plays a lone match at once (trying a move there costs
/// nothing).
pub struct Turn<'a> {
    pub taking: bool,
    pub idle: Option<&'a str>,
    pub immediate: bool,
}

/// What the page should do with a phrase.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Nothing,
    Play(String),
}

/// Where the choices' element ids start in the bar (`voice_bar` numbers its own from 0).
const CHOICE_IDS: usize = 10;

/// A board's voice input: on or off, the choice waiting, and what was last said.
pub struct VoiceMoves {
    pub source: &'static str,
    /// On or off, and the answer to the last phrase.
    pub control: Control,
    pub choices: Vec<Choice>,
    /// The position the choices belong to.
    context: String,
    /// The moves (SAN) that led to the position, when the page tells them.
    line: Option<Vec<String>>,
    /// The phrase that offered the choices waiting.
    pending: Option<Entry>,
    /// The last phrase that came to nothing, which the next one retries.
    last_miss: Option<Entry>,
    /// Phrases said in this position, and the line when the first was.
    position_entries: Vec<Entry>,
    position_line: Vec<String>,
}

/// Every legal move the words could be (`spokenMove`): `move` with its choices, `confirm`,
/// `cancel`, or nothing.
pub fn spoken_move(text: &str, setup: &Value) -> Value {
    crate::game::rules("spokenMove", &[json!(text), setup.clone()]).unwrap_or(Value::Null)
}

/// The move played after `before`, when `line` is `before` and one move more.
fn meant<'a>(before: &[String], line: &'a [String]) -> Option<&'a str> {
    (line.len() == before.len() + 1 && line.starts_with(before))
        .then(|| line[before.len()].as_str())
}

impl VoiceMoves {
    pub fn new(source: &'static str, cx: &mut App) -> VoiceMoves {
        VoiceMoves {
            source,
            control: Control::new(true, cx),
            choices: Vec::new(),
            context: String::new(),
            line: None,
            pending: None,
            last_miss: None,
            position_entries: Vec::new(),
            position_line: Vec::new(),
        }
    }

    /// Whether `event` is this board's.
    pub fn owns(&self, event: &VoiceEvent) -> bool {
        self.control.owns(event)
    }

    /// Voice on or off for this board.
    pub fn toggle(&mut self, on: bool, grammar: Vec<String>, active: bool, cx: &mut App) {
        self.settle("abandoned", None, cx);
        self.choices.clear();
        self.control.toggle(on, grammar, active, cx);
    }

    /// The board changed (a move, the turn, the position shown): a choice waiting is dropped,
    /// the phrases said before learn which move followed (`line`, the moves that lead to the
    /// position now), and the recognizer starts a fresh phrase.
    pub fn sync(
        &mut self,
        context: &str,
        line: Option<Vec<String>>,
        grammar: Vec<String>,
        active: bool,
        cx: &mut App,
    ) {
        if context != self.context {
            self.context = context.to_string();
            if !self.choices.is_empty() {
                self.settle("abandoned", None, cx);
                self.choices.clear();
                self.control.feedback = None;
            }
            self.learn(line.as_deref(), cx);
        }
        self.line = line;
        self.control.sync(grammar, active, cx);
    }

    /// Voice is on here: switched on, and still this board's (leaving the page turns it off).
    pub fn is_on(&self, cx: &App) -> bool {
        self.control.is_on(cx)
    }

    /// Record a phrase in the voice history, when Settings keep one (`logVoice`). `track`: the
    /// phrase was about this position's move, so it learns which move was then played.
    fn record(
        &mut self,
        heard: &Heard,
        outcome: &str,
        parsed: Option<&str>,
        fen: &str,
        track: bool,
        cx: &mut App,
    ) -> Option<Entry> {
        let entry = super::log(
            &super::Attempt {
                source: self.source,
                heard,
                outcome,
                parsed,
                expected: None,
                fen: Some(fen),
            },
            self.last_miss.clone(),
            cx,
        )?;
        if track {
            if self.position_entries.is_empty() {
                self.position_line = self.line.clone().unwrap_or_default();
            }
            self.position_entries.push(entry.clone());
        }
        if outcome == "invalid" || outcome == "unclear" {
            self.last_miss = Some(entry.clone());
        }
        Some(entry)
    }

    /// Close the choice waiting with how it ended.
    fn settle(&mut self, outcome: &str, expected: Option<&str>, cx: &App) {
        let Some(attempt) = self.pending.take() else {
            return;
        };
        if !matches!(outcome, "played" | "confirmed" | "picked") {
            self.last_miss = Some(attempt.clone());
        }
        let mut update = json!({ "outcome": outcome });
        if let Some(expected) = expected {
            update["expected"] = json!(expected);
        }
        super::amend(attempt, update, cx);
    }

    /// The position moved on: its phrases learn which move was played there, when the line
    /// grew by one move from where they were said.
    fn learn(&mut self, line: Option<&[String]>, cx: &App) {
        let entries = std::mem::take(&mut self.position_entries);
        let before = std::mem::take(&mut self.position_line);
        self.last_miss = None;
        let Some(san) = line.and_then(|line| meant(&before, line)) else {
            return;
        };
        for entry in entries {
            super::amend(entry, json!({ "expected": san }), cx);
        }
    }

    /// Heard but too unsure (or only noise) to act on; `taking` as for `hear`.
    pub fn missed(&mut self, heard: &Heard, fen: &str, taking: bool, cx: &mut App) {
        if self.control.enabled {
            self.record(heard, "unclear", None, fen, taking, cx);
        }
    }

    /// A phrase on this board: the page's command, a choice's number, confirm or cancel, or a
    /// move.
    pub fn hear(
        &mut self,
        heard: &Heard,
        setup: &Value,
        turn: Turn,
        command: impl FnOnce(&str) -> Option<Reply>,
        cx: &mut App,
    ) -> Action {
        let Turn {
            taking,
            idle,
            immediate,
        } = turn;
        if !self.control.enabled {
            return Action::Nothing;
        }
        let fen = setup["fen"].as_str().unwrap_or_default().to_string();
        self.control.heard = Some(heard.text.clone().into());
        if let Some(reply) = command(&heard.text) {
            let outcome = if reply.understood {
                "command"
            } else {
                "invalid"
            };
            self.record(heard, outcome, Some(&reply.said), &fen, false, cx);
            // Drop a pending move now, so that the command's own answer stays on screen.
            if !self.choices.is_empty() {
                self.settle("abandoned", None, cx);
            }
            self.choices.clear();
            self.control.feedback = Some(reply.feedback.into());
            return Action::Nothing;
        }
        if !taking {
            self.record(heard, "invalid", Some("not your move"), &fen, false, cx);
            self.control.feedback = idle.map(|idle| SharedString::from(idle.to_string()));
            return Action::Nothing;
        }
        let number = crate::game::rules("spokenChoice", &[json!(heard.text)])
            .ok()
            .and_then(|n| n.as_f64());
        if let Some(number) = number
            && !self.choices.is_empty()
            && !heard.unclear
        {
            self.record(
                heard,
                "command",
                Some(&format!("#{number}")),
                &fen,
                true,
                cx,
            );
            return match self
                .choices
                .get((number as usize).saturating_sub(1))
                .cloned()
            {
                Some(choice) => self.choose(choice, "confirmed", cx),
                None => {
                    self.control.feedback = Some("Choose one of the numbered moves.".into());
                    Action::Nothing
                }
            };
        }
        let parsed = spoken_move(&heard.text, setup);
        match parsed["kind"].as_str() {
            Some("cancel") if !heard.unclear => {
                self.record(heard, "command", Some("cancel"), &fen, true, cx);
                self.cancel(cx);
                Action::Nothing
            }
            Some("confirm") if !heard.unclear => {
                self.record(heard, "command", Some("confirm"), &fen, true, cx);
                match self.choices.as_slice() {
                    [only] => {
                        let only = only.clone();
                        self.choose(only, "confirmed", cx)
                    }
                    _ => {
                        self.control.feedback = Some("Say a move, or choose its number.".into());
                        Action::Nothing
                    }
                }
            }
            Some("move") => {
                let offered: Vec<Choice> = parsed["choices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|choice| {
                        Some(Choice {
                            uci: choice["uci"].as_str()?.to_string(),
                            san: choice["san"].as_str()?.to_string(),
                        })
                    })
                    .collect();
                self.offer(heard, offered, immediate, &fen, cx)
            }
            _ => {
                self.record(
                    heard,
                    if heard.unclear { "unclear" } else { "invalid" },
                    None,
                    &fen,
                    true,
                    cx,
                );
                self.control.feedback = Some(
                    if heard.unclear {
                        "Didn’t catch that. Repeat, or try “Echo four” for E4.".to_string()
                    } else {
                        format!("Heard “{}”: no legal move matches.", heard.text)
                    }
                    .into(),
                );
                Action::Nothing
            }
        }
    }

    fn offer(
        &mut self,
        heard: &Heard,
        offered: Vec<Choice>,
        immediate: bool,
        fen: &str,
        cx: &mut App,
    ) -> Action {
        let sans = offered
            .iter()
            .map(|choice| choice.san.as_str())
            .collect::<Vec<_>>()
            .join("|");
        // Saying the move waiting again confirms it.
        if let ([only], [pending]) = (offered.as_slice(), self.choices.as_slice())
            && only.uci == pending.uci
        {
            let pending = pending.clone();
            self.record(
                heard,
                "command",
                Some(&format!("repeat {sans}")),
                fen,
                true,
                cx,
            );
            return self.choose(pending, "confirmed", cx);
        }
        if !self.choices.is_empty() {
            self.settle("replaced", None, cx);
        }
        let confirm = cx
            .global::<crate::settings::SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .voice_confirm_moves;
        if let [only] = offered.as_slice()
            && (immediate || (!confirm && !heard.unclear))
        {
            self.record(heard, "played", Some(&sans), fen, true, cx);
            let only = only.clone();
            return self.choose(only, "played", cx);
        }
        self.pending = self.record(heard, "pending", Some(&sans), fen, true, cx);
        self.control.feedback = Some(
            match offered.as_slice() {
                [only] if heard.unclear => {
                    format!(
                        "Did you mean {}? Say “confirm”, or repeat the move.",
                        only.san
                    )
                }
                [only] => format!("Play {}? Say “confirm” or “cancel”.", only.san),
                _ => "Several moves match: say its number, or select it.".into(),
            }
            .into(),
        );
        self.choices = offered;
        Action::Nothing
    }

    fn choose(&mut self, choice: Choice, outcome: &str, cx: &App) -> Action {
        // Settled before the move: the board's change abandons any choice still waiting.
        self.settle(outcome, Some(&choice.san), cx);
        self.choices.clear();
        self.control.feedback = Some(format!("Played {}.", choice.san).into());
        Action::Play(choice.uci)
    }

    /// The numbered choices, and Cancel; `pick` plays the one clicked.
    pub fn choice_buttons(
        &self,
        id: &'static str,
        pick: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
        cancel: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Vec<AnyElement> {
        let mut buttons: Vec<AnyElement> = self
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let pick = pick.clone();
                let uci = choice.uci.clone();
                // The bar's own buttons take the low numbers.
                Button::new((id, CHOICE_IDS + index))
                    .xsmall()
                    .outline()
                    .label(format!("{} · {}", index + 1, choice.san))
                    .on_click(move |_, window, cx| pick(uci.clone(), window, cx))
                    .into_any_element()
            })
            .collect();
        if !self.choices.is_empty() {
            buttons.push(
                Button::new((id, 99usize))
                    .xsmall()
                    .ghost()
                    .icon(icon("x"))
                    .accessibility_label("Cancel move")
                    .on_click(move |_, window, cx| cancel(window, cx))
                    .into_any_element(),
            );
        }
        buttons
    }

    /// A choice was clicked (`picked`).
    pub fn picked(&mut self, uci: &str, cx: &App) -> Action {
        match self
            .choices
            .iter()
            .find(|choice| choice.uci == uci)
            .cloned()
        {
            Some(choice) => self.choose(choice, "picked", cx),
            None => Action::Nothing,
        }
    }

    pub fn cancel(&mut self, cx: &App) {
        self.settle("cancelled", None, cx);
        self.choices.clear();
        self.control.feedback = Some("Move cancelled.".into());
    }
}

/// What a board page passes to its voice bar.
pub struct BoardBar {
    pub id: &'static str,
    pub hint: &'static str,
    /// The page takes moves now.
    pub active: bool,
    /// The recognizer listens now (moves, or the page's commands).
    pub listening: bool,
    pub grammar: Vec<String>,
}

/// How a page reaches its voice state to change it.
pub struct Access<V> {
    pub write: fn(&mut V) -> &mut VoiceMoves,
    /// Plays a move the player chose.
    pub play: fn(&mut V, &str, &mut Window, &mut Context<V>),
}

impl<V> Clone for Access<V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for Access<V> {}

/// A board page's voice bar: its toggle, the numbered choices and Cancel.
/// `state` is the page's voice state, read here as the page renders.
pub fn board_bar<V: 'static>(
    state: &VoiceMoves,
    page: WeakEntity<V>,
    access: Access<V>,
    bar: BoardBar,
    cx: &App,
) -> AnyElement {
    let (owner, on) = (state.control.owner, state.is_on(cx));
    let (feedback, heard) = (state.control.feedback.clone(), state.control.heard.clone());
    let (pick, cancel, toggle) = (page.clone(), page.clone(), page);
    let extra = state.choice_buttons(
        bar.id,
        move |uci, window, cx| {
            let _ = pick.update(cx, |this, cx| {
                if let Action::Play(uci) = (access.write)(this).picked(&uci, cx) {
                    (access.play)(this, &uci, window, cx);
                }
                cx.notify();
            });
        },
        move |_, cx| {
            let _ = cancel.update(cx, |this, cx| {
                (access.write)(this).cancel(cx);
                cx.notify();
            });
        },
    );
    let (grammar, listening) = (bar.grammar, bar.listening);
    super::voice_bar(
        super::bar::Bar {
            id: bar.id,
            owner,
            enabled: on,
            active: bar.active,
            hint: bar.hint.into(),
            feedback,
            heard,
            extra,
        },
        move |on, _, cx| {
            let grammar = grammar.clone();
            let _ = toggle.update(cx, |this, cx| {
                (access.write)(this).toggle(on, grammar, listening, cx);
                cx.notify();
            });
        },
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::{meant, spoken_move};
    use serde_json::json;

    #[test]
    fn phrases_learn_the_move_played_next_and_nothing_else() {
        let line = |sans: &[&str]| sans.iter().map(|san| san.to_string()).collect::<Vec<_>>();
        assert_eq!(meant(&line(&["e4"]), &line(&["e4", "e5"])), Some("e5"));
        assert_eq!(meant(&[], &line(&["d4"])), Some("d4"));
        // A takeback, a jump through the moves, or another line teaches nothing.
        assert_eq!(meant(&line(&["e4", "e5"]), &line(&["e4"])), None);
        assert_eq!(meant(&line(&["e4"]), &line(&["e4", "e5", "Nf3"])), None);
        assert_eq!(meant(&line(&["e4"]), &line(&["d4", "d5"])), None);
    }

    #[test]
    fn spoken_words_become_legal_moves() {
        let start = json!({
            "variant": "standard",
            "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        });
        let e4 = spoken_move("e four", &start);
        assert_eq!(e4["kind"], "move");
        assert_eq!(e4["choices"][0]["uci"], "e2e4");
        assert_eq!(e4["choices"][0]["san"], "e4");
        assert_eq!(
            spoken_move("knight f three", &start)["choices"][0]["uci"],
            "g1f3"
        );
        assert_eq!(spoken_move("confirm", &start)["kind"], "confirm");
    }
}

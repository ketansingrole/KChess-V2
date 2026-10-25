//! Spoken moves on a board (`VoiceMoveInput.vue`): a clear move is played (unless Settings ask to
//! confirm moves), an unsure or ambiguous one waits for “confirm”, its number, or the move said
//! again. A page's own commands (“take back”, “back”) are tried first. Each phrase goes to the
//! voice history with what came of it.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use serde_json::{Value, json};

use super::{Heard, VoiceEvent};
use crate::assets::icon;
use crate::core::AppCore;

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

/// A board's voice input: on or off, the choice waiting, and what was last said.
pub struct VoiceMoves {
    pub source: &'static str,
    pub owner: u64,
    pub enabled: bool,
    pub choices: Vec<Choice>,
    pub feedback: Option<SharedString>,
    pub heard: Option<SharedString>,
    /// The position the choices belong to.
    context: String,
}

/// Every legal move the words could be (`spokenMove`): `move` with its choices, `confirm`,
/// `cancel`, or nothing.
pub fn spoken_move(text: &str, setup: &Value) -> Value {
    crate::game::rules("spokenMove", &[json!(text), setup.clone()]).unwrap_or(Value::Null)
}

impl VoiceMoves {
    pub fn new(source: &'static str, cx: &mut App) -> VoiceMoves {
        let owner = super::handle(cx).update(cx, |voice, _| voice.new_owner());
        VoiceMoves {
            source,
            owner,
            enabled: false,
            choices: Vec::new(),
            feedback: None,
            heard: None,
            context: String::new(),
        }
    }

    /// Whether `event` is this board's.
    pub fn owns(&self, event: &VoiceEvent) -> bool {
        match event {
            VoiceEvent::Heard { owner, .. } | VoiceEvent::Missed { owner, .. } => {
                *owner == self.owner
            }
        }
    }

    fn push_to_talk(cx: &App) -> bool {
        cx.global::<crate::settings::SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .voice_push_to_talk
    }

    /// Voice on or off for this board.
    pub fn toggle(&mut self, on: bool, grammar: Vec<String>, active: bool, cx: &mut App) {
        self.enabled = on;
        self.choices.clear();
        self.feedback = None;
        self.heard = None;
        let (owner, push) = (self.owner, Self::push_to_talk(cx));
        super::handle(cx).update(cx, |voice, cx| {
            if on {
                voice.enable(owner, grammar, active, push, cx)
            } else {
                voice.disable(owner, cx)
            }
        });
    }

    /// The board changed (a move, the turn, the position shown): a choice waiting is dropped,
    /// and the recognizer starts a fresh phrase.
    pub fn sync(&mut self, context: &str, grammar: Vec<String>, active: bool, cx: &mut App) {
        if context != self.context {
            self.context = context.to_string();
            if !self.choices.is_empty() {
                self.choices.clear();
                self.feedback = None;
            }
        }
        if !self.enabled {
            return;
        }
        let (owner, push) = (self.owner, Self::push_to_talk(cx));
        super::handle(cx).update(cx, |voice, cx| {
            voice.update(owner, grammar, active, push, cx)
        });
    }

    /// Voice is on here: switched on, and still this board's (leaving the page turns it off).
    pub fn is_on(&self, cx: &App) -> bool {
        self.enabled && super::handle(cx).read(cx).owner() == Some(self.owner)
    }

    /// Record a phrase in the voice history, when Settings keep one (`logVoice`).
    fn record(&self, heard: &Heard, outcome: &str, parsed: Option<&str>, fen: &str, cx: &mut App) {
        let keep = cx
            .global::<crate::settings::SettingsHandle>()
            .0
            .read(cx)
            .settings
            .core
            .voice_history;
        if !keep {
            return;
        }
        let mut attempt = json!({
            "source": self.source,
            "heard": heard.raw.chars().take(200).collect::<String>(),
            "confidence": (heard.confidence.clamp(0., 1.) * 1000.).round() / 1000.,
            "words": heard.words.iter().take(40).map(|word| json!({
                "word": word.word.chars().take(40).collect::<String>(),
                "conf": word.conf.clamp(0., 1.),
            })).collect::<Vec<_>>(),
            "outcome": outcome,
        });
        if let Some(parsed) = parsed {
            attempt["parsed"] = json!(parsed.chars().take(120).collect::<String>());
        }
        if !fen.is_empty() {
            attempt["fen"] = json!(fen);
        }
        let save = cx
            .global::<AppCore>()
            .call("saveVoiceAttempt", vec![attempt]);
        cx.background_executor()
            .spawn(async move {
                // The history is a study aid; a lost entry never disturbs play.
                if let Err(cause) = save.await {
                    crate::log::write(
                        kchess_core::Level::Warn,
                        "voice-log",
                        &format!("saving a voice attempt failed: {}", cause.message),
                    );
                }
            })
            .detach();
    }

    /// Heard but too unsure (or only noise) to act on.
    pub fn missed(&mut self, heard: &Heard, fen: &str, cx: &mut App) {
        if self.enabled {
            self.record(heard, "unclear", None, fen, cx);
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
        if !self.enabled {
            return Action::Nothing;
        }
        let fen = setup["fen"].as_str().unwrap_or_default().to_string();
        self.heard = Some(heard.text.clone().into());
        if let Some(reply) = command(&heard.text) {
            let outcome = if reply.understood {
                "command"
            } else {
                "invalid"
            };
            self.record(heard, outcome, Some(&reply.said), &fen, cx);
            self.choices.clear();
            self.feedback = Some(reply.feedback.into());
            return Action::Nothing;
        }
        if !taking {
            self.record(heard, "invalid", Some("not your move"), &fen, cx);
            self.feedback = idle.map(|idle| SharedString::from(idle.to_string()));
            return Action::Nothing;
        }
        let number = crate::game::rules("spokenChoice", &[json!(heard.text)])
            .ok()
            .and_then(|n| n.as_f64());
        if let Some(number) = number
            && !self.choices.is_empty()
            && !heard.unclear
        {
            self.record(heard, "command", Some(&format!("#{number}")), &fen, cx);
            return match self
                .choices
                .get((number as usize).saturating_sub(1))
                .cloned()
            {
                Some(choice) => self.choose(choice),
                None => {
                    self.feedback = Some("Choose one of the numbered moves.".into());
                    Action::Nothing
                }
            };
        }
        let parsed = spoken_move(&heard.text, setup);
        match parsed["kind"].as_str() {
            Some("cancel") if !heard.unclear => {
                self.record(heard, "command", Some("cancel"), &fen, cx);
                self.choices.clear();
                self.feedback = Some("Move cancelled.".into());
                Action::Nothing
            }
            Some("confirm") if !heard.unclear => {
                self.record(heard, "command", Some("confirm"), &fen, cx);
                match self.choices.as_slice() {
                    [only] => {
                        let only = only.clone();
                        self.choose(only)
                    }
                    _ => {
                        self.feedback = Some("Say a move, or choose its number.".into());
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
                    cx,
                );
                self.feedback = Some(
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
            self.record(heard, "command", Some(&format!("repeat {sans}")), fen, cx);
            let pending = pending.clone();
            return self.choose(pending);
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
            self.record(heard, "played", Some(&sans), fen, cx);
            let only = only.clone();
            return self.choose(only);
        }
        self.record(heard, "pending", Some(&sans), fen, cx);
        self.feedback = Some(
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

    fn choose(&mut self, choice: Choice) -> Action {
        self.choices.clear();
        self.feedback = Some(format!("Played {}.", choice.san).into());
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
                Button::new((id, index))
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
    pub fn picked(&mut self, uci: &str) -> Action {
        match self
            .choices
            .iter()
            .find(|choice| choice.uci == uci)
            .cloned()
        {
            Some(choice) => self.choose(choice),
            None => Action::Nothing,
        }
    }

    pub fn cancel(&mut self) {
        self.choices.clear();
        self.feedback = Some("Move cancelled.".into());
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

/// How a page reaches its voice state, to read and to change it.
pub struct Access<V> {
    pub read: fn(&V) -> &VoiceMoves,
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

/// A board page's voice bar: its toggle, the numbered choices and Cancel (`VoiceMoveInput.vue`'s
/// template).
pub fn board_bar<V: 'static>(
    page: &Entity<V>,
    access: Access<V>,
    bar: BoardBar,
    cx: &App,
) -> AnyElement {
    let state = (access.read)(page.read(cx));
    let (owner, on) = (state.owner, state.is_on(cx));
    let (feedback, heard) = (state.feedback.clone(), state.heard.clone());
    let weak = page.downgrade();
    let (pick, cancel, toggle) = (weak.clone(), weak.clone(), weak);
    let extra = state.choice_buttons(
        bar.id,
        move |uci, window, cx| {
            let _ = pick.update(cx, |this, cx| {
                if let Action::Play(uci) = (access.write)(this).picked(&uci) {
                    (access.play)(this, &uci, window, cx);
                }
                cx.notify();
            });
        },
        move |_, cx| {
            let _ = cancel.update(cx, |this, cx| {
                (access.write)(this).cancel();
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
    use super::spoken_move;
    use serde_json::json;

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

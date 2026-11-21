//! Voice on the page: when it listens, what it listens for, and what a heard move or command
//! does.

use super::*;

impl ComputerPage {
    /// Spoken moves are taken on your turn; commands whenever Stockfish is there to play.
    pub(super) fn voice_taking(&self) -> bool {
        self.interactive() && !self.thinking() && self.asked.is_none()
    }

    pub(super) fn voice_listening(&self) -> bool {
        self.voice_taking() || self.engine == Engine::Ready
    }

    pub(super) fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let context = format!(
            "{}:{}:{}",
            self.user_color() == Color::White,
            self.moves().join(" "),
            self.asked.is_some()
        );
        let listening = self.voice_listening();
        self.voice.sync(
            &context,
            Some(self.sans()),
            kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Game).to_vec(),
            listening,
            cx,
        );
    }

    /// What voice input says while no move is taken.
    fn voice_idle(&self) -> &'static str {
        if self.game.result().is_some() {
            "The game is over. Say “new game” to play again."
        } else if self.ply() != self.moves().len() {
            "Go to the last move to keep playing"
        } else {
            "Wait for Stockfish, then say your move"
        }
    }

    /// `text` said clearly, with voice on: as the recognizer's phrase reaches the page.
    #[cfg(test)]
    pub(crate) fn hear_for_test(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.voice.control.enabled = true;
        let heard = crate::voice::Heard {
            text: text.into(),
            raw: text.into(),
            confidence: 1.,
            words: Vec::new(),
            unclear: false,
        };
        let owner = self.voice.control.owner;
        self.voice_heard(
            &crate::voice::VoiceEvent::Heard { owner, heard },
            window,
            cx,
        );
    }

    pub(super) fn voice_heard(
        &mut self,
        event: &crate::voice::VoiceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.voice.owns(event) {
            return;
        }
        let setup = self
            .position_at(self.moves().len())
            .map(|position| Setup {
                variant: self.game.setup().variant,
                fen: kchess_domain::rules::make_fen(&position),
            })
            .unwrap_or_else(|| Setup::standard_at(""));
        let fen = setup.fen.clone();
        let heard = match event {
            crate::voice::VoiceEvent::Missed { heard, .. } => {
                self.voice.missed(heard, &fen, self.voice_taking(), cx);
                return;
            }
            crate::voice::VoiceEvent::Heard { heard, .. } => heard.clone(),
        };
        let taking = self.voice_taking();
        let idle = self.voice_idle();
        let asked = self.asked;
        let mut run: Option<Deferred> = None;
        let action = {
            let command = |text: &str| -> Option<crate::voice::moves::Reply> {
                let reply = |said: &str, feedback: &str| crate::voice::moves::Reply {
                    said: said.into(),
                    feedback: feedback.into(),
                    understood: true,
                };
                if let Some(question) = asked {
                    let answer = crate::voice::moves::spoken_move(text, &setup);
                    return Some(match answer {
                        kchess_domain::voice::SpokenMove::Confirm => {
                            run = Some(Box::new(move |this, window, cx| {
                                this.asked = None;
                                window.close_dialog(cx);
                                this.answer(question, window, cx);
                            }));
                            reply("confirm", "Done")
                        }
                        kchess_domain::voice::SpokenMove::Cancel => {
                            run = Some(Box::new(|this, window, cx| {
                                this.asked = None;
                                window.close_dialog(cx);
                                cx.notify();
                            }));
                            reply("cancel", "Cancelled")
                        }
                        _ => crate::voice::moves::Reply {
                            said: "expected confirm or cancel".into(),
                            feedback: "Say “confirm” or “cancel”".into(),
                            understood: false,
                        },
                    });
                }
                use kchess_domain::voice::GameCommand;
                let (said, question, feedback) = match kchess_domain::voice::game_command(text)? {
                    GameCommand::Takeback => {
                        run = Some(Box::new(|this, _, cx| {
                            if this.can_takeback() {
                                this.takeback(cx);
                            }
                        }));
                        ("take back", None, "Took back your last move")
                    }
                    GameCommand::NewGame => (
                        "new game",
                        Some(Question::NewGame),
                        "Start a new game? Say “confirm” or “cancel”.",
                    ),
                    GameCommand::Resign => (
                        "resign",
                        Some(Question::Resign),
                        "Resign this game? Say “confirm” or “cancel”.",
                    ),
                    GameCommand::Flip => (
                        "switch sides",
                        Some(Question::SwitchSides),
                        "Switch sides and restart? Say “confirm” or “cancel”.",
                    ),
                };
                if let Some(question) = question {
                    run = Some(Box::new(move |this, window, cx| {
                        this.ask(question, window, cx)
                    }));
                }
                Some(reply(said, feedback))
            };
            self.voice.hear(
                &heard,
                &setup,
                crate::voice::moves::Turn {
                    taking,
                    idle: Some(idle),
                    immediate: false,
                },
                command,
                cx,
            )
        };
        if let Some(run) = run {
            run(self, window, cx);
        }
        if let crate::voice::moves::Action::Play(uci) = action {
            self.play(&uci, cx);
        }
        cx.notify();
    }
}

//! A page's voice switch: whether voice is on for it, what
//! it answered to the last phrase, and its bar. Boards build spoken moves on it (`moves`); the
//! board editor and the drills use it as it is.

use gpui_kit::*;

use super::{State, VoiceEvent};

pub struct Control {
    pub owner: u64,
    pub enabled: bool,
    pub feedback: Option<SharedString>,
    pub heard: Option<SharedString>,
    /// Push to talk applies here when Settings ask for it.
    push: bool,
}

/// What a page passes to its bar.
pub struct BarSpec {
    pub id: &'static str,
    pub hint: &'static str,
    /// The page takes phrases now.
    pub active: bool,
    pub grammar: Vec<String>,
}

impl Control {
    pub fn new(push: bool, cx: &mut App) -> Control {
        let owner = super::handle(cx).update(cx, |voice, _| voice.new_owner());
        Control {
            owner,
            enabled: false,
            feedback: None,
            heard: None,
            push,
        }
    }

    /// `text` said clearly to this page with voice on, as the recognizer's phrase arrives.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hear_clearly(&mut self, text: &str) -> VoiceEvent {
        self.enabled = true;
        VoiceEvent::Heard {
            owner: self.owner,
            heard: super::Heard::clear(text),
        }
    }

    /// Whether `event` is this page's.
    pub fn owns(&self, event: &VoiceEvent) -> bool {
        match event {
            VoiceEvent::Heard { owner, .. } | VoiceEvent::Missed { owner, .. } => {
                *owner == self.owner
            }
        }
    }

    fn push_to_talk(&self, cx: &App) -> bool {
        self.push && crate::settings::frontend(cx).voice_push_to_talk
    }

    /// Voice on or off for this page.
    pub fn toggle(&mut self, on: bool, grammar: Vec<String>, active: bool, cx: &mut App) {
        self.enabled = on;
        self.feedback = None;
        self.heard = None;
        let (owner, push) = (self.owner, self.push_to_talk(cx));
        super::handle(cx).update(cx, |voice, cx| {
            if on {
                voice.enable(owner, grammar, active, push, cx)
            } else {
                voice.disable(owner, cx)
            }
        });
    }

    /// The page changed what it takes: the recognizer starts a fresh phrase.
    pub fn sync(&mut self, grammar: Vec<String>, active: bool, cx: &mut App) {
        if !self.enabled {
            return;
        }
        let (owner, push) = (self.owner, self.push_to_talk(cx));
        super::handle(cx).update(cx, |voice, cx| {
            voice.update(owner, grammar, active, push, cx)
        });
    }

    /// Voice is on here: switched on, and still this page's (leaving the page turns it off).
    pub fn is_on(&self, cx: &App) -> bool {
        self.enabled && super::handle(cx).read(cx).owner() == Some(self.owner)
    }

    /// Whether voice can take a phrase as soon as the page asks: `Some(true)` once
    /// the model and microphone are ready, `Some(false)` while they load, `None` when voice is
    /// off or failed.
    pub fn ready(&self, cx: &App) -> Option<bool> {
        if !self.is_on(cx) {
            return None;
        }
        match super::handle(cx).read(cx).state {
            State::Paused | State::Listening => Some(true),
            State::Loading => Some(false),
            State::Off | State::Error => None,
        }
    }

    /// A run asked to start waits for voice: true when it can start now. `waiting`
    /// stays set while voice loads, so the page starts the run once it is ready, and clears
    /// when voice is off or failed.
    pub fn gate(&self, waiting: &mut bool, cx: &App) -> bool {
        let ready = self.ready(cx);
        *waiting = ready == Some(false);
        ready == Some(true)
    }

    /// The page's bar; `control` reaches this control in the page to switch it.
    pub fn bar<V: 'static>(
        &self,
        page: WeakEntity<V>,
        control: fn(&mut V) -> &mut Control,
        spec: BarSpec,
        cx: &App,
    ) -> AnyElement {
        let (bar, grammar) = self.spec(spec, true, cx);
        Self::draw(bar, grammar, page, control, cx)
    }

    /// The bar at the end of the move box's row under the board, as on the game pages, and its
    /// line on a row of its own below, while it has something to say.
    pub fn board_bar<V: 'static>(
        &self,
        page: WeakEntity<V>,
        control: fn(&mut V) -> &mut Control,
        spec: BarSpec,
        cx: &App,
    ) -> (AnyElement, Option<AnyElement>) {
        let (bar, grammar) = self.spec(spec, false, cx);
        let line = super::bar::voice_line(&bar, cx);
        (Self::draw(bar, grammar, page, control, cx), line)
    }

    /// The bar `spec` asks for: its line in the bar (`inline`), or under the board with the
    /// button at the row's end.
    fn spec(&self, spec: BarSpec, inline: bool, cx: &App) -> (super::bar::Bar, Vec<String>) {
        let BarSpec {
            id,
            hint,
            active,
            grammar,
        } = spec;
        let bar = super::bar::Bar {
            id,
            owner: self.owner,
            enabled: self.is_on(cx),
            active,
            hint: hint.into(),
            feedback: self.feedback.clone(),
            heard: self.heard.clone(),
            extra: Vec::new(),
            inline,
            end: !inline,
        };
        (bar, grammar)
    }

    fn draw<V: 'static>(
        bar: super::bar::Bar,
        grammar: Vec<String>,
        page: WeakEntity<V>,
        control: fn(&mut V) -> &mut Control,
        cx: &App,
    ) -> AnyElement {
        let active = bar.active;
        super::voice_bar(
            bar,
            move |on, _, cx| {
                let grammar = grammar.clone();
                page.update(cx, |this, cx| {
                    control(this).toggle(on, grammar, active, cx);
                    cx.notify();
                })
                .ok();
            },
            cx,
        )
    }
}

/// A drill's phrases: the question asked is known, so each says what was meant,
/// and a phrase that tries the same question again after a miss points back to it.
#[derive(Default)]
pub struct DrillLog {
    last_miss: Option<(String, super::Entry)>,
}

impl DrillLog {
    /// `asked` names the question (the run and its number).
    pub fn log(&mut self, attempt: &super::Attempt, asked: &str, cx: &App) {
        let retry = self
            .last_miss
            .as_ref()
            .filter(|(question, _)| question == asked)
            .map(|(_, entry)| entry.clone());
        let entry = super::log(attempt, retry, cx);
        if !matches!(
            attempt.outcome,
            super::VoiceOutcome::Correct | super::VoiceOutcome::Played
        ) && let Some(entry) = entry
        {
            self.last_miss = Some((asked.to_string(), entry));
        }
    }
}

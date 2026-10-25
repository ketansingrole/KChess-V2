//! A page's voice switch (`VoiceInput.vue`'s props and model): whether voice is on for it, what
//! it answered to the last phrase, and its bar. Boards build spoken moves on it (`moves`); the
//! board editor and the drills use it as it is.

use gpui_kit::*;

use super::{State, VoiceEvent};

pub struct Control {
    pub owner: u64,
    pub enabled: bool,
    pub feedback: Option<SharedString>,
    pub heard: Option<SharedString>,
    /// Push to talk applies here when Settings ask for it (`allowPushTalk`).
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

    /// Whether `event` is this page's.
    pub fn owns(&self, event: &VoiceEvent) -> bool {
        match event {
            VoiceEvent::Heard { owner, .. } | VoiceEvent::Missed { owner, .. } => {
                *owner == self.owner
            }
        }
    }

    fn push_to_talk(&self, cx: &App) -> bool {
        self.push
            && cx
                .global::<crate::settings::SettingsHandle>()
                .0
                .read(cx)
                .settings
                .frontend
                .voice_push_to_talk
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

    /// Whether voice can take a phrase as soon as the page asks (`prepare`): `Some(true)` once
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

    /// The page's bar; `control` reaches this control in the page to switch it.
    pub fn bar<V: 'static>(
        &self,
        page: WeakEntity<V>,
        control: fn(&mut V) -> &mut Control,
        spec: BarSpec,
        cx: &App,
    ) -> AnyElement {
        let BarSpec {
            id,
            hint,
            active,
            grammar,
        } = spec;
        super::voice_bar(
            super::bar::Bar {
                id,
                owner: self.owner,
                enabled: self.is_on(cx),
                active,
                hint: hint.into(),
                feedback: self.feedback.clone(),
                heard: self.heard.clone(),
                extra: Vec::new(),
            },
            move |on, _, cx| {
                let grammar = grammar.clone();
                let _ = page.update(cx, |this, cx| {
                    control(this).toggle(on, grammar, active, cx);
                    cx.notify();
                });
            },
            cx,
        )
    }
}

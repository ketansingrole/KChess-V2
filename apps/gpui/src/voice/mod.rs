//! Voice input (`components/VoiceInput.vue`, `utils/voiceCapture.ts`): one recognizer for the
//! whole app, owned by the control that turned voice on last. It downloads the offline model the
//! first time, opens the microphone, sends its audio to the core's recognizer (`voiceStart`,
//! `voiceAccept`, `voiceFinish`, `voiceStop`) in order, and hands each phrase heard to its owner.

use std::collections::VecDeque;
use std::sync::Arc;

use futures::future::Shared;
use futures::{FutureExt as _, StreamExt as _};
use gpui_kit::*;
use kchess_core::voice::{self as model, VoiceModelCache};
use serde_json::{Value, json};

use crate::core::AppCore;

pub mod bar;
pub mod capture;
pub mod control;
pub mod moves;

pub use bar::voice_bar;

/// What voice input is doing (`VoiceState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Loading,
    Paused,
    Listening,
    Error,
}

/// Why it stopped, so the bar can offer the right fix (`VoiceErrorKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Denied,
    Missing,
    Model,
    Other,
}

/// One word heard, and how sure the recognizer was of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub word: String,
    pub conf: f64,
}

/// A finished phrase (`VoiceResult`).
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    /// The words, noise markers (`[unk]`) left out.
    pub text: String,
    /// The recognizer's own text.
    pub raw: String,
    /// The real words' average confidence, 0 to 1.
    pub confidence: f64,
    pub words: Vec<Word>,
    /// Below the confidence normally needed.
    pub unclear: bool,
}

/// A phrase for the owner: used, or only heard (too unsure, or nothing but noise).
#[derive(Clone, Debug)]
pub enum VoiceEvent {
    Heard { owner: u64, heard: Heard },
    Missed { owner: u64, heard: Heard },
}

impl EventEmitter<VoiceEvent> for Voice {}

/// Requests to the core's recognizer, sent one at a time in order.
enum Request {
    Start {
        grammar: Vec<String>,
        rate: f64,
        epoch: u64,
    },
    Accept {
        audio: String,
        epoch: u64,
    },
    Finish {
        epoch: u64,
    },
    Stop,
}

pub struct Voice {
    owner: Option<u64>,
    next_owner: u64,
    pub state: State,
    pub error: Option<(String, ErrorKind)>,
    pub partial: String,
    pub level: f32,
    /// What preparing says: downloading the model and how far, loading it.
    pub preparing: String,
    grammar: Vec<String>,
    active: bool,
    push_to_talk: bool,
    held: bool,
    /// Each utterance; answers to an earlier one are dropped.
    epoch: u64,
    ready: bool,
    capture: Option<capture::Capture>,
    rate: f64,
    requests: Option<futures::channel::mpsc::UnboundedSender<Request>>,
    _pump: Option<Task<()>>,
    _worker: Option<Task<()>>,
}

#[derive(Clone)]
pub struct VoiceHandle(pub Entity<Voice>);

impl Global for VoiceHandle {}

pub fn handle(cx: &App) -> Entity<Voice> {
    cx.global::<VoiceHandle>().0.clone()
}

/// Something only the app needs to hear from the voice model's download.
struct QuietHost;

impl kchess_core::Host for QuietHost {
    fn log(&self, level: kchess_core::Level, scope: &str, message: &str) {
        crate::log::write(level, scope, message);
    }

    fn emit(&self, _: &str, _: Value) {}
}

/// The profile's voice model (`VoiceModelCache` in the core).
fn model_cache() -> &'static VoiceModelCache {
    static CACHE: std::sync::OnceLock<VoiceModelCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let dir = crate::profile::data_dir().unwrap_or_default().join("voice");
        let (archive, official) = model::active_model();
        VoiceModelCache::new(Arc::new(QuietHost), dir, archive, official)
    })
}

/// Whether the offline model is installed, or being prepared (`voiceModelStatus`).
pub fn model_status(cx: &App) -> impl Future<Output = Option<model::Status>> + 'static + use<> {
    let bridge = Arc::clone(&cx.global::<AppCore>().bridge);
    bridge.spawn(async move { model_cache().status().await })
}

/// Download and prepare the model ahead of use (`ensureVoiceModel`); `progress` hears how far.
pub fn download_model(
    progress: model::Listener,
    cx: &App,
) -> impl Future<Output = Option<Result<(), String>>> + 'static + use<> {
    let bridge = Arc::clone(&cx.global::<AppCore>().bridge);
    bridge.spawn(async move {
        model_cache()
            .ensure(Some(progress))
            .await
            .map(|_| ())
            .map_err(|cause| cause.message)
    })
}

/// The real words' average confidence: noise markers would sink a clearly heard command, and a
/// homophone in the grammar splits a word's confidence even when it was heard perfectly.
pub fn heard_of(result: &Value) -> Option<Heard> {
    let raw = result["text"].as_str()?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let words: Vec<Word> = result["words"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|word| {
            let text = word["word"].as_str()?;
            (text != "[unk]").then(|| Word {
                word: text.to_string(),
                conf: (word["conf"].as_f64().unwrap_or_default() * 1000.).round() / 1000.,
            })
        })
        .collect();
    let confidence = if words.is_empty() {
        0.
    } else {
        words.iter().map(|word| word.conf).sum::<f64>() / words.len() as f64
    };
    let text = raw
        .split_whitespace()
        .filter(|word| *word != "[unk]")
        .collect::<Vec<_>>()
        .join(" ");
    let unclear = text.is_empty() || confidence < 0.6;
    Some(Heard {
        text,
        raw,
        confidence,
        words,
        unclear,
    })
}

impl Voice {
    /// A new control's claim on voice input.
    pub fn new_owner(&mut self) -> u64 {
        self.next_owner += 1;
        self.next_owner
    }

    pub fn owner(&self) -> Option<u64> {
        self.owner
    }

    /// Voice is on for `owner`: listening when `active`, for `grammar`'s phrases.
    pub fn enable(
        &mut self,
        owner: u64,
        grammar: Vec<String>,
        active: bool,
        push_to_talk: bool,
        cx: &mut Context<Self>,
    ) {
        if self.owner != Some(owner) {
            self.disable_now(cx);
        }
        self.owner = Some(owner);
        self.grammar = grammar;
        self.active = active;
        self.push_to_talk = push_to_talk;
        self.held = false;
        self.error = None;
        self.partial.clear();
        if self.capture.is_some() {
            self.reset(cx);
        } else {
            self.open(cx);
        }
        cx.notify();
    }

    /// The owner's position or turn changed: a fresh utterance with its grammar.
    pub fn update(
        &mut self,
        owner: u64,
        grammar: Vec<String>,
        active: bool,
        push_to_talk: bool,
        cx: &mut Context<Self>,
    ) {
        if self.owner != Some(owner) || self.state == State::Error {
            return;
        }
        let changed =
            grammar != self.grammar || active != self.active || push_to_talk != self.push_to_talk;
        self.grammar = grammar;
        self.active = active;
        self.push_to_talk = push_to_talk;
        self.partial.clear();
        if changed && self.capture.is_some() {
            self.held = false;
            self.reset(cx);
        }
        cx.notify();
    }

    pub fn disable(&mut self, owner: u64, cx: &mut Context<Self>) {
        if self.owner == Some(owner) {
            self.disable_now(cx);
            self.owner = None;
            cx.notify();
        }
    }

    fn disable_now(&mut self, _cx: &mut Context<Self>) {
        self.epoch += 1;
        self.capture = None;
        self._pump = None;
        if let Some(requests) = &self.requests {
            let _ = requests.unbounded_send(Request::Stop);
        }
        self.state = State::Off;
        self.partial.clear();
        self.level = 0.;
        self.held = false;
    }

    /// Another page was opened: voice stops, and the microphone is released (`dispose`).
    pub fn release(&mut self, cx: &mut Context<Self>) {
        if self.owner.is_some() {
            self.disable_now(cx);
            self.owner = None;
            cx.notify();
        }
    }

    /// Push to talk: speaking while held, the phrase finished on release.
    pub fn talk(&mut self, held: bool, cx: &mut Context<Self>) {
        if !self.can_talk() || held == self.held {
            return;
        }
        self.held = held;
        if held {
            self.reset(cx);
        } else {
            if let Some(requests) = &self.requests {
                let _ = requests.unbounded_send(Request::Finish { epoch: self.epoch });
            }
            self.level = 0.;
            self.state = State::Paused;
        }
        cx.notify();
    }

    /// Push to talk is what the owner is waiting for: Space and the hold button speak.
    pub fn can_talk(&self) -> bool {
        self.active && self.push_to_talk && self.capture.is_some()
    }

    fn listening(&self) -> bool {
        self.active && (!self.push_to_talk || self.held)
    }

    /// The model (downloaded the first time), the microphone, then the recognizer loaded.
    fn open(&mut self, cx: &mut Context<Self>) {
        self.state = State::Loading;
        self.preparing = "Preparing voice input…".into();
        cx.notify();
        let (progress, mut progress_updates) =
            futures::channel::mpsc::unbounded::<model::Progress>();
        let listener: model::Listener = Arc::new(move |update: &model::Progress| {
            let _ = progress.unbounded_send(update.clone());
        });
        let bridge = Arc::clone(&cx.global::<AppCore>().bridge);
        let ensure = bridge.spawn(async move { model_cache().ensure(Some(listener)).await });
        let epoch = self.epoch;
        cx.spawn(async move |this, cx| {
            let watch = {
                let this = this.clone();
                let cx = cx.clone();
                async move {
                    while let Some(update) = progress_updates.next().await {
                        let mut cx = cx.clone();
                        let _ = this.update(&mut cx, |this, cx| {
                            this.preparing = match update.phase {
                                model::Phase::Downloading => match update.total {
                                    Some(total) if total > 0 => format!(
                                        "Downloading voice model · {}%…",
                                        (update.received * 100 / total).min(100)
                                    ),
                                    _ => "Downloading voice model · about 40 MB…".into(),
                                },
                                model::Phase::Preparing => "Preparing voice model…".into(),
                                model::Phase::Checking => "Loading voice model…".into(),
                            };
                            cx.notify();
                        });
                    }
                }
            };
            let (ensured, ()) = futures::join!(ensure, watch);
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch || this.owner.is_none() {
                    return;
                }
                match ensured {
                    Some(Ok(_)) => {
                        this.ready = true;
                        this.open_microphone(cx);
                    }
                    Some(Err(cause)) => this.fail(cause.message, ErrorKind::Model, cx),
                    None => this.fail(
                        "The voice model couldn’t download. Connect to the internet and try again."
                            .into(),
                        ErrorKind::Model,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    fn open_microphone(&mut self, cx: &mut Context<Self>) {
        let (capture, mut chunks) = match capture::start() {
            Ok(opened) => opened,
            Err(cause) => {
                let kind = match cause {
                    capture::CaptureError::Denied => ErrorKind::Denied,
                    capture::CaptureError::Missing => ErrorKind::Missing,
                    capture::CaptureError::Busy(_) => ErrorKind::Other,
                };
                return self.fail(cause.message(), kind, cx);
            }
        };
        self.rate = capture.sample_rate;
        self.capture = Some(capture);
        self.ensure_worker(cx);
        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Some(chunk) = chunks.next().await {
                let alive = this.update(cx, |this, cx| this.chunk(chunk, cx)).is_ok();
                if !alive {
                    return;
                }
            }
        }));
        self.reset(cx);
    }

    /// A chunk of audio: heard only while listening, for the utterance under way.
    fn chunk(&mut self, chunk: capture::Chunk, cx: &mut Context<Self>) {
        if !self.listening() || self.grammar.is_empty() {
            return;
        }
        self.level = capture::level(&chunk);
        if let Some(requests) = &self.requests {
            let _ = requests.unbounded_send(Request::Accept {
                audio: capture::pcm_base64(&chunk),
                epoch: self.epoch,
            });
        }
        cx.notify();
    }

    /// A fresh utterance with the current grammar (`resetRecognizer`).
    fn reset(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        if !self.ready || self.capture.is_none() {
            return;
        }
        if self.active
            && !self.grammar.is_empty()
            && let Some(requests) = &self.requests
        {
            let _ = requests.unbounded_send(Request::Start {
                grammar: self.grammar.clone(),
                rate: self.rate,
                epoch: self.epoch,
            });
        }
        self.state = if self.listening() {
            State::Listening
        } else {
            State::Paused
        };
        if !self.listening() {
            self.level = 0.;
        }
        cx.notify();
    }

    /// One task sends the requests in order and brings back what was heard.
    fn ensure_worker(&mut self, cx: &mut Context<Self>) {
        if self.requests.is_some() {
            return;
        }
        let (sender, mut queue) = futures::channel::mpsc::unbounded::<Request>();
        self.requests = Some(sender);
        let core = cx.global::<AppCore>().clone();
        self._worker = Some(cx.spawn(async move |this, cx| {
            let mut pending: VecDeque<Request> = VecDeque::new();
            loop {
                let request = match pending.pop_front() {
                    Some(request) => request,
                    None => match queue.next().await {
                        Some(request) => request,
                        None => return,
                    },
                };
                let (call, epoch) = match request {
                    Request::Start {
                        grammar,
                        rate,
                        epoch,
                    } => (
                        core.call("voiceStart", vec![json!(grammar), json!(rate)]),
                        Some((epoch, true)),
                    ),
                    Request::Accept { audio, epoch } => (
                        core.call("voiceAccept", vec![json!(audio)]),
                        Some((epoch, false)),
                    ),
                    Request::Finish { epoch } => {
                        (core.call("voiceFinish", Vec::new()), Some((epoch, false)))
                    }
                    Request::Stop => (core.call("voiceStop", Vec::new()), None),
                };
                let answer = call.await;
                let Some((epoch, starting)) = epoch else {
                    continue;
                };
                let alive = this
                    .update(cx, |this, cx| this.answered(epoch, starting, answer, cx))
                    .is_ok();
                if !alive {
                    return;
                }
            }
        }));
    }

    fn answered(
        &mut self,
        epoch: u64,
        starting: bool,
        answer: Result<Value, kchess_core::CoreError>,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.epoch || self.owner.is_none() {
            return;
        }
        let heard = match answer {
            Ok(heard) => heard,
            Err(cause) => {
                let message = if starting {
                    format!("The offline voice model couldn’t load. {}", cause.message)
                } else {
                    cause.message
                };
                return self.fail(message, ErrorKind::Model, cx);
            }
        };
        if let Some(result) = heard.get("result").filter(|result| result.is_object()) {
            self.partial.clear();
            if let (Some(heard), Some(owner)) = (heard_of(result), self.owner) {
                let event = if heard.text.is_empty() {
                    VoiceEvent::Missed { owner, heard }
                } else {
                    VoiceEvent::Heard { owner, heard }
                };
                cx.emit(event);
            }
            // One action per finished phrase; the owner may already have started a new one.
            if self.epoch == epoch {
                self.reset(cx);
            }
        } else if let Some(partial) = heard["partial"].as_str() {
            self.partial = partial.to_string();
            cx.notify();
        }
    }

    fn fail(&mut self, message: String, kind: ErrorKind, cx: &mut Context<Self>) {
        self.capture = None;
        self._pump = None;
        self.state = State::Error;
        self.error = Some((message, kind));
        self.level = 0.;
        cx.notify();
    }

    /// Turn it off and on again after an error (`retryVoice`).
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some(owner) = self.owner {
            let (grammar, active, push) = (self.grammar.clone(), self.active, self.push_to_talk);
            self.disable_now(cx);
            self.enable(owner, grammar, active, push, cx);
        }
    }
}

pub fn init(cx: &mut App) {
    let voice = cx.new(|_| Voice {
        owner: None,
        next_owner: 0,
        state: State::Off,
        error: None,
        partial: String::new(),
        level: 0.,
        preparing: String::new(),
        grammar: Vec::new(),
        active: false,
        push_to_talk: false,
        held: false,
        epoch: 0,
        ready: false,
        capture: None,
        rate: 16_000.,
        requests: None,
        _pump: None,
        _worker: None,
    });
    // The recognizer session on a lost connection ended with it: a listening control starts a
    // fresh one.
    let events = cx.global::<AppCore>().events.clone();
    cx.subscribe(&events, {
        let voice = voice.downgrade();
        move |_, event: &crate::core::CoreEvent, cx| {
            if event.name == crate::core::RECONNECTED {
                let _ = voice.update(cx, |voice, cx| voice.reset(cx));
            }
        }
    })
    .detach();
    cx.set_global(VoiceHandle(voice));
}

/// A rules grammar (`MOVE_GRAMMAR`, `GAME_GRAMMAR`, …).
/// A phrase's place in the voice history, once saved (none when Settings keep no history).
pub type Entry = Shared<Task<Option<i64>>>;

/// A phrase for the voice history (`VoiceAttemptInput`).
pub struct Attempt<'a> {
    pub source: &'a str,
    pub heard: &'a Heard,
    pub outcome: &'a str,
    pub parsed: Option<&'a str>,
    /// What was meant, when the page knows (a drill's question).
    pub expected: Option<&'a str>,
    pub fen: Option<&'a str>,
}

/// Save a phrase to the voice history, when Settings keep one (`logVoice`); `retry` is the
/// phrase it tries again.
pub fn log(attempt: &Attempt, retry: Option<Entry>, cx: &App) -> Option<Entry> {
    let keep = cx
        .global::<crate::settings::SettingsHandle>()
        .0
        .read(cx)
        .settings
        .core
        .voice_history;
    if !keep {
        return None;
    }
    let heard = attempt.heard;
    let mut record = json!({
        "source": attempt.source,
        "heard": heard.raw.chars().take(200).collect::<String>(),
        "confidence": (heard.confidence.clamp(0., 1.) * 1000.).round() / 1000.,
        "words": heard.words.iter().take(40).map(|word| json!({
            "word": word.word.chars().take(40).collect::<String>(),
            "conf": word.conf.clamp(0., 1.),
        })).collect::<Vec<_>>(),
        "outcome": attempt.outcome,
    });
    if let Some(parsed) = attempt.parsed {
        record["parsed"] = json!(parsed.chars().take(120).collect::<String>());
    }
    if let Some(expected) = attempt.expected {
        record["expected"] = json!(expected);
    }
    if let Some(fen) = attempt.fen.filter(|fen| !fen.is_empty()) {
        record["fen"] = json!(fen);
    }
    let core = cx.global::<AppCore>().clone();
    let entry = cx
        .foreground_executor()
        .spawn(async move {
            if let Some(retry) = retry
                && let Some(id) = retry.await
            {
                record["retryOf"] = json!(id);
            }
            // The history is a study aid; a lost entry never disturbs play.
            match core.call("saveVoiceAttempt", vec![record]).await {
                Ok(id) => id.as_i64(),
                Err(cause) => {
                    crate::log::write(
                        kchess_core::Level::Warn,
                        "voice-log",
                        &format!("saving a voice attempt failed: {}", cause.message),
                    );
                    None
                }
            }
        })
        .shared();
    // Saved even when nothing waits for its place.
    cx.foreground_executor()
        .spawn(entry.clone().map(|_| ()))
        .detach();
    Some(entry)
}

/// A saved phrase's new outcome or meant move (`updateVoice`).
pub fn amend(entry: Entry, update: Value, cx: &App) {
    let core = cx.global::<AppCore>().clone();
    cx.foreground_executor()
        .spawn(async move {
            let Some(id) = entry.await else { return };
            if let Err(cause) = core
                .call("updateVoiceAttempt", vec![json!(id), update])
                .await
            {
                crate::log::write(
                    kchess_core::Level::Warn,
                    "voice-log",
                    &format!("updating a voice attempt failed: {}", cause.message),
                );
            }
        })
        .detach();
}

pub fn grammar(name: &str) -> Vec<String> {
    crate::game::rules(name, &[])
        .ok()
        .and_then(|list| serde_json::from_value(list).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{grammar, heard_of};
    use serde_json::json;

    #[test]
    fn confidence_ignores_noise_markers() {
        let heard = heard_of(&json!({
            "text": "[unk] e four",
            "words": [
                { "word": "[unk]", "conf": 0.1 },
                { "word": "e", "conf": 0.9 },
                { "word": "four", "conf": 0.7 },
            ],
        }))
        .unwrap();
        assert_eq!(heard.text, "e four");
        assert_eq!(heard.raw, "[unk] e four");
        assert!((heard.confidence - 0.8).abs() < 1e-9);
        assert!(!heard.unclear);
        let noise =
            heard_of(&json!({ "text": "[unk]", "words": [{ "word": "[unk]", "conf": 1 }] }))
                .unwrap();
        assert!(noise.text.is_empty() && noise.unclear);
        assert!(heard_of(&json!({ "text": " ", "words": [] })).is_none());
    }

    #[test]
    fn the_rules_supply_the_grammars() {
        let moves = grammar("MOVE_GRAMMAR");
        assert!(moves.contains(&"e four".to_string()) && moves.contains(&"knight".to_string()));
        assert!(grammar("GAME_GRAMMAR").contains(&"take back".to_string()));
    }
}

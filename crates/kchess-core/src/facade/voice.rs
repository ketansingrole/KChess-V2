//! The facade's offline speech recognition (`voiceStart`, `voiceAccept`, `voiceFinish`,
//! `voiceStop`), answered by the `kchess-voice` helper (`crate::voice::recognizer`), and the
//! speech model it loads (`voiceModelStatus`, `voiceModelInstall`, reporting
//! `voicemodel:progress`).

use kchess_contracts::CoreEvent;
use std::sync::Arc;

use kchess_contracts::core_api::VoiceApi;
use serde_json::Value;

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::voice::recognizer::heard_value;

/// Another client's audio while one has voice input: one microphone at a time.
const ELSEWHERE: &str = "Voice input is in use in another KChess window.";

/// Whether `caller` has voice input, or nobody does yet.
fn is_mine(owner: Option<u64>, caller: u64) -> bool {
    owner.is_none_or(|owner| owner == caller)
}

impl Core {
    /// A client went away: if it still has voice input, its session is stopped. The check waits
    /// for the owner, so a client that started listening meanwhile keeps its session.
    pub(crate) fn release_voice(&self, connection: u64) {
        // Most clients never listen: no task is needed when it plainly is not theirs.
        if self
            .voice_owner
            .try_lock()
            .is_ok_and(|owner| *owner != Some(connection))
        {
            return;
        }
        let owner = std::sync::Arc::clone(&self.voice_owner);
        let voice = std::sync::Arc::clone(&self.voice);
        tokio::spawn(async move {
            let mut owner = owner.lock().await;
            if *owner == Some(connection) {
                *owner = None;
                voice.stop().await;
            }
        });
    }
}

impl VoiceApi for Core {
    async fn voice_start(&self, grammar: Value, sample_rate: Value) -> Result<Value> {
        let grammar: Vec<String> = grammar
            .as_array()
            .map(|phrases| {
                phrases
                    .iter()
                    .filter_map(|phrase| phrase.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let sample_rate = sample_rate.as_f64().unwrap_or(16_000.0);
        // The window that starts listening last has the microphone.
        let mut owner = self.voice_owner.lock().await;
        *owner = Some(crate::connection::current_or_app());
        self.voice.start(&grammar, sample_rate).await?;
        Ok(Value::Null)
    }

    async fn voice_accept(&self, audio: Value) -> Result<Value> {
        let owner = self.voice_owner.lock().await;
        if !is_mine(*owner, crate::connection::current_or_app()) {
            return Err(CoreError::new(ELSEWHERE));
        }
        let audio = audio.as_str().unwrap_or_default();
        Ok(heard_value(&self.voice.accept(audio).await?))
    }

    async fn voice_finish(&self) -> Result<Value> {
        let owner = self.voice_owner.lock().await;
        if !is_mine(*owner, crate::connection::current_or_app()) {
            return Err(CoreError::new(ELSEWHERE));
        }
        Ok(heard_value(&self.voice.finish().await?))
    }

    async fn voice_stop(&self) -> Result<Value> {
        // Another window's stop leaves the owner listening.
        let mut owner = self.voice_owner.lock().await;
        if is_mine(*owner, crate::connection::current_or_app()) {
            *owner = None;
            self.voice.stop().await;
        }
        Ok(Value::Null)
    }

    async fn voice_model_status(&self) -> Result<Value> {
        serde_json::to_value(self.voice_model.status().await)
            .map_err(|cause| CoreError::wrap("voice model status", cause))
    }

    async fn voice_model_install(&self) -> Result<Value> {
        let host = Arc::clone(&self.host);
        let progress: crate::voice::Listener = Arc::new(move |progress| {
            if let Ok(payload) = serde_json::to_value(progress) {
                host.emit(CoreEvent::VoiceModelProgress(payload));
            }
        });
        self.voice_model.ensure(Some(progress)).await?;
        Ok(Value::Null)
    }
}

//! The facade's offline speech recognition (`voiceStart`, `voiceAccept`, `voiceFinish`,
//! `voiceStop`), answered by the `kchess-voice` helper (`crate::voice::recognizer`), and the
//! speech model it loads (`voiceModelStatus`, `voiceModelInstall`, reporting
//! `voicemodel:progress`).

use kchess_contracts::CoreEvent;
use std::sync::Arc;

use kchess_contracts::check::{SampleRate, VoiceAudio, VoiceGrammar};
use kchess_contracts::core_api::VoiceApi;
use kchess_contracts::voice::{Heard, Status};

use crate::core::Core;
use kchess_contracts::error::{CoreError, Result};

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
    async fn voice_start(&self, grammar: VoiceGrammar, sample_rate: SampleRate) -> Result<()> {
        // The window that starts listening last has the microphone.
        let mut owner = self.voice_owner.lock().await;
        *owner = Some(crate::connection::current_or_app());
        self.voice.start(&grammar.0, sample_rate.0).await
    }

    async fn voice_accept(&self, audio: VoiceAudio) -> Result<Heard> {
        let owner = self.voice_owner.lock().await;
        if !is_mine(*owner, crate::connection::current_or_app()) {
            return Err(CoreError::new(ELSEWHERE));
        }
        self.voice.accept(&audio.0).await
    }

    async fn voice_finish(&self) -> Result<Heard> {
        let owner = self.voice_owner.lock().await;
        if !is_mine(*owner, crate::connection::current_or_app()) {
            return Err(CoreError::new(ELSEWHERE));
        }
        self.voice.finish().await
    }

    async fn voice_stop(&self) -> Result<()> {
        // Another window's stop leaves the owner listening.
        let mut owner = self.voice_owner.lock().await;
        if is_mine(*owner, crate::connection::current_or_app()) {
            *owner = None;
            self.voice.stop().await;
        }
        Ok(())
    }

    async fn voice_model_status(&self) -> Result<Status> {
        Ok(self.voice_model.status().await)
    }

    async fn voice_model_install(&self) -> Result<()> {
        let host = Arc::clone(&self.host);
        let progress: crate::voice::Listener = Arc::new(move |progress| {
            if let Ok(payload) = serde_json::to_value(progress) {
                host.emit(CoreEvent::VoiceModelProgress(payload));
            }
        });
        self.voice_model.ensure(Some(progress)).await?;
        Ok(())
    }
}

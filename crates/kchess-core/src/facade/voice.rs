//! The facade's offline speech recognition (`voiceStart`, `voiceAccept`, `voiceFinish`,
//! `voiceStop`), answered by the `kchess-voice` helper (`crate::voice::recognizer`). The
//! arguments were checked against `CORE_CONTRACTS` before these run.

use serde_json::Value;

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::voice::recognizer::heard_value;

/// Another client's audio while one has voice input: one microphone at a time.
const ELSEWHERE: &str = "Voice input is in use in another KChess window.";

impl Core {
    /// The calling client has voice input, or nobody does yet.
    fn voice_is_mine(&self) -> bool {
        let caller = crate::connection::current_or_app();
        self.voice_owner
            .lock()
            .map(|owner| owner.is_none_or(|owner| owner == caller))
            .unwrap_or(true)
    }

    /// A client went away: if it had voice input, the recognizer is released.
    pub(crate) fn release_voice(&self, connection: u64) {
        let owned = self
            .voice_owner
            .lock()
            .map(|mut owner| {
                let owned = *owner == Some(connection);
                if owned {
                    *owner = None;
                }
                owned
            })
            .unwrap_or(false);
        if owned {
            let voice = std::sync::Arc::clone(&self.voice);
            tokio::spawn(async move { voice.stop().await });
        }
    }

    pub(crate) async fn run_voice(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "voiceStart" => {
                let grammar: Vec<String> = args
                    .first()
                    .and_then(Value::as_array)
                    .map(|phrases| {
                        phrases
                            .iter()
                            .filter_map(|phrase| phrase.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let sample_rate = args.get(1).and_then(Value::as_f64).unwrap_or(16_000.0);
                // The window that starts listening last has the microphone.
                if let Ok(mut owner) = self.voice_owner.lock() {
                    *owner = Some(crate::connection::current_or_app());
                }
                self.voice.start(&grammar, sample_rate).await?;
                Ok(Value::Null)
            }
            "voiceAccept" => {
                if !self.voice_is_mine() {
                    return Err(CoreError::new(ELSEWHERE));
                }
                let audio = args.first().and_then(Value::as_str).unwrap_or_default();
                Ok(heard_value(&self.voice.accept(audio).await?))
            }
            "voiceFinish" => {
                if !self.voice_is_mine() {
                    return Err(CoreError::new(ELSEWHERE));
                }
                Ok(heard_value(&self.voice.finish().await?))
            }
            "voiceStop" => {
                // Another window's stop leaves the owner listening.
                if self.voice_is_mine() {
                    if let Ok(mut owner) = self.voice_owner.lock() {
                        *owner = None;
                    }
                    self.voice.stop().await;
                }
                Ok(Value::Null)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}

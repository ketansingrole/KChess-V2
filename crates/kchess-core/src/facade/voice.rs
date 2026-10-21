//! The facade's offline speech recognition (`voiceStart`, `voiceAccept`, `voiceFinish`,
//! `voiceStop`), answered by the `kchess-voice` helper (`crate::voice::recognizer`). The
//! arguments were checked against `CORE_CONTRACTS` before these run.

use serde_json::Value;

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::voice::recognizer::heard_value;

impl Core {
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
                self.voice.start(&grammar, sample_rate).await?;
                Ok(Value::Null)
            }
            "voiceAccept" => {
                let audio = args.first().and_then(Value::as_str).unwrap_or_default();
                Ok(heard_value(&self.voice.accept(audio).await?))
            }
            "voiceFinish" => Ok(heard_value(&self.voice.finish().await?)),
            "voiceStop" => {
                self.voice.stop().await;
                Ok(Value::Null)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}

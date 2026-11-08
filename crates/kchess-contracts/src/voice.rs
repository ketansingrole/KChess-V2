//! The offline speech model as the core reports it (`voiceModelStatus`, `voiceModelInstall` and
//! `voicemodel:progress`).

use serde::{Deserialize, Serialize};

/// What the preparation is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Checking,
    Downloading,
    Preparing,
}

/// How far the preparation is (`VoiceModelProgress`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    pub received: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// What the profile holds (`VoiceModelStatus`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    pub bytes: u64,
    pub busy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_status_reads_back_as_the_core_sends_it() {
        let status = Status {
            installed: false,
            bytes: 0,
            busy: true,
            progress: Some(Progress {
                phase: Phase::Downloading,
                received: 10,
                total: Some(40),
            }),
        };
        let sent = json!({
            "installed": false,
            "bytes": 0,
            "busy": true,
            "progress": { "phase": "downloading", "received": 10, "total": 40 }
        });
        assert_eq!(serde_json::to_value(&status).unwrap(), sent);
        assert_eq!(serde_json::from_value::<Status>(sent).unwrap(), status);
        let idle = json!({ "installed": true, "bytes": 5, "busy": false });
        assert_eq!(
            serde_json::from_value::<Status>(idle.clone()).unwrap(),
            Status {
                installed: true,
                bytes: 5,
                busy: false,
                progress: None
            }
        );
        assert_eq!(
            serde_json::to_value(Progress {
                phase: Phase::Checking,
                received: 0,
                total: None
            })
            .unwrap(),
            json!({ "phase": "checking", "received": 0 })
        );
        assert_eq!(
            serde_json::to_value(Phase::Preparing).unwrap(),
            json!("preparing")
        );
    }
}

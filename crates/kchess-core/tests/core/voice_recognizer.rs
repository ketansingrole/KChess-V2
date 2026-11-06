//! The recognizer's side of the `kchess-voice` protocol, against a scripted helper
//! (`tests/support/fake_voice.rs`): startup, ordering, results, and recovery from a helper that
//! refuses, crashes or hangs.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use kchess_core::host::{Host, Level};
use kchess_core::voice::recognizer::{Heard, Recognized, VoiceRecognizer, Word};

#[derive(Default)]
struct Logs(Mutex<Vec<String>>);

impl Host for Logs {
    fn log(&self, level: Level, scope: &str, message: &str) {
        self.0
            .lock()
            .expect("log lock")
            .push(format!("{} [{scope}] {message}", level.name()));
    }

    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
}

fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kchess-fake-voice"))
}

fn model(mode: &str) -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("model folder");
    fs::write(folder.path().join("mode"), mode).expect("mode");
    folder
}

fn recognizer(folder: &Path) -> (VoiceRecognizer, Arc<Logs>) {
    let logs = Arc::new(Logs::default());
    let recognizer = VoiceRecognizer::new(logs.clone(), Some(helper()), folder.to_path_buf())
        .with_deadlines(Duration::from_secs(10), Duration::from_millis(300));
    (recognizer, logs)
}

fn requests(folder: &Path) -> String {
    fs::read_to_string(folder.join("log"))
        .unwrap_or_default()
        .split_whitespace()
        .collect()
}

fn pcm(samples: &[i16]) -> String {
    STANDARD.encode(
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<u8>>(),
    )
}

fn grammar() -> Vec<String> {
    vec!["e four".into(), "[unk]".into()]
}

#[tokio::test]
async fn recognizes_in_order_and_reuses_one_helper() {
    let folder = model("normal");
    let (voice, _) = recognizer(folder.path());
    voice.start(&grammar(), 16_000.0).await.expect("start");
    assert_eq!(
        voice.accept(&pcm(&[0; 160])).await.expect("silence"),
        Heard {
            partial: Some("e".into()),
            result: None
        }
    );
    assert_eq!(
        voice.accept(&pcm(&[0, 1200, -800])).await.expect("speech"),
        Heard {
            partial: None,
            result: Some(Recognized {
                text: "e four".into(),
                words: vec![
                    Word {
                        word: "e".into(),
                        conf: 0.9
                    },
                    Word {
                        word: "four".into(),
                        conf: 0.8
                    },
                ],
            }),
        }
    );
    let finished = voice.finish().await.expect("finish");
    assert_eq!(
        finished.result.map(|result| result.text),
        Some(String::new())
    );
    // A new grammar goes to the same helper.
    voice.start(&grammar(), 16_000.0).await.expect("restart");
    assert_eq!(requests(folder.path()), "GAAFG");
}

#[tokio::test]
async fn refuses_without_a_helper_a_model_or_a_valid_chunk() {
    let folder = model("normal");
    let logs = Arc::new(Logs::default());
    let none = VoiceRecognizer::new(logs.clone(), None, folder.path().to_path_buf());
    assert_eq!(
        none.start(&grammar(), 16_000.0)
            .await
            .expect_err("no helper")
            .message,
        "Voice input isn’t available in this build."
    );
    let missing = VoiceRecognizer::new(logs, Some(helper()), folder.path().join("absent"));
    assert_eq!(
        missing
            .start(&grammar(), 16_000.0)
            .await
            .expect_err("no model")
            .message,
        "The offline voice model isn’t installed. Download it and try again."
    );

    let (voice, _) = recognizer(folder.path());
    assert_eq!(
        voice
            .accept(&pcm(&[1]))
            .await
            .expect_err("not started")
            .message,
        "Voice input isn’t listening."
    );
    voice.start(&grammar(), 16_000.0).await.expect("start");
    assert_eq!(
        voice
            .accept("not base64!")
            .await
            .expect_err("bad audio")
            .message,
        "Invalid audio."
    );
    assert_eq!(
        voice
            .accept(&STANDARD.encode([1u8, 2, 3]))
            .await
            .expect_err("odd")
            .message,
        "Invalid audio."
    );
    // Requests larger than the helper's 1 MiB frame would end it.
    assert_eq!(
        voice
            .accept(&STANDARD.encode(vec![0u8; (1 << 20) + 2]))
            .await
            .expect_err("too long")
            .message,
        "Invalid audio."
    );
    let wide = vec!["ж".repeat(200); 4096];
    assert_eq!(
        voice
            .start(&wide, 16_000.0)
            .await
            .expect_err("too large")
            .message,
        "Invalid voice grammar."
    );
    // Rejected chunks never reach the helper.
    assert_eq!(requests(folder.path()), "G");
}

#[tokio::test]
async fn reports_a_helper_that_cannot_load_the_model() {
    let folder = model("refuse");
    let (voice, logs) = recognizer(folder.path());
    assert_eq!(
        voice
            .start(&grammar(), 16_000.0)
            .await
            .expect_err("refused")
            .message,
        "The offline voice model couldn’t load. Try again."
    );
    assert!(
        logs.0
            .lock()
            .expect("logs")
            .iter()
            .any(|line| line.starts_with("warn [voice] Voice helper refused to start"))
    );
}

#[tokio::test]
async fn recovers_from_a_crashed_or_hung_helper_with_a_new_one() {
    for mode in ["crash-on-audio", "hang-on-audio"] {
        let folder = model(mode);
        let (voice, _) = recognizer(folder.path());
        voice.start(&grammar(), 16_000.0).await.expect("start");
        assert_eq!(
            voice.accept(&pcm(&[5])).await.expect_err(mode).message,
            "Speech recognition stopped. Try again.",
            "{mode}"
        );
        // The failed helper is gone: further audio needs a new start, which launches a new one.
        assert_eq!(
            voice.accept(&pcm(&[5])).await.expect_err("stopped").message,
            "Voice input isn’t listening."
        );
        fs::write(folder.path().join("mode"), "normal").expect("mode");
        voice.start(&grammar(), 16_000.0).await.expect("restart");
        assert!(
            voice
                .accept(&pcm(&[5]))
                .await
                .expect("heard")
                .result
                .is_some()
        );
        voice.stop().await;
        assert_eq!(
            voice.finish().await.expect_err("stopped").message,
            "Voice input isn’t listening."
        );
    }
}

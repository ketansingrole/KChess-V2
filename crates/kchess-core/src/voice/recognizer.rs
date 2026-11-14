//! Offline speech recognition through the `kchess-voice` helper (`crates/kchess-voice`), which
//! runs the Vosk library in its own process: a native crash or a bad model file ends the helper,
//! never the core. The core starts it on the first `voiceStart`, keeps it while voice input is in
//! use, and stops it on `voiceStop`, a failure or close.
//!
//! Requests are framed on the helper's stdin and each is answered with one JSON line
//! (`kchess_voice::protocol`). Calls are serialized, so audio reaches the recognizer in the order
//! it was sent.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
pub use kchess_contracts::types::VoiceWord;
pub use kchess_contracts::voice::{Heard, Utterance};
use kchess_voice::protocol::Recognized;
use kchess_voice::protocol::{self, Answer, Grammar, MAX_LINE, MAX_PAYLOAD, Tag};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::host::{Host, Level};
use kchess_contracts::error::{CoreError, Result};

/// Loading the model reads about 40 MB; a slow disk gets a generous bound. Taking a grammar
/// (compiling it into the recognizer's graph) is part of starting and shares this bound.
const STARTUP_DEADLINE: Duration = Duration::from_secs(60);
/// A 100 ms chunk is recognized in milliseconds; anything this slow has hung.
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

const UNAVAILABLE: &str = "Voice input isn’t available in this build.";
const NOT_INSTALLED: &str = "The offline voice model isn’t installed. Download it and try again.";
const STOPPED: &str = "Speech recognition stopped. Try again.";

struct Helper {
    // Held for `kill_on_drop`: dropping the helper ends the process.
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Helper {
    async fn send(&mut self, tag: Tag, payload: &[u8]) -> std::io::Result<()> {
        let frame = protocol::frame(tag, payload)
            .ok_or_else(|| std::io::Error::other("request too large"))?;
        self.stdin.write_all(&frame).await?;
        self.stdin.flush().await
    }

    async fn answer(&mut self) -> std::io::Result<Answer> {
        let mut line = Vec::new();
        let read = (&mut self.stdout)
            .take(MAX_LINE)
            .read_until(b'\n', &mut line)
            .await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the helper exited",
            ));
        }
        Answer::read(&line).map_err(std::io::Error::other)
    }

    async fn request(&mut self, tag: Tag, payload: &[u8]) -> std::io::Result<Answer> {
        self.send(tag, payload).await?;
        self.answer().await
    }
}

/// The recognizer service: one helper process at a time.
pub struct VoiceRecognizer {
    host: Arc<dyn Host>,
    helper_path: Option<PathBuf>,
    model_dir: PathBuf,
    startup_deadline: Duration,
    request_deadline: Duration,
    helper: Mutex<Option<Helper>>,
}

impl VoiceRecognizer {
    /// `helper_path` is the `kchess-voice` executable (`CorePlatform.voiceHelperPath`), with
    /// libvosk beside it; `model_dir` is the prepared model folder.
    pub fn new(host: Arc<dyn Host>, helper_path: Option<PathBuf>, model_dir: PathBuf) -> Self {
        VoiceRecognizer {
            host,
            helper_path,
            model_dir,
            startup_deadline: STARTUP_DEADLINE,
            request_deadline: REQUEST_DEADLINE,
            helper: Mutex::new(None),
        }
    }

    /// Shorter bounds for model loading and for each request (tests).
    pub fn with_deadlines(mut self, startup: Duration, request: Duration) -> Self {
        self.startup_deadline = startup;
        self.request_deadline = request;
        self
    }

    fn log(&self, level: Level, message: &str) {
        self.host.log(level, "voice", message);
    }

    async fn spawn(&self) -> Result<Helper> {
        let path = self
            .helper_path
            .as_ref()
            .ok_or_else(|| CoreError::new(UNAVAILABLE))?;
        if !tokio::fs::try_exists(&self.model_dir)
            .await
            .unwrap_or(false)
        {
            return Err(CoreError::new(NOT_INSTALLED));
        }
        let mut child = crate::process::spawn(
            Command::new(path)
                .arg(&self.model_dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true),
        )
        .map_err(|cause| {
            self.log(
                Level::Error,
                &format!("Voice helper did not start: {cause}"),
            );
            CoreError::new(UNAVAILABLE)
        })?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(CoreError::new(UNAVAILABLE));
        };
        let mut helper = Helper {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        match tokio::time::timeout(self.startup_deadline, helper.answer()).await {
            Ok(Ok(Answer::Ready(true))) => Ok(helper),
            Ok(Ok(Answer::Error(message))) => {
                self.log(
                    Level::Warn,
                    &format!("Voice helper refused to start: {message}"),
                );
                Err(CoreError::new(
                    "The offline voice model couldn’t load. Try again.",
                ))
            }
            Ok(Ok(_)) => Err(CoreError::new(STOPPED)),
            Ok(Err(cause)) => {
                self.log(
                    Level::Warn,
                    &format!("Voice helper failed to start: {cause}"),
                );
                Err(CoreError::new(STOPPED))
            }
            Err(_) => Err(CoreError::new(
                "The voice model took too long to load. Try again.",
            )),
        }
    }

    /// One request to the running helper, answered within `deadline`. A broken pipe, a timeout or
    /// an unreadable answer ends the helper; the next `start` launches a new one.
    async fn exchange(&self, tag: Tag, payload: &[u8], deadline: Duration) -> Result<Answer> {
        let mut slot = self.helper.lock().await;
        // Out of its slot until its answer is read: a call cancelled after sending would leave
        // the answer in the pipe, and every later request would read the one before it. Dropped
        // instead, the helper ends, and the next `start` launches a new one.
        let mut helper = slot
            .take()
            .ok_or_else(|| CoreError::new("Voice input isn’t listening."))?;
        match tokio::time::timeout(deadline, helper.request(tag, payload)).await {
            Ok(Ok(Answer::Error(message))) => {
                *slot = Some(helper);
                self.log(
                    Level::Warn,
                    &format!("Speech recognition failed: {message}"),
                );
                Err(CoreError::new(STOPPED))
            }
            Ok(Ok(answer)) => {
                *slot = Some(helper);
                Ok(answer)
            }
            outcome => {
                let cause = match outcome {
                    Ok(Err(cause)) => cause.to_string(),
                    _ => "no answer in time".into(),
                };
                self.log(Level::Warn, &format!("Voice helper stopped: {cause}"));
                Err(CoreError::new(STOPPED))
            }
        }
    }

    /// Starts recognizing `grammar` (the phrases that may be said) at `sample_rate`, launching
    /// the helper when it is not running. Any utterance in progress is discarded.
    pub async fn start(&self, grammar: &[String], sample_rate: f64) -> Result<()> {
        {
            let mut slot = self.helper.lock().await;
            if slot.is_none() {
                *slot = Some(self.spawn().await?);
            }
        }
        // The helper takes rates from 8 kHz to 192 kHz, exact in f32.
        let grammar = Grammar {
            sample_rate: sample_rate as f32,
            grammar: grammar.to_vec(),
        };
        let payload = serde_json::to_vec(&grammar).unwrap_or_default();
        if payload.len() > MAX_PAYLOAD {
            return Err(CoreError::new("Invalid voice grammar."));
        }
        // Not the per-chunk bound: a new helper's first request and a large grammar take longer
        // than a chunk of audio, and a helper that is merely slow to set up has not hung.
        match self
            .exchange(Tag::Grammar, &payload, self.startup_deadline)
            .await?
        {
            Answer::Ok(true) => Ok(()),
            _ => Err(CoreError::new(STOPPED)),
        }
    }

    /// Feeds base64 16-bit little-endian mono PCM, answering the words heard so far or, at the
    /// end of an utterance, the result.
    pub async fn accept(&self, audio: &str) -> Result<Heard> {
        let pcm = STANDARD
            .decode(audio)
            .map_err(|_| CoreError::new("Invalid audio."))?;
        if pcm.len() % 2 != 0 || pcm.len() > MAX_PAYLOAD {
            return Err(CoreError::new("Invalid audio."));
        }
        heard(
            self.exchange(Tag::Audio, &pcm, self.request_deadline)
                .await?,
        )
    }

    /// Ends the utterance (push-to-talk released) and answers what it said.
    pub async fn finish(&self) -> Result<Heard> {
        heard(
            self.exchange(Tag::Final, &[], self.request_deadline)
                .await?,
        )
    }

    /// Stops the helper, releasing the model's memory.
    pub async fn stop(&self) {
        self.helper.lock().await.take();
    }
}

fn heard(answer: Answer) -> Result<Heard> {
    match answer {
        Answer::Result(result) => Ok(Heard {
            partial: None,
            result: Some(utterance(result)),
        }),
        Answer::Partial(partial) => Ok(Heard {
            partial: Some(partial),
            result: None,
        }),
        _ => Err(CoreError::new(STOPPED)),
    }
}

fn utterance(recognized: Recognized) -> Utterance {
    let words = recognized.words.into_iter();
    Utterance {
        text: recognized.text,
        words: words
            .map(|word| VoiceWord {
                word: word.word,
                conf: word.conf,
            })
            .collect(),
    }
}

//! Audio for voice input: the default
//! microphone, mono, in chunks of about 100 ms. Under the test switches a WAV file named by
//! `KCHESS_TEST_VOICE_AUDIO` is played in its place, in real time, so the app can be driven
//! without opening a microphone.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use futures::channel::mpsc;

/// One chunk of samples, -1 to 1.
pub type Chunk = Vec<f32>;

/// Where an open microphone's chunks arrive.
pub type ChunkReceiver = mpsc::UnboundedReceiver<Chunk>;

/// What the capture thread answers once the microphone opened: its rate and device name.
type Opened = Result<(f64, String), CaptureError>;

/// A capture running on its own thread; dropping it stops it.
pub struct Capture {
    stop: Arc<AtomicBool>,
    pub sample_rate: f64,
    /// The microphone's name, as the system gives it.
    pub device: String,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Why the microphone did not open, in words the player can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Missing,
    Denied,
    Busy(String),
}

impl CaptureError {
    pub fn message(&self) -> String {
        match self {
            CaptureError::Missing => "No microphone was found. Connect one and try again.".into(),
            CaptureError::Denied => "KChess isn’t allowed to use the microphone. Allow it in your system privacy settings, then try again.".into(),
            CaptureError::Busy(detail) => format!(
                "The microphone couldn’t start. Another app may be using it, or the system hasn’t granted access yet. ({detail})"
            ),
        }
    }
}

/// A recording to play instead of the microphone, when the app was started with one (the core's
/// `TestSwitches::voice_audio`, which the app sets here).
pub struct TestAudio(pub PathBuf);

impl gpui_kit::Global for TestAudio {}

fn test_audio(cx: &gpui_kit::App) -> Option<PathBuf> {
    cx.try_global::<TestAudio>()
        .map(|TestAudio(path)| path.clone())
}

/// A microphone being opened. `wait` blocks until it answers, for up to `OPEN_TIMEOUT`, so it
/// runs off the main thread: opening it there once froze every window while a slow headset or
/// a permission prompt answered. Dropped unopened, the microphone closes as it answers.
pub struct Opening {
    capture: Capture,
    chunks: mpsc::UnboundedReceiver<Chunk>,
    opened: std::sync::mpsc::Receiver<Opened>,
}

impl Opening {
    /// The capture once the microphone answers; chunks arrive on the receiver until it drops.
    pub fn wait(self) -> Result<(Capture, mpsc::UnboundedReceiver<Chunk>), CaptureError> {
        let Opening {
            mut capture,
            chunks,
            opened,
        } = self;
        let (sample_rate, device) = await_open(opened, &capture.stop, OPEN_TIMEOUT)?;
        capture.sample_rate = sample_rate;
        capture.device = device;
        Ok((capture, chunks))
    }

    /// An opening that answers when the test sends on the returned sender.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pending() -> (Self, std::sync::mpsc::Sender<Opened>) {
        let (ready, opened) = std::sync::mpsc::channel();
        let (_, chunks) = mpsc::unbounded();
        let capture = Capture {
            stop: Arc::new(AtomicBool::new(false)),
            sample_rate: 0.,
            device: String::new(),
        };
        (
            Opening {
                capture,
                chunks,
                opened,
            },
            ready,
        )
    }
}

/// Start opening the microphone, without waiting for it: see `Opening::wait`.
pub fn start(cx: &gpui_kit::App) -> Opening {
    let (sender, receiver) = mpsc::unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let (ready, opened) = std::sync::mpsc::channel::<Opened>();
    let opening = Opening {
        capture: Capture {
            stop: Arc::clone(&stop),
            sample_rate: 0.,
            device: String::new(),
        },
        chunks: receiver,
        opened,
    };
    if let Some(path) = test_audio(cx) {
        match read_wav(&path) {
            Ok((samples, rate)) => {
                let flag = Arc::clone(&stop);
                std::thread::spawn(move || play_file(samples, rate, sender, flag));
                let _ = ready.send(Ok((f64::from(rate), "Test audio".into())));
            }
            Err(cause) => {
                let _ = ready.send(Err(CaptureError::Busy(cause)));
            }
        }
        return opening;
    }
    // The stream lives on its own thread: some platforms' streams cannot move between threads.
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        let host = cpal::default_host();
        let Some(device) = host.default_input_device() else {
            let _ = ready.send(Err(CaptureError::Missing));
            return;
        };
        let config = match device.default_input_config() {
            Ok(config) => config,
            Err(cause) => {
                let _ = ready.send(Err(classify(&cause)));
                return;
            }
        };
        let rate = config.sample_rate();
        let channels = usize::from(config.channels()).max(1);
        let chunk_len = (rate / 10).max(1) as usize;
        let mut buffer: Chunk = Vec::with_capacity(chunk_len);
        let mut push = move |mono: f32| {
            buffer.push(mono);
            if buffer.len() >= chunk_len {
                let _ = sender.unbounded_send(std::mem::replace(
                    &mut buffer,
                    Vec::with_capacity(chunk_len),
                ));
            }
        };
        let failed = |cause: cpal::Error| {
            crate::log::write(
                kchess_contracts::Level::Warn,
                "voice",
                &format!("Microphone: {cause}"),
            );
        };
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                config.into(),
                move |data: &[f32], _| {
                    for frame in data.chunks(channels) {
                        push(frame.iter().sum::<f32>() / frame.len() as f32);
                    }
                },
                failed,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                config.into(),
                move |data: &[i16], _| {
                    for frame in data.chunks(channels) {
                        let sum: f32 = frame.iter().map(|s| f32::from(*s) / 32768.).sum();
                        push(sum / frame.len() as f32);
                    }
                },
                failed,
                None,
            ),
            format => {
                let _ = ready.send(Err(CaptureError::Busy(format!(
                    "unsupported sample format {format:?}"
                ))));
                return;
            }
        };
        let stream = match stream {
            Ok(stream) => stream,
            Err(cause) => {
                let _ = ready.send(Err(classify(&cause)));
                return;
            }
        };
        if let Err(cause) = stream.play() {
            let _ = ready.send(Err(classify(&cause)));
            return;
        }
        let name = device
            .description()
            .map(|description| description.name().to_string())
            .unwrap_or_default();
        // The caller gave up waiting (`await_open`) or the opening was dropped: nobody owns this stream, so it closes.
        if ready.send(Ok((f64::from(rate), name))).is_err() {
            return;
        }
        while !flag.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(stream);
    });
    opening
}

/// How long a microphone may take to open.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// The opened stream's rate and device name. On a timeout the capture thread is told to stop and
/// its late answer goes nowhere, so a microphone that opens afterwards (a slow headset, a
/// permission prompt answered late) closes at once instead of staying on for the session.
fn await_open(
    opened: std::sync::mpsc::Receiver<Result<(f64, String), CaptureError>>,
    stop: &AtomicBool,
    timeout: Duration,
) -> Result<(f64, String), CaptureError> {
    match opened.recv_timeout(timeout) {
        Ok(answer) => answer,
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err(CaptureError::Busy("the microphone did not answer".into()))
        }
    }
}

fn classify(cause: &cpal::Error) -> CaptureError {
    match cause.kind() {
        cpal::ErrorKind::PermissionDenied => CaptureError::Denied,
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable => {
            CaptureError::Missing
        }
        _ => CaptureError::Busy(cause.to_string()),
    }
}

/// The test file's samples, chunk by chunk in real time, then silence (so the recognizer hears
/// the utterance end).
fn play_file(
    samples: Vec<f32>,
    rate: u32,
    sender: mpsc::UnboundedSender<Chunk>,
    stop: Arc<AtomicBool>,
) {
    let chunk_len = (rate / 10).max(1) as usize;
    let silence = std::iter::repeat_n(0., chunk_len * 20);
    let all: Vec<f32> = samples.into_iter().chain(silence).collect();
    for chunk in all.chunks(chunk_len) {
        if stop.load(Ordering::Relaxed) || sender.unbounded_send(chunk.to_vec()).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    while !stop.load(Ordering::Relaxed) {
        if sender.unbounded_send(vec![0.; chunk_len]).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A 16-bit PCM WAV file's samples (mono, or its channels averaged) and rate.
pub fn read_wav(path: &std::path::Path) -> Result<(Vec<f32>, u32), String> {
    let bytes = std::fs::read(path).map_err(|cause| cause.to_string())?;
    parse_wav(&bytes)
}

fn parse_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    let invalid = || "Not a 16-bit PCM WAV file".to_string();
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid());
    }
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32_at(at + 4) as usize;
        let body = at + 8;
        if body + len > bytes.len() {
            break;
        }
        if id == b"fmt " && len >= 16 {
            channels = u16_at(body + 2);
            rate = u32_at(body + 4);
            bits = u16_at(body + 14);
        } else if id == b"data" {
            if bits != 16 || channels == 0 {
                return Err(invalid());
            }
            let frame = usize::from(channels) * 2;
            let samples = bytes[body..body + len]
                .chunks_exact(frame)
                .map(|frame| {
                    let sum: f32 = frame
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| f32::from(i16::from_le_bytes(*s)) / 32768.)
                        .sum();
                    sum / f32::from(channels)
                })
                .collect();
            return Ok((samples, rate));
        }
        at = body + len + (len & 1);
    }
    Err(invalid())
}

/// Samples as base64 16-bit little-endian PCM, as the core's recognizer takes them.
pub fn pcm_base64(audio: &[f32]) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = audio
        .iter()
        .flat_map(|sample| ((sample.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The quietest level the meter shows (a quiet room), and the loudness that fills it, in dBFS.
const METER_FLOOR_DB: f32 = -60.;
const METER_FULL_DB: f32 = -20.;

/// How loud a chunk is, 0 to 1, for the level meter: in decibels, as the ear hears it. Speech
/// into a laptop's microphone is about -35 to -25 dBFS RMS (0.02 to 0.06), which a linear scale
/// showed as a sliver; here it fills most of the meter, and a quiet room barely moves it.
pub fn level(audio: &[f32]) -> f32 {
    if audio.is_empty() {
        return 0.;
    }
    let power: f32 = audio.iter().map(|s| s * s).sum::<f32>() / audio.len() as f32;
    if power <= 0. {
        return 0.;
    }
    let db = 10. * power.log10();
    ((db - METER_FLOOR_DB) / (METER_FULL_DB - METER_FLOOR_DB)).clamp(0., 1.)
}

/// What a microphone test has heard that the player should act on (`heard`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// Nothing above a quiet room: unplugged, muted or the wrong input.
    Nothing,
    /// Never as loud as speech into a laptop's microphone: the input level is low.
    Quiet,
}

/// After `chunks` of about 100 ms whose loudest `level` was `loudest`, what the player should
/// hear about it. A headset whose input was at its lowest showed a still bar and no word why;
/// one that sent only -40 dBFS when shouted into filled half the bar, and the recognizer hears it
/// as quietly.
pub fn heard(loudest: f32, chunks: usize) -> Option<Heard> {
    /// The loudest level speech reaches into a working microphone: -36 dBFS.
    const SPEECH: f32 = 0.6;
    if chunks >= 30 && loudest <= 0. {
        Some(Heard::Nothing)
    } else if chunks >= 50 && loudest < SPEECH {
        Some(Heard::Quiet)
    } else {
        None
    }
}

impl Heard {
    pub fn message(self, device: &str) -> String {
        let device = if device.is_empty() {
            "The microphone"
        } else {
            device
        };
        match self {
            Heard::Nothing => {
                format!("No sound from {device}. Check that it’s plugged in and not muted.")
            }
            Heard::Quiet => format!(
                "{device} sounds quiet. Speak up, or raise its input level in your system sound settings."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Heard, heard};

    /// A still bar said nothing for a silent input, and half a bar nothing for a quiet one.
    #[test]
    fn a_silent_or_quiet_microphone_is_named() {
        assert_eq!(
            heard(0., 10),
            None,
            "not yet: the player may not have spoken"
        );
        assert_eq!(heard(0., 30), Some(Heard::Nothing));
        // Shouting into the headset that reached -40 dBFS.
        assert_eq!(heard(0.5, 30), None);
        assert_eq!(heard(0.5, 50), Some(Heard::Quiet));
        // Speech into a laptop's microphone, -30 dBFS.
        assert_eq!(heard(0.75, 50), None);
        assert!(
            Heard::Nothing
                .message("Headset")
                .starts_with("No sound from Headset.")
        );
        assert!(
            Heard::Quiet
                .message("")
                .starts_with("The microphone sounds quiet.")
        );
    }

    /// A recording the app was started with plays instead of the microphone; none, none.
    #[gpui_kit::test]
    fn the_recording_the_app_set_plays(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(super::test_audio(cx), None);
            cx.set_global(super::TestAudio("voice.wav".into()));
            assert_eq!(super::test_audio(cx), Some("voice.wav".into()));
        });
    }

    use super::{
        AtomicBool, CaptureError, Duration, Ordering, await_open, level, parse_wav, pcm_base64,
    };

    /// A microphone that answered after the 10 s wait stayed open for the rest of the session:
    /// nothing told its thread to stop, and its answer was taken as delivered.
    #[test]
    fn a_microphone_that_answers_late_is_closed() {
        let stop = AtomicBool::new(false);
        let (ready, opened) = std::sync::mpsc::channel::<Result<(f64, String), CaptureError>>();
        let answer = await_open(opened, &stop, Duration::from_millis(10));
        assert!(matches!(answer, Err(CaptureError::Busy(_))));
        assert!(
            stop.load(Ordering::Relaxed),
            "the capture thread is told to stop"
        );
        assert!(
            ready.send(Ok((48_000.0, "Late headset".into()))).is_err(),
            "the late answer goes nowhere, so the thread drops its stream"
        );
    }

    fn wav(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = b"RIFF".to_vec();
        out.extend((36 + data.len() as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * u32::from(channels) * 2).to_le_bytes());
        out.extend((channels * 2).to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn a_wav_reads_as_mono_samples() {
        let (samples, rate) = parse_wav(&wav(&[16384, -16384, 0, 0], 16_000, 2)).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(samples, vec![0., 0.]);
        let (mono, _) = parse_wav(&wav(&[16384], 8_000, 1)).unwrap();
        assert_eq!(mono, vec![0.5]);
        assert!(parse_wav(b"not a wav").is_err());
    }

    #[test]
    fn samples_travel_as_little_endian_pcm() {
        // 0 and the loudest positive sample, as the core decodes them.
        assert_eq!(pcm_base64(&[0., 1.]), "AAD/fw==");
        assert_eq!(level(&[]), 0.);
        assert_eq!(level(&[0.; 10]), 0.);
        assert!((level(&[0.2; 10]) - 1.).abs() < 1e-6);
        // Normal speech into a laptop microphone (-30 dBFS) filled 16% of the meter; now most.
        let speech = level(&[0.0316; 10]);
        assert!((speech - 0.75).abs() < 0.01, "{speech}");
        // A quiet room (-55 dBFS) barely moves it, and quieter shows nothing.
        let room = level(&[0.00178; 10]);
        assert!((room - 0.125).abs() < 0.01, "{room}");
        assert_eq!(level(&[0.0005; 10]), 0.);
        // Louder is fuller.
        assert!(level(&[0.01; 10]) < speech && speech < level(&[0.08; 10]));
    }
}

//! Audio for voice input: the default
//! microphone, mono, in chunks of about 100 ms. Under the store's test switch a WAV file named by
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

/// Why the microphone did not open, in words the player can act on (`describeMicError`).
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

/// The test file to play instead of the microphone, while the test switch is on.
fn test_audio() -> Option<PathBuf> {
    let debug = std::env::var("KCHESS_STORE_DEBUG").is_ok_and(|value| value == "1");
    debug
        .then(|| std::env::var_os("KCHESS_TEST_VOICE_AUDIO"))
        .flatten()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Start capturing: chunks arrive on the returned receiver until the capture is dropped.
pub fn start() -> Result<(Capture, mpsc::UnboundedReceiver<Chunk>), CaptureError> {
    let (sender, receiver) = mpsc::unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    if let Some(path) = test_audio() {
        let (samples, rate) = read_wav(&path).map_err(CaptureError::Busy)?;
        let flag = Arc::clone(&stop);
        std::thread::spawn(move || play_file(samples, rate, sender, flag));
        return Ok((
            Capture {
                stop,
                sample_rate: f64::from(rate),
                device: "Test audio".into(),
            },
            receiver,
        ));
    }
    // The stream lives on its own thread: some platforms' streams cannot move between threads.
    let (ready, opened) = std::sync::mpsc::channel::<Result<(f64, String), CaptureError>>();
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
                kchess_core::Level::Warn,
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
        let _ = ready.send(Ok((f64::from(rate), name)));
        while !flag.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(stream);
    });
    let (sample_rate, device) = opened
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| CaptureError::Busy("the microphone did not answer".into()))??;
    Ok((
        Capture {
            stop,
            sample_rate,
            device,
        },
        receiver,
    ))
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

/// Samples as base64 16-bit little-endian PCM, as the core's recognizer takes them (`pcmBase64`).
pub fn pcm_base64(audio: &[f32]) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = audio
        .iter()
        .flat_map(|sample| ((sample.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// How loud a chunk is, 0 to 1, for the level meter (`reportLevel`).
pub fn level(audio: &[f32]) -> f32 {
    if audio.is_empty() {
        return 0.;
    }
    let power: f32 = audio.iter().map(|s| s * s).sum::<f32>() / audio.len() as f32;
    // Speech rarely passes 0.2 RMS; scaled so normal speaking fills most of the meter.
    (power.sqrt() * 5.).min(1.)
}

#[cfg(test)]
mod tests {
    use super::{level, parse_wav, pcm_base64};

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
        assert!((level(&[0.2; 10]) - 1.).abs() < 1e-6);
    }
}

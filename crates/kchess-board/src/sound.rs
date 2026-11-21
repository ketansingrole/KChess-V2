//! Move and game sounds: the move, capture,
//! low-time and notification clips. A sound thread owns the
//! output device, opened at the first sound and again after its stream ends, and decodes each
//! clip once. On Windows it also tells a Bluetooth speaker that KChess is playing (`playing`).

#[cfg(windows)]
mod playing;

use std::collections::HashMap;
use std::fmt::Display;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use cpal::traits::{DeviceTrait as _, HostTrait as _};
use gpui_kit::{App, Global};
use kchess_contracts::Level;
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, DeviceSinkBuilder, DeviceSinkError, MixerDeviceSink, Source as _};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sound {
    Move,
    Capture,
    LowTime,
    GenericNotify,
}

impl Sound {
    fn file(self) -> &'static str {
        match self {
            Sound::Move => "Move.mp3",
            Sound::Capture => "Capture.mp3",
            Sound::LowTime => "LowTime.mp3",
            Sound::GenericNotify => "GenericNotify.mp3",
        }
    }
}

/// The sound thread's queue: a clip and its volume.
struct Player(mpsc::Sender<(Sound, f32)>);

impl Global for Player {}

pub fn init(cx: &mut App) {
    let (sender, queue) = mpsc::channel::<(Sound, f32)>();
    let started = std::thread::Builder::new()
        .name("kchess-sound".into())
        .spawn(move || {
            let mut output = Output::default();
            let mut clips: HashMap<Sound, SamplesBuffer> = HashMap::new();
            #[cfg(windows)]
            let playing = playing::Playing::start()
                .inspect_err(|cause| {
                    crate::log::write(
                        Level::Warn,
                        "sound",
                        &format!("the playing signal's thread could not start: {cause}"),
                    );
                })
                .ok();
            while let Ok((sound, volume)) = queue.recv() {
                let Some(device) = output.ready(open_default) else {
                    continue;
                };
                let clip = match clips.get(&sound) {
                    Some(clip) => clip.clone(),
                    None => match decode(sound) {
                        Some(clip) => clips.entry(sound).or_insert(clip).clone(),
                        None => continue,
                    },
                };
                // A Bluetooth speaker that waits for "playing" opens before the first sound.
                #[cfg(windows)]
                if let Some(playing) = &playing {
                    let wait = playing.before_sound(playing::default_is_bluetooth());
                    if !wait.is_zero() {
                        std::thread::sleep(wait);
                    }
                }
                device.mixer().add(clip.amplify(volume));
            }
        });
    match started {
        Ok(_) => cx.set_global(Player(sender)),
        Err(cause) => crate::log::write(
            Level::Warn,
            "sound",
            &format!("the sound thread could not start: {cause}"),
        ),
    }
}

/// The device the clips play on, and whether its stream has ended.
///
/// A stream ends for good when its device goes away or, on Windows, whenever the default output
/// changes (a Bluetooth speaker reconnecting, another device chosen): the stream never follows
/// it. The next sound then opens the default device again instead of mixing into a dead stream.
struct Output<D> {
    device: Option<(D, Arc<AtomicBool>)>,
    /// Opening failed since the last success; said once, tried again at each sound.
    failed: bool,
}

impl<D> Default for Output<D> {
    fn default() -> Self {
        Self {
            device: None,
            failed: false,
        }
    }
}

impl<D> Output<D> {
    /// The device to play on, opened with `open` when there is none or its stream ended. `open`
    /// gets the flag its stream's errors set through [`stream_error`].
    fn ready<E: Display>(
        &mut self,
        open: impl FnOnce(Arc<AtomicBool>) -> Result<D, E>,
    ) -> Option<&D> {
        if self
            .device
            .as_ref()
            .is_some_and(|(_, ended)| ended.load(Ordering::Acquire))
        {
            self.device = None;
        }
        if self.device.is_none() {
            let ended = Arc::new(AtomicBool::new(false));
            match open(ended.clone()) {
                Ok(device) => {
                    self.failed = false;
                    self.device = Some((device, ended));
                }
                // Sound is optional: say so once, and try again at the next sound.
                Err(cause) => {
                    if !self.failed {
                        self.failed = true;
                        crate::log::write(
                            Level::Warn,
                            "sound",
                            &format!("opening the sound device failed: {cause}"),
                        );
                    }
                    return None;
                }
            }
        }
        self.device.as_ref().map(|(device, _)| device)
    }
}

/// A stream's error: one that stops it marks it ended (said once); a glitch or a reroute the
/// stream survives changes nothing.
fn stream_error(ended: &AtomicBool, cause: &cpal::Error) {
    use cpal::ErrorKind;
    if matches!(
        cause.kind(),
        ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied
    ) {
        return;
    }
    if !ended.swap(true, Ordering::AcqRel) {
        crate::log::write(
            Level::Warn,
            "sound",
            &format!("the sound device stopped, opening it again at the next sound: {cause}"),
        );
    }
}

/// The default output device, or failing that the first other one that opens (as
/// `DeviceSinkBuilder::open_default_sink`), with its stream's errors reported to `ended`.
fn open_default(ended: Arc<AtomicBool>) -> Result<MixerDeviceSink, DeviceSinkError> {
    let on_error = move |cause: cpal::Error| stream_error(&ended, &cause);
    let open = |builder: DeviceSinkBuilder| {
        builder
            .with_error_callback(on_error.clone())
            .open_sink_or_fallback()
    };
    let mut opened = DeviceSinkBuilder::from_default_device()
        .and_then(open)
        .or_else(|first| {
            let Ok(devices) = cpal::default_host().output_devices() else {
                return Err(first);
            };
            devices
                .filter(|device| {
                    device
                        .description()
                        .is_ok_and(|about| about.driver().is_some_and(|driver| driver != "null"))
                })
                .find_map(|device| DeviceSinkBuilder::from_device(device).and_then(open).ok())
                .ok_or(first)
        })?;
    opened.log_on_drop(false);
    Ok(opened)
}

fn decode(sound: Sound) -> Option<SamplesBuffer> {
    let bytes = crate::assets::sound(sound.file())?;
    let decoder = match Decoder::new(Cursor::new(bytes)) {
        Ok(decoder) => decoder,
        Err(cause) => {
            crate::log::write(
                Level::Warn,
                "sound",
                &format!("{} could not be decoded: {cause}", sound.file()),
            );
            return None;
        }
    };
    let (channels, rate) = (decoder.channels(), decoder.sample_rate());
    Some(SamplesBuffer::new(
        channels,
        rate,
        decoder.collect::<Vec<_>>(),
    ))
}

/// The clip for a move: a capture sounds different from a quiet move.
pub fn move_sound(san: &str, cx: &mut App) {
    play(
        if san.contains('x') {
            Sound::Capture
        } else {
            Sound::Move
        },
        cx,
    );
}

/// Play a clip at the volume Settings chose, unless sound is off.
pub fn play(sound: Sound, cx: &mut App) {
    play_at(sound, 1., cx);
}

/// The clips asked for, in order, once a test sets it.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct Played(pub Vec<Sound>);

#[cfg(any(test, feature = "test-support"))]
impl Global for Played {}

/// Play a clip at `gain` times the chosen volume (the trainers' quieter low-time beep).
pub fn play_at(sound: Sound, gain: f64, cx: &mut App) {
    #[cfg(any(test, feature = "test-support"))]
    if cx.has_global::<Played>() {
        cx.global_mut::<Played>().0.push(sound);
    }
    let frontend = &crate::settings::frontend(cx);
    if !frontend.sound_enabled {
        return;
    }
    let volume = (frontend.sound_volume.clamp(0., 1.) * gain) as f32;
    if let Some(player) = cx.try_global::<Player>() {
        let _ = player.0.send((sound, volume));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test that asks to hear the clips gets each one played, in order.
    #[gpui_kit::test]
    fn the_clips_played_are_kept_for_a_test(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            kchess_app::settings::init_for_tests(cx);
            cx.set_global(Played::default());
            play_at(Sound::Capture, 1., cx);
            play_at(Sound::LowTime, 0.5, cx);
            assert!(matches!(
                cx.global::<Played>().0.as_slice(),
                [Sound::Capture, Sound::LowTime]
            ));
        });
    }

    /// Opens device number `next`, keeping each one's ended flag for the test to fire.
    fn open_counting<'a>(
        next: &'a mut u32,
        flags: &'a mut Vec<Arc<AtomicBool>>,
    ) -> impl FnOnce(Arc<AtomicBool>) -> Result<u32, &'static str> + 'a {
        move |ended| {
            *next += 1;
            flags.push(ended);
            Ok(*next)
        }
    }

    /// Windows' default output changed (a Bluetooth speaker reconnected) and ended the stream:
    /// the next move opened the device again instead of playing into the dead stream for good.
    #[test]
    fn a_stream_that_ended_is_opened_again_at_the_next_sound() {
        let (mut next, mut flags) = (0, Vec::new());
        let mut output = Output::default();
        assert_eq!(output.ready(open_counting(&mut next, &mut flags)), Some(&1));
        assert_eq!(output.ready(open_counting(&mut next, &mut flags)), Some(&1));

        stream_error(
            &flags[0],
            &cpal::Error::new(cpal::ErrorKind::StreamInvalidated),
        );
        assert_eq!(output.ready(open_counting(&mut next, &mut flags)), Some(&2));
        assert_eq!(output.ready(open_counting(&mut next, &mut flags)), Some(&2));

        stream_error(
            &flags[1],
            &cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable),
        );
        assert_eq!(output.ready(open_counting(&mut next, &mut flags)), Some(&3));
    }

    /// A glitch, a reroute the stream follows, or a refused priority leaves the stream playing.
    #[test]
    fn a_stream_that_survives_its_error_is_kept() {
        let (mut next, mut flags) = (0, Vec::new());
        let mut output = Output::default();
        output.ready(open_counting(&mut next, &mut flags));
        for kind in [
            cpal::ErrorKind::Xrun,
            cpal::ErrorKind::DeviceChanged,
            cpal::ErrorKind::RealtimeDenied,
        ] {
            stream_error(&flags[0], &cpal::Error::new(kind));
        }
        assert_eq!(output.ready(open_counting(&mut next, &mut flags)), Some(&1));
    }

    /// No device yet (none plugged in): each sound tries again, and plays once one opens.
    #[test]
    fn a_device_that_failed_to_open_is_tried_at_each_sound() {
        let mut output = Output::<u32>::default();
        let mut tries = 0;
        for _ in 0..2 {
            let missing = output.ready(|_| {
                tries += 1;
                Err("no device")
            });
            assert_eq!(missing, None);
        }
        assert_eq!(output.ready(|_| Ok::<_, &str>(7)), Some(&7));
        assert_eq!(tries, 2);
    }

    #[test]
    fn every_clip_decodes() {
        for sound in [
            Sound::Move,
            Sound::Capture,
            Sound::LowTime,
            Sound::GenericNotify,
        ] {
            let clip = decode(sound).unwrap_or_else(|| panic!("{} did not decode", sound.file()));
            assert!(
                clip.total_duration()
                    .is_some_and(|length| !length.is_zero())
            );
        }
    }
}

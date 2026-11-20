//! Move and game sounds: the move, capture,
//! low-time and notification clips. A sound thread owns the
//! output device, opened at the first sound, and decodes each clip once.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::mpsc;

use gpui_kit::{App, Global};
use kchess_contracts::Level;
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, MixerDeviceSink, Source as _};

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
            let mut device: Option<MixerDeviceSink> = None;
            let mut clips: HashMap<Sound, SamplesBuffer> = HashMap::new();
            let mut failed = false;
            while let Ok((sound, volume)) = queue.recv() {
                if device.is_none() {
                    match rodio::DeviceSinkBuilder::open_default_sink() {
                        Ok(mut opened) => {
                            opened.log_on_drop(false);
                            device = Some(opened);
                        }
                        // Sound is optional: say so once, and try again at the next sound.
                        Err(cause) if !failed => {
                            failed = true;
                            crate::log::write(
                                Level::Warn,
                                "sound",
                                &format!("opening the sound device failed: {cause}"),
                            );
                            continue;
                        }
                        Err(_) => continue,
                    }
                }
                let clip = match clips.get(&sound) {
                    Some(clip) => clip.clone(),
                    None => match decode(sound) {
                        Some(clip) => clips.entry(sound).or_insert(clip).clone(),
                        None => continue,
                    },
                };
                if let Some(device) = &device {
                    device.mixer().add(clip.amplify(volume));
                }
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

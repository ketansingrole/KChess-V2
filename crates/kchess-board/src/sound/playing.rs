//! Windows' "playing" signal, for the Bluetooth speakers that stay silent without it.
//!
//! Some Bluetooth speakers play what the PC sends only while Windows tells them media is playing
//! (the play/pause state it passes them, as a browser playing a video does). Measured 2026-11-21 on
//! an Echo Show 5: Windows' meter showed a move sound and a 10-second tone reaching the speaker at
//! full level and nothing came out, yet both were heard at once while a muted video played, and
//! while a script only reported "playing" to Windows. So while the default output is a Bluetooth
//! one and no other app is playing media, a sound turns the signal on: KChess shows in Windows'
//! media controls until a few minutes pass without a sound.
//!
//! Windows passes the speaker the state of its current session only, and lists no session whose
//! buttons are all off: with none on, a paused browser tab stayed current and the Echo stayed
//! silent. So Pause is on, and pressing it (keyboard, flyout or speaker) makes KChess step aside,
//! giving the media keys back to the app that had them.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait as _, HostTrait as _};
use kchess_contracts::Level;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as Sessions,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as SessionStatus,
};
use windows::Media::Playback::MediaPlayer;
use windows::Media::{MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls};
use windows::core::{HSTRING, Result};

/// How long the signal stays on after the last sound: long enough for the moves of a game, so
/// only the first sound after a pause waits for the speaker.
const IDLE: Duration = Duration::from_secs(180);

/// How long the first sound waits once the signal comes on, so the speaker has started to play.
/// The Echo Show played a click sent as the signal came on, in both of two tries; this is margin.
const OPENING: Duration = Duration::from_millis(100);

/// What the signal does next. Pure, so the tests drive it without Windows' media controls.
#[derive(Debug, Default)]
struct Signal {
    /// The last sound while the signal is on; `None` while it is off.
    last_sound: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Keep,
    TurnOn,
    TurnOff,
}

impl Signal {
    /// A sound is about to play. `bluetooth`: the default output is a Bluetooth one.
    /// `others_playing` is asked only when the signal would come on: another app playing media has
    /// already told the speaker, and keeps Windows' media keys.
    fn sound(
        &mut self,
        now: Instant,
        bluetooth: bool,
        others_playing: impl FnOnce() -> bool,
    ) -> Step {
        match (self.last_sound.is_some(), bluetooth) {
            (true, true) => {
                self.last_sound = Some(now);
                Step::Keep
            }
            // The output moved off Bluetooth (headphones plugged in): the signal has nothing to do.
            (true, false) => {
                self.last_sound = None;
                Step::TurnOff
            }
            (false, true) if !others_playing() => {
                self.last_sound = Some(now);
                Step::TurnOn
            }
            (false, _) => Step::Keep,
        }
    }

    /// Pause was pressed while KChess was Windows' current session: whoever pressed it meant
    /// other media, so the signal goes off and the keys reach that app again.
    fn pressed(&mut self) -> Step {
        if self.last_sound.take().is_some() {
            Step::TurnOff
        } else {
            Step::Keep
        }
    }

    /// When the signal goes off if no sound comes first.
    fn deadline(&self) -> Option<Instant> {
        self.last_sound.map(|last| last + IDLE)
    }

    /// Time passed: off once [`IDLE`] has gone by without a sound.
    fn tick(&mut self, now: Instant) -> Step {
        if self.deadline().is_some_and(|deadline| now >= deadline) {
            self.last_sound = None;
            return Step::TurnOff;
        }
        Step::Keep
    }
}

/// The key under Windows' audio endpoints for an output's endpoint id (`{0.0.0.00000000}.{guid}`,
/// as cpal's `DeviceId::id` gives it): its last braced part.
fn endpoint_key(endpoint: &str) -> Option<&str> {
    let start = endpoint.rfind('{')?;
    let key = &endpoint[start..];
    (key.len() > 2 && key.ends_with('}')).then_some(key)
}

/// Whether Windows' default output is a Bluetooth one: its endpoint's enumerator, as Windows
/// records it, is `BTHENUM` (a wired or HDMI output's is `HDAUDIO` or `USB`).
pub(super) fn default_is_bluetooth() -> bool {
    const ENUMERATOR: &str = "{a45c254e-df1c-4efd-8020-67d146a850e0},24";
    let Some(device) = cpal::default_host().default_output_device() else {
        return false;
    };
    let Ok(id) = device.id() else {
        return false;
    };
    let Some(key) = endpoint_key(id.id()) else {
        return false;
    };
    windows_registry::LOCAL_MACHINE
        .open(format!(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render\{key}\Properties"
        ))
        .and_then(|properties| properties.get_string(ENUMERATOR))
        .is_ok_and(|enumerator| enumerator.eq_ignore_ascii_case("BTHENUM"))
}

enum Message {
    /// A sound is about to play; the answer is how long it waits.
    Sound {
        bluetooth: bool,
        reply: mpsc::Sender<Duration>,
    },
    /// Pause was pressed in Windows' media controls.
    Pressed,
}

/// The thread that owns KChess's media session, away from the COM apartment cpal sets up on the
/// sound thread; the sound thread asks it before each sound.
pub(super) struct Playing(mpsc::Sender<Message>);

impl Playing {
    pub(super) fn start() -> std::io::Result<Self> {
        let (sender, queue) = mpsc::channel();
        let presses = sender.clone();
        std::thread::Builder::new()
            .name("kchess-playing".into())
            .spawn(move || run(&queue, presses))?;
        Ok(Self(sender))
    }

    /// A sound is about to play: how long it waits for the speaker (only when the signal has
    /// just come on). Never longer than [`OPENING`], whatever the thread does.
    pub(super) fn before_sound(&self, bluetooth: bool) -> Duration {
        let (reply, answer) = mpsc::channel();
        if self.0.send(Message::Sound { bluetooth, reply }).is_err() {
            return Duration::ZERO;
        }
        answer
            .recv_timeout(OPENING * 5)
            .unwrap_or(Duration::ZERO)
            .min(OPENING)
    }
}

fn run(queue: &mpsc::Receiver<Message>, presses: mpsc::Sender<Message>) {
    let mut signal = Signal::default();
    let mut session = Session::new(presses);
    loop {
        let next = match signal.deadline() {
            Some(deadline) => {
                queue.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
            None => queue
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(Message::Sound { bluetooth, reply }) => {
                let step = signal.sound(Instant::now(), bluetooth, || session.others_playing());
                let wait = if session.apply(&step) && step == Step::TurnOn {
                    OPENING
                } else {
                    Duration::ZERO
                };
                // The sound thread stopped waiting: the sound plays without the pause.
                reply.send(wait).ok();
            }
            Ok(Message::Pressed) => {
                let step = signal.pressed();
                session.apply(&step);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let step = signal.tick(Instant::now());
                session.apply(&step);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// KChess's entry in Windows' media controls: made at the first time it is needed, so nobody
/// whose output is not Bluetooth ever gets one.
struct Session {
    controls: Option<(MediaPlayer, SystemMediaTransportControls)>,
    /// Where a press of Pause goes: the thread's own queue.
    presses: mpsc::Sender<Message>,
    /// A failure since the last success; said once.
    failed: bool,
}

impl Session {
    fn new(presses: mpsc::Sender<Message>) -> Self {
        Self {
            controls: None,
            presses,
            failed: false,
        }
    }

    /// Carries out a step; `true` when the signal is as the step wants it.
    fn apply(&mut self, step: &Step) -> bool {
        let done = match step {
            Step::Keep => return true,
            Step::TurnOn => self.controls().and_then(|controls| {
                controls.SetIsEnabled(true)?;
                controls.SetPlaybackStatus(MediaPlaybackStatus::Playing)
            }),
            Step::TurnOff => match &self.controls {
                Some((_, controls)) => controls
                    .SetPlaybackStatus(MediaPlaybackStatus::Stopped)
                    .and_then(|()| controls.SetIsEnabled(false)),
                None => Ok(()),
            },
        };
        self.said(
            done,
            "Windows' media controls refused KChess's playing state",
        )
    }

    fn controls(&mut self) -> Result<SystemMediaTransportControls> {
        if let Some((_, controls)) = &self.controls {
            return Ok(controls.clone());
        }
        let player = MediaPlayer::new()?;
        // KChess plays nothing through the player: its own commands would answer the buttons.
        player.CommandManager()?.SetIsEnabled(false)?;
        let controls = player.SystemMediaTransportControls()?;
        for enable in [
            SystemMediaTransportControls::SetIsPlayEnabled,
            SystemMediaTransportControls::SetIsStopEnabled,
            SystemMediaTransportControls::SetIsNextEnabled,
            SystemMediaTransportControls::SetIsPreviousEnabled,
        ] {
            enable(&controls, false)?;
        }
        // Windows lists no session without a button, and a session it does not list never tells
        // the speaker it is playing.
        controls.SetIsPauseEnabled(true)?;
        let presses = self.presses.clone();
        controls.ButtonPressed(&TypedEventHandler::new(move |_, _| {
            // The thread ended with the app: nothing left to step aside.
            presses.send(Message::Pressed).ok();
            Ok(())
        }))?;
        let display = controls.DisplayUpdater()?;
        display.SetType(MediaPlaybackType::Music)?;
        display
            .MusicProperties()?
            .SetTitle(&HSTRING::from("KChess"))?;
        display.Update()?;
        self.controls = Some((player, controls.clone()));
        Ok(controls)
    }

    /// Whether another app is playing media now. Unknown counts as no: the signal then comes
    /// on beside it, which only costs KChess an entry in the media controls.
    fn others_playing(&mut self) -> bool {
        let playing = Sessions::RequestAsync()
            .and_then(|request| request.join())
            .and_then(|sessions| sessions.GetSessions())
            .and_then(|sessions| {
                for index in 0..sessions.Size()? {
                    let status = sessions.GetAt(index)?.GetPlaybackInfo()?.PlaybackStatus()?;
                    if status == SessionStatus::Playing {
                        return Ok(true);
                    }
                }
                Ok(false)
            });
        let known = playing.as_ref().copied().unwrap_or(false);
        self.said(
            playing.map(|_| ()),
            "Windows' media sessions could not be read",
        );
        known
    }

    fn said(&mut self, done: Result<()>, what: &str) -> bool {
        match done {
            Ok(()) => {
                self.failed = false;
                true
            }
            Err(cause) => {
                if !self.failed {
                    self.failed = true;
                    crate::log::write(Level::Warn, "sound", &format!("{what}: {cause}"));
                }
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Echo Show sequence: the first sound turns the signal on, the next ones keep it, and
    /// three quiet minutes turn it off; the next sound turns it on again.
    #[test]
    fn a_sound_on_bluetooth_turns_the_signal_on_until_a_quiet_spell() {
        let start = Instant::now();
        let mut signal = Signal::default();
        assert_eq!(signal.sound(start, true, || false), Step::TurnOn);
        let later = start + Duration::from_secs(30);
        assert_eq!(signal.sound(later, true, || unreachable!()), Step::Keep);
        assert_eq!(signal.deadline(), Some(later + IDLE));
        assert_eq!(
            signal.tick(later + IDLE - Duration::from_secs(1)),
            Step::Keep
        );
        assert_eq!(signal.tick(later + IDLE), Step::TurnOff);
        assert_eq!(signal.deadline(), None);
        assert_eq!(signal.sound(later + IDLE * 2, true, || false), Step::TurnOn);
    }

    /// Another app already playing media has told the speaker, and keeps the media keys.
    #[test]
    fn media_already_playing_leaves_the_signal_off() {
        let mut signal = Signal::default();
        assert_eq!(signal.sound(Instant::now(), true, || true), Step::Keep);
        assert_eq!(signal.deadline(), None);
    }

    /// Pause pressed to resume a paused video reached KChess, Windows' current session: KChess
    /// steps aside so the next press reaches the video; a press with the signal off does nothing.
    #[test]
    fn pause_pressed_turns_the_signal_off() {
        let now = Instant::now();
        let mut signal = Signal::default();
        assert_eq!(signal.pressed(), Step::Keep);
        assert_eq!(signal.sound(now, true, || false), Step::TurnOn);
        assert_eq!(signal.pressed(), Step::TurnOff);
        assert_eq!(signal.deadline(), None);
        // The video now plays: the next move leaves it the keys.
        assert_eq!(signal.sound(now, true, || true), Step::Keep);
    }

    /// Wired speakers and headphones play everything: no entry in the media controls for them,
    /// and none left behind when the output moves off Bluetooth.
    #[test]
    fn an_output_that_is_not_bluetooth_never_has_the_signal() {
        let now = Instant::now();
        let mut signal = Signal::default();
        assert_eq!(signal.sound(now, false, || unreachable!()), Step::Keep);
        assert_eq!(signal.sound(now, true, || false), Step::TurnOn);
        assert_eq!(signal.sound(now, false, || unreachable!()), Step::TurnOff);
        assert_eq!(signal.deadline(), None);
    }

    #[test]
    fn an_endpoint_id_names_its_key() {
        assert_eq!(
            endpoint_key("{0.0.0.00000000}.{c260f4c5-792f-4684-8af9-c124697a7553}"),
            Some("{c260f4c5-792f-4684-8af9-c124697a7553}")
        );
        assert_eq!(endpoint_key("default"), None);
        assert_eq!(endpoint_key("{0.0.0.00000000}.{"), None);
    }
}

//! The stand-alone chess clock for a real board (`useOtbClock` in the Electron renderer's
//! `stores/clock.ts`, which still keeps its own copy): two faces, top (the player across the table)
//! and bottom, each with its own minutes and a shared increment. Times are milliseconds; `at` is
//! the host's monotonic clock.

/// A face of the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Bottom,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }
}

/// A value for each face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Faces<T> {
    pub top: T,
    pub bottom: T,
}

impl<T: Copy> Faces<T> {
    pub fn get(&self, side: Side) -> T {
        match side {
            Side::Top => self.top,
            Side::Bottom => self.bottom,
        }
    }

    pub fn set(&mut self, side: Side, value: T) {
        match side {
            Side::Top => self.top = value,
            Side::Bottom => self.bottom = value,
        }
    }
}

/// The times set before a session: the top player's minutes, the bottom's, and the increment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub minutes: f64,
    pub bottom_minutes: f64,
    pub increment: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OtbClock {
    pub config: Config,
    /// Each face's time when it last stopped.
    left: Faces<f64>,
    pub running: Option<Side>,
    pub flagged: Option<Side>,
    pub moves: Faces<u32>,
    /// When the running face started.
    started: f64,
    /// Whose clock a pause stopped: resuming restarts that side.
    paused: Option<Side>,
    /// The times shown before anyone pressed: they follow the minutes as they are typed.
    shown_start: Faces<f64>,
}

impl Default for OtbClock {
    fn default() -> OtbClock {
        OtbClock::new(Config {
            minutes: 5.,
            bottom_minutes: 5.,
            increment: 3.,
        })
    }
}

impl OtbClock {
    pub fn new(config: Config) -> OtbClock {
        let start = Faces {
            top: config.minutes * 60_000.,
            bottom: config.bottom_minutes * 60_000.,
        };
        OtbClock {
            config,
            left: start,
            running: None,
            flagged: None,
            moves: Faces { top: 0, bottom: 0 },
            started: 0.,
            paused: None,
            shown_start: start,
        }
    }

    /// A face's time at `at`.
    pub fn left(&self, side: Side, at: f64) -> f64 {
        if self.running != Some(side) {
            return self.left.get(side);
        }
        (self.left.get(side) - (at - self.started)).max(0.)
    }

    /// Let the running face's flag fall when its time is up; true when it did.
    pub fn expire(&mut self, at: f64) -> bool {
        let Some(side) = self.running else {
            return false;
        };
        if self.left(side, at) > 0. {
            return false;
        }
        self.left.set(side, 0.);
        self.flagged = Some(side);
        self.running = None;
        true
    }

    /// A player pressed their face: theirs stops (plus the increment), the other side's starts.
    /// True when a flag fell instead.
    pub fn press(&mut self, side: Side, at: f64) -> bool {
        if self.flagged.is_some() {
            return false;
        }
        if self.expire(at) {
            return true;
        }
        if self.running.is_some_and(|running| running != side) {
            return false;
        }
        if self.running == Some(side) {
            let left = self.left(side, at) + self.config.increment * 1000.;
            self.left.set(side, left);
            self.moves.set(side, self.moves.get(side) + 1);
        }
        self.started = at;
        self.paused = None;
        self.running = Some(side.other());
        false
    }

    /// Stop the running face. True when a flag fell instead.
    pub fn pause(&mut self, at: f64) -> bool {
        if self.expire(at) {
            return true;
        }
        let Some(side) = self.running else {
            return false;
        };
        self.left.set(side, self.left(side, at));
        self.paused = Some(side);
        self.running = None;
        false
    }

    /// Back to the set times, nobody running.
    pub fn reset(&mut self) {
        self.paused = None;
        self.running = None;
        self.flagged = None;
        self.moves = Faces { top: 0, bottom: 0 };
        self.left = Faces {
            top: self.config.minutes * 60_000.,
            bottom: self.config.bottom_minutes * 60_000.,
        };
        self.shown_start = self.left;
    }

    /// Start (or resume) the clock; after a flag it begins a fresh session at the set times.
    pub fn start(&mut self, at: f64) -> bool {
        if self.flagged.is_some() {
            self.reset();
        }
        if self.running.is_some() {
            return false;
        }
        if let Some(side) = self.paused.take() {
            self.started = at;
            self.running = Some(side);
            return false;
        }
        self.press(Side::Top, at)
    }

    /// New set times. A clock nobody has started shows them at once; a started one keeps its time.
    pub fn configure(&mut self, config: Config) {
        self.config = config;
        if self.running.is_some() || self.flagged.is_some() {
            return;
        }
        for (side, minutes) in [
            (Side::Top, config.minutes),
            (Side::Bottom, config.bottom_minutes),
        ] {
            let untouched =
                self.moves.get(side) == 0 && self.left.get(side) == self.shown_start.get(side);
            if minutes > 0. && untouched {
                self.left.set(side, minutes * 60_000.);
                self.shown_start.set(side, minutes * 60_000.);
            }
        }
    }

    /// Something has happened worth keeping in Game history.
    pub fn has_play(&self) -> bool {
        self.moves.top + self.moves.bottom > 0 || self.flagged.is_some()
    }

    /// How the session ended, for Game history.
    pub fn reason(&self) -> String {
        match self.flagged {
            Some(Side::Top) => "Top player ran out of time".into(),
            Some(Side::Bottom) => "Bottom player ran out of time".into(),
            None => "Clock session".into(),
        }
    }

    /// The time control as Game history shows it, bottom first.
    pub fn time_control(&self) -> String {
        format!(
            "{}+{} / {}+{}",
            self.config.bottom_minutes * 60.,
            self.config.increment,
            self.config.minutes * 60.,
            self.config.increment
        )
    }

    /// The moves and time left on each face, for Game history.
    pub fn summary(&self) -> String {
        format!(
            "Bottom: {} moves · {}s left; Top: {} moves · {}s left",
            self.moves.bottom,
            (self.left.bottom / 1000.).ceil(),
            self.moves.top,
            (self.left.top / 1000.).ceil()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> OtbClock {
        OtbClock::new(Config {
            minutes: 5.,
            bottom_minutes: 5.,
            increment: 3.,
        })
    }

    #[test]
    fn pressing_hands_the_turn_over_with_the_increment() {
        let mut clock = clock();
        assert!(!clock.start(0.));
        assert_eq!(clock.running, Some(Side::Bottom));
        assert_eq!(clock.left(Side::Bottom, 10_000.), 290_000.);
        // Only the running side's press counts.
        assert!(!clock.press(Side::Top, 10_000.));
        assert_eq!(clock.running, Some(Side::Bottom));
        clock.press(Side::Bottom, 10_000.);
        assert_eq!(clock.left(Side::Bottom, 99_000.), 293_000.);
        assert_eq!(clock.moves.bottom, 1);
        assert_eq!(clock.running, Some(Side::Top));
    }

    #[test]
    fn a_pause_resumes_the_side_it_stopped() {
        let mut clock = clock();
        clock.start(0.);
        clock.press(Side::Bottom, 1_000.);
        clock.pause(2_000.);
        assert_eq!(clock.running, None);
        assert_eq!(clock.left(Side::Top, 50_000.), 299_000.);
        clock.start(50_000.);
        assert_eq!(clock.running, Some(Side::Top));
        assert_eq!(clock.left(Side::Top, 51_000.), 298_000.);
    }

    #[test]
    fn a_flag_falls_once_and_a_new_start_resets() {
        let mut clock = OtbClock::new(Config {
            minutes: 1.,
            bottom_minutes: 1.,
            increment: 0.,
        });
        clock.start(0.);
        assert!(clock.expire(60_000.));
        assert_eq!(clock.flagged, Some(Side::Bottom));
        assert!(!clock.expire(61_000.));
        assert!(!clock.press(Side::Bottom, 62_000.));
        assert_eq!(clock.reason(), "Bottom player ran out of time");
        clock.start(70_000.);
        assert_eq!(clock.flagged, None);
        assert_eq!(clock.running, Some(Side::Bottom));
    }

    #[test]
    fn set_times_show_until_the_clock_is_used() {
        let mut clock = clock();
        clock.configure(Config {
            minutes: 10.,
            bottom_minutes: 3.,
            increment: 2.,
        });
        assert_eq!(clock.left(Side::Top, 0.), 600_000.);
        assert_eq!(clock.left(Side::Bottom, 0.), 180_000.);
        assert_eq!(clock.time_control(), "180+2 / 600+2");
        clock.start(0.);
        clock.configure(Config {
            minutes: 1.,
            bottom_minutes: 1.,
            increment: 0.,
        });
        assert_eq!(clock.left(Side::Top, 0.), 600_000.);
    }
}

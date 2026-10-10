//! The client clock (`crates/kchess-wasm/js/clock.ts`, modelled on lila's clockCtrl): the server sends
//! authoritative times, and the client interpolates from them. Every transition takes `now`, so
//! the state is plain data and the rules are deterministic.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Out, js_max, js_min, num_out, pad2, text_arg, text_of, to_value, value_arg};

/// A value per side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct Sides<T> {
    pub white: T,
    pub black: T,
}

impl<T: Copy> Sides<T> {
    fn get(&self, color: &str) -> Option<T> {
        match color {
            "white" => Some(self.white),
            "black" => Some(self.black),
            _ => None,
        }
    }

    fn set(&mut self, color: &str, value: T) {
        match color {
            "white" => self.white = value,
            "black" => self.black = value,
            _ => {}
        }
    }
}

/// `Clock`'s state.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockState {
    pub times: Sides<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticking: Option<String>,
    pub last_update: f64,
    pub emerg_ms: f64,
    pub alerted: Sides<bool>,
}

/// `set(data)`: the server's times for both sides.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetData {
    pub white: f64,
    pub black: f64,
    #[serde(default)]
    pub ticking: Option<String>,
    #[serde(default)]
    pub initial_seconds: Option<f64>,
    #[serde(default)]
    pub delay_centis: Option<f64>,
}

/// lila's low-time threshold: min(60, 12.5% of initial), at least 10s (2s for short games).
fn threshold(initial: f64) -> f64 {
    1000.0
        * js_min(
            60.0,
            if initial < 60.0 {
                js_max(2.0, initial * 0.2)
            } else {
                js_max(10.0, initial * 0.125)
            },
        )
}

impl ClockState {
    /// A clock that has never been set.
    pub fn initial() -> Self {
        Self {
            times: Sides::default(),
            ticking: None,
            last_update: 0.0,
            emerg_ms: 20_000.0,
            alerted: Sides::default(),
        }
    }

    /// `set`
    pub fn set(&mut self, data: &SetData, now: f64) {
        self.times = Sides {
            white: js_max(0.0, data.white),
            black: js_max(0.0, data.black),
        };
        self.ticking = data.ticking.clone();
        self.last_update = now + data.delay_centis.unwrap_or(0.0) * 10.0;
        if let Some(initial) = data.initial_seconds.filter(|s| *s != 0.0 && !s.is_nan()) {
            self.emerg_ms = threshold(initial);
        }
        if let Some(color) = &data.ticking {
            self.alerted.set(color, false);
        }
    }

    /// `pause`: freezes both clocks at their remaining times.
    pub fn pause(&mut self, now: f64) {
        self.times = Sides {
            white: self.remaining("white", now),
            black: self.remaining("black", now),
        };
        self.ticking = None;
    }

    /// `remaining(color, now)`
    pub fn remaining(&self, color: &str, now: f64) -> f64 {
        let elapsed = if self.ticking.as_deref() == Some(color) {
            js_max(0.0, now - self.last_update)
        } else {
            0.0
        };
        js_max(0.0, self.times.get(color).unwrap_or(f64::NAN) - elapsed)
    }

    /// `lowTimeAlert(color, now)`: true once per player, when that clock drops below the threshold.
    pub fn low_time_alert(&mut self, color: &str, now: f64) -> bool {
        let alerted = self.alerted.get(color).unwrap_or(true);
        if alerted || self.remaining(color, now) > self.emerg_ms {
            return false;
        }
        self.alerted.set(color, true);
        true
    }
}

/// `formatClock`: `m:ss`, `h:mm:ss` past an hour, and days plus hours for correspondence clocks.
pub fn format_clock(millis: f64) -> String {
    let seconds = js_max(0.0, (millis / 1000.0).floor());
    if seconds >= 86_400.0 {
        let days = (seconds / 86_400.0).floor();
        let hours = ((seconds % 86_400.0) / 3600.0).floor();
        format!("{}d {}h", text_of(days), text_of(hours))
    } else if seconds >= 3600.0 {
        format!(
            "{}:{}:{}",
            text_of((seconds / 3600.0).floor()),
            pad2(((seconds % 3600.0) / 60.0).floor()),
            pad2(seconds % 60.0)
        )
    } else {
        format!(
            "{}:{}",
            text_of((seconds / 60.0).floor()),
            pad2(seconds % 60.0)
        )
    }
}

fn state_arg(args: &[Value]) -> Result<ClockState, String> {
    value_arg(args, 0, "state")
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "clockInitial" => to_value(ClockState::initial()),
        "clockSet" => state_arg(args).and_then(|mut state| {
            let data: SetData = value_arg(args, 1, "data")?;
            state.set(&data, super::number_arg(args, 2));
            to_value(state)
        }),
        "clockPause" => state_arg(args).and_then(|mut state| {
            state.pause(super::number_arg(args, 1));
            to_value(state)
        }),
        "clockRemaining" => state_arg(args).and_then(|state| {
            let color = text_arg(args, 1, "color")?;
            to_value(num_out(state.remaining(color, super::number_arg(args, 2))))
        }),
        "clockLowTimeAlert" => state_arg(args).and_then(|mut state| {
            let color = text_arg(args, 1, "color")?;
            let result = state.low_time_alert(color, super::number_arg(args, 2));
            to_value(Stepped { state, result })
        }),
        "formatClock" => Ok(Value::String(format_clock(super::number_arg(args, 0)))),
        _ => return None,
    };
    Some(result)
}

/// A transition's new state and its own result.
#[derive(Serialize)]
struct Stepped<T> {
    state: ClockState,
    result: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_interpolates_and_alerts_once() {
        let mut clock = ClockState::initial();
        clock.set(
            &SetData {
                white: 15_000.0,
                black: 60_000.0,
                ticking: Some("white".into()),
                initial_seconds: Some(60.0),
                delay_centis: None,
            },
            1000.0,
        );
        assert_eq!(clock.remaining("white", 1000.0 + 4000.0), 11_000.0);
        assert_eq!(clock.remaining("black", 9_000.0), 60_000.0);
        assert!(!clock.low_time_alert("white", 1000.0 + 4000.0));
        assert!(clock.low_time_alert("white", 1000.0 + 6000.0));
        assert!(!clock.low_time_alert("white", 1000.0 + 7000.0));
        clock.pause(1000.0 + 8000.0);
        assert_eq!(
            (clock.times.white, clock.ticking.as_deref()),
            (7000.0, None)
        );
    }

    #[test]
    fn clock_text() {
        assert_eq!(format_clock(9_000.0), "0:09");
        assert_eq!(format_clock(3_723_000.0), "1:02:03");
        assert_eq!(format_clock(90_000_000.0), "1d 1h");
        assert_eq!(format_clock(-5.0), "0:00");
    }
}

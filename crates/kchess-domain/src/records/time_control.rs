//! Lichess Board API time-control rules.

use crate::perf::Speed;

/// Lichess perf formula: estimated game length = initial time + 40 × increment.
pub fn total_seconds(minutes: f64, increment: f64) -> f64 {
    minutes * 60.0 + 40.0 * increment
}

/// The Lichess speed of a time control.
pub fn speed_for(minutes: f64, increment: f64) -> Speed {
    let total = total_seconds(minutes, increment);
    if total < 30.0 {
        Speed::UltraBullet
    } else if total < 180.0 {
        Speed::Bullet
    } else if total < 480.0 {
        Speed::Blitz
    } else if total < 1500.0 {
        Speed::Rapid
    } else {
        Speed::Classical
    }
}

/// The name of a time control's Lichess perf: "Blitz".
pub fn perf_for(minutes: f64, increment: f64) -> &'static str {
    speed_for(minutes, increment).label()
}

/// Public matchmaking (`POST /api/board/seek`): Rapid and slower only.
pub fn can_board_seek(minutes: f64, increment: f64) -> bool {
    total_seconds(minutes, increment) >= 480.0
}

/// Direct challenge (`POST /api/challenge/{username}`): Blitz and slower.
pub fn can_direct_challenge(minutes: f64, increment: f64) -> bool {
    total_seconds(minutes, increment) >= 180.0
}

/// Whether the Board API plays this clock: a challenge to someone (`targeted`) from Blitz, a
/// seek from Rapid.
pub fn can_play_online(minutes: f64, increment: f64, targeted: bool) -> bool {
    if targeted {
        can_direct_challenge(minutes, increment)
    } else {
        can_board_seek(minutes, increment)
    }
}

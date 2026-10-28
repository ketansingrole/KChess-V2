//! Choice lists the core accepts, as numbers.

/// Days a correspondence game allows per move.
pub const CORRESPONDENCE_DAYS: &[f64] = &[1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0];
/// Arena clock minutes.
pub const ARENA_CLOCK_MINUTES: &[f64] = &[
    0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 15.0, 20.0, 25.0,
    30.0, 40.0, 50.0, 60.0,
];
/// Arena clock increments, in seconds.
pub const ARENA_INCREMENTS: &[f64] = &[
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 10.0, 15.0, 20.0, 25.0, 30.0, 40.0, 50.0, 60.0,
];
/// Arena durations, in minutes.
pub const ARENA_DURATIONS: &[f64] = &[
    20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0,
    150.0, 180.0, 210.0, 240.0, 270.0, 300.0, 330.0, 360.0, 420.0, 480.0, 540.0, 600.0, 720.0,
];
/// Minutes before an arena starts.
pub const ARENA_WAIT_MINUTES: &[f64] = &[1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 30.0, 45.0, 60.0];
/// Rating buckets of the opening explorer.
pub const EXPLORER_RATINGS: &[f64] = &[
    400.0, 1000.0, 1200.0, 1400.0, 1600.0, 1800.0, 2000.0, 2200.0, 2500.0,
];

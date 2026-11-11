//! Tables the validators check against: the contract types, settings, variants and library
//! documents.

pub const APPEARANCES: &[&str] = &["system", "light", "dark"];
pub const COORDINATE_MODES: &[&str] = &["none", "inside", "outside"];
/// The efforts every AI feature can run at.
pub const AI_EFFORTS: &[&str] = &["auto", "low", "medium", "high"];
pub const ENGINE_LEVELS: &[&str] = &[
    "beginner",
    "novice",
    "casual",
    "club",
    "strong-club",
    "expert",
    "cm",
    "fm",
    "im",
    "gm",
    "super-gm",
    "max",
];
pub const PROMOTION_MODES: &[&str] = &["ask", "queen", "premove"];
pub const PIECE_ANIMATIONS: &[&str] = &["none", "fast", "normal", "slow"];
pub const ONLINE_ACTIONS: &[&str] = &[
    "resign",
    "abort",
    "takeback",
    "declineTakeback",
    "offerDraw",
    "acceptDraw",
    "declineDraw",
    "claimVictory",
    "claimDraw",
    "berserk",
];
/// The days per move a correspondence game may give.
pub const CORRESPONDENCE_DAYS: [f64; 7] = [1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0];
pub const DECLINE_REASONS: &[&str] = &[
    "generic",
    "later",
    "tooFast",
    "tooSlow",
    "timeControl",
    "rated",
    "casual",
    "standard",
    "variant",
];
pub const CHAT_ROOMS: &[&str] = &["player", "spectator"];
pub const RELATION_ACTIONS: &[&str] = &["follow", "unfollow", "block", "unblock"];
pub const CHALLENGE_COLORS: &[&str] = &["random", "white", "black"];
pub const PUZZLE_DIFFICULTIES: &[&str] = &["easiest", "easier", "normal", "harder", "hardest"];
pub const RUN_KINDS: &[&str] = &[
    "storm",
    "streak",
    "rush",
    "coordinates",
    "squareColor",
    "knightPath",
    "endgame",
    "lesson",
];
pub const PUZZLE_TRAIN_MODES: &[&str] = &["practice", "offline", "rated"];
pub const VOICE_SOURCES: &[&str] = &[
    "computer",
    "coordinates",
    "squareColor",
    "knightPath",
    "analysis",
    "editor",
    "local",
    "puzzles",
];
pub const VOICE_OUTCOMES: &[&str] = &[
    "played",
    "pending",
    "confirmed",
    "picked",
    "cancelled",
    "replaced",
    "abandoned",
    "invalid",
    "unclear",
    "command",
    "correct",
    "wrong",
];
pub const ARENA_CLOCK_MINUTES: &[f64] = &[
    0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 15.0, 20.0, 25.0,
    30.0, 40.0, 50.0, 60.0,
];
pub const ARENA_INCREMENTS: &[f64] = &[
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 10.0, 15.0, 20.0, 25.0, 30.0, 40.0, 50.0, 60.0,
];
pub const ARENA_DURATIONS: &[f64] = &[
    20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0,
    150.0, 180.0, 210.0, 240.0, 270.0, 300.0, 330.0, 360.0, 420.0, 480.0, 540.0, 600.0, 720.0,
];
pub const ARENA_WAIT_MINUTES: &[f64] = &[1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 30.0, 45.0, 60.0];
/// Arena variants Lichess accepts.
pub const ARENA_VARIANTS: &[&str] = &[
    "standard",
    "chess960",
    "crazyhouse",
    "antichess",
    "atomic",
    "horde",
    "kingOfTheHill",
    "racingKings",
    "threeCheck",
];
pub const PERF_TYPES: &[&str] = &[
    "ultraBullet",
    "bullet",
    "blitz",
    "rapid",
    "classical",
    "correspondence",
    "chess960",
    "kingOfTheHill",
    "threeCheck",
    "antichess",
    "atomic",
    "horde",
    "racingKings",
    "crazyhouse",
];
pub const REVIEW_AUTO: &[&str] = &["off", "recent", "all"];
pub const EXPLORER_SPEEDS: &[&str] = &[
    "ultraBullet",
    "bullet",
    "blitz",
    "rapid",
    "classical",
    "correspondence",
];
/// The rating groups of the Lichess opening explorer, in order.
pub const EXPLORER_RATINGS: [f64; 9] = [
    400.0, 1000.0, 1200.0, 1400.0, 1600.0, 1800.0, 2000.0, 2200.0, 2500.0,
];
/// Lichess variant keys KChess can play: the one list every crate checks a variant against.
pub const VARIANTS: &[&str] = &[
    "standard",
    "chess960",
    "kingOfTheHill",
    "threeCheck",
    "antichess",
    "atomic",
    "horde",
    "racingKings",
    "crazyhouse",
];
pub const SESSION_KINDS: &[&str] = &[
    "analysis",
    "local",
    "computer",
    "archive:computer",
    "archive:board",
    "archive:clock",
];
pub const TOURNAMENT_SYSTEMS: &[&str] = &["arena", "swiss"];
/// The Lichess TV channels: key (as their links and the API name them) and label, in display
/// order.
pub const TV_CHANNELS: [(&str, &str); 16] = [
    ("best", "Top rated"),
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("ultraBullet", "UltraBullet"),
    ("chess960", "Chess960"),
    ("kingOfTheHill", "King of the Hill"),
    ("threeCheck", "Three-check"),
    ("antichess", "Antichess"),
    ("atomic", "Atomic"),
    ("horde", "Horde"),
    ("racingKings", "Racing Kings"),
    ("crazyhouse", "Crazyhouse"),
    ("bot", "Bots"),
    ("computer", "Computer"),
];

/// Largest serialized document the core stores (`MAX_DOCUMENT`).
pub const MAX_DOCUMENT: usize = 2_000_000;
pub const MAX_CHAPTERS: usize = 64;
pub const MAX_MOVES: usize = 1024;
/// `Number.MAX_SAFE_INTEGER`
pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

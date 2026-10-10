//! Tables the validators check against, copied from `crates/kchess-contracts/ts/types.ts`,
//! `contracts/settings.ts`, `domain/variant.ts` and `domain/library.ts`.

pub const APPEARANCES: &[&str] = &["system", "light", "dark"];
pub const COORDINATE_MODES: &[&str] = &["none", "inside", "outside"];
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
pub const NOTIFICATION_KINDS: &[&str] = &[
    "opponentMove",
    "lowTime",
    "gameEvents",
    "computerMove",
    "challenge",
    "test",
];
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
pub const CORRESPONDENCE_DAYS: &[f64] = &[1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0];
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
];
pub const VOICE_SOURCES: &[&str] = &["computer", "coordinates", "analysis", "editor", "local"];
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
/// Arena variants Lichess accepts (crazyhouse included, which KChess cannot play).
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
pub const EXPLORER_RATINGS: &[f64] = &[
    400.0, 1000.0, 1200.0, 1400.0, 1600.0, 1800.0, 2000.0, 2200.0, 2500.0,
];
/// Lichess variant keys KChess can play (`domain/variant.ts`).
pub const VARIANTS: &[&str] = &[
    "standard",
    "chess960",
    "kingOfTheHill",
    "threeCheck",
    "antichess",
    "atomic",
    "horde",
    "racingKings",
];
/// The settings the core keeps (`contracts/settings.ts`), in its order.
pub const CORE_SETTINGS_KEYS: &[&str] = &[
    "enginePath",
    "engineLevels",
    "reviewAuto",
    "reviewOnBattery",
    "receiveChallenges",
    "onlineChat",
    "correspondencePoll",
    "cloudEval",
    "voiceHistory",
];
pub const SESSION_KINDS: &[&str] = &[
    "analysis",
    "local",
    "computer",
    "archive:computer",
    "archive:board",
    "archive:clock",
];
/// Earlier releases' renderer storage keys (`domain/library.ts`), in its order.
pub const LEGACY_DOCUMENT_KEYS: &[&str] = &[
    "kchess:studies:v1",
    "kchess:game-history:v1",
    "kchess:mistakes:v1",
    "kchess:analysis:v1",
    "kchess:local:v1",
    "kchess:computer:v1",
    "kchess:history-session:computer",
    "kchess:history-session:board",
    "kchess:history-session:clock",
    "kchess:tournaments-joined",
    "kchess:repertoire-misses",
];
pub const TOURNAMENT_SYSTEMS: &[&str] = &["arena", "swiss"];

/// The renderer pages diagnostics may name (`RENDERER_ROUTES`, `apps/desktop/contracts/rendererDiagnostics.ts`).
pub const RENDERER_ROUTES: &[&str] = &[
    "/",
    "/online",
    "/tournaments",
    "/watch",
    "/local",
    "/computer",
    "/analysis",
    "/studies",
    "/editor",
    "/puzzles",
    "/practice",
    "/history",
    "/insights",
    "/friends",
    "/players",
    "/settings",
];

/// The timings the renderer may record (`PERFORMANCE_NAMES`): fixed names, then one per route.
pub fn performance_names() -> Vec<String> {
    ["app.ready", "board.frame", "voice.activation"]
        .into_iter()
        .map(String::from)
        .chain(
            RENDERER_ROUTES
                .iter()
                .map(|route| format!("page.navigation:{route}")),
        )
        .collect()
}
/// Largest serialized document the core stores (`MAX_DOCUMENT`).
pub const MAX_DOCUMENT: usize = 2_000_000;
pub const MAX_CHAPTERS: usize = 64;
pub const MAX_MOVES: usize = 1024;
/// Largest export the core saves (`EXPORT_MAX`).
pub const EXPORT_MAX: usize = 40 * 1024 * 1024;
/// `Number.MAX_SAFE_INTEGER`
pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

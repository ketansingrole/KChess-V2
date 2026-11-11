//! The string unions domain rules read, and the tables the validators check against: the
//! contract types, settings, variants and library documents. The contracts re-export the unions.

crate::wire_enum!(
    /// Which color scheme the app uses.
    Appearance, APPEARANCES {
        System = "system",
        Light = "light",
        Dark = "dark",
    }
);
crate::wire_enum!(
    /// Built-in engine strength, from beginner to max.
    EngineLevel, ENGINE_LEVELS {
        Beginner = "beginner",
        Novice = "novice",
        Casual = "casual",
        Club = "club",
        StrongClub = "strong-club",
        Expert = "expert",
        Cm = "cm",
        Fm = "fm",
        Im = "im",
        Gm = "gm",
        SuperGm = "super-gm",
        Max = "max",
    }
);
crate::wire_enum!(
    /// The unfinished game or analysis a page keeps to resume, and the played game a board shows.
    SessionKind, SESSION_KINDS {
        Analysis = "analysis",
        Local = "local",
        Computer = "computer",
        ArchiveComputer = "archive:computer",
        ArchiveBoard = "archive:board",
        ArchiveClock = "archive:clock",
    }
);
crate::union!(
    /// Where Game history keeps a game played in KChess: against the computer, two people at one
    /// board, or the chess clock.
    HistoryKind {
        Computer = "computer",
        Board = "board",
        Clock = "clock",
    }
);

impl HistoryKind {
    /// The session a place of this kind keeps its game's identity as.
    pub const fn session(self) -> SessionKind {
        match self {
            HistoryKind::Computer => SessionKind::ArchiveComputer,
            HistoryKind::Board => SessionKind::ArchiveBoard,
            HistoryKind::Clock => SessionKind::ArchiveClock,
        }
    }
}

crate::union!(
    /// What a desktop notification is about; each kind (except `test`) has its own switch in
    /// Settings.
    NotificationKind {
        OpponentMove = "opponentMove",
        LowTime = "lowTime",
        GameEvents = "gameEvents",
        ComputerMove = "computerMove",
        Challenge = "challenge",
        Test = "test",
    }
);
crate::union!(
    /// Tournament pairing system.
    TournamentSystem {
        Arena = "arena",
        Swiss = "swiss",
    }
);

/// The days per move a correspondence game may give.
pub const CORRESPONDENCE_DAYS: [f64; 7] = [1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0];
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
pub const PERF_TYPES: &[&str] = &crate::perf::PERF_KEYS;
/// The rating groups of the Lichess opening explorer, in order.
pub const EXPLORER_RATINGS: [f64; 9] = [
    400.0, 1000.0, 1200.0, 1400.0, 1600.0, 1800.0, 2000.0, 2200.0, 2500.0,
];
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

/// Largest serialized document the core stores.
pub const MAX_DOCUMENT: usize = 2_000_000;
pub const MAX_CHAPTERS: usize = 64;
pub const MAX_MOVES: usize = 1024;

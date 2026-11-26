//! Every third-party address the core reaches, each with who runs it. Plain data with no imports:
//! `cargo xtask source-check` (`tooling/xtask/src/source_check.rs`) compiles this file too and,
//! weekly (`.github/workflows/sources.yml`), fetches a small part of every download here to see it
//! still serves what KChess reads, so a source that died, moved or was taken over is caught before
//! a player meets it. Community sources rot fast: one chess database's domain now serves a casino.
//!
//! A new address goes here with its owner, never as a literal elsewhere in the core (xtask's
//! `every_third_party_address_is_a_source` test refuses one); a new download also gets a check.

/// A third-party address and who answers for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    /// What KChess fetches from it, as a failed check names it.
    pub name: &'static str,
    /// Who runs it, and where to look or ask when it changes.
    pub owner: &'static str,
    pub url: &'static str,
}

/// Lichess's run of the database site, for every archive on it.
const LICHESS_DATABASE: &str =
    "Lichess (database.lichess.org; its scripts are github.com/lichess-org/database)";

/// The broadcast games: `list.txt`, `sha256sums.txt` and a `lichess_db_broadcast_YYYY-MM.pgn.zst`
/// a month beside them.
pub const LICHESS_BROADCAST: Source = Source {
    name: "Lichess broadcast games",
    owner: LICHESS_DATABASE,
    url: "https://database.lichess.org/broadcast/",
};

/// Every rated standard game, laid out as the broadcast games are
/// (`lichess_db_standard_rated_YYYY-MM.pgn.zst`).
pub const LICHESS_STANDARD: Source = Source {
    name: "Lichess rated games",
    owner: LICHESS_DATABASE,
    url: "https://database.lichess.org/standard/",
};

/// The puzzle database `puzzleDbInstall` downloads.
pub const LICHESS_PUZZLES: Source = Source {
    name: "Lichess puzzles",
    owner: LICHESS_DATABASE,
    url: "https://database.lichess.org/lichess_db_puzzle.csv.zst",
};

/// The Lichess website and API: accounts, live games and each player's games
/// (`/api/games/user/{name}`).
pub const LICHESS: Source = Source {
    name: "Lichess",
    owner: "Lichess (lichess.org; its API is documented at lichess.org/api)",
    url: "https://lichess.org",
};

/// GitHub's answer naming TCEC's latest release of its games.
pub const TCEC: Source = Source {
    name: "TCEC engine games",
    owner: "TCEC, the Top Chess Engine Championship (tcec-chess.com; GitHub TCEC-Chess/tcecgames)",
    url: "https://api.github.com/repos/TCEC-Chess/tcecgames/releases/latest",
};
/// Where a TCEC release's files are, by tag: `{TCEC_DOWNLOADS}{tag}/{TCEC_ASSET}`.
pub const TCEC_DOWNLOADS: &str = "https://github.com/TCEC-Chess/tcecgames/releases/download/";
/// The release file KChess reads: every game, its comments replaced by book exits.
pub const TCEC_ASSET: &str = "TCEC-everything-compact.zip";

/// GitHub's answer naming the latest official Stockfish release.
pub const STOCKFISH: Source = Source {
    name: "Stockfish",
    owner: "The Stockfish developers (stockfishchess.org; GitHub official-stockfish/Stockfish)",
    url: "https://api.github.com/repos/official-stockfish/Stockfish/releases/latest",
};
/// GitHub's answer about one Stockfish release, by tag: `{STOCKFISH_TAGS}{tag}`.
pub const STOCKFISH_TAGS: &str =
    "https://api.github.com/repos/official-stockfish/Stockfish/releases/tags/";
/// Where every official Stockfish file downloads from: `<tag>/<asset name>` follows.
pub const STOCKFISH_DOWNLOADS: &str =
    "https://github.com/official-stockfish/Stockfish/releases/download/";

/// The Vosk English speech model voice input downloads.
pub const VOSK_MODEL: Source = Source {
    name: "Vosk English speech model",
    owner: "Alpha Cephei, Vosk's authors (alphacephei.com/vosk; GitHub alphacep/vosk-api)",
    url: "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip",
};

/// Lichess's mirror of the Syzygy endgame tables, each under `3-4-5-wdl/` or `3-4-5-dtz/`.
pub const SYZYGY: Source = Source {
    name: "Syzygy endgame tables",
    owner: "Lichess (tablebase.lichess.ovh, its mirror of the Syzygy tables)",
    url: "https://tablebase.lichess.ovh/tables/standard",
};

/// Chess.com's public API: each player's monthly archives of games.
pub const CHESSCOM: Source = Source {
    name: "Chess.com games",
    owner: "Chess.com (its published-data API, chess.com/news/view/published-data-api)",
    url: "https://api.chess.com",
};

/// Every download `cargo xtask source-check` checks, each by a part small enough to fetch weekly.
pub const DOWNLOADS: &[Source] = &[
    LICHESS_BROADCAST,
    LICHESS_STANDARD,
    LICHESS_PUZZLES,
    LICHESS,
    TCEC,
    STOCKFISH,
    VOSK_MODEL,
    SYZYGY,
    CHESSCOM,
];

/// The opening explorer, asked one position at a time.
pub const LICHESS_EXPLORER: Source = Source {
    name: "Lichess opening explorer",
    owner: "Lichess (explorer.lichess.org; GitHub lichess-org/lila-openingexplorer)",
    url: "https://explorer.lichess.org",
};

/// The tablebase lookup, asked one position at a time.
pub const LICHESS_TABLEBASE: Source = Source {
    name: "Lichess tablebase lookup",
    owner: "Lichess (tablebase.lichess.org; GitHub lichess-org/lila-tablebase)",
    url: "https://tablebase.lichess.org",
};

/// Wikibooks' API, for an opening's description.
pub const WIKIBOOKS: Source = Source {
    name: "Wikibooks opening descriptions",
    owner: "The Wikimedia Foundation (en.wikibooks.org, its Chess Opening Theory book)",
    url: "https://en.wikibooks.org/w/api.php",
};
/// A Wikibooks page, by its title: `{WIKIBOOKS_PAGES}{title}`, linked from the description.
pub const WIKIBOOKS_PAGES: &str = "https://en.wikibooks.org/wiki/";

/// OpenAI's sign-in, for the AI coach.
pub const OPENAI_AUTH: Source = Source {
    name: "OpenAI sign-in",
    owner: "OpenAI (auth.openai.com)",
    url: "https://auth.openai.com",
};

/// OpenAI's API, which the AI coach asks.
pub const OPENAI_API: Source = Source {
    name: "OpenAI API",
    owner: "OpenAI (api.openai.com)",
    url: "https://api.openai.com",
};
/// The resource the AI coach's sign-in asks access to.
pub const OPENAI_RESOURCE: &str = "https://api.openai.com/v1";

/// The services asked a question at a time, which a person sees fail where they ask: no weekly
/// fetch, but each has its owner.
pub const SERVICES: &[Source] = &[
    LICHESS_EXPLORER,
    LICHESS_TABLEBASE,
    WIKIBOOKS,
    OPENAI_AUTH,
    OPENAI_API,
];

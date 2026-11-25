//! The local game database: the databases a profile holds, an import's progress, and a search's
//! games.

use super::*;

union!(#[doc = "Where a database's games came from."] DatabaseSource {
    Imported = "imported",
    Own = "own",
    Master = "master",
    Player = "player",
    Lichess = "lichess",
});

union!(#[doc = "Where a player's games are downloaded from."] GameSite {
    Lichess = "lichess",
    Chesscom = "chesscom",
});

/// One player's games to download into a database of their own (`gameDbPlayerInstall`). The
/// database keeps it, so an update brings the same games again.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerGames {
    pub site: GameSite,
    pub username: String,
    /// Only the games of the newest this many months, the current month among them; every
    /// game when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub months: Option<u32>,
    /// Only games of these speeds; every speed when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speeds: Option<Vec<ExplorerSpeed>>,
    /// Only rated games.
    #[serde(default)]
    pub rated: bool,
}

/// A month of Lichess's rated games to keep the strong games of (`gameDbLichessInstall`): those
/// whose players were both rated at least `min_rating`, bullet and bots' games left out. Each
/// floor has a database of its own, which gathers months.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LichessGames {
    /// `YYYY-MM`; the newest month the database lacks when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month: Option<String>,
    pub min_rating: u32,
}

/// Lichess's months of rated games (`gameDbLichessMonths`), and which of them each database of
/// strong Lichess games holds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LichessArchive {
    /// `YYYY-MM`, newest first.
    pub months: Vec<String>,
    pub held: Vec<LichessHeld>,
}

/// The months a database of strong Lichess games holds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LichessHeld {
    pub min_rating: u32,
    /// `YYYY-MM`, newest first.
    pub months: Vec<String>,
}

union!(#[doc = "Progress stage of a game database import; `downloading` while a month of the \
    master games arrives, before it is read."] GameDbPhase {
    Downloading = "downloading",
    Reading = "reading",
    Indexing = "indexing",
    Done = "done",
    Cancelled = "cancelled",
    Failed = "failed",
});

impl GameDbPhase {
    /// The last report of an import: none follows it.
    pub fn is_terminal(self) -> bool {
        match self {
            GameDbPhase::Done | GameDbPhase::Cancelled | GameDbPhase::Failed => true,
            GameDbPhase::Downloading | GameDbPhase::Reading | GameDbPhase::Indexing => false,
        }
    }
}

/// One database of the profile.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInfo {
    pub id: String,
    pub name: String,
    pub source: DatabaseSource,
    pub games: u64,
    /// Size on disk.
    pub bytes: u64,
    pub updated_at: i64,
    /// Moves from the start the opening tree covers.
    pub index_depth: u32,
    /// Whose games they are and on what terms, when the source asks for it to be shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// The download a player's database was made from (`source` `player`), which an update
    /// repeats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<PlayerGames>,
    /// The rating both players of its games had at least (`source` `lichess`), which adding a
    /// month keeps to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lichess_rating: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbStatus {
    pub databases: Vec<DatabaseInfo>,
    /// True while an import runs; its progress arrives as events.
    pub busy: bool,
}

/// An import in progress (or finished), streamed to every window.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbProgress {
    pub phase: GameDbPhase,
    /// The database being written.
    pub database: String,
    /// Games kept so far.
    pub games: u64,
    /// Games that could not be kept (another variant, an illegal move, too long).
    pub skipped: u64,
    /// Bytes of the file read (while downloading, received) so far.
    pub bytes: u64,
    /// The file's size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    /// Why it failed; while the master games arrive, the month being brought in (`2026-10`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// What an import added.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbImported {
    pub database: DatabaseInfo,
    pub added: u64,
    pub skipped: u64,
}

union!(#[doc = "Which side a searched player had."] GameDbSide {
    White = "white",
    Black = "black",
    Either = "either",
});

union!(#[doc = "A game's result, as a search filters it."] GameDbResult {
    White = "white",
    Draw = "draw",
    Black = "black",
});

union!(#[doc = "The order a search lists games in."] GameDbSort {
    Newest = "newest",
    Oldest = "oldest",
    Rating = "rating",
});

/// A search of one database: every filter given must match.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbQuery {
    pub database: String,
    /// A player's name, or the start of one of its words ("carlsen", "magnus c").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
    /// Which side `player` had; either when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<GameDbSide>,
    /// The other player's name, as `player` matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opponent: Option<String>,
    /// Words the event's name holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_to: Option<i32>,
    /// Both players rated at least this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_elo: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<GameDbResult>,
    /// An ECO code or its start (`B9`, `C65`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eco: Option<String>,
    /// Only the games that reached this position (a FEN), with a move played from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    /// Newest first when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<GameDbSort>,
    pub offset: u32,
    pub limit: u32,
}

/// A game as a search lists it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbRow {
    pub id: u64,
    pub white: String,
    pub black: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white_elo: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black_elo: Option<u32>,
    /// `1-0`, `0-1`, `1/2-1/2` or `*`.
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// As written, `YYYY.MM.DD` with `??` for what is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eco: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening: Option<String>,
    pub plies: u32,
}

/// One page of a search.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbPage {
    pub games: Vec<GameDbRow>,
    /// Every game the search matched.
    pub total: u64,
    pub offset: u32,
}

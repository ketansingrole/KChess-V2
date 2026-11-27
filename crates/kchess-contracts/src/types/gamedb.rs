//! The local game database: the databases a profile holds, an import's progress, and a search's
//! games.

use super::*;

union!(#[doc = "Where a database's games came from."] DatabaseSource {
    Imported = "imported",
    Own = "own",
    Master = "master",
    Player = "player",
    Lichess = "lichess",
    Tcec = "tcec",
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
    pub speeds: Option<Vec<Speed>>,
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
    broadcast games arrives, before it is read."] GameDbPhase {
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
    /// Why it failed; while the broadcast games arrive, the month being brought in (`2026-10`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A main line from a position through the game databases: each move the one played most, and
/// how the games through its first move ended.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbOpeningLine {
    /// Each with the games that played it there.
    pub moves: Vec<NoveltyMove>,
    pub white: u64,
    pub draws: u64,
    pub black: u64,
}

/// A game of a database, as a search lists it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbFoundGame {
    pub database: String,
    pub game: GameDbRow,
}

/// A position through every game database (Your games aside): how its games ended, its main
/// lines and its strongest games.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbOpeningReport {
    pub white: u64,
    pub draws: u64,
    pub black: u64,
    /// The most played first.
    pub lines: Vec<GameDbOpeningLine>,
    /// The highest rated first.
    pub best: Vec<GameDbFoundGame>,
}

/// Games' results from one side: won, drawn and lost; `games` counts those with no result too.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbRecord {
    pub games: u64,
    pub wins: u64,
    pub draws: u64,
    pub losses: u64,
}

/// An opening a player played, by its family's name ("Sicilian Defense"), its ECO code or its
/// first moves, and how it went.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbOpening {
    pub name: String,
    pub record: GameDbRecord,
}

/// What a player did with one colour: their results and their openings, the most played first.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbSideReport {
    pub record: GameDbRecord,
    pub openings: Vec<GameDbOpening>,
}

/// A player's rating in a month: their last rated game's.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbRating {
    pub year: i32,
    pub month: u8,
    pub rating: u32,
}

/// One player's games in a database at a glance: as White and as Black, their rating over the
/// months, and their wins over the strongest opponents.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbPlayerReport {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_year: Option<i32>,
    pub white: GameDbSideReport,
    pub black: GameDbSideReport,
    /// Oldest first.
    pub ratings: Vec<GameDbRating>,
    /// The strongest opponent first.
    pub best: Vec<GameDbRow>,
}

/// One edition of an event in a database (`gameDbEvent`): the games under its name that were
/// played around one game's date, with each player's standing and results.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbEventReport {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// The first and last games' dates as their headers give them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_date: Option<String>,
    pub games: u64,
    /// The standings: the most points first, then Sonneborn–Berger, then the name.
    pub players: Vec<GameDbEventPlayer>,
}

/// A player's standing in an event.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbEventPlayer {
    pub name: String,
    /// Their rating in the first of the event's games that gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u32>,
    /// Points in halves: 13 is 6½.
    pub half_points: u32,
    pub wins: u32,
    pub draws: u32,
    pub losses: u32,
    /// Sonneborn–Berger in quarter points: the points of each opponent beaten, and half those of
    /// each drawn.
    pub tiebreak_quarters: u32,
    /// The FIDE performance rating over their rated opponents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<u32>,
    /// Their games with a result, in date and round order.
    pub results: Vec<GameDbEventResult>,
}

/// One of a player's games in an event.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbEventResult {
    pub game: u64,
    /// The opponent's place in the standings, from 0.
    pub opponent: u32,
    pub white: bool,
    /// The player's points in halves: 2 a win, 1 a draw, 0 a loss.
    pub half_points: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<String>,
}

/// A player's name in a database, with the games it is in.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbName {
    pub name: String,
    pub games: u64,
}

/// Names in a database that are likely one player's, the most played first.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbNameGroup {
    pub names: Vec<GameDbName>,
}

/// Where a line leaves the games of the game databases: its first move no game played from
/// its position.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbNovelty {
    /// The new move's index in the line; none when every move looked at was played before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ply: Option<u32>,
    /// The games that went on from the position before it.
    pub games: u64,
    /// The moves they played there, most played first: at most three.
    pub played: Vec<NoveltyMove>,
}

/// A move games played where a line's novelty was played instead.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NoveltyMove {
    pub san: String,
    pub uci: String,
    pub games: u64,
}

/// A position of a repertoire where it is the repertoire's side to move, and the moves it
/// plays there (`gameDbRepertoire`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepertoireStep {
    pub fen: String,
    /// In UCI.
    pub moves: Vec<String>,
}

union!(#[doc = "What the databases' games say of a repertoire's move."] GameDbVerdict {
    Played = "played",
    Unplayed = "unplayed",
    Rare = "rare",
    Weaker = "weaker",
});

/// A repertoire's position through the databases: the games that went on from it, and its
/// moves with the most played others.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbRepertoireCheck {
    pub fen: String,
    pub games: u64,
    /// The repertoire's moves first, as it gave them, then the others most played first.
    pub moves: Vec<GameDbRepertoireMove>,
}

/// A move from a repertoire's position, with its games and how they ended.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbRepertoireMove {
    pub uci: String,
    pub san: String,
    pub games: u64,
    pub white: u64,
    pub draws: u64,
    pub black: u64,
    /// The average rating of its rated games' players.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u32>,
    /// The verdict on a repertoire's move; none for the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<GameDbVerdict>,
}

/// A repertoire through your own games as its side (`gameDbRepertoireGames`): where they left
/// it, most often first.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbRepertoireGames {
    /// Your games with the repertoire's side.
    pub games: u64,
    /// Those of them that reached one of its positions.
    pub reached: u64,
    pub gaps: Vec<GameDbRepertoireGap>,
}

/// A repertoire made from one player's games with one side (`gameDbRepertoireBuild`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbRepertoireBuilt {
    /// Their games with that side from the standard start.
    pub games: u64,
    /// The moves it has.
    pub moves: u64,
    /// Its lines as PGN: their most played move at their turn, the other side's common replies
    /// at its turn, the most played first.
    pub pgn: String,
}

/// A move your games played where the repertoire has none: yours, where it plays another, or
/// your opponent's, which it has no answer to. With the games that played it, how they ended
/// for you and when the last one was played.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbRepertoireGap {
    /// The step it was played from, by its place in the steps asked.
    pub step: u32,
    pub uci: String,
    pub san: String,
    /// Your move, not your opponent's.
    pub yours: bool,
    /// The repertoire has it: one of a player's choices it is ready for
    /// (`gameDbRepertoirePlayer`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prepared: bool,
    pub games: u64,
    pub won: u64,
    pub drawn: u64,
    pub lost: u64,
    /// As written, `YYYY.MM.DD` with `??` for what is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
}

/// A game saved into a database: the database as it is now, and the game's id in it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameDbSaved {
    pub database: DatabaseInfo,
    pub game: u64,
}

/// Where `gameDbMergeGames` put the games merged: the study and its new chapter, and how many
/// games it merged and left out.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameDbMerged {
    pub study: String,
    pub chapter: String,
    pub merged: u64,
    /// Games from another start position, or another variant, than the first game merged.
    pub left_out: u64,
    /// Games past what one chapter holds: 200 games, and its moves.
    pub no_room: u64,
    /// The studies it changed, as an incremental `studyCommand` answers them.
    pub saved: StudyCommandResult,
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

union!(#[doc = "A pawn structure a deep search looks for, of either colour."] GameDbStructure {
    IsolatedQueenPawn = "isolatedQueenPawn",
    HangingPawns = "hangingPawns",
    Carlsbad = "carlsbad",
    MaroczyBind = "maroczyBind",
    Stonewall = "stonewall",
});

impl From<GameDbStructure> for kchess_domain::gamedb::patterns::Structure {
    fn from(structure: GameDbStructure) -> Self {
        use kchess_domain::gamedb::patterns::Structure;
        match structure {
            GameDbStructure::IsolatedQueenPawn => Structure::IsolatedQueenPawn,
            GameDbStructure::HangingPawns => Structure::HangingPawns,
            GameDbStructure::Carlsbad => Structure::Carlsbad,
            GameDbStructure::MaroczyBind => Structure::MaroczyBind,
            GameDbStructure::Stonewall => Structure::Stonewall,
        }
    }
}

impl From<kchess_domain::gamedb::patterns::Structure> for GameDbStructure {
    fn from(structure: kchess_domain::gamedb::patterns::Structure) -> Self {
        use kchess_domain::gamedb::patterns::Structure;
        match structure {
            Structure::IsolatedQueenPawn => GameDbStructure::IsolatedQueenPawn,
            Structure::HangingPawns => GameDbStructure::HangingPawns,
            Structure::Carlsbad => GameDbStructure::Carlsbad,
            Structure::MaroczyBind => GameDbStructure::MaroczyBind,
            Structure::Stonewall => GameDbStructure::Stonewall,
        }
    }
}

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
    /// The games of at least, and at most, this many moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_moves: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_moves: Option<u32>,
    /// The games played at this speed, as their time control says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<Speed>,
    /// The games that reach this material balance and keep it ("R vs B", "KRP vs KR"), either
    /// colour on either side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// The games that reach this pawn structure and keep it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<GameDbStructure>,
    /// The games that put these pieces on these squares and keep them there ("Nd5 vs e6": one
    /// side's pieces against the other's), either colour on either side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pieces: Option<String>,
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

/// The endgames of Your games that slipped away (`gameDbYourEndgames`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct YourEndgames {
    /// Whether the endgame tablebases are on this computer: without them nothing is found.
    pub tables: bool,
    /// Your games gone through.
    pub games: u64,
    /// The newest first.
    pub endgames: Vec<YourEndgame>,
}

/// A position of one of your games with five pieces or fewer where your move gave away a win,
/// or a draw, as the endgame tablebases have it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct YourEndgame {
    /// The game's id in Your games.
    pub game: String,
    /// The position, before your move.
    pub fen: String,
    /// Your side: `white` or `black`.
    pub side: String,
    /// What the position held for you: `win` or `draw`.
    pub goal: String,
    /// Your move there.
    pub san: String,
    /// What it held after your move: `draw` or `loss`.
    pub became: String,
    /// The pieces, yours first: "KRP vs KR".
    pub material: String,
    /// The move's number in the game.
    pub number: u32,
    pub opponent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
}

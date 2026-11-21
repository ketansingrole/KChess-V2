//! Players as Lichess shows them: profiles, ratings, records, activity and who is followed.

use super::*;

wire_enum!(#[doc = "Lichess rating category, including variants."] PerfType, PERF_TYPES {
    UltraBullet = "ultraBullet",
    Bullet = "bullet",
    Blitz = "blitz",
    Rapid = "rapid",
    Classical = "classical",
    Correspondence = "correspondence",
    Chess960 = "chess960",
    KingOfTheHill = "kingOfTheHill",
    ThreeCheck = "threeCheck",
    Antichess = "antichess",
    Atomic = "atomic",
    Horde = "horde",
    RacingKings = "racingKings",
    Crazyhouse = "crazyhouse",
});

wire_enum!(#[doc = "What a connected account can do about a player on Lichess."] RelationAction, RELATION_ACTIONS {
    Follow = "follow",
    Unfollow = "unfollow",
    Block = "block",
    Unblock = "unblock",
});

/// A Lichess user as its profile (`profile`) and the many-players lookup (`profiles`) report
/// them: what KChess reads of it. Lichess leaves out what a user has not got.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LichessUser {
    #[serde(default)]
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// By rating category: `blitz`, `puzzle`, `storm`…
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub perfs: BTreeMap<String, UserPerf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<UserCount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_time: Option<PlayTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<UserBio>,
}

impl LichessUser {
    /// The games played in rating category `key`; 0 when none.
    pub fn games(&self, key: &str) -> i64 {
        self.perfs
            .get(key)
            .and_then(|perf| perf.games)
            .unwrap_or_default()
    }

    /// The time they have spent playing, in seconds: it grows with every finished game.
    pub fn played_seconds(&self) -> Option<i64> {
        self.play_time.as_ref()?.total
    }

    /// Lichess has games of theirs a sync that saw `synced` seconds of play did not download.
    pub fn has_games_since(&self, synced: Option<i64>) -> bool {
        matches!((synced, self.played_seconds()), (Some(synced), Some(now)) if now > synced)
    }

    /// Of `perfs` (key, name), those with games, most played first.
    pub fn played<'a>(
        &self,
        perfs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Vec<(&'a str, &'a str)> {
        let mut played: Vec<_> = perfs
            .into_iter()
            .filter(|(key, _)| self.games(key) > 0)
            .collect();
        played.sort_by_key(|(key, _)| std::cmp::Reverse(self.games(key)));
        played
    }
}

/// A user's record in one rating category (or their Storm and Streak bests).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserPerf {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub games: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<i64>,
    /// The rating's recent change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prog: Option<i64>,
    /// Provisional: too few games for the rating to settle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prov: Option<bool>,
    /// Storm and Streak: the runs played and the best score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
}

/// A user's lifetime games.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct UserCount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw: Option<i64>,
}

/// Time spent playing, in seconds: it grows with every finished game.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct PlayTime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct UserBio {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
}

/// An account's puzzle results over a period, as Lichess reports them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct PuzzleDashboard {
    #[serde(default)]
    pub global: PuzzleResults,
    /// By theme key, in Lichess's order.
    #[serde(default, with = "ordered_map")]
    pub themes: Vec<(String, PuzzleTheme)>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct PuzzleTheme {
    #[serde(default)]
    pub results: PuzzleResults,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleResults {
    /// Puzzles played.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nb: Option<i64>,
    /// Solved at the first try.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_wins: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<i64>,
}

/// Anyone's Storm results: the high scores and the days played lately.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct StormDashboard {
    #[serde(default)]
    pub high: StormHighs,
    #[serde(default)]
    pub days: Vec<StormDay>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StormHighs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_time: Option<i64>,
}

/// One day's Storm runs.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct StormDay {
    /// The day, `YYYY/MM/DD`.
    #[serde(rename = "_id", default)]
    pub id: String,
    #[serde(default)]
    pub runs: i64,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub moves: i64,
    #[serde(default)]
    pub errors: i64,
    #[serde(default)]
    pub combo: i64,
}

/// A JSON object as its entries in order (`serde_json`'s `preserve_order` keeps Lichess's).
mod ordered_map {
    use serde::de::{DeserializeOwned, MapAccess, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize>(
        entries: &[(String, T)],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (key, value) in entries {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: DeserializeOwned>(
        deserializer: D,
    ) -> Result<Vec<(String, T)>, D::Error> {
        struct Entries<T>(std::marker::PhantomData<T>);
        impl<'de, T: DeserializeOwned> Visitor<'de> for Entries<T> {
            type Value = Vec<(String, T)>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(entries)
            }
        }
        deserializer.deserialize_map(Entries(std::marker::PhantomData))
    }
}

/// A game in a player's record: the opponent and when.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfResult {
    pub opponent: String,
    pub opponent_rating: f64,
    pub at: String,
    pub game_id: String,
}

/// Lichess's game counts for one speed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PerfCount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rated: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tour: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub berserk: Option<i64>,
    /// The opponents' average rating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_avg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<i64>,
}

/// A rating and the game it came from.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfMark {
    pub rating: f64,
    pub at: String,
    pub game_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PerfStreak {
    pub current: i64,
    pub best: i64,
}

/// One player's record in one rating category, as Lichess's perf page shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfStats {
    pub perf: PerfType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deviation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisional: Option<bool>,
    /// Position on the leaderboard, when ranked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<i64>,
    /// Better than this share of players.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<i64>,
    /// Lichess's game counts, null when it sent none.
    pub count: Option<PerfCount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highest: Option<PerfMark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowest: Option<PerfMark>,
    pub best_wins: Vec<PerfResult>,
    pub worst_losses: Vec<PerfResult>,
    pub win_streak: PerfStreak,
    pub loss_streak: PerfStreak,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrosstableMatchup {
    pub users: BTreeMap<String, f64>,
    pub nb_games: i64,
}

/// Two players' lifetime score against each other (and in their current match, if any).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Crosstable {
    /// Lower-cased username → points (wins plus half-points for draws).
    pub users: BTreeMap<String, f64>,
    pub nb_games: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matchup: Option<CrosstableMatchup>,
}

/// Whether a connected account follows or blocks a player on Lichess.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LichessRelation {
    pub following: bool,
    pub blocking: bool,
}

/// One place on a Lichess leaderboard.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntry {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub rating: f64,
    /// Rating change over the last games.
    pub progress: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
}

/// A player whose name starts with what was typed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSuggestion {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
}

/// One line of a player's activity or of a timeline, ready to show: `kind` picks the icon
/// (`games`, `puzzles`, `tournament`, `follow`, `study`, `team`, `post`, `simul`, `patron`,
/// `stream`, `correspondence`, `other`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    pub kind: String,
    pub text: String,
    /// A player the line is about, for a profile link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// A game the line is about, to open it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    /// The rating over the line's games or puzzles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<RatingChange>,
}

/// A rating before and after a day's games or puzzles.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct RatingChange {
    pub before: f64,
    pub after: f64,
}

/// A day of a player's Lichess activity, newest first.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDay {
    /// Start of the day, epoch milliseconds.
    pub start: f64,
    pub items: Vec<ActivityItem>,
}

/// One line of the Following list's timeline (what its players did), newest first.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    /// Start of the day, epoch milliseconds.
    pub date: f64,
    /// Who on the Following list the line is about.
    pub player: String,
    pub item: ActivityItem,
}

/// A followed player's Lichess ratings, of the categories they have one in.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FollowedRatings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bullet: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blitz: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rapid: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classical: Option<i64>,
}

/// A player one of the connected accounts follows on Lichess.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowedUser {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub ratings: FollowedRatings,
    /// Connected accounts that follow them.
    pub followed_by: Vec<String>,
    /// Already a friend in KChess (or one of your own accounts).
    pub already_added: bool,
    /// Removed as a friend earlier; not suggested again unless asked.
    pub dismissed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowingProblem {
    pub account: String,
    pub message: String,
    /// Connecting the account again grants the missing permission.
    pub needs_reconnect: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowingReport {
    pub users: Vec<FollowedUser>,
    pub problems: Vec<FollowingProblem>,
}

/// Lichess's view of one user's connection. `signal` runs 1 (poor, lag > 500 ms) to 4 (great,
/// lag < 150 ms).
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserPresence {
    pub online: bool,
    pub playing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<Number>,
    /// The game they are playing, when Lichess says (watchable from the Watch page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceReport {
    /// Lower-cased username → status; users Lichess did not report are absent.
    pub users: BTreeMap<String, UserPresence>,
    /// Round trip of the status request itself, i.e. this computer's latency to Lichess.
    pub latency_ms: u64,
}

//! The contract shapes as frontends rely on them: wire values, defaults and the method table.

use kchess_contracts::defaults::default_settings;
use kchess_contracts::types::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

/// Every wire value reads as a variant and writes back as itself, the variants are as many as the
/// values, and a value outside the list is refused.
fn wire_values_round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    name: &str,
    values: &[&str],
) {
    let mut seen = Vec::new();
    for value in values {
        let parsed: T = serde_json::from_value(json!(value))
            .unwrap_or_else(|cause| panic!("{name}: {value} does not read: {cause}"));
        assert_eq!(
            serde_json::to_value(&parsed).ok(),
            Some(json!(value)),
            "{name}"
        );
        assert!(!seen.contains(&parsed), "{name}: {value} repeats a variant");
        seen.push(parsed);
    }
    assert!(
        serde_json::from_value::<T>(json!("not-a-wire-value")).is_err(),
        "{name} accepts an unknown value"
    );
}

#[test]
fn every_wire_enum_round_trips_its_listed_values() {
    macro_rules! check {
        ($($ty:ty: $values:ident),+ $(,)?) => {
            $(wire_values_round_trip::<$ty>(stringify!($ty), $values);)+
        };
    }
    check!(
        Appearance: APPEARANCES,
        ChallengeColor: CHALLENGE_COLORS,
        ChatRoom: CHAT_ROOMS,
        CoordinateMode: COORDINATE_MODES,
        DeclineReason: DECLINE_REASONS,
        EngineLevel: ENGINE_LEVELS,
        ExplorerSpeed: EXPLORER_SPEEDS,
        OnlineAction: ONLINE_ACTIONS,
        PerfType: PERF_TYPES,
        PieceAnimation: PIECE_ANIMATIONS,
        PositionLookupKind: POSITION_LOOKUP_KINDS,
        PromotionMode: PROMOTION_MODES,
        PuzzleDifficulty: PUZZLE_DIFFICULTIES,
        PuzzleTrainMode: PUZZLE_TRAIN_MODES,
        ReviewAuto: REVIEW_AUTO,
        RunKind: RUN_KINDS,
        VoiceOutcome: VOICE_OUTCOMES,
        VoiceSource: VOICE_SOURCES,
    );
}

#[test]
fn the_default_settings_round_trip_through_json() {
    let settings = default_settings().expect("the bundled defaults read");
    assert!(!settings.core.engine_levels.is_empty());
    let wire = serde_json::to_value(&settings).expect("settings write");
    // One flat object: the core's preferences and the frontend's side by side.
    assert!(wire.get("engineLevels").is_some() && wire.get("appearance").is_some());
    let back: Settings = serde_json::from_value(wire).expect("settings read back");
    assert_eq!(back, settings);
}

#[test]
fn an_unknown_setting_value_is_refused() {
    let mut wire = serde_json::to_value(default_settings().expect("defaults")).expect("write");
    wire["appearance"] = json!("sepia");
    assert!(serde_json::from_value::<Settings>(wire).is_err());
}

/// A player's rating categories with games, most played first; a category without games is left
/// out.
#[test]
fn a_users_categories_are_those_played_most_played_first() {
    let user: LichessUser = serde_json::from_value(json!({
        "id": "alice",
        "username": "Alice",
        "perfs": {
            "bullet": { "games": 0, "rating": 1500 },
            "blitz": { "games": 12, "rating": 1600 },
            "rapid": { "games": 40, "rating": 1700 },
            "classical": { "rating": 1500 },
        },
    }))
    .expect("a user");
    assert_eq!(user.games("rapid"), 40);
    assert_eq!(user.games("classical"), 0);
    assert_eq!(user.games("chess960"), 0);
    let perfs = [
        ("bullet", "Bullet"),
        ("blitz", "Blitz"),
        ("rapid", "Rapid"),
        ("classical", "Classical"),
    ];
    assert_eq!(user.played(perfs), [("rapid", "Rapid"), ("blitz", "Blitz")]);
}

/// A rated game counts at the player's rating after it; an unrated one, or one without both
/// figures, does not count.
#[test]
fn a_rated_game_counts_at_the_rating_it_ended_on() {
    let game = |rated: bool, diff: Option<i64>| -> LichessGame {
        let mut game = json!({
            "id": "g", "account": "alice", "createdAt": 1_000, "lastMoveAt": 2_000,
            "rated": rated, "speed": "blitz", "perf": "blitz", "status": "mate",
            "color": "white", "opponent": "bob", "moves": "e4 e5",
            "playerRating": 1500,
        });
        if let Some(diff) = diff {
            game["ratingDiff"] = json!(diff);
        }
        serde_json::from_value(game).expect("a game")
    };
    let rated = game(true, Some(-7));
    let counted = rated.rated_game().expect("a rated game counts");
    assert_eq!(
        (counted.created_at, counted.perf, counted.rating),
        (1_000, "blitz", 1_493)
    );
    assert_eq!(
        game(true, Some(3)).rated_game().map(|g| g.rating),
        Some(1_503)
    );
    assert!(game(false, Some(3)).rated_game().is_none());
    assert!(game(true, None).rated_game().is_none());
}

/// The puzzle dashboard keeps its themes in Lichess's order, writes them back as an object, and
/// refuses themes that are not one.
#[test]
fn a_dashboards_themes_keep_lichesss_order() {
    let text = r#"{"global":{},"themes":{"mateIn2":{"results":{"nb":3}},"fork":{"results":{"nb":5}},"endgame":{"results":{}}}}"#;
    let dashboard: PuzzleDashboard = serde_json::from_str(text).expect("a dashboard");
    let keys: Vec<&str> = dashboard
        .themes
        .iter()
        .map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(keys, ["mateIn2", "fork", "endgame"]);
    let written = serde_json::to_string(&dashboard).expect("written");
    let again: PuzzleDashboard = serde_json::from_str(&written).expect("read again");
    assert_eq!(again, dashboard);
    let refused = serde_json::from_str::<PuzzleDashboard>(r#"{"themes":[1]}"#)
        .expect_err("themes must be an object");
    assert!(refused.to_string().contains("an object"), "{refused}");
}

#[test]
fn only_a_correspondence_speed_is_correspondence() {
    let game = |speed: &str| -> OngoingGame {
        serde_json::from_value(json!({
            "gameId": "g", "account": "alice", "opponent": { "name": "bob" },
            "fen": "8/8/8/8/8/8/8/8 w - - 0 1", "isMyTurn": true,
            "variant": "standard", "speed": speed, "rated": false,
        }))
        .expect("an ongoing game")
    };
    assert!(game("correspondence").correspondence());
    assert!(!game("blitz").correspondence());
}

/// Lichess asks every API caller to name its program and where to reach it, to tell of a
/// problem or a breaking change; a bare "KChess" named the program but gave no way back.
#[test]
fn the_user_agent_names_kchess_its_version_and_where_to_reach_it() {
    assert_eq!(
        kchess_contracts::USER_AGENT,
        format!(
            "KChess/{} (+https://github.com/ketansingrole/KChess-V2)",
            env!("CARGO_PKG_VERSION")
        )
    );
}

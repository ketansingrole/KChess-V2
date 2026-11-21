use super::*;

/// Lichess's play time grows with every finished game: more than at the sync means new ones.
#[test]
fn a_longer_play_time_than_at_the_sync_means_new_games() {
    let played = |total: Option<i64>| -> LichessUser {
        serde_json::from_value(serde_json::json!({ "playTime": { "total": total } }))
            .expect("a user")
    };
    assert!(played(Some(1300)).has_games_since(Some(1000)));
    assert!(!played(Some(1000)).has_games_since(Some(1000)));
    // Without a play time saved at a sync, or in the profile, nothing is claimed.
    assert!(!played(Some(1300)).has_games_since(None));
    assert!(!played(None).has_games_since(Some(1000)));
    assert!(!LichessUser::default().has_games_since(Some(1000)));
}

/// Accuracy and time use, which the recorded reports leave out, keep the numbers the core
/// wrote by hand: averages whole when they are, per-game rates always with a fraction.
#[test]
fn insights_accuracy_and_time_use_keep_their_bytes() {
    let record = r#"{"total":1,"win":1,"loss":0,"draw":0}"#;
    let text = format!(
        concat!(
            r#"{{"accuracy":{{"games":2,"average":87.5,"acpl":30,"#,
            r#""perGame":{{"inaccuracy":1.0,"mistake":0.5,"blunder":0.0}}}},"#,
            r#""timeUse":{{"games":1,"trouble":{r},"spare":{r},"#,
            r#""phases":[{{"phase":"opening","moves":3,"seconds":2,"share":12.5}},"#,
            r#"{{"phase":"endgame","moves":1,"seconds":0.5}}],"#,
            r#""clockAt":[{{"move":20,"games":1,"player":50,"opponent":41.5}}],"#,
            r#""blunders":{{"games":1,"short":{{"moves":1,"blunders":0}},"#,
            r#""spare":{{"moves":2,"blunders":1}}}}}}}}"#
        ),
        r = record
    );
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Parts {
        accuracy: AccuracyReport,
        time_use: TimeUseReport,
    }
    let parts: Parts = serde_json::from_str(&text).unwrap();
    assert_eq!(serde_json::to_string(&parts).unwrap(), text);
}

/// A joined tournament keeps the bytes the core wrote by hand: its end a whole number.
#[test]
fn a_joined_tournament_keeps_its_bytes() {
    for text in [
        r#"[{"system":"arena","id":"abcd","account":"Alice","name":"Hourly","until":1700000000000}]"#,
        r#"[{"system":"swiss","id":"efgh","account":"bob","name":"Weekly","until":1.5}]"#,
    ] {
        let joined: Vec<JoinedTournament> = serde_json::from_str(text).unwrap();
        assert_eq!(serde_json::to_string(&joined).unwrap(), text);
    }
}

#[test]
fn a_result_filter_s_name_is_its_wire_value() {
    for filter in GameResultFilter::ALL {
        assert_eq!(
            serde_json::to_value(filter).unwrap(),
            serde_json::json!(filter.as_str())
        );
    }
}

#[test]
fn a_computer_search_is_sent_as_its_options() {
    use kchess_domain::games::computer::SearchOptions;
    let options = SearchOptions {
        fen: Some("8/8/8/8/8/8/8/K6k w - - 0 1".into()),
        chess960: true,
        variant: Some(Variant::Atomic),
        movetime: Some(500.0),
    };
    assert_eq!(
        BestMoveOptions::from(&options),
        BestMoveOptions {
            fen: options.fen.clone(),
            movetime: Some(500),
            chess960: Some(true),
            variant: Some("atomic".into()),
        }
    );
    let plain = BestMoveOptions::from(&SearchOptions::default());
    assert_eq!(plain, BestMoveOptions::default());
}

/// An absent field, a null one and a value read apart.
#[test]
fn a_field_may_be_absent_null_or_a_value() {
    let read = |text: &str| serde_json::from_str::<PuzzleSolveResult>(text).unwrap();
    assert_eq!(read("{}").rating_diff, None);
    assert_eq!(read(r#"{"ratingDiff":null}"#).rating_diff, Some(None));
    assert_eq!(read(r#"{"ratingDiff":-7}"#).rating_diff, Some(Some(-7)));
}

/// Each command names the study and the chapter it changes.
#[test]
fn a_study_command_names_what_it_changes() {
    use serde_json::json;
    let chapter = json!({ "id": "c5", "name": "n", "pgn": "*" });
    let remote = json!({ "id": "r", "name": "n" });
    for (command, study, chapter) in [
        (
            json!({ "op": "save", "name": "n", "pgn": "*", "id": "s1", "chapterId": "c1" }),
            Some("s1"),
            Some("c1"),
        ),
        (json!({ "op": "save", "name": "n", "pgn": "*" }), None, None),
        (
            json!({ "op": "saveChapters", "name": "n", "chapters": [], "id": "s1" }),
            Some("s1"),
            None,
        ),
        (
            json!({ "op": "addChapter", "id": "s1", "name": "n" }),
            Some("s1"),
            None,
        ),
        (
            json!({ "op": "renameChapter", "id": "s1", "chapterId": "c1", "name": "n" }),
            Some("s1"),
            Some("c1"),
        ),
        (
            json!({ "op": "duplicateChapter", "id": "s1", "chapterId": "c2" }),
            Some("s1"),
            Some("c2"),
        ),
        (
            json!({ "op": "removeChapter", "id": "s1", "chapterId": "c3" }),
            Some("s1"),
            Some("c3"),
        ),
        (
            json!({ "op": "moveChapter", "id": "s1", "chapterId": "c4", "index": 0 }),
            Some("s1"),
            Some("c4"),
        ),
        (
            json!({ "op": "restoreChapter", "id": "s1", "index": 0, "chapter": chapter }),
            Some("s1"),
            Some("c5"),
        ),
        (
            json!({ "op": "markCloud", "id": "s2", "account": "a", "remoteId": "r", "baseline": "*" }),
            Some("s2"),
            None,
        ),
        (
            json!({ "op": "offline", "account": "a", "remote": remote, "chapters": [] }),
            None,
            None,
        ),
        (json!({ "op": "remove", "id": "s3" }), Some("s3"), None),
        (
            json!({ "op": "restore", "study": { "id": "s4" } }),
            Some("s4"),
            None,
        ),
        (
            json!({ "op": "rename", "id": "s5", "name": "n" }),
            Some("s5"),
            None,
        ),
        (json!({ "op": "duplicate", "id": "s6" }), Some("s6"), None),
    ] {
        let command: StudyCommand = serde_json::from_value(command).unwrap();
        assert_eq!(command.study_id(), study, "{command:?}");
        assert_eq!(command.chapter_id(), chapter, "{command:?}");
    }
}

#[test]
fn a_tournament_is_joined_by_its_account_in_any_case() {
    let joined: JoinedTournament = serde_json::from_value(serde_json::json!({
        "system": "arena", "id": "abcd", "account": "Alice", "name": "Hourly", "until": 1,
    }))
    .unwrap();
    assert!(joined.joined_by("ALICE"));
    assert!(!joined.joined_by("bob"));
}

/// A game saved before `playable` existed is one the Board API can play.
#[test]
fn an_ongoing_game_without_playable_is_playable() {
    let game: OngoingGame = serde_json::from_value(serde_json::json!({
        "gameId": "abcd1234",
        "account": "me",
        "opponent": { "name": "them" },
        "color": "white",
        "fen": "8/8/8/8/8/8/8/8 w - - 0 1",
        "isMyTurn": true,
        "variant": "standard",
        "speed": "blitz",
        "rated": false,
    }))
    .unwrap();
    assert!(game.playable);
}

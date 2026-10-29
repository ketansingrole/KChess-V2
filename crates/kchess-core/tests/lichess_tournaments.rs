//! Tournaments over local fixtures: the timeline's listing and its order, arena
//! creation as the chosen account, the reconnect answer for a login without the permission, the
//! signed-in team lookup with its short explanation, and an arena's featured game, duels, podium
//! and totals.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use fixtures::{
    MemoryStore, MemoryTokens, Recording, Reply, capabilities_for, http_client, ndjson, serve,
};
use kchess_core::host::Host;
use kchess_core::lichess::accounts::{Lichess, LichessStore, Reply as Outcome};
use kchess_core::lichess::client::{LichessClient, TokenSource};
use kchess_core::lichess::policy::Policy;
use kchess_core::lichess::tournaments::{
    NewArena, arena_extras, create_tournament, tournament, tournaments,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn lichess(base: &str, host: &Arc<Recording>, tokens: Arc<MemoryTokens>) -> Lichess {
    let policy = Policy::with_timeout(
        Arc::clone(host) as Arc<dyn Host>,
        http_client(),
        CancellationToken::new(),
        Duration::from_secs(5),
    );
    let client = LichessClient::with_base(base, policy, http_client());
    Lichess::new(
        client,
        capabilities_for(host),
        MemoryStore::with_accounts(&[]) as Arc<dyn LichessStore>,
        tokens as Arc<dyn TokenSource>,
        Arc::clone(host) as Arc<dyn Host>,
        CancellationToken::new(),
    )
}

/// `arena()` of the TypeScript suite: an arena as Lichess lists it.
fn arena(overrides: Value) -> Value {
    let mut arena = json!({
        "id": "Abcd1234",
        "fullName": "≤1700 Rapid Arena",
        "status": 10,
        "variant": { "key": "standard", "name": "Standard" },
        "rated": true,
        "clock": { "limit": 600, "increment": 0 },
        "minutes": 57,
        "nbPlayers": 12,
        "startsAt": 1_791_259_200_000i64,
        "finishesAt": 1_791_262_620_000i64,
        "perf": { "key": "rapid" },
        "schedule": { "freq": "hourly", "speed": "rapid" },
        "maxRating": { "rating": 1700 }
    });
    for (key, value) in overrides.as_object().cloned().unwrap_or_default() {
        arena[key] = value;
    }
    arena
}

#[tokio::test]
async fn lists_running_upcoming_and_finished_arenas_with_what_the_timeline_groups_them_by() {
    let fixture = serve(|seen| match seen.path() {
        "/api/tournament" => Reply::Json(
            200,
            json!({
                "started": [arena(json!({ "id": "Started1", "status": 20 }))],
                "created": [arena(json!({}))],
                "finished": [arena(json!({ "id": "Finish01", "status": 30, "maxRating": null, "schedule": null }))],
            })
            .to_string(),
        ),
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = lichess(&fixture.base, &host, Arc::new(MemoryTokens::default()));
    let list = tournaments(&lichess, "").await.unwrap();
    let listed: Vec<(&str, &str)> = list
        .arenas
        .iter()
        .map(|t| (t.id.as_str(), t.status.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("Started1", "started"),
            ("Abcd1234", "created"),
            ("Finish01", "finished")
        ]
    );
    let created = serde_json::to_value(&list.arenas[1]).unwrap();
    assert_eq!(created["perf"], "rapid");
    assert_eq!(created["freq"], "hourly");
    assert_eq!(created["maxRating"], 1700);
    let finished = serde_json::to_value(&list.arenas[2]).unwrap();
    assert!(finished.get("freq").is_none());
    assert!(finished.get("maxRating").is_none());
    assert!(list.problems.is_empty());
}

#[tokio::test]
async fn creates_an_arena_as_the_chosen_account_and_returns_it() {
    let fixture = serve(|seen| match seen.path() {
        "/api/tournament" if seen.method == "POST" => Reply::Json(
            200,
            arena(json!({ "fullName": "Club Night Arena", "maxRating": null })).to_string(),
        ),
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let host = Arc::new(Recording::default());
    let tokens = MemoryTokens::with(&[("Alice", "Alice-token")]);
    let lichess = lichess(&fixture.base, &host, tokens);
    let created = create_tournament(
        &lichess,
        "Alice",
        &NewArena {
            name: Some("Club Night".into()),
            clock_time: serde_json::Number::from(10),
            clock_increment: serde_json::Number::from(0),
            minutes: serde_json::Number::from(60),
            wait_minutes: Some(serde_json::Number::from(5)),
            start_date: None,
            variant: "standard".into(),
            rated: true,
            password: None,
            description: None,
        },
    )
    .await
    .unwrap();
    let Outcome::Done(summary) = created else {
        panic!("the arena was created");
    };
    assert_eq!(summary.id, "Abcd1234");
    assert_eq!(summary.name, "Club Night Arena");
    assert!(summary.playable);
    let posted = fixture.to("/api/tournament");
    assert_eq!(posted.len(), 1);
    assert_eq!(
        posted[0].header("Authorization"),
        Some("Bearer Alice-token")
    );
    let form: Vec<(String, String)> = reqwest::Url::parse(&format!("http://x/?{}", posted[0].body))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let get = |key: &str| form.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    assert_eq!(get("name"), Some("Club Night"));
    assert_eq!(get("clockTime"), Some("10"));
    assert_eq!(get("minutes"), Some("60"));
    assert_eq!(get("waitMinutes"), Some("5"));
}

#[tokio::test]
async fn reads_the_created_arena_lichess_sends_on_its_own() {
    // `POST /api/tournament` answers with the full arena (as `GET /api/tournament/{id}`): a
    // string variant, an ISO start and no `status`. Reading it as a list entry failed after
    // Lichess had already created the arena.
    let fixture = serve(|seen| match seen.path() {
        "/api/tournament" if seen.method == "POST" => Reply::Json(
            200,
            json!({
                "id": "QITRjufu",
                "fullName": "KChess button QA Arena",
                "rated": false,
                "clock": { "limit": 600, "increment": 0 },
                "minutes": 60,
                "createdBy": "alice",
                "system": "arena",
                "secondsToStart": 300,
                "isFinished": false,
                "startsAt": "2026-10-17T10:24:16Z",
                "nbPlayers": 0,
                "perf": { "key": "rapid", "name": "Rapid", "icon": "#" },
                "variant": "standard",
                "berserkable": true,
                "standing": { "page": 1, "players": [] },
            })
            .to_string(),
        ),
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = lichess(
        &fixture.base,
        &host,
        MemoryTokens::with(&[("Alice", "Alice-token")]),
    );
    let created = create_tournament(
        &lichess,
        "Alice",
        &NewArena {
            name: Some("KChess button QA".into()),
            clock_time: serde_json::Number::from(10),
            clock_increment: serde_json::Number::from(0),
            minutes: serde_json::Number::from(60),
            wait_minutes: Some(serde_json::Number::from(5)),
            start_date: None,
            variant: "standard".into(),
            rated: false,
            password: Some("entry".into()),
            description: None,
        },
    )
    .await
    .unwrap();
    let Outcome::Done(summary) = created else {
        panic!("the arena was created");
    };
    assert_eq!(summary.id, "QITRjufu");
    assert_eq!(summary.name, "KChess button QA Arena");
    assert_eq!(summary.status, "created");
    assert!(!summary.rated);
    assert_eq!(summary.starts_at, 1_792_232_656_000);
}

#[tokio::test]
async fn asks_to_reconnect_when_the_login_lacks_the_tournament_permission() {
    let fixture =
        serve(|_| Reply::Json(403, json!({ "error": "Missing scope" }).to_string())).await;
    let host = Arc::new(Recording::default());
    let lichess = lichess(
        &fixture.base,
        &host,
        MemoryTokens::with(&[("Alice", "Alice-token")]),
    );
    let outcome = create_tournament(
        &lichess,
        "Alice",
        &NewArena {
            name: None,
            clock_time: serde_json::Number::from(3),
            clock_increment: serde_json::Number::from(2),
            minutes: serde_json::Number::from(60),
            wait_minutes: None,
            start_date: None,
            variant: "standard".into(),
            rated: true,
            password: None,
            description: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(outcome, Outcome::NeedsReconnect));
}

#[tokio::test]
async fn looks_up_the_accounts_teams_signed_in_and_explains_a_refusal_briefly() {
    // Lichess answers /api/team/of only with a login ("Missing authorization header").
    let fixture = serve(|seen| match seen.path() {
        "/api/tournament" => Reply::Json(
            200,
            json!({ "started": [], "created": [], "finished": [] }).to_string(),
        ),
        "/api/team/of/Alice" => Reply::Json(
            401,
            json!({ "error": "Missing authorization header" }).to_string(),
        ),
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = lichess(
        &fixture.base,
        &host,
        MemoryTokens::with(&[("Alice", "Alice-token")]),
    );
    let list = tournaments(&lichess, "Alice").await.unwrap();
    let teams = fixture.to("/api/team/of/Alice");
    assert!(!teams.is_empty());
    assert_eq!(teams[0].header("Authorization"), Some("Bearer Alice-token"));
    assert_eq!(
        list.problems,
        vec![
            "Couldn’t load the teams of @Alice. Connect the account again to allow it.".to_string()
        ]
    );
    assert!(!list.problems.join(" ").contains("authorization header"));
}

#[tokio::test]
async fn reads_an_arenas_top_game_games_in_progress_podium_and_totals() {
    let extras = arena_extras(&json!({
        "featured": {
            "id": "ShLmo1Hs",
            "fen": "8/5rpp/4R3/6KP/3k4/5qP1/8/8 w",
            "orientation": "black",
            "lastMove": "d1f3",
            "white": { "name": "weaky_player", "rating": 2482, "rank": 3 },
            "black": { "name": "Coach13", "rating": 2868, "rank": 1 },
            "c": { "white": 4, "black": 10 }
        },
        "duels": [{
            "id": "ShLmo1Hs",
            "p": [
                { "n": "weaky_player", "r": 2482, "k": 3 },
                { "n": "Coach13", "r": 2868, "k": 1 }
            ]
        }],
        "podium": [{ "name": "agill47", "rank": 1, "rating": 1262, "score": 8, "performance": 1637 }],
        "stats": {
            "games": 8,
            "moves": 378,
            "whiteWins": 3,
            "blackWins": 5,
            "draws": 0,
            "berserks": 0,
            "averageRating": 1132
        }
    }));
    let featured = extras.featured.expect("featured game");
    assert_eq!(featured.fen, "8/5rpp/4R3/6KP/3k4/5qP1/8/8 w - - 0 1");
    assert_eq!(featured.orientation, "black");
    assert_eq!(featured.black.name, "Coach13");
    assert_eq!(featured.black.rank, Some(serde_json::Number::from(1)));
    let clocks = featured.clocks.expect("clocks");
    assert_eq!(clocks.white, serde_json::Number::from(4));
    assert_eq!(clocks.black, serde_json::Number::from(10));
    let duels = extras.duels.expect("duels");
    assert_eq!(duels.len(), 1);
    assert_eq!(duels[0].id, "ShLmo1Hs");
    assert_eq!(duels[0].white.name, "weaky_player");
    assert_eq!(duels[0].black.rank, Some(serde_json::Number::from(1)));
    let podium = extras.podium.expect("podium");
    assert_eq!(podium[0].name, "agill47");
    assert_eq!(podium[0].performance, Some(serde_json::Number::from(1637)));
    let stats = extras.stats.expect("stats");
    assert_eq!(stats.games, serde_json::Number::from(8));
    assert_eq!(stats.black_wins, serde_json::Number::from(5));
    // Nothing readable: no extras rather than a broken event page.
    let nothing = arena_extras(&json!({ "duels": [{ "id": 1 }] }));
    assert_eq!(nothing, Default::default());
}

/// `swiss()`: a Swiss event as Lichess sends it, in a team's list or on its own.
fn swiss(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "name": format!("Swiss {id}"),
        "status": status,
        "variant": "standard",
        "rated": true,
        "clock": { "limit": 900, "increment": 10 },
        "round": 2,
        "nbRounds": 7,
        "nbPlayers": 30,
        "startsAt": "2026-11-01T18:00:00Z",
        "nextRound": { "in": 120 },
        "verdicts": { "accepted": false, "list": [{ "condition": "Team member", "verdict": "ok" }] }
    })
}

#[tokio::test]
async fn lists_the_upcoming_swiss_events_of_each_team_and_names_a_team_that_failed() {
    let fixture = serve(|seen| match seen.path() {
        "/api/tournament" => Reply::Json(
            200,
            json!({ "started": [arena(json!({}))], "created": [], "finished": [] }).to_string(),
        ),
        "/api/team/of/Alice" => Reply::Json(
            200,
            json!([{ "id": "club", "name": "The Club" }, { "id": "broken", "name": "Broken" }])
                .to_string(),
        ),
        "/api/team/club/swiss" => Reply::Text(
            200,
            "application/x-ndjson",
            ndjson(&[
                swiss("Later111", "created"),
                json!({ "id": "unreadable" }),
                swiss("Done1111", "finished"),
                json!(swiss("Now11111", "started")),
            ]),
        ),
        _ => Reply::Json(500, "{}".into()),
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = lichess(
        &fixture.base,
        &host,
        MemoryTokens::with(&[("Alice", "Alice-token")]),
    );
    let list = tournaments(&lichess, "Alice").await.unwrap();
    assert_eq!(list.arenas.len(), 1);
    // Finished and unreadable events are left out.
    let ids: Vec<&str> = list.swiss.iter().map(|event| event.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"Later111") && ids.contains(&"Now11111"));
    let later = list
        .swiss
        .iter()
        .find(|event| event.id == "Later111")
        .unwrap();
    assert_eq!(
        later.team.as_ref().map(|team| team.name.as_str()),
        Some("The Club")
    );
    assert_eq!(later.status, "created");
    assert_eq!(later.starts_at, 1_793_556_000_000);
    assert_eq!(
        fixture.to("/api/team/club/swiss")[0]
            .query("max")
            .as_deref(),
        Some("10")
    );
    assert_eq!(list.problems.len(), 1);
    assert!(list.problems[0].starts_with("Couldn’t load Swiss events of Broken"));
    assert!(
        host.logged()
            .contains("Could not load Swiss events: broken")
    );
}

#[tokio::test]
async fn reads_an_arena_with_its_leaderboard_page_and_the_accounts_place() {
    let fixture = serve(|seen| match seen.path() {
        "/api/tournament/Abcd1234" => Reply::Json(
            200,
            json!({
                "id": "Abcd1234",
                "fullName": "Hourly Rapid",
                "clock": { "limit": 600, "increment": 0 },
                "variant": "standard",
                "rated": true,
                "nbPlayers": 2,
                "startsAt": "2026-11-01T18:00:00Z",
                "secondsToFinish": 900,
                "berserkable": true,
                "description": "Have fun",
                "standing": { "page": 2, "players": [
                    { "rank": 1, "name": "Bob", "rating": 1800, "score": 9, "sheet": { "scores": "5432", "fire": true } },
                    { "rank": 2, "username": "Alice", "rating": 1700, "points": 4 },
                    { "rank": "not a rank" }
                ] },
                "me": { "rank": 2, "withdraw": false, "gameId": "game1234" },
                "verdicts": { "accepted": true, "list": [{ "condition": "Rated ≥ 10 games", "verdict": "ok" }] }
            })
            .to_string(),
        ),
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let host = Arc::new(Recording::default());
    let tokens = MemoryTokens::with(&[("Alice", "Alice-token")]);
    let lichess = lichess(&fixture.base, &host, Arc::clone(&tokens));
    let detail = tournament(&lichess, tokens.as_ref(), "arena", "Abcd1234", "Alice", 2)
        .await
        .unwrap();
    let seen = &fixture.to("/api/tournament/Abcd1234")[0];
    assert_eq!(seen.query("page").as_deref(), Some("2"));
    assert_eq!(seen.header("Authorization"), Some("Bearer Alice-token"));
    let value = serde_json::to_value(&detail).unwrap();
    assert_eq!(value["status"], json!("started"));
    assert_eq!(value["description"], json!("Have fun"));
    assert_eq!(value["berserkable"], json!(true));
    assert_eq!(value["standingPage"], json!(2));
    assert_eq!(
        value["me"],
        json!({ "rank": 2, "withdraw": false, "gameId": "game1234" })
    );
    assert_eq!(value["verdicts"]["accepted"], json!(true));
    let standing = value["standing"].as_array().unwrap();
    assert_eq!(standing.len(), 2);
    assert_eq!(standing[0]["sheet"], json!("5432"));
    assert_eq!(standing[1]["name"], json!("Alice"));
    assert_eq!(standing[1]["score"], json!(4));

    // An arena whose clock cannot be read is refused, not shown half-read.
    let fixture = serve(|_| Reply::Json(200, json!({ "id": "Abcd1234" }).to_string())).await;
    let lichess = self::lichess(&fixture.base, &host, MemoryTokens::with(&[]));
    let error = tournament(&lichess, tokens.as_ref(), "arena", "Abcd1234", "", 1)
        .await
        .unwrap_err();
    assert_eq!(
        error.message(),
        "Lichess sent an unreadable tournament clock."
    );
    assert!(fixture.requests()[0].header("Authorization").is_none());
}

#[tokio::test]
async fn reads_a_swiss_event_with_its_results_and_next_round() {
    let fixture = serve(|seen| match seen.path() {
        "/api/swiss/Swiss123" => Reply::Json(200, swiss("Swiss123", "started").to_string()),
        "/api/swiss/Swiss123/results" => Reply::Text(
            200,
            "application/x-ndjson",
            ndjson(&[
                json!({ "rank": 1, "username": "Bob", "rating": 1900, "points": 2 }),
                json!({ "rank": 2, "username": "Alice", "rating": 1700, "points": 1.5 }),
            ]),
        ),
        "/api/swiss/Garbled1" => Reply::Json(200, json!({ "id": "Garbled1" }).to_string()),
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let host = Arc::new(Recording::default());
    let tokens = MemoryTokens::with(&[]);
    let lichess = lichess(&fixture.base, &host, Arc::clone(&tokens));
    let detail = tournament(&lichess, tokens.as_ref(), "swiss", "Swiss123", "", 1)
        .await
        .unwrap();
    let value = serde_json::to_value(&detail).unwrap();
    assert_eq!(value["system"], json!("swiss"));
    assert_eq!(value["round"], json!(2));
    assert_eq!(value["nbRounds"], json!(7));
    assert_eq!(value["nextRoundIn"], json!(120));
    assert_eq!(
        value["verdicts"]["list"][0]["condition"],
        json!("Team member")
    );
    assert_eq!(value["standing"][1]["score"], json!(1.5));
    assert_eq!(
        fixture.to("/api/swiss/Swiss123/results")[0]
            .query("nb")
            .as_deref(),
        Some("50")
    );
    let error = tournament(&lichess, tokens.as_ref(), "swiss", "Garbled1", "", 1)
        .await
        .unwrap_err();
    assert_eq!(
        error.message(),
        "Lichess sent details KChess could not read."
    );
}

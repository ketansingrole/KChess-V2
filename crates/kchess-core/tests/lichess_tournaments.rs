//! Tournaments (`core/src/services/tournaments.ts`) over local fixtures. Ports
//! `core/tests/unit/tournament-create.test.ts`: the timeline's listing and its order, arena
//! creation as the chosen account, the reconnect answer for a login without the permission, the
//! signed-in team lookup with its short explanation, and an arena's featured game, duels, podium
//! and totals.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use fixtures::{MemoryStore, MemoryTokens, Recording, Reply, capabilities_for, http_client, serve};
use kchess_core::host::Host;
use kchess_core::lichess::accounts::{Lichess, LichessStore, Reply as Outcome};
use kchess_core::lichess::client::{LichessClient, TokenSource};
use kchess_core::lichess::policy::Policy;
use kchess_core::lichess::tournaments::{NewArena, arena_extras, create_tournament, tournaments};
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

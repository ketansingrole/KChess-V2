//! The facade's Lichess methods for a signed-in account over a local Lichess: each reaches its
//! endpoint, with the account's login when it needs one; position assistance waits until no live game is
//! confirmed; and after logging out, account calls ask to reconnect and send nothing.
#[path = "support/lichess_http.rs"]
mod fixtures;
#[path = "support/signed_in.rs"]
mod signed_in;

use fixtures::{Reply, serve};
use serde_json::{Value, json};
use signed_in::ALICE_LOGIN;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lichess_methods_reach_their_endpoints_as_the_account() {
    let fixture = serve(|seen| match seen.path() {
        "/api/stream/event" => Reply::Hang,
        "/api/account/playing" => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        "/api/cloud-eval" => Reply::Json(
            200,
            json!({ "fen": START, "knodes": 1000, "depth": 30,
                    "pvs": [{ "moves": "e2e4 e7e5", "cp": 20 }] })
            .to_string(),
        ),
        _ => Reply::Json(200, "{}".into()),
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;

    // Position assistance is blocked until recovery confirms no live game.
    let blocked = core
        .call("cloudEval", vec![json!(START), json!(1)])
        .await
        .unwrap_err();
    assert_eq!(
        blocked.message,
        "Analysis is unavailable during a live Lichess game."
    );
    core.call("resumeOnline", vec![]).await.unwrap();
    let off = core
        .call("cloudEval", vec![json!(START), json!(1)])
        .await
        .unwrap_err();
    assert_eq!(
        off.message,
        "Turn on cloud evaluation in Settings → Analysis first."
    );
    let mut settings = core.call("settings", vec![]).await.unwrap();
    settings["cloudEval"] = json!(true);
    core.call("saveSettings", vec![settings]).await.unwrap();

    // (method, arguments, the requests it sends, whether they need Alice's login)
    let calls: Vec<(&str, Vec<Value>, Vec<&str>, bool)> = vec![
        (
            "profile",
            vec![json!("Bob")],
            vec!["GET /api/user/Bob"],
            false,
        ),
        (
            "ratingHistory",
            vec![json!("Bob")],
            vec!["GET /api/user/Bob/rating-history"],
            false,
        ),
        ("following", vec![], vec!["GET /api/rel/following"], true),
        (
            "exportGame",
            vec![json!("abcdefgh")],
            vec!["POST /api/games/export/_ids?clocks=true&evals=false&opening=true"],
            false,
        ),
        ("puzzleDaily", vec![], vec!["GET /api/puzzle/daily"], false),
        (
            "puzzleDashboard",
            vec![json!("Alice"), json!(30)],
            vec!["GET /api/puzzle/dashboard/30"],
            true,
        ),
        (
            "puzzleActivity",
            vec![json!("Alice"), json!(10)],
            vec!["GET /api/puzzle/activity?max=10"],
            true,
        ),
        (
            "stormDashboard",
            vec![json!("Alice"), json!(30)],
            vec!["GET /api/storm/dashboard/Alice?days=30"],
            false,
        ),
        (
            "lichessStudies",
            vec![json!("Alice")],
            vec!["GET /api/study/by/Alice"],
            true,
        ),
        (
            "lichessStudyChapters",
            vec![json!("Alice"), json!("abcdEFGH")],
            vec![
                "GET /api/study/abcdEFGH.pgn?comments=true&variations=true&clocks=false&orientation=true",
            ],
            true,
        ),
        ("tvChannels", vec![], vec!["GET /api/tv/channels"], false),
        (
            "broadcasts",
            vec![Value::Null],
            vec!["GET /api/broadcast/top?page=1"],
            false,
        ),
        (
            "broadcastTour",
            vec![json!("abcdEFGH")],
            vec!["GET /api/broadcast/abcdEFGH"],
            false,
        ),
        (
            "cloudEval",
            vec![json!(START), json!(1)],
            vec!["GET /api/cloud-eval"],
            false,
        ),
        (
            "syncGames",
            vec![json!("Alice")],
            vec![
                "GET /api/games/user/Alice?max=1000&ongoing=true&opening=true&moves=true&pgnInJson=true&clocks=true&evals=true&accuracy=true",
            ],
            true,
        ),
        (
            "timeline",
            vec![json!("Alice")],
            vec!["GET /api/timeline?nb=30"],
            true,
        ),
        (
            "bookmarkedGames",
            vec![json!("Alice")],
            vec!["GET /api/games/export/bookmarks?max=100&moves=false&opening=true"],
            true,
        ),
        (
            "relation",
            vec![json!("Alice"), json!("Bob")],
            vec!["GET /api/user/Bob"],
            true,
        ),
        (
            "sendMessage",
            vec![json!("Alice"), json!("Bob"), json!("hi")],
            vec!["POST /inbox/Bob"],
            true,
        ),
        (
            "setRelation",
            vec![json!("Alice"), json!("Bob"), json!("follow")],
            vec!["POST /api/rel/follow/Bob"],
            true,
        ),
        (
            "importGame",
            vec![json!("Alice"), json!("1. e4 *")],
            vec!["POST /api/import"],
            true,
        ),
    ];
    let mut answers = std::collections::HashMap::new();
    for (method, args, expected, signed) in calls {
        let before = fixture.requests().len();
        answers.insert(method, core.call(method, args).await);
        let sent = &fixture.requests()[before..];
        let targets: Vec<String> = sent
            .iter()
            .map(|seen| {
                format!(
                    "{} {}",
                    seen.method,
                    seen.target.split("?fen").next().unwrap()
                )
            })
            .collect();
        let expected: Vec<String> = expected.iter().map(|target| target.to_string()).collect();
        assert_eq!(targets, expected, "{method}");
        if signed {
            for seen in sent {
                assert_eq!(seen.header("Authorization"), Some(ALICE_LOGIN), "{method}");
            }
        }
    }
    assert_eq!(
        answers["sendMessage"].as_ref().unwrap(),
        &json!({ "sent": true })
    );
    assert_eq!(answers["setRelation"].as_ref().unwrap(), &json!(true));
    assert_eq!(
        answers["relation"].as_ref().unwrap(),
        &json!({ "following": false, "blocking": false })
    );
    assert_eq!(answers["cloudEval"].as_ref().unwrap()["depth"], json!(30));
    assert_eq!(
        fixture.to("/api/cloud-eval")[0].query("multiPv").as_deref(),
        Some("1")
    );
    let channels = answers["tvChannels"].as_ref().unwrap();
    assert_eq!(channels[0], json!({ "key": "best", "label": "Top rated" }));
    // The import's answer is checked, not trusted.
    assert_eq!(
        answers["importGame"].as_ref().unwrap_err().message,
        "Lichess returned an invalid import response."
    );
    // A second cloud evaluation of the position is served from the cache.
    core.call("cloudEval", vec![json!(START), json!(1)])
        .await
        .unwrap();
    assert_eq!(fixture.to("/api/cloud-eval").len(), 1);

    // Logging out revokes the login on Lichess; then account calls ask to reconnect.
    core.call("logout", vec![json!("Alice")]).await.unwrap();
    let revoked = fixture.to("/api/token");
    assert_eq!(revoked[0].method, "DELETE");
    assert_eq!(revoked[0].header("Authorization"), Some(ALICE_LOGIN));
    let before = fixture.requests().len();
    assert_eq!(
        core.call("timeline", vec![json!("Alice")]).await.unwrap(),
        json!({ "needsReconnect": true })
    );
    assert_eq!(fixture.requests().len(), before);
    core.close().await;
}

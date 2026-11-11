//! The answers of the settings, accounts, Chess.com, games and player methods, byte for byte
//! (`support::wire`), over a local Lichess and Chess.com with `Alice` signed in. Times the core
//! stamps with the clock are replaced by `1` (or `1.0`), and the counts by local weekday and hour
//! (which depend on the time zone) by `0`, keeping their JSON number kind.
use crate::fixtures;
use crate::signed_in;
use crate::support::wire::assert_wire;

use fixtures::{Reply, ndjson, serve};
use serde_json::{Value, json};

const STAMPED: &[&str] = &["lastSyncedAt", "fetchedAt", "profileFetchedAt", "addedAt"];
const LOCAL: &[&str] = &["byWeekday", "byHour"];

/// `value` with the clock's stamps and the local time's counts made constant, their number kind
/// kept.
fn stamped(mut value: Value) -> Value {
    fn walk(value: &mut Value, with: Option<(i64, f64)>) {
        match value {
            Value::Number(n) => {
                if let Some((int, float)) = with {
                    *value = if n.is_f64() { json!(float) } else { json!(int) };
                }
            }
            Value::Object(map) => {
                for (key, field) in map.iter_mut() {
                    let with = match key.as_str() {
                        key if STAMPED.contains(&key) => Some((1, 1.0)),
                        key if LOCAL.contains(&key) => Some((0, 0.0)),
                        _ => with,
                    };
                    walk(field, with);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| walk(item, with)),
            _ => {}
        }
    }
    walk(&mut value, None);
    value
}

fn lichess_game(
    id: &str,
    created_at: i64,
    white: &str,
    black: &str,
    winner: Option<&str>,
) -> Value {
    let mut game = fixtures::game_json(id, created_at, white, black);
    match winner {
        Some(side) => game["winner"] = json!(side),
        None => {
            game.as_object_mut().unwrap().remove("winner");
            game["status"] = json!("draw");
        }
    }
    game["pgn"] = json!(format!(
        "[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"*\"]\n\n1. e4 c5 *"
    ));
    game
}

fn chesscom_game(
    number: u64,
    white: &str,
    black: &str,
    white_result: &str,
    black_result: &str,
) -> Value {
    json!({
        "url": format!("https://www.chess.com/game/live/{number}"),
        "pgn": format!("[White \"{white}\"]\n[Black \"{black}\"]\n[UTCDate \"2020.01.0{number}\"]\n[UTCTime \"10:00:00\"]\n\n1. e4 e5 2. Nf3 Nc6 *"),
        "time_control": "600",
        "time_class": "rapid",
        "end_time": 1_578_000_000 + number as i64,
        "rated": true,
        "rules": "chess",
        "white": { "username": white, "rating": 1500, "result": white_result },
        "black": { "username": black, "rating": 1480, "result": black_result },
    })
}

fn lichess(path: &str, query: &str) -> Reply {
    let json = |value: Value| Reply::Json(200, value.to_string());
    match path {
        "/api/stream/event" => Reply::Hang,
        "/api/account/playing" => json(json!({ "nowPlaying": [] })),
        "/api/users" => json(json!([
            { "id": "bob", "username": "Bob", "title": "FM", "playTime": { "total": 5000, "tv": 0 },
              "perfs": { "blitz": { "games": 10, "rating": 1500, "rd": 60, "prog": 12 } } },
            { "id": "alice", "username": "Alice", "playTime": { "total": 300, "tv": 0 } },
        ])),
        "/api/games/user/Alice" => Reply::Json(
            200,
            ndjson(&[
                lichess_game("game0001", 1_600_000_000_000, "Alice", "Bob", Some("white")),
                lichess_game("game0002", 1_600_100_000_000, "Bob", "Alice", Some("white")),
                lichess_game("game0003", 1_600_200_000_000, "Alice", "Carol", None),
            ]),
        ),
        "/api/games/user/Bob" => {
            let mut sparse = fixtures::game_json("game0009", 1_600_300_000_000, "Bob", "Dave");
            sparse.as_object_mut().unwrap().remove("opening");
            sparse["players"]["black"] = json!({ "aiLevel": 3 });
            Reply::Json(
                200,
                ndjson(&[
                    fixtures::game_json("game0008", 1_600_250_000_000, "Bob", "Alice"),
                    sparse,
                ]),
            )
        }
        "/api/games/export/bookmarks" => Reply::Json(
            200,
            ndjson(&[fixtures::game_json(
                "game0007",
                1_600_150_000_000,
                "Carol",
                "Alice",
            )]),
        ),
        "/api/user/Bob" => json(json!({
            "id": "bob", "username": "Bob", "title": "FM", "createdAt": 1_500_000_000_000_i64,
            "perfs": { "blitz": { "games": 10, "rating": 1500, "rd": 60.5, "prog": 12, "prov": true } },
            "count": { "all": 10, "win": 4 }, "following": true, "blocking": false,
            "playTime": { "total": 5000, "tv": 0 }
        })),
        "/api/user/Bob/rating-history" => json(json!([
            { "name": "Blitz", "points": [[2020, 0, 1, 1500], [2020, 1, 2, 1512]] },
            { "name": "Bullet", "points": [] }
        ])),
        "/api/user/Bob/perf/blitz" => json(json!({
            "perf": { "glicko": { "rating": 1500.5, "deviation": 60.25, "provisional": false },
                      "nb": 10, "progress": -12 },
            "rank": 1234, "percentile": 55.5,
            "stat": {
                "count": { "all": 10, "rated": 9, "win": 4, "loss": 4, "draw": 2, "tour": 1,
                           "berserk": 0, "opAvg": 1490.5, "seconds": 3600, "disconnects": 0 },
                "highest": { "int": 1550, "at": "2020-01-02T10:00:00.000Z", "gameId": "game0001" },
                "lowest": { "int": 1450, "at": "2019-01-02T10:00:00.000Z", "gameId": "game0002" },
                "bestWins": { "results": [{ "opRating": 1600, "opId": { "id": "carol", "name": "Carol" },
                                            "at": "2020-02-02T10:00:00.000Z", "gameId": "game0003" }] },
                "worstLosses": { "results": [{ "opRating": 1300, "at": "2020-03-02T10:00:00.000Z",
                                               "gameId": "game0004" }] },
                "resultStreak": { "win": { "cur": { "v": 2 }, "max": { "v": 5 } },
                                  "loss": { "cur": { "v": 0 }, "max": { "v": 3 } } }
            }
        })),
        "/api/user/Bob/perf/bullet" => json(json!({ "perf": {}, "stat": {} })),
        "/api/crosstable/Alice/Bob" => json(json!({
            "users": { "Alice": 5.5, "Bob": 3.5 }, "nbGames": 9,
            "matchup": { "users": { "Alice": 1, "Bob": 0 }, "nbGames": 1 }
        })),
        "/api/crosstable/Alice/Carol" => {
            json(json!({ "users": { "Alice": 0, "Carol": 0 }, "nbGames": 0 }))
        }
        "/api/user/Bob/activity" => json(json!([
            { "interval": { "start": 1_600_000_000_000_i64, "end": 1_600_086_400_000_i64 },
              "games": { "blitz": { "win": 2, "loss": 1, "draw": 0, "rp": { "before": 1500, "after": 1512.4 } } },
              "puzzles": { "score": { "win": 3, "loss": 1, "draw": 0, "rp": { "before": 1800, "after": 1810 } } },
              "follows": { "out": { "ids": ["carol"] }, "in": { "ids": ["dave", "erin"] } },
              "tournaments": { "nb": 1, "best": [{ "tournament": { "id": "abcd1234", "name": "Hourly Blitz" },
                                                     "nbGames": 7, "score": 10, "rank": 3, "rankPercent": 5 }] } },
            { "interval": { "start": 1_599_900_000_000_i64, "end": 1_599_986_400_000_i64 },
              "storm": { "runs": 2, "score": 30 } }
        ])),
        "/api/user/Carol/activity" => json(json!([
            { "interval": { "start": 1_600_050_000_000_i64, "end": 1_600_136_400_000_i64 },
              "puzzles": { "score": { "win": 0, "loss": 2, "draw": 0 } } }
        ])),
        "/api/player/top/100/blitz" => json(json!({ "users": [
            { "id": "dave", "username": "Dave", "title": "GM", "online": true,
              "perfs": { "blitz": { "rating": 3000, "progress": 15 } } },
            { "id": "erin", "username": "Erin", "perfs": { "blitz": { "rating": 2950.5 } } }
        ] })),
        "/api/player/autocomplete" if query.contains("term=bob") => json(json!({ "result": [
            { "id": "bob", "name": "Bob", "title": "FM", "online": false },
            { "id": "bobby", "name": "Bobby" }
        ] })),
        "/api/rel/following" => Reply::Json(
            200,
            ndjson(&[
                json!({ "id": "bob", "username": "Bob", "title": "FM",
                        "perfs": { "blitz": { "rating": 1500 }, "rapid": { "rating": 1600 } } }),
                json!({ "id": "frank", "username": "Frank", "perfs": {} }),
            ]),
        ),
        "/api/import" => json(json!({ "id": "imported", "url": "https://lichess.org/imported" })),
        // Chess.com
        "/pub/player/alice" => {
            json(json!({ "username": "alice", "url": "https://www.chess.com/member/Alice" }))
        }
        "/pub/player/alice/games/archives" => json(json!({ "archives": [
            "https://api.chess.com/pub/player/alice/games/2020/01"
        ] })),
        "/pub/player/alice/games/2020/01" => json(json!({ "games": [
            chesscom_game(1, "Alice", "bob", "win", "resigned"),
            chesscom_game(2, "carol", "alice", "agreed", "agreed"),
        ] })),
        "/pub/player/nobody" => Reply::Json(404, "{}".into()),
        _ => json(json!({})),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_account_and_player_answers_keep_their_bytes() {
    let fixture = serve(|seen| {
        let query = seen.target.split_once('?').map_or("", |(_, query)| query);
        lichess(seen.path(), query)
    })
    .await;
    let profile = signed_in::open_with_chesscom(&fixture.base).await;
    let core = &profile.core;
    let call = |method: &'static str, args: Vec<Value>| async move {
        core.call(method, args)
            .await
            .unwrap_or_else(|cause| panic!("{method}: {}", cause.message))
    };
    let record = |name: &str, answer: Value| assert_wire(name, &stamped(answer));
    core.call("resumeOnline", vec![]).await.unwrap();

    // Settings and the stored data.
    record("loadData.signed-in", call("loadData", vec![]).await);
    let settings = call("settings", vec![]).await;
    record("settings", settings.clone());
    let desktop = call("desktopSettings", vec![]).await;
    record("desktopSettings", desktop.clone());
    let mut changed = settings.clone();
    changed["correspondencePoll"] = json!(5);
    changed["engineLevels"] = json!(["gm", "beginner"]);
    record("saveSettings", call("saveSettings", vec![changed]).await);
    let mut edited = desktop.clone();
    edited["soundVolume"] = json!(0.5);
    edited["zenMode"] = json!(true);
    record(
        "saveDesktopSettings.base",
        call("saveDesktopSettings", vec![edited.clone(), desktop.clone()]).await,
    );
    edited["soundVolume"] = json!(1);
    record(
        "saveDesktopSettings",
        call("saveDesktopSettings", vec![edited]).await,
    );

    // Friends and synced games.
    record(
        "addFriends",
        call("addFriends", vec![json!(["Bob", " Carol "])]).await,
    );
    record("syncGames", call("syncGames", vec![json!("Alice")]).await);
    let page = |query: Value| call("gamePage", vec![query]);
    record(
        "gamePage.all",
        page(json!({ "offset": 0, "limit": 10 })).await,
    );
    record(
        "gamePage.filtered",
        page(
            json!({ "account": "Alice", "result": "win", "rated": true, "search": " bob ",
                     "offset": 0, "limit": 2 }),
        )
        .await,
    );
    record(
        "gamePage.empty",
        page(json!({ "account": "Bob", "offset": 5, "limit": 1 })).await,
    );
    record(
        "gameLibraryOverview",
        call("gameLibraryOverview", vec![]).await,
    );
    record(
        "insights",
        call("insights", vec![json!({ "account": "Alice" })]).await,
    );
    record(
        "insights.filtered",
        call(
            "insights",
            vec![json!({ "account": "Alice", "speed": "blitz", "rated": true, "variant": "standard" })],
        )
        .await,
    );
    record(
        "insights.empty",
        call("insights", vec![json!({ "account": "Bob" })]).await,
    );
    record(
        "gameRatingHistory",
        call("gameRatingHistory", vec![json!("Alice")]).await,
    );
    record(
        "gamePgn",
        call("gamePgn", vec![json!("Alice"), json!("game0001")]).await,
    );
    record(
        "gamePgn.missing",
        call("gamePgn", vec![json!("Alice"), json!("nothere1")]).await,
    );

    // Profiles.
    record(
        "cachedProfile.never",
        call("cachedProfile", vec![json!("Bob")]).await,
    );
    record("profile", call("profile", vec![json!("Bob")]).await);
    record(
        "ratingHistory",
        call("ratingHistory", vec![json!("Bob")]).await,
    );
    record(
        "cachedProfile",
        call("cachedProfile", vec![json!("Bob")]).await,
    );
    record("following", call("following", vec![]).await);
    record(
        "stayConnected",
        call("stayConnected", vec![json!("Alice")]).await,
    );
    record(
        "stayConnected.none",
        call("stayConnected", vec![json!("")]).await,
    );

    // Players.
    record(
        "playerPerf",
        call("playerPerf", vec![json!("Bob"), json!("blitz")]).await,
    );
    record(
        "playerPerf.sparse",
        call("playerPerf", vec![json!("Bob"), json!("bullet")]).await,
    );
    record(
        "crosstable",
        call("crosstable", vec![json!("Alice"), json!("Bob")]).await,
    );
    record(
        "crosstable.unplayed",
        call("crosstable", vec![json!("Alice"), json!("Carol")]).await,
    );
    record("recentGames", call("recentGames", vec![json!("Bob")]).await);
    record(
        "recentGames.rated",
        call("recentGames", vec![json!("Bob"), json!(true)]).await,
    );
    record(
        "sendMessage",
        call(
            "sendMessage",
            vec![json!("Alice"), json!("Bob"), json!(" hi ")],
        )
        .await,
    );
    record(
        "sendMessage.reconnect",
        call(
            "sendMessage",
            vec![json!("Carol"), json!("Bob"), json!("hi")],
        )
        .await,
    );
    record(
        "relation",
        call("relation", vec![json!("Alice"), json!("Bob")]).await,
    );
    record(
        "relation.reconnect",
        call("relation", vec![json!("Carol"), json!("Bob")]).await,
    );
    record(
        "setRelation",
        call(
            "setRelation",
            vec![json!("Alice"), json!("Bob"), json!("follow")],
        )
        .await,
    );
    record(
        "setRelation.reconnect",
        call(
            "setRelation",
            vec![json!("Carol"), json!("Bob"), json!("block")],
        )
        .await,
    );
    record(
        "profiles",
        call("profiles", vec![json!(["Bob", "Alice"])]).await,
    );
    record(
        "leaderboard",
        call("leaderboard", vec![json!("blitz")]).await,
    );
    record(
        "searchPlayers",
        call("searchPlayers", vec![json!("bob")]).await,
    );
    record(
        "searchPlayers.short",
        call("searchPlayers", vec![json!("Bo1")]).await,
    );
    record("activity", call("activity", vec![json!("Bob")]).await);
    record("timeline", call("timeline", vec![]).await);
    record(
        "importGame",
        call("importGame", vec![json!("Alice"), json!("1. e4 *")]).await,
    );
    record(
        "importGame.anonymous",
        call("importGame", vec![json!(""), json!("1. e4 *")]).await,
    );
    record(
        "bookmarkedGames",
        call("bookmarkedGames", vec![json!("Alice")]).await,
    );
    record(
        "bookmarkedGames.reconnect",
        call("bookmarkedGames", vec![json!("Carol")]).await,
    );

    // Chess.com.
    record(
        "chesscomAccounts.none",
        call("chesscomAccounts", vec![]).await,
    );
    record(
        "addChesscomAccount",
        call("addChesscomAccount", vec![json!(" ALICE ")]).await,
    );
    record("chesscomAccounts", call("chesscomAccounts", vec![]).await);
    record("syncChesscomGames", call("syncChesscomGames", vec![]).await);
    record(
        "syncChesscomGames.one",
        call("syncChesscomGames", vec![json!("alice")]).await,
    );
    record(
        "gamePage.chesscom",
        page(json!({ "account": "alice@chess.com", "offset": 0, "limit": 10 })).await,
    );
    record(
        "removeChesscomAccount",
        call("removeChesscomAccount", vec![json!("alice")]).await,
    );

    // Accounts.
    record(
        "clearAccountData",
        call("clearAccountData", vec![json!("Bob")]).await,
    );
    record("addAccount", call("addAccount", vec![json!("Dave")]).await);
    record(
        "removeAccount",
        call("removeAccount", vec![json!("Dave")]).await,
    );
    record("logout", call("logout", vec![json!(" Alice ")]).await);
    record("logoutAll", call("logoutAll", vec![]).await);
    record("loadData.signed-out", call("loadData", vec![]).await);
    core.close().await;
}

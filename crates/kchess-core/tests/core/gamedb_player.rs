//! A player's games downloaded into a database of their own, through the core's methods,
//! against local stand-ins for Lichess's game export and Chess.com's monthly archives.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use kchess_core::{Config, Core, TestSwitches};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::fixtures::{self, Fixture, Reply};
use crate::support;

fn game(white: &str, black: &str, result: &str, moves: &str) -> String {
    format!(
        "[Event \"Rated blitz game\"]\n[Site \"https://lichess.org/x\"]\n[Date \"2026.10.01\"]\n\
         [White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result}\"]\n\
         [WhiteElo \"2500\"]\n[BlackElo \"2400\"]\n\n{moves} {result}\n\n\n"
    )
}

/// What the stand-ins answer: Lichess's export of each player (its body, or a stream a test
/// feeds), and each Chess.com player's months.
#[derive(Clone, Default)]
struct Sites {
    lichess: Arc<Mutex<HashMap<String, String>>>,
    streams: Arc<Mutex<HashMap<String, mpsc::UnboundedReceiver<String>>>>,
    chesscom: Arc<Mutex<HashMap<String, Vec<(String, Value)>>>>,
}

impl Sites {
    async fn serve(&self) -> Fixture {
        let sites = self.clone();
        fixtures::serve(move |request| {
            let path = request.path().to_string();
            if let Some(name) = path.strip_prefix("/api/games/user/") {
                // Lichess reads a name without its case.
                let name = name.to_lowercase();
                if let Some(stream) = sites.streams.lock().unwrap().remove(&name) {
                    return Reply::StreamText(200, "application/x-chess-pgn", stream);
                }
                return match sites.lichess.lock().unwrap().get(&name) {
                    Some(pgn) => Reply::Text(200, "application/x-chess-pgn", pgn.clone()),
                    None => Reply::Json(404, json!({ "error": "Not found" }).to_string()),
                };
            }
            if let Some(rest) = path.strip_prefix("/pub/player/") {
                let (name, rest) = rest.split_once('/').unwrap_or((rest, ""));
                let months = sites.chesscom.lock().unwrap();
                let Some(months) = months.get(name) else {
                    return Reply::Json(404, json!({ "code": 0 }).to_string());
                };
                if rest == "games/archives" {
                    let archives: Vec<String> = months
                        .iter()
                        .map(|(month, _)| {
                            format!("https://api.chess.com/pub/player/{name}/games/{month}")
                        })
                        .collect();
                    return Reply::Json(200, json!({ "archives": archives }).to_string());
                }
                let month = rest.trim_start_matches("games/");
                return match months.iter().find(|(listed, _)| listed == month) {
                    Some((_, games)) => Reply::Json(200, json!({ "games": games }).to_string()),
                    None => Reply::Json(404, json!({ "code": 0 }).to_string()),
                };
            }
            Reply::Text(404, "text/plain", String::new())
        })
        .await
    }
}

fn open(profile: &Path, base: &str) -> (Arc<Core>, Arc<support::Recording>) {
    let host = support::host();
    let config = Config {
        test: Some(TestSwitches {
            lichess_origin: Some(base.to_string()),
            chesscom_origin: Some(base.to_string()),
            ..TestSwitches::default()
        }),
        ..Config::new(profile.to_path_buf())
    };
    (
        Arc::new(Core::open(config, host.clone()).expect("opens")),
        host,
    )
}

/// The core knows no game is live, so it shows games (`gameDbGames` is withheld until then).
async fn ready(core: &Core) {
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
}

/// What the databases folder holds besides the databases themselves.
fn leftovers(profile: &Path) -> Vec<String> {
    std::fs::read_dir(profile.join("databases"))
        .map(|entries| {
            entries
                .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                .filter(|name| {
                    !name.ends_with(".kdb") && !name.ends_with("-wal") && !name.ends_with("-shm")
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn databases(core: &Core) -> Vec<Value> {
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    status["databases"].as_array().expect("databases").clone()
}

async fn players(core: &Core, database: &str, side: &str) -> Vec<(String, String)> {
    let page = core
        .call(
            "gameDbGames",
            vec![json!({
                "database": database, "player": "Some_Player", "side": side,
                "offset": 0, "limit": 50
            })],
        )
        .await
        .expect("a page");
    page["games"]
        .as_array()
        .expect("games")
        .iter()
        .map(|row| {
            (
                row["white"].as_str().unwrap().to_string(),
                row["black"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_lichess_players_games_make_their_database_and_a_second_download_replaces_them() {
    let profile = tempfile::tempdir().expect("profile");
    let sites = Sites::default();
    sites.lichess.lock().unwrap().insert(
        "some_player".into(),
        [
            game("Some_Player", "Rival", "1-0", "1. e4 e5 2. Nf3 Nc6"),
            game("Rival", "Some_Player", "0-1", "1. d4 Nf6"),
            game("Some_Player", "Other", "1/2-1/2", "1. e4 c5"),
        ]
        .concat(),
    );
    let fixture = sites.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    ready(&core).await;
    let request = json!({
        "site": "lichess", "username": " Some_Player ", "months": 3,
        "speeds": ["rapid", "blitz", "blitz"], "rated": true
    });
    let installed = core
        .call("gameDbPlayerInstall", vec![request])
        .await
        .expect("downloads");
    assert_eq!(installed["added"], 3);
    let database = &installed["database"];
    let id = database["id"].as_str().expect("an id").to_string();
    assert!(id.starts_with("lichess-"), "{id}");
    assert_eq!(database["name"], "Some_Player · Lichess");
    assert_eq!(database["source"], "player");
    assert_eq!(database["games"], 3);
    // The request is kept as checked, for the next update.
    assert_eq!(
        database["player"],
        json!({
            "site": "lichess", "username": "Some_Player", "months": 3,
            "speeds": ["blitz", "rapid"], "rated": true
        })
    );
    let asked = fixture.to("/api/games/user/Some_Player");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].query("perfType").as_deref(), Some("blitz%2Crapid"));
    assert_eq!(asked[0].query("rated").as_deref(), Some("true"));
    assert!(asked[0].query("since").is_some());
    assert_eq!(asked[0].header("accept"), Some("application/x-chess-pgn"));
    let reported: Vec<String> = host
        .named("gamedb:progress")
        .iter()
        .map(|p| p["phase"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(reported.first().map(String::as_str), Some("downloading"));
    assert_eq!(reported.last().map(String::as_str), Some("done"));
    // Preparing against them: their games with White, then with Black.
    assert_eq!(players(&core, &id, "white").await.len(), 2);
    assert_eq!(
        players(&core, &id, "black").await,
        [("Rival".to_string(), "Some_Player".to_string())]
    );
    assert!(leftovers(profile.path()).is_empty());

    // A second download of the same player, now with one game, replaces their games.
    sites.lichess.lock().unwrap().insert(
        "some_player".into(),
        game("Some_Player", "Newcomer", "1-0", "1. c4 e5"),
    );
    let again = core
        .call(
            "gameDbPlayerInstall",
            vec![json!({ "site": "lichess", "username": "some_player" })],
        )
        .await
        .expect("downloads again");
    assert_eq!(again["database"]["id"], id.as_str());
    assert_eq!(again["database"]["games"], 1);
    assert_eq!(again["database"]["player"]["months"], Value::Null);
    assert_eq!(databases(&core).await.len(), 1);
    assert_eq!(
        players(&core, &id, "either").await,
        [("Some_Player".to_string(), "Newcomer".to_string())]
    );
}

#[tokio::test]
async fn an_unknown_player_or_one_without_games_leaves_no_database() {
    let profile = tempfile::tempdir().expect("profile");
    let sites = Sites::default();
    sites
        .lichess
        .lock()
        .unwrap()
        .insert("quiet".into(), String::new());
    let fixture = sites.serve().await;
    let (core, _) = open(profile.path(), &fixture.base);
    ready(&core).await;
    let unknown = core
        .call(
            "gameDbPlayerInstall",
            vec![json!({ "site": "lichess", "username": "Nobody" })],
        )
        .await
        .expect_err("refused");
    assert_eq!(unknown.message, "No Lichess player is called Nobody.");
    let quiet = core
        .call(
            "gameDbPlayerInstall",
            vec![json!({ "site": "lichess", "username": "Quiet" })],
        )
        .await
        .expect_err("refused");
    assert_eq!(quiet.message, "Quiet has no games that match.");
    let chesscom = core
        .call(
            "gameDbPlayerInstall",
            vec![json!({ "site": "chesscom", "username": "nobody" })],
        )
        .await
        .expect_err("refused");
    assert_eq!(chesscom.message, "No Chess.com player is called nobody.");
    assert!(databases(&core).await.is_empty());
    assert!(leftovers(profile.path()).is_empty());
}

#[tokio::test]
async fn a_chesscom_players_games_are_kept_by_speed_and_rating() {
    let profile = tempfile::tempdir().expect("profile");
    let sites = Sites::default();
    let played = |class: &str, rated: bool, rules: &str, black: &str| {
        json!({
            "rules": rules, "time_class": class, "rated": rated,
            "pgn": game("Hikaru", black, "1-0", "1. e4 e5"),
        })
    };
    sites.chesscom.lock().unwrap().insert(
        "hikaru".into(),
        vec![
            (
                "2026/10".into(),
                Value::Array(vec![played("blitz", true, "chess", "Old")]),
            ),
            (
                "2026/11".into(),
                Value::Array(vec![
                    played("blitz", true, "chess", "Kept"),
                    played("blitz", false, "chess", "Casual"),
                    played("bullet", true, "chess", "Bullet"),
                    played("blitz", true, "chess960", "Variant"),
                ]),
            ),
        ],
    );
    let fixture = sites.serve().await;
    let (core, _) = open(profile.path(), &fixture.base);
    ready(&core).await;
    let installed = core
        .call(
            "gameDbPlayerInstall",
            vec![json!({
                "site": "chesscom", "username": "Hikaru", "speeds": ["blitz"], "rated": true
            })],
        )
        .await
        .expect("downloads");
    let database = &installed["database"];
    assert_eq!(database["name"], "Hikaru · Chess.com");
    assert_eq!(database["games"], 2);
    let id = database["id"].as_str().expect("an id");
    assert!(id.starts_with("chesscom-"), "{id}");
    let page = core
        .call(
            "gameDbGames",
            vec![json!({ "database": id, "offset": 0, "limit": 50 })],
        )
        .await
        .expect("a page");
    let mut blacks: Vec<&str> = page["games"]
        .as_array()
        .expect("games")
        .iter()
        .map(|row| row["black"].as_str().unwrap())
        .collect();
    blacks.sort_unstable();
    assert_eq!(blacks, ["Kept", "Old"]);
    // Requests go one at a time: the list, then each month.
    assert_eq!(fixture.to("/pub/player/hikaru/games/archives").len(), 1);
    assert_eq!(fixture.to("/pub/player/hikaru/games/2026/11").len(), 1);
}

#[tokio::test]
async fn a_cancelled_download_keeps_the_players_database_as_it_was() {
    let profile = tempfile::tempdir().expect("profile");
    let sites = Sites::default();
    sites.lichess.lock().unwrap().insert(
        "some_player".into(),
        game("Some_Player", "Rival", "1-0", "1. e4 e5"),
    );
    let fixture = sites.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    ready(&core).await;
    let request = json!({ "site": "lichess", "username": "Some_Player" });
    let first = core
        .call("gameDbPlayerInstall", vec![request.clone()])
        .await
        .expect("downloads");
    let id = first["database"]["id"].as_str().unwrap().to_string();

    // The next download sends one game, then waits: cancelled, the database keeps its game.
    let (send, receive) = mpsc::unbounded_channel();
    sites
        .streams
        .lock()
        .unwrap()
        .insert("some_player".into(), receive);
    send.send(game("Some_Player", "Second", "1-0", "1. d4 d5"))
        .expect("sends");
    let running = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.call("gameDbPlayerInstall", vec![request]).await })
    };
    let partial = profile.path().join(format!("databases/{id}.pgn.partial"));
    let arrived = async {
        while std::fs::metadata(&partial).map_or(0, |m| m.len()) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    };
    tokio::time::timeout(fixtures::REQUEST_WAIT, arrived)
        .await
        .expect("the game arrives");
    core.call("gameDbCancel", vec![]).await.expect("cancels");
    let cancelled = running.await.expect("joins").expect_err("cancelled");
    assert_eq!(cancelled.message, "Download cancelled.");
    assert_eq!(
        host.named("gamedb:progress")
            .last()
            .map(|p| p["phase"].clone()),
        Some(json!("cancelled"))
    );
    let listed = databases(&core).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["games"], 1);
    assert_eq!(
        players(&core, &id, "either").await,
        [("Some_Player".to_string(), "Rival".to_string())]
    );
    assert!(leftovers(profile.path()).is_empty());
    drop(send);
}

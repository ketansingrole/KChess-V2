use super::*;
use crate::test_host::TestHost;
use kchess_contracts::types::{
    GameDbResult, GameDbSide, GameDbSort, PositionLookupKind, SavedLookup,
};
use kchess_domain::pgn::{make_pgn, parse_pgn};
use kchess_domain::rules;
use kchess_testkit::random;
use shakmaty::variant::Variant;
use shakmaty::{Color, Position as _};

fn service(profile: &Path) -> (GameDbService, Arc<TestHost>) {
    let host = TestHost::new();
    let config = Config::new(profile.to_path_buf());
    (GameDbService::new(&config, host.clone()), host)
}

const GAMES: &str = r#"[Event "Tata Steel"]
[Site "Wijk aan Zee"]
[Date "2024.01.20"]
[White "Carlsen, Magnus"]
[Black "Gukesh D"]
[Result "1-0"]
[WhiteElo "2830"]
[BlackElo "2725"]
[ECO "C65"]

1. e4 e5 2. Nf3 Nc6 3. Bb5 Nf6 4. O-O 1-0

[Event "Tata Steel"]
[Date "2024.01.21"]
[White "Gukesh D"]
[Black "Carlsen, Magnus"]
[Result "1/2-1/2"]
[WhiteElo "2725"]
[BlackElo "2830"]

1. Nf3 Nc6 2. e4 e5 3. d4 {an annotated game} exd4 1/2-1/2

[Event "Club"]
[Date "1999.??.??"]
[White "Smith"]
[Black "Jones"]
[Result "0-1"]

1. d4 d5 2. c4 0-1

[Variant "Atomic"]
[White "A"]
[Black "B"]

1. e4 *

[White "Bad"]
[Black "Moves"]

1. e4 e5 2. Ke3 *
"#;

/// Writes `text` as a file outside the profile, as a user picks one.
fn source(dir: &Path, name: &str, text: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

fn query(database: &str) -> GameDbQuery {
    GameDbQuery {
        database: database.to_string(),
        offset: 0,
        limit: 50,
        ..GameDbQuery::default()
    }
}

async fn imported(games: &GameDbService, path: String) -> GameDbImported {
    games
        .import(path, false, None, None, DatabaseSource::Imported)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_import_keeps_the_usable_games_and_lists_the_database() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, host) = service(profile.path());
    let result = imported(
        &games,
        source(files.path(), "Tata 2024.pgn", GAMES.as_bytes()),
    )
    .await;
    assert_eq!((result.added, result.skipped), (3, 2));
    assert_eq!(result.database.name, "Tata 2024");
    assert_eq!(result.database.games, 3);
    let status = games.status().await.unwrap();
    assert_eq!(status.databases, vec![result.database.clone()]);
    assert!(!status.busy);
    let phases: Vec<String> = host
        .named("gamedb:progress")
        .iter()
        .map(|p| p["phase"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(phases.first().map(String::as_str), Some("reading"));
    assert_eq!(phases.last().map(String::as_str), Some("done"));
    assert!(phases.contains(&"indexing".to_string()));
    // Nothing of the import is left beside the database.
    let left: Vec<String> = std::fs::read_dir(profile.path().join("databases"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| {
            !name.ends_with(".kdb") && !name.ends_with("-wal") && !name.ends_with("-shm")
        })
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn searches_filter_by_player_side_event_year_rating_result_and_eco() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let id = imported(&games, source(files.path(), "g.pgn", GAMES.as_bytes()))
        .await
        .database
        .id;
    let names = |page: GameDbPage| -> Vec<(String, String)> {
        page.games.into_iter().map(|g| (g.white, g.black)).collect()
    };
    let carlsen = || GameDbQuery {
        player: Some("magnus C".to_string()),
        ..query(&id)
    };
    assert_eq!(games.games(carlsen()).await.unwrap().total, 2);
    let white = GameDbQuery {
        side: Some(GameDbSide::White),
        ..carlsen()
    };
    assert_eq!(
        names(games.games(white).await.unwrap()),
        [("Carlsen, Magnus".to_string(), "Gukesh D".to_string())]
    );
    let against = GameDbQuery {
        opponent: Some("gukesh".to_string()),
        side: Some(GameDbSide::Black),
        ..carlsen()
    };
    assert_eq!(games.games(against).await.unwrap().total, 1);
    let cases = [
        (
            GameDbQuery {
                event: Some("tata".into()),
                ..query(&id)
            },
            2,
        ),
        (
            GameDbQuery {
                year_to: Some(2000),
                ..query(&id)
            },
            1,
        ),
        (
            GameDbQuery {
                year_from: Some(2024),
                year_to: Some(2024),
                ..query(&id)
            },
            2,
        ),
        (
            GameDbQuery {
                min_elo: Some(2800),
                ..query(&id)
            },
            0,
        ),
        (
            GameDbQuery {
                min_elo: Some(2700),
                ..query(&id)
            },
            2,
        ),
        (
            GameDbQuery {
                result: Some(GameDbResult::Black),
                ..query(&id)
            },
            1,
        ),
        (
            GameDbQuery {
                eco: Some("C6".into()),
                ..query(&id)
            },
            1,
        ),
        (
            GameDbQuery {
                player: Some("nobody".into()),
                ..query(&id)
            },
            0,
        ),
        // `%` and `_` are letters to a search, not patterns.
        (
            GameDbQuery {
                player: Some("%".into()),
                ..query(&id)
            },
            0,
        ),
    ];
    for (query, total) in cases {
        assert_eq!(
            games.games(query.clone()).await.unwrap().total,
            total,
            "{query:?}"
        );
    }
    let newest = games.games(query(&id)).await.unwrap();
    assert_eq!(newest.games[0].date.as_deref(), Some("2024.01.21"));
    let oldest = GameDbQuery {
        sort: Some(GameDbSort::Oldest),
        limit: 1,
        offset: 1,
        ..query(&id)
    };
    let page = games.games(oldest).await.unwrap();
    assert_eq!((page.total, page.games.len(), page.offset), (3, 1, 1));
    assert_eq!(page.games[0].date.as_deref(), Some("2024.01.20"));
}

#[tokio::test]
async fn a_games_pgn_is_its_own_text_or_written_again() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let id = imported(&games, source(files.path(), "g.pgn", GAMES.as_bytes()))
        .await
        .database
        .id;
    let listed = games.games(query(&id)).await.unwrap().games;
    let originals = parse_pgn(GAMES);
    for row in listed {
        let pgn = games.game(&id, row.id).await.unwrap();
        let game = parse_pgn(&pgn).remove(0);
        let original = originals
            .iter()
            .find(|o| o.header("Date") == game.header("Date"))
            .unwrap();
        assert_eq!(pgn, make_pgn(original));
    }
    assert!(games.game(&id, 999).await.is_err());
    assert!(games.game("missing", 1).await.is_err());
}

/// A broadcast's game, with its clocks, evals, extra headers and a name written with spaces
/// around it, reads back exactly as it was written, from the columns of the tables it is in.
#[tokio::test]
async fn a_broadcast_game_reads_back_exactly() {
    let text = "[Event \" Open 2026 \"]\n[Site \"https://example.org/open/r1/g1\"]\n\
        [Date \"2026.10.01\"]\n[Round \"1.4\"]\n[White \" Wang, Justin \"]\n[Black \"Lee, Ann\"]\n\
        [Result \"0-1\"]\n[WhiteElo \"2103\"]\n[WhiteTitle \"CM\"]\n[BlackElo \"1996\"]\n\
        [GameURL \"https://example.org/open/r1/g1\"]\n\n\
        1. e4 { [%eval 0.18] [%clk 1:30:58] } 1... c5 { [%eval 0.32] [%clk 1:30:35] } \
        2. Qh5?? { [%eval -1.5] } { Blunder. } { [%clk 1:29:00] } 2... Nf6 0-1\n";
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let id = imported(&games, source(files.path(), "b.pgn", text.as_bytes()))
        .await
        .database
        .id;
    let row = games.games(query(&id)).await.unwrap().games.remove(0);
    let pgn = games.game(&id, row.id).await.unwrap();
    assert_eq!(pgn, make_pgn(&parse_pgn(text).remove(0)));
}

/// Every tree row is what replaying each game counts, transpositions included.
#[tokio::test]
async fn the_tree_counts_each_position_however_it_was_reached() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let id = imported(&games, source(files.path(), "g.pgn", GAMES.as_bytes()))
        .await
        .database
        .id;
    let mut pos = rules::default_position(Variant::Chess);
    let start = games.tree(&id, gamedb::position_key(&pos)).await.unwrap();
    let total: u64 = start.iter().map(|m| m.stats.games()).sum();
    assert_eq!(total, 3);
    // 1.e4 e5 2.Nf3 Nc6 and 1.Nf3 Nc6 2.e4 e5 meet: both games continue from there.
    for san in ["e4", "e5", "Nf3", "Nc6"] {
        let m = rules::parse_san(&pos, san).unwrap();
        pos.play_unchecked(m);
    }
    let met = games.tree(&id, gamedb::position_key(&pos)).await.unwrap();
    let mut sans: Vec<(String, u64)> = met
        .iter()
        .map(|m| {
            let mv = gamedb::unpack_move(&pos, m.mv).unwrap();
            (rules::san_and_play(&mut pos.clone(), mv), m.stats.games())
        })
        .collect();
    sans.sort();
    assert_eq!(sans, [("Bb5".to_string(), 1), ("d4".to_string(), 1)]);
    let bb5 = met.iter().find(|m| m.stats.white == 1).unwrap();
    assert_eq!(bb5.stats.best_elo, (2830 + 2725) / 2);
}

#[tokio::test]
async fn a_zstd_file_imports_as_its_text_and_a_damaged_one_imports_nothing() {
    use ruzstd::encoding::{CompressionLevel, compress_to_vec};
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, host) = service(profile.path());
    let packed = compress_to_vec(GAMES.as_bytes(), CompressionLevel::Fastest);
    let path = source(files.path(), "g.pgn.zst", &packed);
    let result = games
        .import(path, true, None, None, DatabaseSource::Imported)
        .await
        .unwrap();
    assert_eq!(result.added, 3);
    let mut damaged = packed.clone();
    let middle = damaged.len() / 2;
    damaged[middle] ^= 0xff;
    damaged.truncate(damaged.len() - 3);
    let path = source(files.path(), "bad.pgn.zst", &damaged);
    let error = games
        .import(path, true, None, None, DatabaseSource::Imported)
        .await
        .unwrap_err();
    assert!(
        error.message.contains("not a readable"),
        "{}",
        error.message
    );
    assert_eq!(games.status().await.unwrap().databases.len(), 1);
    let failed = host.named("gamedb:progress");
    assert_eq!(failed.last().unwrap()["phase"], "failed");
}

#[tokio::test]
async fn adding_to_a_database_sums_its_tree_and_a_failed_addition_leaves_it_as_it_was() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let id = imported(&games, source(files.path(), "g.pgn", GAMES.as_bytes()))
        .await
        .database
        .id;
    let start = gamedb::position_key(&rules::default_position(Variant::Chess));
    let more = "[White \"New\"]\n[Black \"Game\"]\n[Result \"1-0\"]\n\n1. e4 c5 1-0\n";
    let added = games
        .import(
            source(files.path(), "more.pgn", more.as_bytes()),
            false,
            Some(id.clone()),
            None,
            DatabaseSource::Imported,
        )
        .await
        .unwrap();
    assert_eq!((added.added, added.database.games), (1, 4));
    let e4 = |moves: Vec<tree::TreeMove>| {
        moves
            .into_iter()
            .find(|m| m.stats.games() >= 2)
            .map(|m| m.stats)
            .unwrap()
    };
    let before = e4(games.tree(&id, start).await.unwrap());
    assert_eq!((before.white, before.games()), (2, 2));
    // A damaged file fails half way: nothing it read stays.
    let broken = [more.as_bytes(), b"\n\n"].concat();
    let mut packed = ruzstd::encoding::compress_to_vec(
        &broken.repeat(50)[..],
        ruzstd::encoding::CompressionLevel::Fastest,
    );
    packed.truncate(packed.len() - 4);
    let failed = games
        .import(
            source(files.path(), "broken.pgn.zst", &packed),
            true,
            Some(id.clone()),
            None,
            DatabaseSource::Imported,
        )
        .await;
    assert!(failed.is_err());
    let status = games.status().await.unwrap();
    assert_eq!(status.databases[0].games, 4);
    assert_eq!(games.games(query(&id)).await.unwrap().total, 4);
    assert_eq!(e4(games.tree(&id, start).await.unwrap()), before);
}

#[tokio::test]
async fn a_cancelled_import_leaves_no_database() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, host) = service(profile.path());
    let many = GAMES.repeat(2_000);
    let path = source(files.path(), "many.pgn", many.as_bytes());
    let games = Arc::new(games);
    let running = {
        let games = Arc::clone(&games);
        tokio::spawn(async move {
            games
                .import(path, false, None, None, DatabaseSource::Imported)
                .await
        })
    };
    while host.named("gamedb:progress").is_empty() {
        tokio::task::yield_now().await;
    }
    games.cancel();
    assert!(running.await.unwrap().is_err());
    assert_eq!(
        host.named("gamedb:progress").last().unwrap()["phase"],
        "cancelled"
    );
    let status = games.status().await.unwrap();
    assert!(status.databases.is_empty() && !status.busy);
    let left = std::fs::read_dir(profile.path().join("databases"))
        .unwrap()
        .count();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn one_import_runs_at_a_time_and_files_of_the_profile_are_refused() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let inside = source(profile.path(), "inside.pgn", GAMES.as_bytes());
    let error = games
        .import(inside, false, None, None, DatabaseSource::Imported)
        .await
        .unwrap_err();
    assert!(error.message.contains("outside"), "{}", error.message);
    let path = source(files.path(), "a.pgn", GAMES.as_bytes());
    let missing = games
        .import(
            path,
            false,
            Some("nope".into()),
            None,
            DatabaseSource::Imported,
        )
        .await
        .unwrap_err();
    assert!(missing.message.contains("not on this computer"));
}

#[tokio::test]
async fn a_database_is_renamed_and_deleted() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let id = imported(&games, source(files.path(), "g.pgn", GAMES.as_bytes()))
        .await
        .database
        .id;
    // Read once, so its file is held open when it is deleted.
    games.games(query(&id)).await.unwrap();
    let renamed = games.rename(&id, "Tata Steel 2024".into()).await.unwrap();
    assert_eq!(renamed.databases[0].name, "Tata Steel 2024");
    let deleted = games.delete(&id).await.unwrap();
    assert!(deleted.databases.is_empty());
    assert!(games.games(query(&id)).await.is_err());
    assert_eq!(
        std::fs::read_dir(profile.path().join("databases"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn searches_by_player_read_the_name_index() {
    let profile = tempfile::tempdir().unwrap();
    let id = write_database(profile.path(), "G", GAMES).unwrap();
    let db =
        schema::open_read(&profile.path().join("databases").join(format!("{id}.kdb"))).unwrap();
    let plan: Vec<String> = db
        .prepare(
            "EXPLAIN QUERY PLAN SELECT id FROM games g
             WHERE g.white IN (SELECT id FROM players WHERE norm LIKE 'c%') ORDER BY date_key",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(
        plan.iter().any(|step| step.contains("games_white")),
        "{plan:?}"
    );
    let tree: String = db
        .query_row(
            "EXPLAIN QUERY PLAN SELECT mv FROM tree WHERE key = 1",
            [],
            |row| row.get(3),
        )
        .unwrap();
    assert!(tree.contains("PRIMARY KEY"), "{tree}");
    let rated: String = db
        .query_row(
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM games g WHERE g.low_elo >= 2600",
            [],
            |row| row.get(3),
        )
        .unwrap();
    assert!(rated.contains("games_low_elo"), "{rated}");
}

/// The tree's memory is one table that never grows: `TREE_ROWS` rows fill it exactly.
#[test]
fn the_tree_buffer_fills_one_table() {
    let rows: HashMap<(u64, u16), tree::Stats> = HashMap::with_capacity(super::TREE_ROWS);
    assert_eq!(rows.capacity(), super::TREE_ROWS);
    assert_eq!(std::mem::size_of::<((u64, u16), tree::Stats)>(), 64);
}

/// Random games, read in random-sized batches of tree memory: the tree always matches a replay.
#[test]
fn the_tree_matches_a_replay_of_random_games() {
    random::check(5, |rng| {
        let profile = tempfile::tempdir().unwrap();
        let mut pgn = String::new();
        let mut expected: HashMap<(u64, u16), u64> = HashMap::new();
        for game in 0..60 {
            let mut pos = rules::default_position(Variant::Chess);
            let mut sans = Vec::new();
            for ply in 0..rng.below(50) {
                let legal = pos.legal_moves();
                let Some(m) = rng.pick(&legal).copied() else {
                    break;
                };
                if ply < schema::INDEX_DEPTH as usize {
                    *expected
                        .entry((gamedb::position_key(&pos), gamedb::pack_move(&m)))
                        .or_default() += 1;
                }
                sans.push(rules::san_and_play(&mut pos, m));
            }
            pgn.push_str(&format!(
                "[White \"P{game}\"]\n[Black \"Q\"]\n[Result \"1/2-1/2\"]\n\n{} 1/2-1/2\n\n",
                kchess_domain::pgn::pgn_from_san(&sans).trim_end()
            ));
        }
        // An unfinished game is in the database but not in its tree.
        pgn.push_str("[White \"Live\"]\n[Black \"Game\"]\n[Result \"*\"]\n\n1. e4 e5 2. Nf3 *\n\n");
        let id = write_database(profile.path(), "R", &pgn).unwrap();
        let db =
            schema::open_read(&profile.path().join("databases").join(format!("{id}.kdb"))).unwrap();
        let rows: HashMap<(u64, u16), u64> = db
            .prepare("SELECT key, mv, draws FROM tree UNION ALL SELECT key, mv, 1 FROM lone")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    (row.get::<_, i64>(0)? as u64, row.get(1)?),
                    row.get::<_, i64>(2)? as u64,
                ))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(rows, expected);
    });
}

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

fn in_database(id: &str) -> LookupOptions {
    LookupOptions {
        database: Some(id.to_string()),
        ..LookupOptions::default()
    }
}

/// A lookup's moves as `san white/draws/black`, sorted.
fn counted(lookup: &SavedLookup) -> Vec<String> {
    let mut moves: Vec<String> = lookup
        .moves
        .iter()
        .map(|m| {
            format!(
                "{} {}/{}/{}",
                m.san,
                m.white.unwrap(),
                m.draws.unwrap(),
                m.black.unwrap()
            )
        })
        .collect();
    moves.sort();
    moves
}

#[tokio::test]
async fn a_lookup_lists_the_moves_and_best_rated_games_of_a_position() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let id = imported(&games, source(files.path(), "g.pgn", GAMES.as_bytes()))
        .await
        .database
        .id;
    let found = games.lookup(START.into(), in_database(&id)).await.unwrap();
    assert!(!found.cached && !found.stale);
    let found = found.saved;
    assert_eq!(found.kind, PositionLookupKind::Database);
    assert_eq!(found.total, Some(3));
    assert_eq!(counted(&found), ["Nf3 0/1/0", "d4 0/0/1", "e4 1/0/0"]);
    assert_eq!(
        found.moves.iter().find(|m| m.san == "e4").unwrap().uci,
        "e2e4"
    );
    // The rated games first, each with the move it played from here; the unrated one last.
    let listed = found.games.unwrap();
    let rows: Vec<(&str, Option<&str>, Option<i64>)> = listed
        .iter()
        .map(|game| (game.white.as_str(), game.san.as_deref(), game.year))
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2], ("Smith", Some("d4"), Some(1999)));
    assert_eq!(listed[2].winner, Some(Color::Black));
    assert!(listed[0].white_rating.is_some() && listed[1].black_rating.is_some());
    // A listed game's id opens it.
    let pgn = games
        .game(&id, listed[0].id.parse().unwrap())
        .await
        .unwrap();
    assert!(pgn.contains("[Event \"Tata Steel\"]"));

    let refused = games.lookup(START.into(), LookupOptions::default()).await;
    assert_eq!(refused.unwrap_err().message, "Choose a game database.");
    let missing = games.lookup(START.into(), in_database("nope")).await;
    assert_eq!(
        missing.unwrap_err().message,
        "That database is not on this computer."
    );
}

#[tokio::test]
async fn filtered_lookups_count_only_the_games_that_match() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let id = imported(&games, source(files.path(), "g.pgn", GAMES.as_bytes()))
        .await
        .database
        .id;
    let look = async |fen: &str, options: LookupOptions| {
        counted(&games.lookup(fen.into(), options).await.unwrap().saved)
    };
    let carlsen = LookupOptions {
        player: Some("carlsen".into()),
        ..in_database(&id)
    };
    assert_eq!(
        look(START, carlsen.clone()).await,
        ["Nf3 0/1/0", "e4 1/0/0"]
    );
    let as_black = LookupOptions {
        color: Some(Color::Black),
        ..carlsen.clone()
    };
    assert_eq!(look(START, as_black).await, ["Nf3 0/1/0"]);
    let rated = LookupOptions {
        min_elo: Some(2700),
        ..in_database(&id)
    };
    assert_eq!(look(START, rated).await, ["Nf3 0/1/0", "e4 1/0/0"]);
    let old = LookupOptions {
        until: Some("2000".into()),
        ..in_database(&id)
    };
    assert_eq!(look(START, old).await, ["d4 0/0/1"]);
    let nobody = LookupOptions {
        player: Some("nobody".into()),
        ..in_database(&id)
    };
    assert!(look(START, nobody).await.is_empty());

    // Replayed, the two Carlsen games still meet after 1.e4 e5 2.Nf3 Nc6 by either order.
    let mut pos = rules::default_position(Variant::Chess);
    for san in ["e4", "e5", "Nf3", "Nc6"] {
        let m = rules::parse_san(&pos, san).unwrap();
        pos.play_unchecked(m);
    }
    let met = rules::make_fen(&pos);
    assert_eq!(look(&met, carlsen).await, ["Bb5 1/0/0", "d4 0/1/0"]);
}

/// A filter every game matches replays them all on every core, by an index or in file order, and
/// must count what the tree counted while importing them, best games included.
#[test]
fn a_filter_every_game_matches_finds_what_the_tree_finds() {
    random::check(3, |rng| {
        let profile = tempfile::tempdir().unwrap();
        let mut pgn = String::new();
        let mut lines = Vec::new();
        // More than two batches (`explore::BATCH`), so their counts are summed.
        for game in 0..2_100 {
            let mut pos = rules::default_position(Variant::Chess);
            let mut sans = Vec::new();
            let mut fens = vec![rules::make_fen(&pos)];
            // Within the tree's depth, so both count the same plies.
            for _ in 0..rng.below(schema::INDEX_DEPTH as usize) {
                let legal = pos.legal_moves();
                let Some(m) = rng.pick(&legal).copied() else {
                    break;
                };
                sans.push(rules::san_and_play(&mut pos, m));
                fens.push(rules::make_fen(&pos));
            }
            let result = ["1-0", "0-1", "1/2-1/2"][game % 3];
            let elo = 2000 + rng.below(800);
            // The moves alone: `pgn_from_san` writes headers of its own, which would replace these.
            let moves: Vec<String> = sans
                .iter()
                .enumerate()
                .map(|(ply, san)| match ply % 2 {
                    0 => format!("{}. {san}", ply / 2 + 1),
                    _ => san.clone(),
                })
                .collect();
            pgn.push_str(&format!(
                "[White \"P{game}\"]\n[Black \"Q\"]\n[Result \"{result}\"]\n\
                 [WhiteElo \"{elo}\"]\n[BlackElo \"{elo}\"]\n\n{} {result}\n\n",
                moves.join(" ")
            ));
            lines.push(fens);
        }
        let id = write_database(profile.path(), "R", &pgn).unwrap();
        let db =
            schema::open_read(&profile.path().join("databases").join(format!("{id}.kdb"))).unwrap();
        let everyone = LookupOptions {
            player: Some("Q".into()),
            ..in_database(&id)
        };
        let rated = LookupOptions {
            min_elo: Some(1),
            ..in_database(&id)
        };
        let go = AtomicBool::new(false);
        for _ in 0..30 {
            let fens = &lines[rng.below(lines.len())];
            let fen = &fens[rng.below(fens.len().min(8))];
            let tree = explore::lookup(&db, &id, fen, &in_database(&id), 0, &go).unwrap();
            // By the player index, and in file order: every game is rated, so the rating is broad.
            for filter in [&everyone, &rated] {
                let replayed = explore::lookup(&db, &id, fen, filter, 0, &go).unwrap();
                assert_eq!(replayed.moves, tree.moves, "{fen}");
                assert_eq!(replayed.total, tree.total, "{fen}");
                assert_eq!(replayed.games, tree.games, "{fen}");
            }
        }
    });
}

/// A lookup its caller dropped stops replaying and says so, as a cancellation, not a failure.
#[test]
fn a_stopped_lookup_ends_as_cancelled() {
    let profile = tempfile::tempdir().unwrap();
    let id = write_database(profile.path(), "S", GAMES).unwrap();
    let db =
        schema::open_read(&profile.path().join("databases").join(format!("{id}.kdb"))).unwrap();
    let carlsen = LookupOptions {
        player: Some("carlsen".into()),
        ..in_database(&id)
    };
    let stopped = AtomicBool::new(true);
    let error = explore::lookup(&db, &id, START, &carlsen, 0, &stopped).unwrap_err();
    assert!(error.aborted, "{error:?}");
    let go = AtomicBool::new(false);
    assert!(explore::lookup(&db, &id, START, &carlsen, 0, &go).is_ok());
}

/// A file an older release wrote has no replay index and no index of its names' words: lookups
/// and searches read the games' rows and every name instead, and answer alike; reading it
/// through the service adds both, the words split as an import splits them.
#[tokio::test]
async fn an_older_file_answers_alike_and_gains_the_replay_and_word_indexes() {
    let profile = tempfile::tempdir().unwrap();
    let id = write_database(profile.path(), "Old", GAMES).unwrap();
    let path = profile.path().join("databases").join(format!("{id}.kdb"));
    let filters = [
        LookupOptions {
            player: Some("carlsen".into()),
            ..in_database(&id)
        },
        LookupOptions {
            min_elo: Some(1),
            ..in_database(&id)
        },
        LookupOptions {
            since: Some("1900".into()),
            ..in_database(&id)
        },
    ];
    let go = AtomicBool::new(false);
    let searches = ["carlsen", "magnus c", "gukesh", "zz"].map(|player| GameDbQuery {
        database: id.clone(),
        player: Some(player.into()),
        limit: 50,
        ..GameDbQuery::default()
    });
    let answers = || {
        let db = schema::open_read(&path).unwrap();
        let looked: Vec<_> = filters
            .iter()
            .map(|options| {
                let found = explore::lookup(&db, &id, START, options, 0, &go).unwrap();
                (found.moves, found.games)
            })
            .collect();
        let searched: Vec<_> = searches
            .iter()
            .map(|query| search::games(&db, query).unwrap().games)
            .collect();
        (looked, searched)
    };
    let words = |db: &rusqlite::Connection| -> Vec<(String, i64)> {
        let mut rows = Vec::new();
        for table in ["player_words", "event_words"] {
            let mut select = db
                .prepare(&format!("SELECT word, id FROM {table} ORDER BY word, id"))
                .unwrap();
            let found = select
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap();
            rows.extend(found.map(|row| row.unwrap()));
        }
        rows
    };
    let indexed = answers();
    assert!(indexed.0.iter().all(|(moves, _)| !moves.is_empty()));
    assert!(indexed.1[..3].iter().all(|rows| !rows.is_empty()));
    assert!(indexed.1[3].is_empty());
    let imported = words(&schema::open_read(&path).unwrap());
    assert!(
        imported.contains(&("magnus".to_string(), 1)),
        "{imported:?}"
    );
    let replay_index = |db: &rusqlite::Connection| -> bool {
        db.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'games_replay')",
            [],
            |row| row.get(0),
        )
        .unwrap()
    };
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "DROP INDEX games_replay; DROP TABLE player_words; DROP TABLE event_words;
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }
    assert_eq!(answers(), indexed);

    let (games, _) = service(profile.path());
    games
        .lookup(START.into(), filters[0].clone())
        .await
        .unwrap();
    let db = schema::open_read(&path).unwrap();
    assert!(replay_index(&db));
    assert_eq!(words(&db), imported);
    drop(db);
    assert_eq!(answers(), indexed);
    games.close().await;
}

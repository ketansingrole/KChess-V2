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
        .import(
            path,
            PgnPacking::Plain,
            None,
            None,
            DatabaseSource::Imported,
        )
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

/// `text` packed as bzip2, as one stream.
fn bzipped(text: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(text).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn a_bzip2_file_imports_every_stream_and_a_damaged_one_imports_nothing() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, host) = service(profile.path());
    // Two streams, as parallel compressors write them, split inside the second game.
    let cut = GAMES.find("[Event \"Club\"]").unwrap() - 40;
    let mut packed = bzipped(&GAMES.as_bytes()[..cut]);
    packed.extend(bzipped(&GAMES.as_bytes()[cut..]));
    let path = source(
        files.path(),
        "ficsgamesdb_2024_standard2000_nomovetimes_7.pgn.bz2",
        &packed,
    );
    let result = games
        .import(
            path,
            PgnPacking::Bzip2,
            None,
            None,
            DatabaseSource::Imported,
        )
        .await
        .unwrap();
    // As the text itself imports: its Atomic and broken games are skipped.
    assert_eq!((result.added, result.skipped), (3, 2));
    assert_eq!(
        result.database.name,
        "ficsgamesdb_2024_standard2000_nomovetimes_7"
    );
    let mut damaged = bzipped(GAMES.as_bytes());
    let middle = damaged.len() / 2;
    damaged[middle] ^= 0xff;
    let path = source(files.path(), "bad.pgn.bz2", &damaged);
    let error = games
        .import(
            path,
            PgnPacking::Bzip2,
            None,
            None,
            DatabaseSource::Imported,
        )
        .await
        .unwrap_err();
    assert!(
        error.message.contains("not a readable .pgn.bz2"),
        "{}",
        error.message
    );
    // A text that is not bzip2 at all.
    let path = source(files.path(), "plain.pgn.bz2", GAMES.as_bytes());
    let error = games
        .import(
            path,
            PgnPacking::Bzip2,
            None,
            None,
            DatabaseSource::Imported,
        )
        .await
        .unwrap_err();
    assert!(
        error.message.contains("not a readable .pgn.bz2"),
        "{}",
        error.message
    );
    assert_eq!(games.status().await.unwrap().databases.len(), 1);
    let failed = host.named("gamedb:progress");
    assert_eq!(failed.last().unwrap()["phase"], "failed");
}

#[tokio::test]
async fn a_zstd_file_imports_as_its_text_and_a_damaged_one_imports_nothing() {
    use ruzstd::encoding::{CompressionLevel, compress_to_vec};
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, host) = service(profile.path());
    let packed = compress_to_vec(GAMES.as_bytes(), CompressionLevel::Fastest);
    let path = source(files.path(), "g.pgn.zst", &packed);
    let result = games
        .import(path, PgnPacking::Zstd, None, None, DatabaseSource::Imported)
        .await
        .unwrap();
    assert_eq!(result.added, 3);
    let mut damaged = packed.clone();
    let middle = damaged.len() / 2;
    damaged[middle] ^= 0xff;
    damaged.truncate(damaged.len() - 3);
    let path = source(files.path(), "bad.pgn.zst", &damaged);
    let error = games
        .import(path, PgnPacking::Zstd, None, None, DatabaseSource::Imported)
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

/// A zip of `entries`, deflated.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        if name.ends_with('/') {
            writer.add_directory(*name, options).unwrap();
        } else {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

/// `zip` with each entry marked as encrypted, as a password-protected archive marks them: bit 0
/// of the flags in every local header (`PK\x03\x04`, at 6) and central directory header
/// (`PK\x01\x02`, at 8).
fn locked(mut zip: Vec<u8>) -> Vec<u8> {
    let headers: Vec<(usize, usize)> = zip
        .windows(4)
        .enumerate()
        .filter_map(|(at, window)| match window {
            b"PK\x03\x04" => Some((at, 6)),
            b"PK\x01\x02" => Some((at, 8)),
            _ => None,
        })
        .collect();
    for (at, flags) in headers {
        zip[at + flags] |= 1;
    }
    zip
}

#[tokio::test]
async fn a_zip_imports_every_pgn_file_it_holds_and_nothing_else() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, host) = service(profile.path());
    // The first file's last game has no line ending: the next file's first game stays its own.
    let first = "[White \"First\"]\n[Black \"File\"]\n[Result \"1-0\"]\n\n1. e4 e5 1-0";
    let packed = zip_of(&[
        ("twic1620/", b""),
        ("twic1620/twic1620.pgn", first.as_bytes()),
        (
            "twic1620/notes.txt",
            b"[White \"Not\"]\n[Black \"Games\"]\n\n1. e4 *\n",
        ),
        (
            "__MACOSX/twic1620/._twic1620.pgn",
            b"\0\x05\x16\x07 attributes",
        ),
        ("twic1620/More.PGN", GAMES.as_bytes()),
    ]);
    let path = source(files.path(), "TWIC 1620.zip", &packed);
    let result = games
        .import(path, PgnPacking::Zip, None, None, DatabaseSource::Imported)
        .await
        .unwrap();
    assert_eq!((result.added, result.skipped), (4, 2));
    assert_eq!(result.database.name, "TWIC 1620");
    let listed = games.games(query(&result.database.id)).await.unwrap();
    let mut players: Vec<String> = listed.games.iter().map(|g| g.white.clone()).collect();
    players.sort();
    assert!(players.contains(&"First".to_string()), "{players:?}");
    assert!(!players.contains(&"Not".to_string()), "{players:?}");
    // Progress counts the archive's bytes, up to all of them.
    let progress = host.named("gamedb:progress");
    let done = progress.last().unwrap();
    assert_eq!(done["phase"], "done");
    assert_eq!(done["totalBytes"], packed.len() as u64);
}

#[tokio::test]
async fn a_zip_with_no_pgn_or_a_damaged_or_locked_one_imports_nothing() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let refused = |name: &str, bytes: &[u8]| {
        let path = source(files.path(), name, bytes);
        let games = &games;
        async move {
            games
                .import(path, PgnPacking::Zip, None, None, DatabaseSource::Imported)
                .await
                .unwrap_err()
                .message
        }
    };
    let scid = zip_of(&[("caissabase.si4", b"SCID"), ("readme.txt", b"hi")]);
    assert_eq!(
        refused("caissabase.zip", &scid).await,
        "That .zip holds no .pgn file."
    );
    let whole = zip_of(&[("games.pgn", GAMES.repeat(40).as_bytes())]);
    let cut = &whole[..whole.len() / 2];
    assert!(
        refused("cut.zip", cut)
            .await
            .starts_with("That .zip could not be read"),
        "a zip cut short"
    );
    // A damaged byte in the deflated games fails their check, half way through.
    let mut damaged = whole.clone();
    let middle = damaged.len() / 3;
    damaged[middle] ^= 0xff;
    assert!(
        refused("damaged.zip", &damaged)
            .await
            .starts_with("That .zip could not be read"),
        "a damaged zip"
    );
    let locked = locked(zip_of(&[("games.pgn", GAMES.as_bytes())]));
    assert_eq!(
        refused("locked.zip", &locked).await,
        "That .zip is password-protected."
    );
    let status = games.status().await.unwrap();
    assert!(status.databases.is_empty(), "{:?}", status.databases);
    let leftovers: Vec<_> = std::fs::read_dir(profile.path().join("databases"))
        .map(|dir| {
            dir.filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name())
                .collect()
        })
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// A `.7z` fixture (`fixtures/README.md` says what each holds): files rather than archives these
/// tests write, so no build turns on the 7z encoders or AES.
fn seven_zip(name: &str) -> &'static [u8] {
    match name {
        "mixed-lzma2" => include_bytes!("fixtures/mixed-lzma2.7z"),
        "mixed-solid-ppmd" => include_bytes!("fixtures/mixed-solid-ppmd.7z"),
        "no-pgn" => include_bytes!("fixtures/no-pgn.7z"),
        "many" => include_bytes!("fixtures/many.7z"),
        "locked" => include_bytes!("fixtures/locked.7z"),
        "locked-names" => include_bytes!("fixtures/locked-names.7z"),
        _ => panic!("no fixture {name}"),
    }
}

#[tokio::test]
async fn a_7z_imports_every_pgn_file_it_holds_solid_or_not() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, host) = service(profile.path());
    // A folder, a text file, macOS's attributes and two PGN files (the first ending without a
    // line ending, the second `GAMES`), each compressed on its own with LZMA2, 7-Zip's default.
    let packed = seven_zip("mixed-lzma2");
    let path = source(files.path(), "CCRL-4040.[5].pgn.7z", packed);
    let result = games
        .import(
            path,
            PgnPacking::SevenZip,
            None,
            None,
            DatabaseSource::Imported,
        )
        .await
        .unwrap();
    assert_eq!((result.added, result.skipped), (4, 2));
    assert_eq!(result.database.name, "CCRL-4040.[5]");
    let listed = games.games(query(&result.database.id)).await.unwrap();
    let players: Vec<String> = listed.games.iter().map(|g| g.white.clone()).collect();
    assert!(players.contains(&"First".to_string()), "{players:?}");
    assert!(!players.contains(&"Not".to_string()), "{players:?}");
    let done = host.named("gamedb:progress");
    let done = done.last().unwrap();
    assert_eq!(done["phase"], "done");
    assert_eq!(done["totalBytes"], packed.len() as u64);
    // The same files in one solid PPMd stream, as 7-Zip packs text: the others decode on the way.
    let path = source(
        files.path(),
        "Lumbra OTB Elite.7z",
        seven_zip("mixed-solid-ppmd"),
    );
    let result = games
        .import(
            path,
            PgnPacking::SevenZip,
            None,
            None,
            DatabaseSource::Imported,
        )
        .await
        .unwrap();
    assert_eq!((result.added, result.skipped), (4, 2));
    assert_eq!(result.database.name, "Lumbra OTB Elite");
}

#[tokio::test]
async fn a_7z_with_no_pgn_or_a_damaged_or_locked_one_imports_nothing() {
    let (profile, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (games, _) = service(profile.path());
    let refused = |name: &str, bytes: &[u8]| {
        let path = source(files.path(), name, bytes);
        let games = &games;
        async move {
            games
                .import(
                    path,
                    PgnPacking::SevenZip,
                    None,
                    None,
                    DatabaseSource::Imported,
                )
                .await
                .unwrap_err()
                .message
        }
    };
    assert_eq!(
        refused("lumbra si5.7z", seven_zip("no-pgn")).await,
        "That .7z holds no .pgn file."
    );
    let whole = seven_zip("many");
    let message = refused("cut.7z", &whole[..whole.len() / 2]).await;
    assert!(
        message.starts_with("That .7z could not be read"),
        "{message}"
    );
    // A damaged byte in the packed games (they start after the 32-byte signature header) fails
    // their check.
    let mut damaged = whole.to_vec();
    damaged[40] ^= 0xff;
    let message = refused("damaged.7z", &damaged).await;
    assert!(
        message.starts_with("That .7z could not be read"),
        "{message}"
    );
    // Encrypted files, then encrypted names too: the reader has no AES, and refuses either.
    for locked in ["locked", "locked-names"] {
        assert_eq!(
            refused(&format!("{locked}.7z"), seven_zip(locked)).await,
            "That .7z is password-protected.",
            "{locked}"
        );
    }
    let status = games.status().await.unwrap();
    assert!(status.databases.is_empty(), "{:?}", status.databases);
    let leftovers: Vec<_> = std::fs::read_dir(profile.path().join("databases"))
        .map(|dir| {
            dir.filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name())
                .collect()
        })
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");
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
            PgnPacking::Plain,
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
            PgnPacking::Zstd,
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
                .import(
                    path,
                    PgnPacking::Plain,
                    None,
                    None,
                    DatabaseSource::Imported,
                )
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
        .import(
            inside,
            PgnPacking::Plain,
            None,
            None,
            DatabaseSource::Imported,
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("outside"), "{}", error.message);
    let path = source(files.path(), "a.pgn", GAMES.as_bytes());
    let missing = games
        .import(
            path,
            PgnPacking::Plain,
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

/* ── "Your games" ── */

/// A profile's store and game databases, with Alice's Lichess account to sync.
struct Own {
    profile: tempfile::TempDir,
    database: Arc<Database>,
    games: Arc<GameDbService>,
    host: Arc<TestHost>,
}

/// Longer than any test: the keeper runs a pass only when the test settles it.
const NEVER_QUIET: Duration = Duration::from_secs(3_600);

impl Own {
    async fn new(profile: tempfile::TempDir) -> Own {
        let host = TestHost::new();
        let config = Config::new(profile.path().to_path_buf());
        let database = Arc::new(Database::new(config.clone(), host.clone()));
        database
            .write_async(|ctx| crate::store::games::add_account(ctx.db, "Alice", true))
            .await
            .unwrap();
        Own {
            games: Arc::new(GameDbService::new(&config, host.clone())),
            profile,
            database,
            host,
        }
    }

    /// The keeper, following the games once they have not changed for `quiet`.
    fn keep(&self, quiet: Duration) {
        self.games.keep_own_after(Arc::clone(&self.database), quiet);
    }

    /// The core closed and another opened on the same profile, as the next start does.
    async fn reopened(self) -> Own {
        self.games.close().await;
        self.database.close_async().await;
        Own::new(self.profile).await
    }

    async fn sync(&self, games: Vec<serde_json::Value>) {
        self.database
            .write_async(move |ctx| {
                crate::store::games::save_games_page(ctx.host, ctx.db, "Alice", &games, &[])
            })
            .await
            .unwrap();
    }

    /// A game against the computer, the fool's mate, finished or still being played.
    async fn play(&self, id: &str, finished: bool) {
        let result = if finished { "0-1" } else { "*" };
        self.play_line(id, &["f2f3", "e7e5", "g2g4", "d8h4"], result)
            .await;
    }

    /// A finished game against the computer: its UCI moves and its result.
    async fn play_line(&self, id: &str, moves: &[&str], result: &str) {
        let finished = result != "*";
        let game = kchess_domain::library::decode_archived_game(&serde_json::json!({
            "id": id, "source": "computer", "startedAt": 1_761_091_200_000.0_f64,
            "updatedAt": 1_761_091_260_000.0_f64, "white": "Stockfish", "black": "You",
            "result": result, "reason": "checkmate",
            "timeControl": "-", "finished": finished,
            "setup": { "variant": "standard", "fen": START },
            "moves": moves,
        }))
        .unwrap();
        self.database
            .write_async(move |ctx| crate::store::library::write_game(ctx.db, &game))
            .await
            .unwrap();
    }

    async fn remove_played(&self, id: &'static str) {
        self.database
            .write_async(move |ctx| crate::store::library::remove_archived_game(ctx.db, id))
            .await
            .unwrap();
    }

    async fn own(&self) -> Option<DatabaseInfo> {
        let status = self.games.status().await.unwrap();
        status
            .databases
            .into_iter()
            .find(|database| database.id == own::OWN_ID)
    }

    /// Waits for the keeper's `rebuilds`th rebuild: told by the keeper, never slept for.
    async fn rebuilt(&self, rebuilds: u64) {
        let mut report = self.games.own_report();
        tokio::time::timeout(
            Duration::from_secs(60),
            report.wait_for(|report| report.rebuilds >= rebuilds),
        )
        .await
        .expect("the rebuild ends")
        .unwrap();
    }

    /// The names of what a rebuild writes before its database is whole.
    fn half_written(&self) -> Vec<String> {
        std::fs::read_dir(&self.games.inner.dir)
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|name| {
                        name.ends_with(".partial")
                            || name.starts_with(".tree-")
                            || name.starts_with(".deep-")
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A synced Lichess game of Alice's: with Lichess's PGN, or with only its moves.
fn synced(id: &str, sans: &[&str], with_pgn: bool) -> serde_json::Value {
    let numbered: Vec<String> = sans
        .chunks(2)
        .enumerate()
        .map(|(n, pair)| format!("{}. {}", n + 1, pair.join(" ")))
        .collect();
    let pgn = format!(
        "[Event \"Rated blitz game\"]\n[Site \"https://lichess.org/{id}\"]\n\
         [Date \"2026.10.01\"]\n[White \"Alice\"]\n[Black \"Bob\"]\n[Result \"1-0\"]\n\
         [WhiteElo \"1900\"]\n[BlackElo \"1850\"]\n\n{} 1-0\n",
        numbered.join(" ")
    );
    serde_json::json!({
        "account": "Alice", "id": id, "createdAt": 1_759_276_800_000_i64,
        "lastMoveAt": 1_759_277_000_000_i64, "rated": true, "speed": "blitz", "perf": "blitz",
        "status": "resign", "winner": "white", "color": "white", "opponent": "Bob",
        "opponentRating": 1850, "playerRating": 1900, "ratingDiff": 6,
        "moves": sans.join(" "),
        "pgn": if with_pgn { serde_json::Value::String(pgn) } else { serde_json::Value::Null },
    })
}

/// A game that reaches its own position only past the opening tree: e4 e5, the knights go out
/// and back ten times (40 plies), then h4 h5, then `then`. Its UCI moves, and the FEN after h4
/// h5, 44 plies in.
fn past_the_tree(then: &[&'static str]) -> (Vec<&'static str>, String) {
    let mut moves: Vec<&'static str> = vec!["e2e4", "e7e5"];
    for _ in 0..10 {
        moves.extend(["g1f3", "g8f6", "f3g1", "f6g8"]);
    }
    moves.extend(["h2h4", "h7h5"]);
    let mut pos = shakmaty::Chess::default();
    for uci in &moves {
        let m = uci
            .parse::<shakmaty::uci::UciMove>()
            .unwrap()
            .to_move(&pos)
            .unwrap();
        pos.play_unchecked(m);
    }
    let fen = shakmaty::fen::Fen::from_position(&pos, shakmaty::EnPassantMode::Legal).to_string();
    moves.extend(then);
    (moves, fen)
}

fn by(player: &str) -> GameDbQuery {
    GameDbQuery {
        player: Some(player.to_string()),
        ..query(own::OWN_ID)
    }
}

/// The synced games and the finished played ones become "Your games" after a quiet moment, and
/// it follows them: a removed game leaves it, and with no games left it is gone. Other databases
/// stay as they are, and "Your games" is never renamed, deleted or imported into.
#[tokio::test]
async fn your_games_follow_the_synced_and_played_games() {
    let own = Own::new(tempfile::tempdir().unwrap()).await;
    own.sync(vec![
        synced("g1", &["e4", "e5", "Nf3"], true),
        // An older game kept without Lichess's PGN is written from its moves.
        synced("g2", &["d4", "d5", "c4"], false),
    ])
    .await;
    own.play("won", true).await;
    // A game still being played waits until it ends.
    own.play("playing", false).await;
    let (long, deep) = past_the_tree(&["a2a4", "a7a5"]);
    own.play_line("long", &long, "0-1").await;
    own.keep(Duration::from_millis(20));
    own.rebuilt(1).await;
    let listed = own.own().await.expect("Your games is listed");
    assert_eq!(
        (listed.name.as_str(), listed.source, listed.games),
        ("Your games", DatabaseSource::Own, 4)
    );
    assert_eq!(own.games.games(by("alice")).await.unwrap().total, 2);
    let played = own.games.games(by("you")).await.unwrap();
    assert_eq!(played.total, 2);
    let pgn = own
        .games
        .game(own::OWN_ID, played.games[0].id)
        .await
        .unwrap();
    assert!(pgn.contains("[Event \"KChess computer game\"]"), "{pgn}");
    assert!(pgn.contains("[Date \"2025.10.22\"]"), "{pgn}");
    let found = own
        .games
        .lookup(START.into(), in_database(own::OWN_ID))
        .await
        .unwrap()
        .saved;
    assert_eq!(counted(&found), ["d4 1/0/0", "e4 1/0/1", "f3 0/0/1"]);
    // Past the opening tree, the position index finds the game too.
    let past = |own: &Own| {
        let (games, deep) = (Arc::clone(&own.games), deep.clone());
        async move {
            let found = games
                .lookup(deep, in_database(own::OWN_ID))
                .await
                .unwrap()
                .saved;
            counted(&found)
        }
    };
    assert_eq!(past(&own).await, ["a4 0/0/1"]);
    let changed = own.host.named("profile:changed");
    assert!(
        changed
            .iter()
            .any(|change| change["areas"] == serde_json::json!(["gameDb"])),
        "{changed:?}"
    );

    // A finished game leaves Game history, and "Your games" follows.
    own.remove_played("won").await;
    own.rebuilt(2).await;
    assert_eq!(own.own().await.unwrap().games, 3);
    assert_eq!(own.games.games(by("you")).await.unwrap().total, 1);
    // Another game through the same late position comes, and the first goes: the position index
    // is the new games' alone, as the tree is.
    let (other, _) = past_the_tree(&["b2b4", "b7b5"]);
    own.play_line("other", &other, "1/2-1/2").await;
    own.rebuilt(3).await;
    assert_eq!(past(&own).await, ["a4 0/0/1", "b4 0/1/0"]);
    own.remove_played("long").await;
    own.rebuilt(4).await;
    assert_eq!(own.own().await.unwrap().games, 3);
    assert_eq!(past(&own).await, ["b4 0/1/0"]);
    let file = schema::open_read(&own.games.inner.file(own::OWN_ID)).unwrap();
    assert!(deep::built(&file).unwrap());
    drop(file);

    let files = tempfile::tempdir().unwrap();
    let theirs = imported(&own.games, source(files.path(), "g.pgn", GAMES.as_bytes()))
        .await
        .database
        .id;
    let refused = own.games.rename(own::OWN_ID, "Mine".into()).await;
    assert_eq!(refused.unwrap_err().message, "Your games can't be renamed.");
    assert!(own.games.delete(own::OWN_ID).await.is_err());
    let into = own
        .games
        .import(
            source(files.path(), "more.pgn", GAMES.as_bytes()),
            PgnPacking::Plain,
            Some(own::OWN_ID.into()),
            None,
            DatabaseSource::Imported,
        )
        .await;
    assert_eq!(
        into.unwrap_err().message,
        "Your games holds only your own games."
    );
    assert_eq!(own.own().await.unwrap().games, 3);

    // Clearing the account's games leaves the played one; with it gone there are none: no
    // database, rather than an empty one.
    own.database
        .write_async(|ctx| crate::store::games::clear_account_data(ctx.db, "Alice"))
        .await
        .unwrap();
    own.rebuilt(5).await;
    assert_eq!(own.own().await.unwrap().games, 1);
    own.remove_played("other").await;
    own.rebuilt(6).await;
    assert!(own.own().await.is_none());
    assert!(!own.games.inner.file(own::OWN_ID).exists());
    let status = own.games.status().await.unwrap();
    let ids: Vec<&str> = status.databases.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, [theirs.as_str()]);
    own.games.close().await;
}

/// The games a position search found in "Your games" are found again once it is written
/// afresh: a new file counts its imports from the first again, as the one it replaced did, so the
/// search kept from before would otherwise stand for it.
#[tokio::test]
async fn a_position_search_follows_your_games_written_afresh() {
    let own = Own::new(tempfile::tempdir().unwrap()).await;
    own.keep(NEVER_QUIET);
    let at_start = || GameDbQuery {
        position: Some(START.to_string()),
        ..query(own::OWN_ID)
    };
    own.sync(vec![synced("g1", &["e4", "e5"], true)]).await;
    own.games.settle_own().await;
    assert_eq!(own.games.games(at_start()).await.unwrap().total, 1);
    own.database
        .write_async(|ctx| crate::store::games::clear_account_data(ctx.db, "Alice"))
        .await
        .unwrap();
    own.games.settle_own().await;
    assert!(own.own().await.is_none());
    own.sync(vec![
        synced("g2", &["d4", "d5"], true),
        synced("g3", &["c4", "e5"], true),
    ])
    .await;
    assert_eq!(own.games.settle_own().await.rebuilds, 3);
    assert_eq!(own.games.games(at_start()).await.unwrap().total, 2);
    own.games.close().await;
}

/// A sync's pages and a played game in a burst are one rebuild, once the writes stop; a sync that
/// writes the same games again writes nothing. The next start writes the database only when it
/// is missing or older than the games.
#[tokio::test]
async fn a_burst_of_writes_is_one_rebuild_and_a_start_catches_up() {
    let own = Own::new(tempfile::tempdir().unwrap()).await;
    own.keep(NEVER_QUIET);
    own.sync(vec![synced("g1", &["e4", "e5"], true)]).await;
    own.sync(vec![synced("g2", &["d4", "d5"], true)]).await;
    own.play("won", true).await;
    own.sync(vec![synced("g3", &["c4", "e5"], true)]).await;
    // Still writing: nothing is written yet.
    assert_eq!(*own.games.own_report().borrow(), own::Report::default());
    assert!(own.own().await.is_none());
    let report = own.games.settle_own().await;
    assert_eq!((report.passes, report.rebuilds), (1, 1));
    assert_eq!(own.own().await.unwrap().games, 4);
    own.sync(vec![synced("g3", &["c4", "e5"], true)]).await;
    let report = own.games.settle_own().await;
    assert_eq!((report.passes, report.rebuilds), (2, 1));

    // The next start finds the database as the games are.
    let own = own.reopened().await;
    own.keep(NEVER_QUIET);
    let report = own.games.settle_own().await;
    assert_eq!((report.passes, report.rebuilds), (1, 0));
    // A game synced while no keeper ran: the database is older than the games.
    let own = own.reopened().await;
    own.sync(vec![synced("g4", &["Nf3", "d5"], true)]).await;
    own.keep(NEVER_QUIET);
    assert_eq!(own.games.settle_own().await.rebuilds, 1);
    assert_eq!(own.own().await.unwrap().games, 5);
    // The database went missing.
    let own = own.reopened().await;
    import::remove_files(&own.games.inner.file(own::OWN_ID));
    own.keep(NEVER_QUIET);
    assert_eq!(own.games.settle_own().await.rebuilds, 1);
    assert_eq!(own.own().await.unwrap().games, 5);
    own.games.close().await;
}

/// A rebuild asked for while the player's import runs waits for the import to end.
#[tokio::test]
async fn a_rebuild_waits_for_the_import_running() {
    let own = Own::new(tempfile::tempdir().unwrap()).await;
    own.keep(NEVER_QUIET);
    let files = tempfile::tempdir().unwrap();
    let path = source(files.path(), "many.pgn", GAMES.repeat(2_000).as_bytes());
    let importing = {
        let games = Arc::clone(&own.games);
        tokio::spawn(async move {
            games
                .import(
                    path,
                    PgnPacking::Plain,
                    None,
                    None,
                    DatabaseSource::Imported,
                )
                .await
        })
    };
    while own.host.named("gamedb:progress").is_empty() {
        tokio::task::yield_now().await;
    }
    own.sync(vec![synced("g1", &["e4", "e5"], true)]).await;
    let settled = {
        let games = Arc::clone(&own.games);
        tokio::spawn(async move { games.settle_own().await })
    };
    own.games.cancel();
    assert!(importing.await.unwrap().is_err());
    assert_eq!(settled.await.unwrap().rebuilds, 1);
    // The import's end came first, then "Your games".
    let events = own.host.events();
    let ended = events
        .iter()
        .position(|(name, body)| name == "gamedb:progress" && body["phase"] == "cancelled")
        .expect("the import ended");
    let rebuilt = events
        .iter()
        .position(|(name, _)| name == "profile:changed")
        .expect("Your games was written");
    assert!(ended < rebuilt, "{events:?}");
    assert_eq!(own.own().await.unwrap().games, 1);
    own.games.close().await;
}

/// Closing the service while it rebuilds ends the rebuild and waits for it: nothing is left
/// half-written, and the next start writes the whole database.
#[tokio::test]
async fn closing_ends_a_rebuild_and_leaves_nothing_half_written() {
    let own = Own::new(tempfile::tempdir().unwrap()).await;
    let moves = ["e4", "e5", "Nf3", "Nc6", "Bb5", "a6"];
    let many: Vec<serde_json::Value> = (0..3_000)
        .map(|n| synced(&format!("g{n}"), &moves, true))
        .collect();
    own.sync(many).await;
    own.keep(NEVER_QUIET);
    let mut report = own.games.own_report();
    let settled = {
        let games = Arc::clone(&own.games);
        tokio::spawn(async move { games.settle_own().await })
    };
    report.wait_for(|report| report.building).await.unwrap();
    tokio::time::timeout(Duration::from_secs(60), own.games.close())
        .await
        .expect("closing waits for the rebuild to end");
    // The keeper has ended: a settle no longer waits for it.
    settled.await.unwrap();
    assert!(!own.games.own_report().borrow().building);
    assert_eq!(own.half_written(), Vec::<String>::new());
    let own = own.reopened().await;
    own.keep(NEVER_QUIET);
    own.games.settle_own().await;
    assert_eq!(own.own().await.unwrap().games, 3_000);
    assert_eq!(own.half_written(), Vec::<String>::new());
    own.games.close().await;
}

/// A position at any depth is found in every game that reached it, as replaying every game in
/// full counts it: the tree's plies and those past it (the position index) together, with
/// filters or without, the same position early in one game and late in another. A file whose
/// index is gone answers alike by replaying every game, and opening it to write builds the index
/// an import builds.
#[test]
fn a_position_past_the_tree_is_found_in_every_game_that_reached_it() {
    random::check(3, |rng| {
        let profile = tempfile::tempdir().unwrap();
        let depth = schema::INDEX_DEPTH as usize;
        // Long lines the games follow for a while, so that many share positions past the tree.
        let mut bases: Vec<Vec<shakmaty::Move>> = Vec::new();
        while bases.len() < 3 {
            let mut pos = rules::default_position(Variant::Chess);
            let mut line = Vec::new();
            for _ in 0..depth + 40 {
                let legal = pos.legal_moves();
                let Some(m) = rng.pick(&legal).copied() else {
                    break;
                };
                pos.play_unchecked(m);
                line.push(m);
            }
            if line.len() > depth + 10 {
                bases.push(line);
            }
        }
        let shuffle = ["Nf3", "Nf6", "Ng1", "Ng8"];
        let mut pgn = String::new();
        let mut games: Vec<(Vec<shakmaty::Move>, &str)> = Vec::new();
        for game in 0..300 {
            let mut pos = rules::default_position(Variant::Chess);
            let mut moves = Vec::new();
            // Knights out and back first, so a base's positions come on later plies in some
            // games than in others, on both sides of the tree's depth.
            for _ in 0..rng.below(4) {
                for san in shuffle {
                    let m = rules::parse_san(&pos, san).unwrap();
                    pos.play_unchecked(m);
                    moves.push(m);
                }
            }
            let base = &bases[rng.below(bases.len())];
            for &m in &base[..10 + rng.below(base.len() - 10)] {
                pos.play_unchecked(m);
                moves.push(m);
            }
            for _ in 0..rng.below(20) {
                let legal = pos.legal_moves();
                let Some(m) = rng.pick(&legal).copied() else {
                    break;
                };
                pos.play_unchecked(m);
                moves.push(m);
            }
            let result = ["1-0", "0-1", "1/2-1/2"][rng.below(3)];
            let mut replay = rules::default_position(Variant::Chess);
            let text: Vec<String> = moves
                .iter()
                .enumerate()
                .map(|(ply, m)| {
                    let san = rules::san_and_play(&mut replay, *m);
                    match ply % 2 {
                        0 => format!("{}. {san}", ply / 2 + 1),
                        _ => san,
                    }
                })
                .collect();
            let elo = 2000 + rng.below(800);
            pgn.push_str(&format!(
                "[White \"P{game}\"]\n[Black \"Q\"]\n[Result \"{result}\"]\n\
                 [WhiteElo \"{elo}\"]\n[BlackElo \"{elo}\"]\n\n{} {result}\n\n",
                text.join(" ")
            ));
            games.push((moves, result));
        }
        // Every move each game played from each position, by the position's key and the move.
        let mut every: HashMap<(u64, String), [i64; 3]> = HashMap::new();
        // The games (their White, `P<n>`) that played a move from each position.
        let mut reached: HashMap<u64, std::collections::BTreeSet<String>> = HashMap::new();
        for (game, (moves, result)) in games.iter().enumerate() {
            let mut pos = rules::default_position(Variant::Chess);
            for m in moves {
                reached
                    .entry(gamedb_key(&pos))
                    .or_default()
                    .insert(format!("P{game}"));
                let counts = every
                    .entry((gamedb_key(&pos), rules::make_uci(m)))
                    .or_default();
                counts[match *result {
                    "1-0" => 0,
                    "1/2-1/2" => 1,
                    _ => 2,
                }] += 1;
                pos.play_unchecked(*m);
            }
        }
        let id = write_database(profile.path(), "Deep", &pgn).unwrap();
        let path = profile.path().join("databases").join(format!("{id}.kdb"));
        let everyone = LookupOptions {
            player: Some("Q".into()),
            ..in_database(&id)
        };
        let rated = LookupOptions {
            min_elo: Some(1),
            ..in_database(&id)
        };
        let go = AtomicBool::new(false);
        // Positions on every ply of the bases, past the tree most of them.
        let mut fens = Vec::new();
        for _ in 0..25 {
            let base = &bases[rng.below(bases.len())];
            let mut pos = rules::default_position(Variant::Chess);
            for &m in &base[..rng.below(base.len())] {
                pos.play_unchecked(m);
            }
            fens.push((rules::make_fen(&pos), gamedb_key(&pos)));
        }
        let answers = || {
            let db = schema::open_read(&path).unwrap();
            fens.iter()
                .map(|(fen, key)| {
                    let all = explore::lookup(&db, &id, fen, &in_database(&id), 0, &go).unwrap();
                    let mut expected: Vec<(String, i64, i64, i64)> = every
                        .iter()
                        .filter(|((at, _), _)| at == key)
                        .map(|((_, uci), c)| (uci.clone(), c[0], c[1], c[2]))
                        .collect();
                    expected.sort();
                    let mut got: Vec<(String, i64, i64, i64)> = all
                        .moves
                        .iter()
                        .map(|m| {
                            (
                                m.uci.clone(),
                                m.white.unwrap_or(0),
                                m.draws.unwrap_or(0),
                                m.black.unwrap_or(0),
                            )
                        })
                        .collect();
                    got.sort();
                    assert_eq!(got, expected, "{fen}");
                    for filter in [&everyone, &rated] {
                        let filtered = explore::lookup(&db, &id, fen, filter, 0, &go).unwrap();
                        assert_eq!(filtered.moves, all.moves, "{fen}");
                        assert_eq!(filtered.total, all.total, "{fen}");
                        assert_eq!(filtered.games, all.games, "{fen}");
                    }
                    // The games listed with the position are those that played from it, each
                    // once, in every order, a page at a time, with the other filters applied.
                    let expected = reached.get(key).cloned().unwrap_or_default();
                    let wins: std::collections::BTreeSet<String> = expected
                        .iter()
                        .filter(|white| {
                            let game: usize = white[1..].parse().unwrap();
                            games[game].1 == "1-0"
                        })
                        .cloned()
                        .collect();
                    let listed = explore::Listed::default();
                    for (filter, wanted) in [
                        (GameDbQuery::default(), &expected),
                        (
                            GameDbQuery {
                                opponent: Some("q".into()),
                                min_elo: Some(1),
                                ..GameDbQuery::default()
                            },
                            &expected,
                        ),
                        (
                            GameDbQuery {
                                result: Some(GameDbResult::White),
                                ..GameDbQuery::default()
                            },
                            &wins,
                        ),
                    ] {
                        for sort in [GameDbSort::Newest, GameDbSort::Oldest, GameDbSort::Rating] {
                            let mut pages = Vec::new();
                            loop {
                                let page = explore::games_at(
                                    &db,
                                    &GameDbQuery {
                                        database: id.clone(),
                                        position: Some(fen.clone()),
                                        sort: Some(sort),
                                        offset: pages.len() as u32,
                                        limit: 7,
                                        ..filter.clone()
                                    },
                                    &listed,
                                    &go,
                                )
                                .unwrap();
                                assert_eq!(page.total as usize, wanted.len(), "{fen}");
                                if page.games.is_empty() {
                                    break;
                                }
                                pages.extend(page.games);
                            }
                            let names: std::collections::BTreeSet<String> =
                                pages.iter().map(|row| row.white.clone()).collect();
                            assert_eq!(names.len(), pages.len(), "each game once: {fen}");
                            assert_eq!(&names, wanted, "{fen}");
                            let ids: Vec<u64> = pages.iter().map(|row| row.id).collect();
                            let mut by_id = ids.clone();
                            by_id.sort_unstable();
                            match sort {
                                // Undated: by id.
                                GameDbSort::Oldest => assert_eq!(ids, by_id),
                                GameDbSort::Newest => {
                                    by_id.reverse();
                                    assert_eq!(ids, by_id);
                                }
                                GameDbSort::Rating => assert!(
                                    pages
                                        .windows(2)
                                        .all(|pair| pair[0].white_elo >= pair[1].white_elo),
                                    "best rated first: {fen}"
                                ),
                            }
                        }
                    }
                    all
                })
                .collect::<Vec<_>>()
        };
        let indexed = answers();
        assert!(
            indexed.iter().any(|found| !found.moves.is_empty()),
            "some position was found"
        );
        let buckets = || -> Vec<(i64, Vec<u8>)> {
            let db = schema::open_read(&path).unwrap();
            let mut select = db
                .prepare("SELECT bucket, entries FROM deep ORDER BY bucket")
                .unwrap();
            select
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .map(|row| row.unwrap())
                .collect()
        };
        let imported = buckets();
        assert!(!imported.is_empty());
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("DELETE FROM deep; UPDATE meta SET deep_built = 0;")
                .unwrap();
        }
        assert_eq!(answers(), indexed, "replaying every game answers alike");
        schema::upgrade(&path).unwrap();
        assert_eq!(buckets(), imported, "built as the import built it");
        assert_eq!(answers(), indexed);
    });
}

fn gamedb_key(pos: &shakmaty::variant::VariantPosition) -> u64 {
    kchess_domain::gamedb::position_key(pos)
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

/// What `HeaderFilter` passes on of `text`, keeping the games whose headers name a white player
/// in `kept`, reading `chunk` bytes at a time.
fn filtered(text: &str, kept: &'static [&'static str], chunk: usize) -> String {
    use std::io::Read as _;
    let keep: import::Keep = Box::new(move |headers| {
        let headers = String::from_utf8_lossy(headers);
        kept.iter()
            .any(|white| headers.contains(&format!("[White \"{white}\"]")))
    });
    let reader = std::io::BufReader::new(std::io::Cursor::new(text.as_bytes().to_vec()));
    let mut filter = import::HeaderFilter::new(reader, keep);
    let (mut out, mut buf) = (Vec::new(), vec![0; chunk]);
    loop {
        let n = filter.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(out).unwrap()
}

#[test]
fn a_header_filter_passes_on_only_the_games_it_keeps_whole() {
    let a = "[White \"A\"]\n[Black \"B\"]\n\n1. e4 { [%clk 0:03:00] } 1... e5\n2. Nf3 1-0\n\n";
    let b = "[White \"B\"]\n[Black \"A\"]\n\n1. d4 d5 0-1\n\n";
    let c = "[White \"C\"]\n[Black \"A\"]\n\n1. c4 1/2-1/2\n";
    // A last game with no moves, and no line ending.
    let d = "[White \"D\"]\n[Black \"A\"]";
    let text = format!("A comment before any game.\n\n{a}{b}{c}\n{d}");
    for chunk in [1, 7, 4096] {
        assert_eq!(filtered(&text, &["A", "C"], chunk), format!("{a}{c}\n"));
        assert_eq!(filtered(&text, &["B", "D"], chunk), format!("{b}{d}"));
        assert_eq!(filtered(&text, &[], chunk), "");
    }
    let games = parse_pgn(&filtered(&text, &["A", "C"], 64));
    assert_eq!(games.len(), 2);
}

#[test]
fn a_game_with_huge_headers_is_left_out() {
    let huge = format!(
        "[White \"A\"]\n[Annotator \"{}\"]\n\n1. e4 1-0\n\n",
        "x".repeat(70 * 1024)
    );
    let small = "[White \"A\"]\n\n1. d4 0-1\n\n";
    assert_eq!(filtered(&format!("{huge}{small}"), &["A"], 4096), small);
}

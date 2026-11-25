//! The Database page: importing a PGN file, searching and paging its games, opening one on the
//! analysis board, the games with a position, and deleting the database.

use super::Harness;

const GAMES: &str = r#"[Event "Tata Steel"]
[Date "2024.01.20"]
[White "Carlsen, Magnus"]
[Black "Gukesh D"]
[Result "1-0"]
[WhiteElo "2830"]
[BlackElo "2725"]

1. e4 e5 2. Nf3 Nc6 3. Bb5 Nf6 1-0

[Event "Tata Steel"]
[Date "2024.01.21"]
[White "Gukesh D"]
[Black "Carlsen, Magnus"]
[Result "1/2-1/2"]
[WhiteElo "2725"]
[BlackElo "2830"]

1. Nf3 Nc6 2. e4 e5 3. d4 exd4 1/2-1/2

[Event "Club"]
[Date "2023.05.02"]
[White "Smith"]
[Black "Jones"]
[Result "0-1"]

1. d4 d5 2. c4 0-1
"#;

/// The games listed, by their White and Black.
fn listed(app: &mut Harness) -> Vec<String> {
    app.with_window(|window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .elements()
            .iter()
            .filter_map(|element| element.label())
            .filter(|label| label.contains(" against ") && !label.starts_with("Actions for "))
            .map(str::to_string)
            .collect()
    })
}

fn wait_for_listed(app: &mut Harness, what: &str, expected: &[&str]) {
    let expected: Vec<String> = expected.iter().map(|game| game.to_string()).collect();
    app.wait_for(what, |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        let shown: Vec<String> = window
            .elements()
            .iter()
            .filter_map(|element| element.label())
            .filter(|label| label.contains(" against ") && !label.starts_with("Actions for "))
            .map(str::to_string)
            .collect();
        shown == expected
    });
}

const CARLSEN_GUKESH: &str = "Carlsen, Magnus against Gukesh D, 1-0";
const GUKESH_CARLSEN: &str = "Gukesh D against Carlsen, Magnus, 1/2-1/2";
const SMITH_JONES: &str = "Smith against Jones, 0-1";

/// `GAMES` imported on the Database page, listed newest first.
fn import_games(app: &mut Harness) {
    app.open("Game database");
    app.find("gamedb-import");
    assert!(!app.shows("database-delete"), "nothing to delete yet");
    let files = tempfile::tempdir().expect("a folder for the file");
    let path = files.path().join("tata.pgn");
    std::fs::write(&path, GAMES).expect("the file");
    app.click("gamedb-import");
    app.wait_for_open_dialog();
    app.cx
        .simulate_path_prompt_response(move |_| Some(vec![path]));
    wait_for_listed(
        app,
        "every game, newest first",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );
}

/// A PGN file imported on the page lists its games, newest first; the filters narrow them; a
/// game opens on the analysis board, whose "Games with this position" comes back to the games
/// that reached it (here by two move orders); and the database is deleted after asking.
#[gpui_kit::test]
fn a_database_is_imported_searched_opened_and_deleted(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_games(&mut app);

    // One player's games, then only those they won, then every game again.
    app.click_label("Player");
    app.type_text("carlsen");
    wait_for_listed(
        &mut app,
        "Carlsen's games",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH],
    );
    app.click_label("White won");
    wait_for_listed(&mut app, "Carlsen's win", &[CARLSEN_GUKESH]);
    app.click("database-clear");
    wait_for_listed(
        &mut app,
        "every game again",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );
    assert!(!app.shows("database-clear"));

    // A game opens on the board; the position after 2...Nc6 lists both games that reached it.
    app.click_label(CARLSEN_GUKESH);
    app.wait_for_page(crate::pages::Page::Analysis);
    app.click("tree-e2e4 e7e5 g1f3 b8c6");
    app.click("analysis-more");
    app.click_label("Games with this position");
    app.wait_for_page(crate::pages::Page::Database);
    wait_for_listed(
        &mut app,
        "the games with the position",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH],
    );
    app.click("database-position-clear");
    wait_for_listed(
        &mut app,
        "every game without the position",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );

    // Deleting asks first; then the page offers an import again.
    app.click("database-delete");
    app.click_label("Delete");
    app.wait_gone("database-delete");
    app.find("gamedb-import");
    assert!(listed(&mut app).is_empty());
}

/// Every game database is listed to manage, with where it came from, its games and size, and
/// their total: from the Database page and from Settings → Data, which shows the total too.
/// Deleting one there asks first, then leaves the list and the page without it.
#[gpui_kit::test]
fn the_databases_are_listed_with_their_size_to_manage(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    // Offered before there is any database, so it is found before the first download.
    app.open("Game database");
    app.find("database-manage");
    import_games(&mut app);
    let bytes = app.cx.update(|cx| {
        cx.global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx)
            .databases()[0]
            .bytes
    });
    assert!(bytes > 0, "a database takes room");
    let size = crate::widgets::format_bytes(bytes as f64);

    app.click("database-manage");
    let row = app
        .find(("manage-database", 0usize))
        .label()
        .unwrap_or_default()
        .to_string();
    assert!(
        row.contains(&format!("Imported · 3 games · {size}")),
        "{row}"
    );
    // An imported file has nothing to fetch again.
    assert!(!app.shows(("manage-update", 0usize)));
    assert_eq!(
        app.find("manage-total").label(),
        Some(format!("Total {size}").as_str())
    );
    app.press("escape");
    app.wait_gone("manage-total");

    // Settings → Storage lists them with every other download, and counts them in the total.
    app.open_settings("Storage");
    assert_eq!(
        app.find("storage-databases").label(),
        Some(format!("Game databases, 1 database · {size}").as_str())
    );
    let total = app
        .find("storage-total")
        .label()
        .unwrap_or_default()
        .to_string();
    assert!(total.starts_with("KChess uses "), "{total}");
    app.click("storage-databases-manage");
    app.wait_for_page(crate::pages::Page::Database);
    app.click(("manage-delete", 0usize));
    app.click_label("Delete");
    app.wait_gone(("manage-database", 0usize));
    app.press("escape");
    app.find("gamedb-import");
    assert!(!app.shows("database-delete"));
}

/// 2026-11-25: opened as the app started, the Database page asked for its games before Lichess
/// had answered whether a game was in progress, was refused ("until your Lichess game status is
/// verified") and showed that until a filter changed. Refused while help is held back, the games
/// are listed once it is lent, with nothing touched.
#[gpui_kit::test]
fn games_refused_while_help_was_held_back_are_listed_once_it_is_lent(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::online::Pause;
    let mut app = Harness::start(cx);
    import_games(&mut app);
    let lichess = app.connect_with("Tester", false, |session| session.fail_check = true);
    app.wait_for("the check for a game in progress to fail", |_, cx| {
        crate::online::handle(cx).read(cx).pause() == Some(Pause::Unverified)
    });
    app.open("Game database");
    app.click_label("Player");
    app.type_text("carlsen");
    wait_for_listed(&mut app, "the search to be refused", &[]);

    // Lichess answers when the app checks again on its own.
    lichess.lock().fail_check = false;
    app.cx
        .executor()
        .advance_clock(std::time::Duration::from_secs(16));
    wait_for_listed(
        &mut app,
        "Carlsen's games once help is lent",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH],
    );
}

/// A PGN file dropped on the Database page is imported, however many games it has, where on
/// another page it would open on the analysis board.
#[gpui_kit::test]
fn a_pgn_file_dropped_on_the_page_is_imported(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Game database");
    app.find("gamedb-import");
    let _folder = app.drop_file("club.pgn", GAMES.as_bytes());
    wait_for_listed(
        &mut app,
        "the dropped file's games",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );
    app.find("database-delete");
}

/// A `.pgn.bz2`, as FICS packs its games, dropped on the Database page is imported, named
/// after the file.
#[gpui_kit::test]
fn a_bzip2_pgn_file_dropped_on_the_page_is_imported(cx: &mut gpui_kit::TestAppContext) {
    use std::io::Write as _;
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(GAMES.as_bytes()).expect("packs");
    let packed = encoder.finish().expect("packed");
    let mut app = Harness::start(cx);
    app.open("Game database");
    app.find("gamedb-import");
    let _folder = app.drop_file(
        "ficsgamesdb_2024_standard2000_nomovetimes_1.pgn.bz2",
        &packed,
    );
    wait_for_listed(
        &mut app,
        "the dropped file's games",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );
    app.wait_for_app("its database, named after the file", |cx| {
        let db = cx
            .global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx);
        db.databases()
            .iter()
            .any(|info| info.name == "ficsgamesdb_2024_standard2000_nomovetimes_1")
    });
}

/// The downloads and the import stay where they were drawn when a database is listed after the
/// page first drew ("Your games" is built as games sync): in a wide window, the database's
/// choice, count and Delete joined their row, which then took two lines and pushed the master
/// and engine games and the import down to the second.
#[gpui_kit::test]
fn the_downloads_hold_still_when_a_database_is_listed(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{px, size};
    const ACTIONS: [&str; 5] = [
        "database-player",
        "database-lichess",
        "database-masters",
        "database-tcec",
        "gamedb-import",
    ];
    fn places(app: &mut Harness) -> Vec<gpui_kit::Point<gpui_kit::Pixels>> {
        ACTIONS
            .iter()
            .map(|&id| app.find(id).bounds().origin)
            .collect()
    }
    let mut app = Harness::start(cx);
    app.cx
        .simulate_window_resize(app.window, size(px(1320.), px(880.)));
    app.open("Game database");
    let before = places(&mut app);
    let _folder = app.drop_file("club.pgn", GAMES.as_bytes());
    wait_for_listed(
        &mut app,
        "the dropped file's games",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );
    app.find("database-delete");
    assert_eq!(places(&mut app), before);
}

/// A `.zip` or a `.7z` of PGN files dropped on the Database page is imported as one database,
/// named after the archive, with the games of every PGN file in it.
#[gpui_kit::test]
fn an_archive_of_pgn_files_dropped_on_the_page_is_imported(cx: &mut gpui_kit::TestAppContext) {
    use std::io::Write as _;
    let (tata, club) = GAMES.split_at(GAMES.find("[Event \"Club\"]").expect("the club game"));
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    // The first file ends without a line ending: its last game stays apart from the next file's.
    for (name, text) in [("twic/tata.pgn", tata.trim_end()), ("twic/club.pgn", club)] {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("starts");
        writer.write_all(text.as_bytes()).expect("writes");
    }
    let zipped = writer.finish().expect("finishes").into_inner();
    let mut app = Harness::start(cx);
    app.open("Game database");
    app.find("gamedb-import");
    let _folder = app.drop_file("twic1620g.zip", &zipped);
    wait_for_listed(
        &mut app,
        "the games of both files",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );
    let names = |app: &mut Harness| -> Vec<String> {
        app.with_window(|_, cx| {
            let mut names: Vec<String> = cx
                .global::<kchess_board::game_db::GameDbHandle>()
                .0
                .read(cx)
                .databases()
                .iter()
                .map(|info| info.name.clone())
                .collect();
            names.sort();
            names
        })
    };
    assert_eq!(names(&mut app), ["twic1620g"]);

    // The core's own `.7z` fixture (`crates/kchess-core/src/gamedb/fixtures/README.md`).
    let packed = include_bytes!("../../../../crates/kchess-core/src/gamedb/fixtures/many.7z");
    let _folder = app.drop_file("Lumbra OTB Elite.7z", packed);
    app.wait_for_app("the 7z's database", |cx| {
        cx.global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .len()
            == 2
    });
    assert_eq!(names(&mut app), ["Lumbra OTB Elite", "twic1620g"]);
}

/// A player's games, downloaded from Lichess to prepare against them, make a database of their
/// own. It opens on their games, with their name as the player filter, and offers to update
/// them; an unknown player is refused by name, leaving no database.
#[gpui_kit::test]
fn a_players_games_are_downloaded_to_prepare_against_them(cx: &mut gpui_kit::TestAppContext) {
    use super::fake_lichess::SCOUTED;
    let mut app = Harness::start(cx);
    app.open("Game database");
    app.click("database-player");
    app.fill_in_dialog("Username", "Nobody_Here");
    app.click("player-download");
    app.wait_for_app("the refusal", |cx| {
        cx.global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx)
            .error()
            .is_some_and(|error| error.contains("No Lichess player is called Nobody_Here"))
    });

    app.click("database-player");
    app.fill_in_dialog("Username", SCOUTED);
    app.click("player-download");
    wait_for_listed(
        &mut app,
        "their games, newest first",
        &[
            "Rival against Scouted_Player, 0-1",
            "Scouted_Player against Rival, 1-0",
        ],
    );
    app.wait_for_app("their database, made from the request", |cx| {
        let db = cx
            .global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx);
        db.databases().len() == 1
            && db.databases()[0].source == kchess_contracts::types::DatabaseSource::Player
            && db.databases()[0].name == "Scouted_Player · Lichess"
            && db.databases()[0]
                .player
                .as_ref()
                .is_some_and(|player| player.rated && player.months == Some(12))
    });
    app.wait_for("their name as the filter", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .try_find_by_label("Player")
            .is_some_and(|input| input.value() == Some(SCOUTED))
    });
    app.find("database-update-player");
}

/// The player download offers names: the Lichess players whose names start with what is typed,
/// which Chess.com has no search for, and once a player's games are in, that player as a recent
/// one on their site. Picking a name puts it in the field.
#[gpui_kit::test]
fn the_player_download_offers_names_to_pick(cx: &mut gpui_kit::TestAppContext) {
    use super::fake_lichess::SCOUTED;
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    let mut app = Harness::start(cx);
    app.online();
    app.open("Game database");
    app.click("database-player");
    app.fill_in_dialog("Username", "mag");
    assert_eq!(
        app.find(("player-match", 0usize)).label(),
        Some("GM MagnusCarlsen, online")
    );
    // Chess.com has no name search: its matches go with Lichess.
    app.click(("player-site", 1usize));
    app.wait_for("the Lichess matches to go", |window, _| {
        window.try_find(("player-match", 0usize)).is_none()
    });
    app.click(("player-site", 0usize));
    app.click(("player-match", 1usize));
    app.wait_for("the picked name in the field", |window, _| {
        window
            .try_find_by_label("Username")
            .is_some_and(|input| input.value() == Some("MagicMike"))
            && window.try_find(("player-match", 0usize)).is_none()
    });

    // Enter downloads, as the button does.
    app.fill_in_dialog("Username", SCOUTED);
    app.press("enter");
    app.wait_for_app("their database", |cx| {
        cx.global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .len()
            == 1
    });
    // Their name is offered next time on Lichess, not on Chess.com.
    app.click("database-player");
    assert_eq!(app.find(("player-recent", 0usize)).label(), Some(SCOUTED));
    app.click(("player-site", 1usize));
    app.wait_for("no Lichess name on Chess.com", |window, _| {
        window.try_find(("player-recent", 0usize)).is_none()
    });
}

/// The master games button downloads Lichess's broadcast archive (here the stand-in's one month)
/// into a database of their own, whose games are listed with the archive's attribution; the
/// button then offers to bring in newer months.
#[gpui_kit::test]
fn the_master_games_are_downloaded_and_listed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Game database");
    assert_eq!(
        app.find("database-masters").label(),
        Some("Master games · 770 MB")
    );
    app.click("database-masters");
    wait_for_listed(
        &mut app,
        "the master games, newest first",
        &[
            "Caruana, Fabiano against Firouzja, Alireza, 1/2-1/2",
            "Firouzja, Alireza against Caruana, Fabiano, 1-0",
        ],
    );
    app.wait_for_app("the archive's attribution", |cx| {
        let db = cx
            .global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx);
        db.databases().iter().any(|info| {
            info.source == kchess_contracts::types::DatabaseSource::Master
                && info
                    .attribution
                    .as_deref()
                    .is_some_and(|text| text.contains("CC BY-SA"))
        })
    });
    app.wait_for("the button to update them", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .elements()
            .iter()
            .any(|element| element.label() == Some("Update master games"))
    });
}

/// With the disk almost full, a download asks first, saying how much is free: Cancel leaves
/// everything as it was, and Download anyway downloads.
#[gpui_kit::test]
fn a_download_asks_first_when_the_disk_is_almost_full(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestQueryExt as _;
    let mut app = Harness::start(cx);
    app.open("Game database");
    // The disk then reads as having 1 GB free.
    app.cx
        .update(|cx| cx.set_global(kchess_board::room::PinnedFreeSpace(1024 * 1024 * 1024)));
    app.click("database-tcec");
    app.wait_for("the warning", |window, _| {
        window.try_find_by_label("Download anyway").is_some()
    });
    app.click_label("Cancel");
    app.wait_for("the warning to go", |window, _| {
        window.try_find_by_label("Download anyway").is_none()
    });
    assert!(
        app.cx.update(|cx| !cx
            .global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx)
            .busy()),
        "nothing downloads"
    );

    app.click("database-tcec");
    app.click_label("Download anyway");
    app.wait_for_app("the engine games in", |cx| {
        cx.global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .iter()
            .any(|info| info.source == kchess_contracts::types::DatabaseSource::Tcec)
    });
}

/// The engine games button downloads TCEC's latest release into a database of its own: its
/// standard competition games alone, listed with the release's attribution; the button then
/// offers to update them.
#[gpui_kit::test]
fn the_engine_games_are_downloaded_and_listed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Game database");
    assert_eq!(
        app.find("database-tcec").label(),
        Some("Engine games · 27 MB")
    );
    app.click("database-tcec");
    wait_for_listed(
        &mut app,
        "the competition game alone",
        &["Stockfish dev against LCZero 0.32, 1-0"],
    );
    app.wait_for_app("the release's attribution", |cx| {
        let db = cx
            .global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx);
        db.databases().iter().any(|info| {
            info.source == kchess_contracts::types::DatabaseSource::Tcec
                && info.games == 1
                && info
                    .attribution
                    .as_deref()
                    .is_some_and(|text| text.contains("CC BY-SA 3.0"))
        })
    });
    app.wait_for("the button to update them", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .elements()
            .iter()
            .any(|element| element.label() == Some("Update engine games"))
    });
}

/// Strong Lichess games: the dialog offers Lichess's newest month and says how large it is; the
/// download keeps only the month's game of two 2400s, in a database of its own, which then offers
/// to add a month, the one month it lacks.
#[gpui_kit::test]
fn a_month_of_strong_lichess_games_is_downloaded(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Game database");
    app.click("database-lichess");
    app.wait_for("the newest month and its size", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("lichess-download")
            .and_then(|button| button.label().map(str::to_string))
            .is_some_and(|label| label.starts_with("Download · ") && label.ends_with(" B"))
    });
    app.click("lichess-download");
    wait_for_listed(
        &mut app,
        "the month's strong game alone",
        &["Strong 2026-10 against Rival, 1-0"],
    );
    app.wait_for_app("its database, named for its floor", |cx| {
        let db = cx
            .global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx);
        db.databases().iter().any(|info| {
            info.source == kchess_contracts::types::DatabaseSource::Lichess
                && info.name == "Lichess 2400+"
                && info.lichess_rating == Some(2400)
                && info.games == 1
        })
    });
    app.click("database-update-lichess");
    app.wait_for("the month it lacks", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .try_find_by_label("Month")
            .is_some_and(|month| month.value() == Some("September 2026"))
    });
    app.click("lichess-download");
    wait_for_listed(
        &mut app,
        "both months' strong games, newest first",
        &[
            "Strong 2026-10 against Rival, 1-0",
            "Strong 2026-09 against Rival, 1-0",
        ],
    );
    app.click("database-update-lichess");
    app.wait_for("every month in: none to choose", |window, _| {
        use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
        window.try_find_by_label("Month").is_none()
            && window
                .try_find("lichess-download")
                .is_some_and(|button| button.disabled() == Some(true))
    });
}

/// A new search never draws the last search's games: kept until its answer came, they and the
/// pager under them jumped to the new search's size a frame later (seen on CI's slower machines,
/// where the answer took longer than a frame).
#[gpui_kit::test]
fn a_new_search_does_not_draw_the_last_ones_games(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Game database");
    let _folder = app.drop_file("club.pgn", GAMES.as_bytes());
    wait_for_listed(
        &mut app,
        "every game",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );
    use gpui_kit::AppContext as _;
    let first: Vec<String> = app
        .cx
        .update_window(app.window, |_, window, cx| {
            use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
            let pill = window
                .elements()
                .into_iter()
                .find(|element| element.label() == Some("White won"))
                .and_then(|element| element.path().last().cloned())
                .expect("the White won filter");
            window.click(pill, cx);
            super::draw(window, cx);
            window
                .elements()
                .iter()
                .filter_map(|element| element.label())
                .filter(|label| label.contains(" against ") && !label.starts_with("Actions for "))
                .map(str::to_string)
                .collect()
        })
        .expect("the main window is open");
    assert!(
        first.is_empty() || first == [CARLSEN_GUKESH],
        "the first frame drew {first:?}"
    );
    wait_for_listed(&mut app, "White's win", &[CARLSEN_GUKESH]);
}

/// The databases listed now, by name and games.
fn databases(app: &mut Harness) -> Vec<(String, u64)> {
    app.cx.update(|cx| {
        cx.global::<kchess_board::game_db::GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .iter()
            .map(|info| (info.name.clone(), info.games))
            .collect()
    })
}

fn wait_for_databases(app: &mut Harness, what: &str, expected: &[(&str, u64)]) {
    let expected: Vec<(String, u64)> = expected
        .iter()
        .map(|(name, games)| (name.to_string(), *games))
        .collect();
    let started = std::time::Instant::now();
    while databases(app) != expected {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "{what}: {:?}",
            databases(app)
        );
        app.settle();
    }
}

/// A game's menu copies it into a new database or deletes it, and leaves the game unopened;
/// the search's menu saves every game it finds as one PGN file, then deletes them all.
#[gpui_kit::test]
fn games_are_copied_saved_and_deleted_from_the_page(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_games(&mut app);

    // Smith against Jones, the oldest, into a new database named in the dialog.
    app.click(("database-game-actions", 2usize));
    app.click_label("Copy to database…");
    app.fill_in_dialog("Name", "Club games");
    app.click("target-confirm");
    wait_for_databases(
        &mut app,
        "the copy listed",
        &[("Club games", 1), ("tata", 3)],
    );
    assert_eq!(
        app.cx.update(|cx| crate::shell::current_page(cx)),
        Some(crate::pages::Page::Database),
        "the menu opened no game"
    );
    wait_for_listed(
        &mut app,
        "the games shown still tata's",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );

    // Every game Carlsen played, saved as one file.
    app.click_label("Player");
    app.type_text("carlsen");
    wait_for_listed(
        &mut app,
        "Carlsen's games",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH],
    );
    assert_eq!(app.find("database-found").label(), Some("All 2 games"));
    app.click("database-found");
    app.click_label("Save as PGN…");
    app.wait_for_save_dialog();
    let folder = tempfile::tempdir().expect("a folder to save in");
    let path = folder.path().join("carlsen.pgn");
    let chosen = path.clone();
    app.cx.simulate_new_path_selection(move |_| Some(chosen));
    let started = std::time::Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "no file saved"
        );
        app.settle();
    }
    let saved = std::fs::read_to_string(&path).expect("the file");
    assert_eq!(
        saved.matches("[Event \"Tata Steel\"]").count(),
        2,
        "{saved}"
    );

    // Then deleted, after asking: only Smith against Jones is left.
    app.click("database-found");
    app.click_label("Delete these games…");
    app.click_label("Delete");
    wait_for_databases(
        &mut app,
        "Carlsen's games deleted",
        &[("Club games", 1), ("tata", 1)],
    );
    app.click("database-clear");
    wait_for_listed(&mut app, "the game left", &[SMITH_JONES]);

    // And the last one, from its own menu.
    app.click(("database-game-actions", 0usize));
    app.click_label("Delete game");
    app.click_label("Delete");
    wait_for_databases(
        &mut app,
        "every game deleted",
        &[("Club games", 1), ("tata", 0)],
    );
    wait_for_listed(&mut app, "no game", &[]);
}

/// A game opened from a database and changed on the board is saved back in its place, or as a
/// new game in another database.
#[gpui_kit::test]
fn the_board_saves_its_game_to_a_database(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_games(&mut app);
    app.click_label(SMITH_JONES);
    app.wait_for_page(crate::pages::Page::Analysis);
    // A move of its own at the start: a variation 1. e4 beside 1. d4.
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("e4");
    app.press("enter");
    app.find("tree-e2e4");

    // The new game is added first and the old one removed after: three games again, once the
    // database has changed. Three alone was true before the save, so the games were once read
    // between the two, four of them (2026-11-25).
    let changed_at = |app: &mut Harness| {
        app.cx.update(|cx| {
            cx.global::<kchess_board::game_db::GameDbHandle>()
                .0
                .read(cx)
                .databases()
                .iter()
                .map(|info| (info.games, info.updated_at))
                .collect::<Vec<_>>()
        })
    };
    let before = changed_at(&mut app);
    app.click("study-actions");
    app.click_label("Save changes to tata");
    let started = std::time::Instant::now();
    loop {
        let now = changed_at(&mut app);
        if now.len() == 1 && now[0].0 == 3 && now != before {
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "the game replaced: {now:?}"
        );
        app.settle();
    }
    let pgn = |app: &mut Harness, database: &str| -> Vec<String> {
        let id = app.cx.update(|cx| {
            cx.global::<kchess_board::game_db::GameDbHandle>()
                .0
                .read(cx)
                .databases()
                .iter()
                .find(|info| info.name == database)
                .map(|info| info.id.clone())
                .expect("the database")
        });
        let page = app.request(kchess_contracts::calls::GameDbGames {
            query: kchess_contracts::types::GameDbQuery {
                database: id.clone(),
                player: Some("smith".into()),
                limit: 20,
                ..Default::default()
            },
        });
        page.games
            .iter()
            .map(|game| {
                app.request(kchess_contracts::calls::GameDbGame {
                    database: kchess_contracts::check::DatabaseId(id.clone()),
                    game: kchess_contracts::check::GameDbGameId(game.id),
                })
            })
            .collect()
    };
    let changed = pgn(&mut app, "tata");
    assert_eq!(changed.len(), 1, "replaced, not added: {changed:?}");
    assert!(
        changed[0].contains("1. d4 ( 1. e4 ) 1... d5"),
        "{}",
        changed[0]
    );

    // Saved again, into a new database; Save changes then means that one.
    app.click("study-actions");
    app.click_label("Save to database…");
    // The game's own database is offered first; the one above it is a new database.
    app.click_label("Database");
    app.press("up");
    app.press("enter");
    app.fill_in_dialog("Name", "Prepared");
    app.click("target-confirm");
    wait_for_databases(&mut app, "a new database", &[("Prepared", 1), ("tata", 3)]);
    app.click("study-actions");
    app.wait_for("Save changes to the new one", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .try_find_by_label("Save changes to Prepared")
            .is_some()
    });
}

/// A game opened from a database marks its novelty: Eve's 3...Nf6 left the other games, one of
/// which reached the position and played 3...a6; another reached 2...Nc6 by other moves. The game
/// itself, which the database holds, does not count against it.
#[gpui_kit::test]
fn the_board_marks_where_its_game_left_the_databases(cx: &mut gpui_kit::TestAppContext) {
    const RUY: &str = "[White \"Ana\"]\n[Black \"Ben\"]\n[Result \"1-0\"]\n\n\
        1. e4 e5 2. Nf3 Nc6 3. Bb5 a6 1-0\n\n\
        [White \"Carl\"]\n[Black \"Dora\"]\n[Result \"0-1\"]\n\n\
        1. Nf3 Nc6 2. e4 e5 3. d4 exd4 0-1\n\n\
        [White \"Eve\"]\n[Black \"Finn\"]\n[Result \"1/2-1/2\"]\n\n\
        1. e4 e5 2. Nf3 Nc6 3. Bb5 Nf6 1/2-1/2\n";
    let mut app = Harness::start(cx);
    app.open("Game database");
    let files = tempfile::tempdir().expect("a folder for the file");
    let path = files.path().join("ruy.pgn");
    std::fs::write(&path, RUY).expect("the file");
    app.click("gamedb-import");
    app.wait_for_open_dialog();
    app.cx
        .simulate_path_prompt_response(move |_| Some(vec![path]));
    app.click_label("Eve against Finn, 1/2-1/2");
    app.wait_for_page(crate::pages::Page::Analysis);
    let novelty = "e2e4 e7e5 g1f3 b8c6 f1b5 g8f6";
    let mark = app.find(format!("novelty-{novelty}"));
    assert_eq!(
        mark.label(),
        Some("Novelty: new to your databases. 1 game reached this position and played a6 (1)")
    );
    assert_eq!(
        app.find(format!("tree-{novelty}")).label(),
        Some("Nf6, novelty")
    );
    // Nothing before it is new: 1. e4 and 3. Bb5 were played, 2...Nc6 by transposition too.
    for known in ["e2e4", "e2e4 e7e5 g1f3 b8c6", "e2e4 e7e5 g1f3 b8c6 f1b5"] {
        assert!(!app.shows(format!("novelty-{known}")), "{known}");
    }
    // The board shows the N over f6 while it shows the move, and not on the move before.
    let badge = |app: &mut Harness| {
        app.with_window(|_, cx| {
            crate::shell::view::<crate::pages::analysis::AnalysisPage>(cx)
                .and_then(|page| page.read(cx).board_badge(cx))
                .map(|badge| (badge.square, badge.glyph))
        })
    };
    app.click(format!("tree-{novelty}"));
    assert_eq!(badge(&mut app), Some((shakmaty::Square::F6, "N")));
    app.click("tree-e2e4 e7e5 g1f3 b8c6 f1b5");
    assert_eq!(badge(&mut app), None);
}

/// Tidy up removes the games imported twice, after asking, and merges one player's spellings
/// into the name chosen to keep, which the games then list.
#[gpui_kit::test]
fn a_database_is_tidied_of_copies_and_spellings(cx: &mut gpui_kit::TestAppContext) {
    const TWICE: &str = "[White \"Carlsen, Magnus\"]\n[Black \"Ding, Liren\"]\n\
        [Date \"2024.01.02\"]\n[Result \"1-0\"]\n\n1. e4 e5 1-0\n\n\
        [White \"Carlsen, Magnus\"]\n[Black \"Ding, Liren\"]\n\
        [Date \"2024.01.02\"]\n[Result \"1-0\"]\n\n1. e4 e5 1-0\n\n\
        [White \"Carlsen, M\"]\n[Black \"Ding, Liren\"]\n\
        [Date \"2023.01.02\"]\n[Result \"0-1\"]\n\n1. d4 d5 0-1\n";
    let mut app = Harness::start(cx);
    app.open("Game database");
    let files = tempfile::tempdir().expect("a folder for the file");
    let path = files.path().join("tidy.pgn");
    std::fs::write(&path, TWICE).expect("the file");
    app.click("gamedb-import");
    app.wait_for_open_dialog();
    app.cx
        .simulate_path_prompt_response(move |_| Some(vec![path]));
    wait_for_databases(&mut app, "the games in", &[("tidy", 3)]);

    app.click("database-tidy");
    app.click_label("Remove duplicate games…");
    app.click_label("Remove");
    wait_for_databases(&mut app, "the copy removed", &[("tidy", 2)]);

    app.click("database-tidy");
    app.click_label("Merge player names…");
    app.find("names-merge-all");
    app.click_label("Carlsen, M");
    app.click_label("Merge into Carlsen, M");
    app.wait_gone(("names-merge", 0usize));
    app.click("names-done");
    wait_for_listed(
        &mut app,
        "both games under the name kept",
        &[
            "Carlsen, M against Ding, Liren, 1-0",
            "Carlsen, M against Ding, Liren, 0-1",
        ],
    );
}

/// A game's menu opens a report on either player: their games and years, how they did with each
/// colour and in which openings, and their best wins, one of which opens on the board. Show
/// games lists all their games.
#[gpui_kit::test]
fn a_player_s_report_reads_their_games_at_a_glance(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_games(&mut app);
    app.click(("database-game-actions", 1usize));
    app.click_label("About Carlsen, Magnus");
    assert_eq!(
        app.find("report-summary").label(),
        Some("2 games · 2024 · Rated 2830")
    );
    assert_eq!(app.find("report-white").label(), Some("As White: 1 game"));
    assert_eq!(app.find("report-black").label(), Some("As Black: 1 game"));
    assert_eq!(
        app.find(("report-white", 0usize)).label(),
        Some("1. e4 e5, 1 game, Won 1 · Drawn 0 · Lost 0 · 100%")
    );
    assert_eq!(
        app.find(("report-black", 0usize)).label(),
        Some("1. Nf3 Nc6, 1 game, Won 0 · Drawn 1 · Lost 0 · 50%")
    );
    app.click_label("Won against Gukesh D");
    app.wait_for_page(crate::pages::Page::Analysis);

    app.open("Game database");
    app.click(("database-game-actions", 0usize));
    app.click_label("About Gukesh D");
    app.click("report-games");
    wait_for_listed(
        &mut app,
        "Gukesh's games",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH],
    );
}

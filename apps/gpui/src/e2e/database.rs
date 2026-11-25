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
            .filter(|label| label.contains(" against "))
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
            .filter(|label| label.contains(" against "))
            .map(str::to_string)
            .collect();
        shown == expected
    });
}

const CARLSEN_GUKESH: &str = "Carlsen, Magnus against Gukesh D, 1-0";
const GUKESH_CARLSEN: &str = "Gukesh D against Carlsen, Magnus, 1/2-1/2";
const SMITH_JONES: &str = "Smith against Jones, 0-1";

/// A PGN file imported on the page lists its games, newest first; the filters narrow them; a
/// game opens on the analysis board, whose "Games with this position" comes back to the games
/// that reached it (here by two move orders); and the database is deleted after asking.
#[gpui_kit::test]
fn a_database_is_imported_searched_opened_and_deleted(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
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
        &mut app,
        "every game, newest first",
        &[GUKESH_CARLSEN, CARLSEN_GUKESH, SMITH_JONES],
    );

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
                .filter(|label| label.contains(" against "))
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

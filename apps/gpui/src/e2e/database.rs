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

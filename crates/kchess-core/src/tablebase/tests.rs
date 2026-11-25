use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::test_host::TestHost;
use crate::usage::Unmetered;

/// The tables of the fixtures, as `files.txt` lists them.
fn fixture_files() -> Vec<TableFile> {
    parse_files(FILES)
        .unwrap()
        .into_iter()
        .filter(|file| file.name.starts_with("KQvK.") || file.name.starts_with("KRvK."))
        .collect()
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tablebase/fixtures")
}

/// Tablebases in a fresh folder holding the fixtures' tables, listing `files`.
fn installed(files: Vec<TableFile>) -> (tempfile::TempDir, Tablebases) {
    let dir = tempfile::tempdir().unwrap();
    for file in fixture_files() {
        std::fs::copy(fixtures().join(&file.name), dir.path().join(&file.name)).unwrap();
    }
    let tables = Tablebases::new(
        TestHost::quiet(),
        Arc::new(Unmetered),
        dir.path().to_path_buf(),
        "http://127.0.0.1:9".into(),
        files,
    );
    (dir, tables)
}

async fn looked_up(tables: &Tablebases, fen: &str) -> Option<SavedLookup> {
    tables.lookup(fen, 7).await.map(|found| found.saved)
}

#[test]
fn the_list_is_every_table_of_up_to_five_pieces_with_its_checksum() {
    let files = parse_files(FILES).unwrap();
    assert_eq!(files.len(), 290);
    assert_eq!(
        files.iter().map(|file| file.bytes).sum::<u64>(),
        983_957_920
    );
    let wdl = files.iter().filter(|file| file.name.ends_with(".rtbw"));
    assert_eq!(wdl.count(), 145);
    assert!(files.iter().all(|file| {
        let stem = file.name.split('.').next().unwrap();
        stem.chars().filter(|c| *c != 'v').count() <= PIECES as usize
    }));
    let kqvk = &files.iter().find(|file| file.name == "KQvK.rtbz").unwrap();
    assert_eq!(
        kqvk.url(ORIGIN),
        "https://tablebase.lichess.ovh/tables/standard/3-4-5-dtz/KQvK.rtbz"
    );
    // Each fixture is the file the list names.
    for file in fixture_files() {
        let digest = kchess_delta::file_digest(&fixtures().join(&file.name)).unwrap();
        assert_eq!(digest, file.sha256, "{}", file.name);
    }
}

#[test]
fn a_list_line_that_could_name_another_folder_is_refused() {
    let hash = "a".repeat(64);
    for line in [
        format!("../KQvK.rtbw 10 {hash}"),
        format!("KQvK.exe 10 {hash}"),
        format!("KQvK.rtbw ten {hash}"),
        "KQvK.rtbw 10 abc".to_string(),
        format!("KQvK.rtbw 10 {hash} more"),
    ] {
        assert!(parse_files(&line).is_err(), "{line}");
    }
    assert_eq!(
        parse_files(&format!("KQvK.rtbw 10 {hash}\n\n"))
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_won_position_lists_the_mate_first_from_the_loser_s_side() {
    let (_dir, tables) = installed(fixture_files());
    // Qb8 mates at once; the rest win more slowly.
    let found = looked_up(&tables, "7k/8/6K1/8/8/8/8/1Q6 w - - 0 1")
        .await
        .expect("covered");
    assert_eq!(found.kind, PositionLookupKind::Tablebase);
    assert_eq!(found.fetched_at, 7);
    assert_eq!(found.category.as_deref(), Some("win"));
    assert!(matches!(found.dtz, Some(Some(dtz)) if dtz > 0));
    let best = &found.moves[0];
    assert_eq!((best.san.as_str(), best.uci.as_str()), ("Qb8#", "b1b8"));
    assert_eq!(best.category.as_deref(), Some("loss"));
    assert_eq!(best.dtz, Some(Some(0)));
    // Every move, the winning ones before the rest.
    let legal = "7k/8/6K1/8/8/8/8/1Q6 w - - 0 1"
        .parse::<Fen>()
        .unwrap()
        .into_position::<Chess>(CastlingMode::Standard)
        .unwrap()
        .legal_moves()
        .len();
    assert_eq!(found.moves.len(), legal);
    let ranks: Vec<u8> = found
        .moves
        .iter()
        .map(|entry| match entry.category.as_deref() {
            Some("loss") => 0,
            Some("draw") => 3,
            _ => 6,
        })
        .collect();
    assert!(ranks.is_sorted(), "{ranks:?}");

    // The same material with the other side to move is lost for it.
    let lost = looked_up(&tables, "7k/8/8/8/8/8/1Q6/K7 b - - 0 1")
        .await
        .expect("covered");
    assert_eq!(lost.category.as_deref(), Some("loss"));
    assert!(
        lost.moves
            .iter()
            .all(|entry| entry.category.as_deref() == Some("win"))
    );
}

#[tokio::test]
async fn taking_the_rook_saves_the_draw() {
    let (_dir, tables) = installed(fixture_files());
    let found = looked_up(&tables, "8/8/8/8/8/5k2/6R1/4K3 b - - 0 1")
        .await
        .expect("covered");
    assert_eq!(found.category.as_deref(), Some("draw"));
    assert_eq!(found.moves[0].san, "Kxg2");
    assert_eq!(found.moves[0].category.as_deref(), Some("draw"));
    assert!(
        found.moves[1..]
            .iter()
            .all(|entry| entry.category.as_deref() == Some("win"))
    );
}

#[tokio::test]
async fn a_position_the_tables_on_disk_do_not_cover_is_left_to_lichess() {
    let (_dir, tables) = installed(fixture_files());
    for fen in [
        // A pawn ending: its table is not on disk.
        "8/8/8/8/8/8/4P3/4K2k w - - 0 1",
        // Six pieces.
        "7k/8/8/8/8/8/PPPP4/K7 w - - 0 1",
        // Castling rights.
        "4k3/8/8/8/8/8/8/R3K3 w Q - 0 1",
        // Not a position.
        "not a fen",
    ] {
        assert!(looked_up(&tables, fen).await.is_none(), "{fen}");
    }
    let (empty, nothing) = (tempfile::tempdir().unwrap(), Vec::new());
    let none = Tablebases::new(
        TestHost::quiet(),
        Arc::new(Unmetered),
        empty.path().join("missing"),
        String::new(),
        nothing,
    );
    assert!(
        looked_up(&none, "7k/8/8/8/8/8/1Q6/K7 b - - 0 1")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn the_status_counts_the_tables_on_disk_and_delete_removes_them() {
    let mut files = fixture_files();
    files.push(TableFile {
        name: "KPvK.rtbw".into(),
        bytes: 7824,
        sha256: "0".repeat(64),
    });
    let (dir, tables) = installed(files);
    let status = tables.status().await;
    assert!(!status.installed);
    assert_eq!((status.files, status.total_files), (4, 5));
    assert_eq!(status.bytes, 272 + 5392 + 208 + 7632);
    assert_eq!(status.total_bytes, status.bytes + 7824);
    assert_eq!(status.pieces, 5);
    assert!(!status.busy);
    // A file cut short is not counted.
    std::fs::write(dir.path().join("KQvK.rtbw"), b"short").unwrap();
    assert_eq!(tables.status().await.files, 3);

    let deleted = tables.delete().await.unwrap();
    assert_eq!((deleted.files, deleted.bytes), (0, 0));
    assert!(!dir.path().exists());
    assert!(
        looked_up(&tables, "7k/8/8/8/8/8/1Q6/K7 b - - 0 1")
            .await
            .is_none()
    );
    // Deleting what is gone succeeds.
    tables.delete().await.unwrap();
}

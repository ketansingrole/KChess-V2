//! Lichess studies over local fixtures (`core/src/services/studies.ts`). Ports
//! `core/tests/unit/study-sync.test.ts` and the study splitting case of
//! `core/tests/unit/watch-and-lookup.test.ts`.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::{Arc, Mutex};

use fixtures::{MemoryStore, MemoryTokens, Recording, Reply, form_field, lichess_service, serve};
use kchess_core::lichess::accounts::Reply as Outcome;
use kchess_core::lichess::studies::{
    StudySyncRequest, export_to_lichess_study, lichess_studies, lichess_study_chapters, split_pgn,
    sync_lichess_study,
};

const BASELINE: &str = "[Site \"https://lichess.org/study/Study001/Chapter1\"]\n[Event \"Example\"]\n[Result \"*\"]\n\n1. e4 {Keep} e5 (1... c5 $1) *";

/// A fixture serving the study's PGN from `cloud` on GET and answering every POST with `post`.
async fn study_fixture(cloud: Arc<Mutex<String>>, post: u16) -> fixtures::Fixture {
    serve(move |seen| {
        if seen.method == "GET" {
            Reply::Text(
                200,
                "application/x-chess-pgn",
                cloud.lock().unwrap().clone(),
            )
        } else if post == 200 {
            Reply::Json(200, "{\"ok\":true}".into())
        } else {
            Reply::Json(post, "{}".into())
        }
    })
    .await
}

fn request(pgn: &str) -> StudySyncRequest {
    StudySyncRequest {
        account: "Alice".into(),
        study_id: "Study001".into(),
        baseline: BASELINE.into(),
        pgn: pgn.into(),
    }
}

fn service(base: &str) -> Arc<kchess_core::lichess::accounts::Lichess> {
    lichess_service(
        base,
        &Arc::new(Recording::default()),
        MemoryStore::with_accounts(&[("Alice", true)]),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    )
}

fn posts(fixture: &fixtures::Fixture) -> Vec<fixtures::Seen> {
    fixture
        .requests()
        .into_iter()
        .filter(|seen| seen.method == "POST")
        .collect()
}

#[tokio::test]
async fn checks_the_cloud_baseline_before_updating_the_same_chapter_preserving_pgn_annotations() {
    let cloud = Arc::new(Mutex::new(BASELINE.to_string()));
    let fixture = study_fixture(Arc::clone(&cloud), 200).await;
    let lichess = service(&fixture.base);
    let edited = BASELINE.replace("{Keep}", "{Edited notes}");
    sync_lichess_study(&lichess, &request(&edited))
        .await
        .unwrap();
    let posts = posts(&fixture);
    let targets: Vec<&str> = posts.iter().map(|seen| seen.path()).collect();
    assert_eq!(
        targets,
        [
            "/api/study/Study001/Chapter1/moves",
            "/api/study/Study001/Chapter1/tags"
        ]
    );
    let pgn = form_field(&posts[0].body, "pgn").unwrap();
    assert!(pgn.contains("Edited notes"));
    assert!(pgn.contains("c5 $1"));
}

#[tokio::test]
async fn blocks_upload_when_the_cloud_changed_without_any_mutations() {
    let cloud = Arc::new(Mutex::new(BASELINE.replace("Keep", "Cloud edit")));
    let fixture = study_fixture(cloud, 200).await;
    let lichess = service(&fixture.base);
    let edited = BASELINE.replace("{Keep}", "{Edited notes}");
    let error = sync_lichess_study(&lichess, &request(&edited))
        .await
        .unwrap_err();
    assert!(error.message.contains("cloud study changed"));
    assert!(posts(&fixture).is_empty());
}

#[tokio::test]
async fn rejects_illegal_moves_structure_changes_and_wrong_chapter_identities_before_writing() {
    let cloud = Arc::new(Mutex::new(BASELINE.to_string()));
    let fixture = study_fixture(Arc::clone(&cloud), 200).await;
    let lichess = service(&fixture.base);
    let illegal = BASELINE.replace("e4", "e5");
    let error = sync_lichess_study(&lichess, &request(&illegal))
        .await
        .unwrap_err();
    assert!(error.message.contains("illegal move"));

    let mut extended = request(BASELINE);
    extended.baseline = format!("{BASELINE}\n\n[Event \"Extra\"]\n\n1. d4 *");
    let error = sync_lichess_study(&lichess, &extended).await.unwrap_err();
    assert!(error.message.contains("structure changed"));

    let wrong = BASELINE.replace("/Study001/", "/Other001/");
    *cloud.lock().unwrap() = wrong.clone();
    let mut mismatched = request(&wrong);
    mismatched.baseline = wrong.clone();
    let error = sync_lichess_study(&lichess, &mismatched).await.unwrap_err();
    assert!(error.message.contains("chapter identity"));
    assert!(posts(&fixture).is_empty());
}

#[tokio::test]
async fn does_not_retry_an_ambiguous_mutation() {
    let cloud = Arc::new(Mutex::new(BASELINE.to_string()));
    let fixture = study_fixture(cloud, 500).await;
    let lichess = service(&fixture.base);
    let edited = BASELINE.replace("{Keep}", "{Edited notes}");
    let error = sync_lichess_study(&lichess, &request(&edited))
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .contains("from POST /api/study/Study001/Chapter1/moves")
    );
    assert_eq!(posts(&fixture).len(), 1);
}

#[tokio::test]
async fn appends_new_chapters_to_the_linked_cloud_study_without_duplicating_the_existing_chapters()
{
    let cloud = Arc::new(Mutex::new(BASELINE.to_string()));
    let fixture = study_fixture(cloud, 200).await;
    let lichess = service(&fixture.base);
    let appended = format!("{BASELINE}\n\n[ChapterName \"Second chapter\"]\n\n1. d4 *");
    sync_lichess_study(&lichess, &request(&appended))
        .await
        .unwrap();
    let posts = posts(&fixture);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path(), "/api/study/Study001/import-pgn");
    assert_eq!(
        form_field(&posts[0].body, "name").as_deref(),
        Some("Second chapter")
    );
}

#[tokio::test]
async fn lists_studies_newest_first_and_drops_invalid_records() {
    let fixture = serve(|_| {
        Reply::Text(
            200,
            "application/x-ndjson",
            concat!(
                "{\"id\":\"Older001\",\"name\":\"Old\",\"createdAt\":1}\n",
                "{\"id\":\"Newer002\",\"name\":\"New\",\"createdAt\":2,\"updatedAt\":9}\n",
                "{\"id\":\"bad\",\"name\":\"Invalid id\"}\n"
            )
            .into(),
        )
    })
    .await;
    let lichess = service(&fixture.base);
    let Outcome::Done(studies) = lichess_studies(&lichess, "Alice").await.unwrap() else {
        panic!("the login is connected");
    };
    let ids: Vec<&str> = studies.iter().map(|study| study.id.as_str()).collect();
    assert_eq!(ids, ["Newer002", "Older001"]);
    assert_eq!(studies[1].updated_at, 1);
}

#[tokio::test]
async fn names_chapters_from_their_tags_and_numbers_the_rest() {
    let text =
        format!("{BASELINE}\n\n[Event \"Other\"]\n\n1. d4 *\n\n[ChapterName \"Named\"]\n\n1. c4 *");
    let fixture = serve(move |_| Reply::Text(200, "application/x-chess-pgn", text.clone())).await;
    let lichess = service(&fixture.base);
    let Outcome::Done(chapters) = lichess_study_chapters(&lichess, "Alice", "Study001")
        .await
        .unwrap()
    else {
        panic!("the login is connected");
    };
    let names: Vec<&str> = chapters
        .iter()
        .map(|chapter| chapter.name.as_str())
        .collect();
    assert_eq!(names, ["Example", "Other", "Named"]);
}

#[tokio::test]
async fn exports_a_game_into_a_new_private_study_and_reports_its_id() {
    let fixture = serve(|seen| {
        if seen.path() == "/api/study" {
            Reply::Json(200, "{\"id\":\"NewStudy1\"}".into())
        } else {
            Reply::Json(200, "{}".into())
        }
    })
    .await;
    let lichess = service(&fixture.base);
    let Outcome::Done(id) = export_to_lichess_study(&lichess, "Alice", "", "Game", BASELINE)
        .await
        .unwrap()
    else {
        panic!("the login is connected");
    };
    assert_eq!(id, "NewStudy1");
    let create = fixture.to("/api/study");
    assert_eq!(
        form_field(&create[0].body, "visibility").as_deref(),
        Some("private")
    );
    assert_eq!(fixture.to("/api/study/NewStudy1/import-pgn").len(), 1);
}

#[test]
fn splits_a_study_into_chapters() {
    let two = format!(
        "{BASELINE}\n\n\n{}\n",
        BASELINE.replace("Example", "Second")
    );
    assert_eq!(split_pgn(&two).len(), 2);
}

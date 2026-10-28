//! Lichess studies over local fixtures: study sync and the splitting of a study into chapters.

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

/// A fixture serving the study's PGN from `cloud` on GET and answering every POST with `post`. A
/// successful import appends its chapter to `cloud`, as Lichess does.
async fn study_fixture(cloud: Arc<Mutex<String>>, post: u16) -> fixtures::Fixture {
    serve(move |seen| {
        if seen.method == "GET" {
            Reply::Text(
                200,
                "application/x-chess-pgn",
                cloud.lock().unwrap().clone(),
            )
        } else if post == 200 && seen.path().ends_with("/import-pgn") {
            let pgn = form_field(&seen.body, "pgn").unwrap_or_default();
            let mut cloud = cloud.lock().unwrap();
            let site = format!(
                "[Site \"https://lichess.org/study/Study001/Added{}\"]\n",
                cloud.matches("[Site").count()
            );
            *cloud = format!("{cloud}\n\n{site}{pgn}");
            Reply::Json(200, "{\"chapters\":[{\"id\":\"Added\"}]}".into())
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
async fn resumes_an_upload_that_stopped_partway_without_rewriting_what_it_wrote() {
    // A failure after the first chapter's write left the cloud matching neither the baseline
    // nor the edit, and every later upload was refused.
    let second = "[Site \"https://lichess.org/study/Study001/Chapter2\"]\n[Event \"Two\"]\n[Result \"*\"]\n\n1. d4 *";
    let baseline = format!("{BASELINE}\n\n{second}");
    let edited = format!(
        "{}\n\n{}",
        BASELINE.replace("{Keep}", "{Edited notes}"),
        second.replace("1. d4", "1. c4")
    );
    // Lichess holds the first edit (with a tag of its own) and still the second baseline.
    let written = BASELINE
        .replace("{Keep}", "{Edited notes}")
        .replace("[Result", "[Annotator \"Alice\"]\n[Result");
    let cloud = Arc::new(Mutex::new(format!("{written}\n\n{second}")));
    let fixture = study_fixture(cloud, 200).await;
    let lichess = service(&fixture.base);
    let mut retry = request(&edited);
    retry.baseline = baseline;
    sync_lichess_study(&lichess, &retry).await.unwrap();
    let targets: Vec<String> = posts(&fixture)
        .iter()
        .map(|seen| seen.path().to_string())
        .collect();
    assert_eq!(
        targets,
        [
            "/api/study/Study001/Chapter2/moves",
            "/api/study/Study001/Chapter2/tags"
        ]
    );
}

/// An upload that wrote a chapter's moves and then failed before its tags: retried, it writes the
/// tags (never the moves again), including a tag the edit removed. It was refused as a cloud
/// change when a tag was edited, and reported done, with the tag still there, when one was removed.
#[tokio::test]
async fn resumes_an_upload_that_stopped_between_a_chapters_moves_and_its_tags() {
    let annotated = BASELINE.replace("[Result", "[Annotator \"Alice\"]\n[Result");
    for (baseline, edited, tag) in [
        (
            BASELINE.to_string(),
            BASELINE
                .replace("Example", "After")
                .replace("{Keep}", "{Edited}"),
            "[Event \"After\"]",
        ),
        (
            annotated.clone(),
            annotated
                .replace("[Annotator \"Alice\"]\n", "")
                .replace("{Keep}", "{Edited}"),
            "[Annotator \"\"]",
        ),
    ] {
        // Lichess holds the edit's moves with the baseline's tags.
        let cloud = Arc::new(Mutex::new(baseline.replace("{Keep}", "{Edited}")));
        let fixture = study_fixture(cloud, 200).await;
        let lichess = service(&fixture.base);
        let mut retry = request(&edited);
        retry.baseline = baseline;
        sync_lichess_study(&lichess, &retry).await.unwrap();
        let sent = posts(&fixture);
        let targets: Vec<&str> = sent.iter().map(|seen| seen.path()).collect();
        assert_eq!(targets, ["/api/study/Study001/Chapter1/tags"]);
        assert!(
            form_field(&sent[0].body, "pgn").unwrap().contains(tag),
            "{tag}"
        );
    }
}

/// A full study answers an import with success and no chapters, and a partly accepted import
/// lists only what it added: neither is reported as exported.
#[tokio::test]
async fn an_export_lichess_did_not_fully_import_is_reported() {
    for (chapters, games, message) in [
        ("[]", 1, "Lichess added no chapters."),
        (
            "[{\"id\":\"Chapter1\"}]",
            2,
            "Lichess added only 1 of 2 chapters.",
        ),
    ] {
        let fixture =
            serve(move |_| Reply::Json(200, format!("{{\"chapters\":{chapters}}}"))).await;
        let lichess = service(&fixture.base);
        let pgn = vec![BASELINE; games].join("\n\n");
        let error = export_to_lichess_study(&lichess, "Alice", "Study001", "Game", &pgn)
            .await
            .unwrap_err();
        assert!(error.message.starts_with(message), "{}", error.message);
    }
}

#[tokio::test]
async fn appends_new_chapters_to_the_linked_cloud_study_without_duplicating_the_existing_chapters()
{
    let cloud = Arc::new(Mutex::new(BASELINE.to_string()));
    let fixture = study_fixture(cloud, 200).await;
    let lichess = service(&fixture.base);
    let appended = format!("{BASELINE}\n\n[ChapterName \"Second chapter\"]\n\n1. d4 *");
    let Outcome::Done(chapters) = sync_lichess_study(&lichess, &request(&appended))
        .await
        .unwrap()
    else {
        panic!("the account needs reconnecting");
    };
    assert_eq!(chapters.len(), 2);
    let posts = posts(&fixture);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path(), "/api/study/Study001/import-pgn");
    assert_eq!(
        form_field(&posts[0].body, "name").as_deref(),
        Some("Second chapter")
    );
}

/// Lichess answered an appended chapter's import with success and no chapter, and the upload was
/// reported done (and marked as the cloud baseline) without it.
#[tokio::test]
async fn an_appended_chapter_lichess_did_not_import_fails_the_upload() {
    let cloud = Arc::new(Mutex::new(BASELINE.to_string()));
    let fixture = serve({
        let cloud = Arc::clone(&cloud);
        move |seen| {
            if seen.method == "GET" {
                Reply::Text(
                    200,
                    "application/x-chess-pgn",
                    cloud.lock().unwrap().clone(),
                )
            } else {
                Reply::Json(200, "{\"chapters\":[]}".into())
            }
        }
    })
    .await;
    let lichess = service(&fixture.base);
    let appended = format!("{BASELINE}\n\n[Event \"Two\"]\n[Result \"*\"]\n\n1. d4 *");
    let error = sync_lichess_study(&lichess, &request(&appended))
        .await
        .unwrap_err();
    assert!(
        error.message.starts_with("Lichess added no chapters."),
        "{}",
        error.message
    );
    assert_eq!(fixture.to("/api/study/Study001/import-pgn").len(), 1);
}

/// Lichess acknowledged the import, yet the study read back lacks the chapter: the upload is not
/// done, and uploading again resumes it.
#[tokio::test]
async fn an_upload_whose_chapters_lichess_does_not_hold_is_not_done() {
    let cloud = Arc::new(Mutex::new(BASELINE.to_string()));
    let fixture = serve({
        let cloud = Arc::clone(&cloud);
        move |seen| {
            if seen.method == "GET" {
                Reply::Text(
                    200,
                    "application/x-chess-pgn",
                    cloud.lock().unwrap().clone(),
                )
            } else {
                Reply::Json(200, "{\"chapters\":[{\"id\":\"Chapter2\"}]}".into())
            }
        }
    })
    .await;
    let lichess = service(&fixture.base);
    let appended = format!("{BASELINE}\n\n[Event \"Two\"]\n[Result \"*\"]\n\n1. d4 *");
    let error = sync_lichess_study(&lichess, &request(&appended))
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .starts_with("Lichess holds only 1 of 2 chapters."),
        "{}",
        error.message
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
            Reply::Json(200, "{\"chapters\":[{\"id\":\"Chapter1\"}]}".into())
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

/// A malformed study record fails the listing as it arrives: the stream stays open, yet the
/// listing ends with the record's error (`JSON.parse` in the TypeScript line callback).
#[tokio::test]
async fn a_malformed_study_record_fails_the_listing_as_it_arrives() {
    let (lines_tx, lines_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut lines = Some(lines_rx);
    let fixture = serve(move |_| match lines.take() {
        Some(open) => Reply::Stream(open),
        None => Reply::Hang,
    })
    .await;
    let lichess = service(&fixture.base);
    lines_tx
        .send("{\"id\": \"Study001\", \"name\": \n".to_string())
        .expect("the stream is open");
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        lichess_studies(&lichess, "Alice"),
    )
    .await
    .expect("the listing ends without waiting for the stream to close");
    let error = outcome.unwrap_err();
    assert!(
        error.message.contains("unreadable study record"),
        "message was {}",
        error.message
    );
    drop(lines_tx);
}

#[tokio::test]
async fn rejects_an_oversized_study_before_the_server_finishes_the_body() {
    let (send, receive) = tokio::sync::mpsc::unbounded_channel();
    let mut receive = Some(receive);
    let fixture =
        serve(move |_| Reply::StreamText(200, "application/x-chess-pgn", receive.take().unwrap()))
            .await;
    let lichess = service(&fixture.base);
    // The sender deliberately stays open: rejection must happen before EOF.
    for _ in 0..9 {
        send.send("x".repeat(500_000)).unwrap();
    }
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        lichess_study_chapters(&lichess, "Alice", "Study001"),
    )
    .await
    .expect("oversized download fails without waiting for EOF")
    .unwrap_err();
    assert_eq!(error.message, "That study is too large to import.");
    drop(send);
}

//! `app-store-review <versions.json> <submissions.json>`: whether App Review already has a
//! version of the Mac app, from what `asc versions list` and `asc review submissions-list` answer
//! (`app-store-publish.yml`). It prints that version and its state, or nothing when none is.
//!
//! Publishing a release stages its version and submits it for review. While another version is
//! with Apple, that would replace it and its review would start again, so the workflow submits
//! nothing then. A version is with Apple while it waits for review, is in review or is approved
//! and not yet released, and also while its submission is still open: rejected with Apple's
//! questions unresolved, say, which the developer answers in the same submission.

use crate::util::Result;
use serde_json::Value;

/// A version's states (`appStoreState`) while Apple has it.
const VERSION_WITH_APPLE: &[&str] = &[
    "WAITING_FOR_REVIEW",
    "IN_REVIEW",
    "READY_FOR_REVIEW",
    "ACCEPTED",
    "PENDING_DEVELOPER_RELEASE",
    "PENDING_APPLE_RELEASE",
    "PROCESSING_FOR_APP_STORE",
    "PROCESSING_FOR_DISTRIBUTION",
];

/// A review submission's states until it is complete or cancelled.
const SUBMISSION_OPEN: &[&str] = &[
    "READY_FOR_REVIEW",
    "WAITING_FOR_REVIEW",
    "IN_REVIEW",
    "UNRESOLVED_ISSUES",
    "CANCELING",
    "COMPLETING",
];

fn data(answer: &Value) -> &[Value] {
    answer["data"].as_array().map_or(&[], Vec::as_slice)
}

fn attribute<'a>(item: &'a Value, name: &str) -> &'a str {
    item["attributes"][name].as_str().unwrap_or_default()
}

/// The version App Review has, as "2026.11.3 (REJECTED, its review unresolved)", from the
/// answers of `asc versions list` and `asc review submissions-list`.
pub fn with_apple(versions: &Value, submissions: &Value) -> Option<String> {
    if let Some(version) = data(versions)
        .iter()
        .find(|v| VERSION_WITH_APPLE.contains(&attribute(v, "appStoreState")))
    {
        return Some(format!(
            "{} ({})",
            attribute(version, "versionString"),
            attribute(version, "appStoreState")
        ));
    }
    let submission = data(submissions)
        .iter()
        .find(|s| SUBMISSION_OPEN.contains(&attribute(s, "state")))?;
    let state = attribute(submission, "state");
    // The submission names no version unless asked to; the newest version not on the store is
    // the one it holds.
    let version = data(versions)
        .iter()
        .find(|v| attribute(v, "appStoreState") != "READY_FOR_SALE");
    Some(match version {
        Some(v) => format!(
            "{} ({}, its review {state})",
            attribute(v, "versionString"),
            attribute(v, "appStoreState")
        ),
        None => format!("A review submission ({state})"),
    })
}

fn read(path: &str) -> Result<Value> {
    let text = std::fs::read_to_string(path).map_err(|cause| format!("{path}: {cause}"))?;
    serde_json::from_str(&text).map_err(|cause| format!("{path}: {cause}"))
}

/// `app-store-review <versions.json> <submissions.json>`.
pub fn run(args: &[String]) -> Result {
    let [versions, submissions] = args else {
        return Err(
            "app-store-review needs the files of `asc versions list` and `asc review \
             submissions-list`."
                .into(),
        );
    };
    if let Some(version) = with_apple(&read(versions)?, &read(submissions)?) {
        println!("{version}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn versions(states: &[(&str, &str)]) -> Value {
        let data: Vec<Value> = states
            .iter()
            .map(|(version, state)| {
                json!({"type": "appStoreVersions", "id": version,
                       "attributes": {"versionString": version, "appStoreState": state}})
            })
            .collect();
        json!({ "data": data })
    }

    fn submissions(states: &[&str]) -> Value {
        let data: Vec<Value> = states
            .iter()
            .map(|state| {
                json!({"type": "reviewSubmissions", "id": "1",
                       "attributes": {"platform": "MAC_OS", "state": state}})
            })
            .collect();
        json!({ "data": data })
    }

    /// 2026-11-27: 2026.11.3 was rejected and Apple's questions were still open in its
    /// submission when 2026.11.10 was published. The version's state alone (REJECTED) said App
    /// Review had nothing, so publishing would have submitted 2026.11.10 in its place.
    #[test]
    fn a_rejected_version_with_its_review_unresolved_is_with_apple() {
        assert_eq!(
            with_apple(
                &versions(&[("2026.11.3", "REJECTED")]),
                &submissions(&["UNRESOLVED_ISSUES"])
            )
            .as_deref(),
            Some("2026.11.3 (REJECTED, its review UNRESOLVED_ISSUES)")
        );
    }

    #[test]
    fn a_version_waiting_for_or_in_review_is_with_apple() {
        for state in ["WAITING_FOR_REVIEW", "IN_REVIEW", "PENDING_DEVELOPER_RELEASE"] {
            assert_eq!(
                with_apple(
                    &versions(&[("2026.11.4", state), ("2026.11.3", "READY_FOR_SALE")]),
                    &submissions(&["COMPLETE"])
                ),
                Some(format!("2026.11.4 ({state})"))
            );
        }
    }

    #[test]
    fn nothing_is_with_apple_once_each_review_is_over() {
        assert_eq!(
            with_apple(
                &versions(&[("2026.11.3", "READY_FOR_SALE")]),
                &submissions(&["COMPLETE", "CANCELED"])
            ),
            None
        );
        assert_eq!(with_apple(&versions(&[]), &submissions(&[])), None);
        // A rejection its developer has withdrawn from review is over too.
        assert_eq!(
            with_apple(
                &versions(&[("2026.11.3", "DEVELOPER_REJECTED")]),
                &submissions(&["COMPLETE"])
            ),
            None
        );
    }

    #[test]
    fn the_command_reads_both_answers() {
        let dir = tempfile::tempdir().unwrap();
        let versions_file = dir.path().join("versions.json");
        let submissions_file = dir.path().join("submissions.json");
        std::fs::write(&versions_file, versions(&[("2026.11.3", "REJECTED")]).to_string())
            .unwrap();
        std::fs::write(&submissions_file, submissions(&["UNRESOLVED_ISSUES"]).to_string())
            .unwrap();
        let args = [versions_file, submissions_file].map(|path| path.display().to_string());
        run(&args).unwrap();
        assert!(run(&args[..1]).is_err());
    }
}

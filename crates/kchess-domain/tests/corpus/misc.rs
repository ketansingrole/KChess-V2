use serde_json::Value;

use kchess_domain::misc::constants::SESSION_KINDS;
use kchess_domain::misc::value::J;
use kchess_domain::misc::{self, checks, documents};

use kchess_contracts::{check, types};

use super::Out;

const DOCUMENTS: &[&str] = &[
    "validateCoreArguments",
    "decodeAnalysisSession",
    "decodeLocalSession",
    "decodeComputerSession",
    "decodeArchiveIdentity",
    "decodeStoredSession",
    "encodeSession",
    "isSessionKind",
    "decodeJoinedTournaments",
    "decodeRepertoireMisses",
    "assertSession",
    "assertWatchTarget",
];

/// An argument's check that is its contract type (`kchess_contracts::check::Validate`): the value
/// it normalized, as JSON.
fn typed<T: check::Validate + serde::Serialize>(value: J) -> Out {
    let checked = T::validate(value)?;
    serde_json::to_value(checked)
        .map(misc::value::integral)
        .map_err(|error| error.to_string())
}

/// A validator of one argument, by the name the recorded suites called it.
fn validator(method: &str) -> Option<fn(J) -> Out> {
    Some(match method {
        "assertAction" => checks::action,
        "assertActivityMax" => typed::<check::ActivityMax>,
        "assertAnalysisRequest" => checks::analysis_request,
        "assertArchivedGameShape" => typed::<check::ArchivedGame>,
        "assertBestMoveOptions" => checks::best_move_options,
        "assertBroadcastQuery" => checks::broadcast_query,
        "assertChatRoom" => checks::chat_room,
        "assertChatText" => checks::chat_text,
        "assertCoreSettings" => checks::core_settings,
        "assertDays" => typed::<check::Days>,
        "assertDeclineReason" => checks::decline_reason,
        "assertFriendList" => checks::friend_list,
        "assertGameId" => checks::game_id,
        "assertGameIds" => checks::game_ids,
        "assertGamePageQuery" => checks::game_page_query,
        "assertInsightsQuery" => checks::insights_query,
        "assertLadderQuery" => typed::<types::LocalLadderQuery>,
        "assertLevel" => checks::level,
        "assertLibraryId" => typed::<check::LibraryId>,
        "assertLichessId" => checks::lichess_id,
        "assertLocalQuery" => typed::<types::LocalPuzzleQuery>,
        "assertLookupOptions" => typed::<types::LookupOptions>,
        "assertMessageText" => checks::message_text,
        "assertMoves" => checks::moves,
        "assertNewArena" => checks::new_arena,
        "assertOnlineOptions" => checks::online_options,
        "assertOptionalAccount" => checks::optional_account,
        "assertPerfType" => checks::perf_type,
        "assertPuzzleAttempt" => typed::<types::PuzzleAttemptInput>,
        "assertPuzzleProgressQuery" => typed::<types::PuzzleProgressQuery>,
        "assertPuzzleRequest" => typed::<types::PuzzleRequest>,
        "assertPuzzleSolve" => typed::<types::PuzzleSolveRequest>,
        "assertRepertoireKey" => typed::<check::RepertoireKey>,
        "assertReviewKey" => typed::<check::ReviewKey>,
        "assertReviewRequest" => checks::review_request,
        "assertRunInput" => typed::<types::RunInput>,
        "assertRunKind" => typed::<types::RunKind>,
        "assertSessionKind" => typed::<types::SessionKind>,
        "assertSettings" => checks::settings,
        "assertSide" => typed::<check::Side>,
        "assertStudyCommandShape" => typed::<types::StudyCommand>,
        "assertStudySyncRequest" => checks::study_sync_request,
        "assertTheme" => checks::theme,
        "assertTournamentId" => checks::tournament_id,
        "assertTournamentPassword" => checks::tournament_password,
        "assertTournamentSystem" => checks::tournament_system,
        "assertUci" => checks::uci,
        "assertUsername" => typed::<check::Username>,
        "assertUsernames" => checks::usernames,
        "assertVoiceAttempt" => typed::<types::VoiceAttemptInput>,
        "assertVoiceId" => typed::<check::VoiceId>,
        "assertVoiceLimit" => typed::<check::VoiceLimit>,
        "assertVoiceUpdate" => typed::<types::VoiceUpdate>,
        _ => return None,
    })
}

/// Each method's first argument is the sidecar of the values JSON cannot carry.
pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    if !DOCUMENTS.contains(&method) && validator(method).is_none() {
        return None;
    }
    let Some((sidecar, user)) = args.split_first() else {
        return Some(Err(format!(
            "{method} was called without its lossless arguments"
        )));
    };
    Some(misc::inputs(user, sidecar).and_then(|values| answer(method, values)))
}

fn answer(method: &str, values: Vec<J>) -> Out {
    let arg = |i: usize| values.get(i).unwrap_or(&J::Undef);
    let kind = || arg(0).str().unwrap_or_default().to_string();
    let decoded = |kind: &str, raw: &J| -> Out {
        Ok(documents::decode_stored(kind, raw)?.map_or(Value::Null, J::into_value))
    };
    match method {
        // The core's argument check, as its method table declares it.
        "validateCoreArguments" => match values.as_slice() {
            [J::Str(name), J::Arr(list)] => {
                kchess_contracts::core_api::check_arguments(name, list.clone())
                    .unwrap_or_else(|| Err(format!("Invalid argument count for {name}.")))
                    .map(|_| Value::Null)
            }
            _ => Err("The arguments of an argument check are a method name and a list.".into()),
        },
        "decodeAnalysisSession" => decoded("analysis", arg(0)),
        "decodeLocalSession" => decoded("local", arg(0)),
        "decodeComputerSession" => decoded("computer", arg(0)),
        "decodeArchiveIdentity" => decoded("archive:board", arg(0)),
        "decodeStoredSession" => decoded(&kind(), arg(1)),
        "encodeSession" => Ok(documents::encode_session(&kind(), arg(1)).into_value()),
        "isSessionKind" => Ok(Value::Bool(
            arg(0).str().is_some_and(|k| SESSION_KINDS.contains(&k)),
        )),
        "decodeJoinedTournaments" => Ok(documents::decode_joined_tournaments(arg(0)).into_value()),
        "decodeRepertoireMisses" => Ok(documents::decode_repertoire_misses(arg(0)).into_value()),
        "assertSession" => documents::assert_session(values).map(J::into_value),
        "assertWatchTarget" => {
            let mut values = values.into_iter();
            let value = values.next().unwrap_or(J::Undef);
            misc::validate::assert_watch_target(value, values.next().unwrap_or(J::Undef))
        }
        _ => match validator(method) {
            Some(check) => check(values.into_iter().next().unwrap_or(J::Undef)),
            None => Err(format!("unknown rules method {method}")),
        },
    }
}

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

/// An optional argument's check as the suites called it: absent passes, null does not.
fn optional<T: check::Validate + serde::Serialize>(value: J) -> Out {
    match value {
        J::Undef => Ok(Value::Null),
        value => typed::<T>(value),
    }
}

fn broadcast_query(value: J) -> Out {
    optional::<check::BroadcastQuery>(value)
}

/// An entry code: an empty one is none.
fn tournament_password(value: J) -> Out {
    optional::<check::TournamentPassword>(value)
        .map(|code| if code == "" { Value::Null } else { code })
}

/// A validator of one argument, by the name the recorded suites called it.
fn validator(method: &str) -> Option<fn(J) -> Out> {
    Some(match method {
        "assertAction" => typed::<types::OnlineAction>,
        "assertActivityMax" => checks::activity_max,
        "assertAnalysisRequest" => checks::analysis_request,
        "assertArchivedGameShape" => checks::archived_game_shape,
        "assertBestMoveOptions" => checks::best_move_options,
        "assertBroadcastQuery" => broadcast_query,
        "assertChatRoom" => typed::<types::ChatRoom>,
        "assertChatText" => typed::<check::ChatText>,
        "assertCoreSettings" => checks::core_settings,
        "assertDays" => checks::days,
        "assertDeclineReason" => typed::<types::DeclineReason>,
        "assertFriendList" => checks::friend_list,
        "assertGameId" => typed::<check::GameId>,
        "assertGameIds" => checks::game_ids,
        "assertGamePageQuery" => checks::game_page_query,
        "assertInsightsQuery" => checks::insights_query,
        "assertLadderQuery" => checks::ladder_query,
        "assertLevel" => checks::level,
        "assertLibraryId" => typed::<check::LibraryId>,
        "assertLichessId" => typed::<check::LichessId>,
        "assertLocalQuery" => checks::local_query,
        "assertLookupOptions" => typed::<types::LookupOptions>,
        "assertMessageText" => checks::message_text,
        "assertMoves" => checks::moves,
        "assertNewArena" => typed::<types::NewArena>,
        "assertOnlineOptions" => typed::<types::OnlineOptions>,
        "assertOptionalAccount" => typed::<check::OptionalAccount>,
        "assertPerfType" => checks::perf_type,
        "assertPuzzleAttempt" => checks::puzzle_attempt,
        "assertPuzzleProgressQuery" => checks::puzzle_progress_query,
        "assertPuzzleRequest" => checks::puzzle_request,
        "assertPuzzleSolve" => checks::puzzle_solve,
        "assertRepertoireKey" => checks::repertoire_key,
        "assertReviewKey" => checks::review_key,
        "assertReviewRequest" => checks::review_request,
        "assertRunInput" => checks::run_input,
        "assertRunKind" => checks::run_kind,
        "assertSessionKind" => checks::session_kind,
        "assertSettings" => checks::settings,
        "assertSide" => checks::side,
        "assertStudyCommandShape" => checks::study_command_shape,
        "assertStudySyncRequest" => checks::study_sync_request,
        "assertTheme" => checks::theme,
        "assertTournamentId" => typed::<check::TournamentId>,
        "assertTournamentPassword" => tournament_password,
        "assertTournamentSystem" => typed::<types::TournamentSystem>,
        "assertUci" => typed::<check::Uci>,
        "assertUsername" => typed::<check::Username>,
        "assertUsernames" => typed::<check::PresenceUsers>,
        "assertVoiceAttempt" => checks::voice_attempt,
        "assertVoiceId" => checks::voice_id,
        "assertVoiceLimit" => checks::voice_limit,
        "assertVoiceUpdate" => checks::voice_update,
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
            let channels = values.next().map(J::into_value).unwrap_or_default();
            let channels: Vec<String> = serde_json::from_value(channels).unwrap_or_default();
            let channels: Vec<&str> = channels.iter().map(String::as_str).collect();
            let target = check::watch_target(value, &channels)?;
            serde_json::to_value(target).map_err(|error| error.to_string())
        }
        _ => match validator(method) {
            Some(check) => check(values.into_iter().next().unwrap_or(J::Undef)),
            None => Err(format!("unknown rules method {method}")),
        },
    }
}

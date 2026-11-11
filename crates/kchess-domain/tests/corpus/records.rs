//! Reviews, ratings, studies, game results, time controls, accounts, Lichess events and errors,
//! OAuth looks and analysis contexts.

use serde_json::{Value, json};
use shakmaty::variant::VariantPosition;

use kchess_domain::js;
use kchess_domain::records::accounts::{ListedAccount, connected_account_index};
use kchess_domain::records::review::Score;
use kchess_domain::records::{
    context, games, lichess, oauth, online, ratings, review, studies, time_control,
};
use kchess_domain::rules;

use super::{Out, arg};

/// Argument `i` as a list.
fn list<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a [Value], String> {
    arg(args, i)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{name} must be a list"))
}

/// A `{ variant, fen }` setup as the legal position it describes.
fn setup_position(value: Option<&Value>) -> Result<VariantPosition, String> {
    let setup = value
        .and_then(Value::as_object)
        .ok_or("setup must be an object")?;
    let variant = setup
        .get("variant")
        .and_then(Value::as_str)
        .ok_or("setup.variant")?;
    let fen = setup
        .get("fen")
        .and_then(Value::as_str)
        .ok_or("setup.fen")?;
    rules::setup_start(variant, fen).ok_or_else(|| "setup is not a legal position".into())
}

/// An argument as text, as a template string reads any value.
fn text_of(args: &[Value], i: usize) -> String {
    js::to_string(arg(args, i))
}

/// List items as text, as `join(' ')` reads them: null becomes ''.
fn texts_of(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|item| {
            if item.is_null() {
                String::new()
            } else {
                js::to_string(Some(item))
            }
        })
        .collect()
}

/// A string argument, '' for anything else.
fn str_or_empty(args: &[Value], i: usize) -> &str {
    arg(args, i).and_then(Value::as_str).unwrap_or_default()
}

fn whole(n: f64) -> Value {
    json!(n as i64)
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let minutes = || js::to_number(arg(args, 0));
    let increment = || js::to_number(arg(args, 1));
    Some(match method {
        "reviewKey" => list(args, 1, "moves").map(|moves| {
            json!(review::review_key(
                &text_of(args, 0),
                &texts_of(moves),
                arg(args, 2).and_then(Value::as_str)
            ))
        }),
        "endEval" => Ok(
            match review::end_eval(str_or_empty(args, 0), str_or_empty(args, 1)) {
                Score::Mate(n) => json!({ "mate": whole(n) }),
                Score::Cp(n) => json!({ "cp": whole(n) }),
            },
        ),
        "isReviewablePerf" => Ok(json!(
            arg(args, 0)
                .and_then(Value::as_str)
                .is_some_and(review::is_reviewable_perf)
        )),
        "normalizeRatingKey" => Ok(json!(ratings::normalize_rating_key(str_or_empty(args, 0)))),
        "ratingDisplayName" => Ok(json!(ratings::rating_display_name(str_or_empty(args, 0)))),
        "isPuzzleHistory" => Ok(json!(ratings::is_puzzle_history(str_or_empty(args, 0)))),
        "ratingDisplayNames" => Ok(Value::Object(
            kchess_domain::records::RATING_DISPLAY_NAMES
                .iter()
                .map(|(key, label)| ((*key).to_string(), json!(label)))
                .collect(),
        )),
        "ratingHistoryFromGames" => list(args, 0, "games").map(ratings::rating_history_from_games),
        "mergeRatingHistories" => list(args, 0, "official").and_then(|official| {
            list(args, 1, "fromGames")
                .map(|from_games| ratings::merge_rating_histories(official, from_games))
        }),
        "summarizeStudy" => arg(args, 0)
            .and_then(Value::as_str)
            .map(|pgn| Ok(json!(studies::summarize_study(pgn))))
            .unwrap_or_else(|| Err("pgn must be a string".into())),
        "opponent" => Ok(json!(games::opponent(str_or_empty(args, 0)))),
        "boardResult" => setup_position(arg(args, 0)).and_then(|pos| {
            let variant = arg(args, 1).and_then(Value::as_str).unwrap_or("");
            let draw = arg(args, 2)
                .filter(|draw| !js::falsy(Some(draw)))
                .map(|draw| js::to_string(Some(draw)));
            serde_json::to_value(games::board_result(&pos, variant, draw.as_deref()))
                .map_err(|e| e.to_string())
        }),
        "timeoutWinner" => setup_position(arg(args, 0))
            .map(|pos| json!(games::timeout_winner(&pos, str_or_empty(args, 1)))),
        "pgnResult" => Ok(json!(games::pgn_result(
            !js::falsy(arg(args, 0)),
            arg(args, 1).and_then(Value::as_str)
        ))),
        "isGameInProgress" => Ok(json!(games::is_game_in_progress(arg(args, 0)))),
        "archivedPlayerSide" => Ok(json!(games::archived_player_side(
            arg(args, 0).unwrap_or(&Value::Null)
        ))),
        "archivedOutcome" => Ok(json!(games::archived_outcome(
            arg(args, 0).unwrap_or(&Value::Null)
        ))),
        "totalSeconds" => Ok(json!(time_control::total_seconds(minutes(), increment()))),
        "perfFor" => Ok(json!(time_control::perf_for(minutes(), increment()))),
        "canBoardSeek" => Ok(json!(time_control::can_board_seek(minutes(), increment()))),
        "canDirectChallenge" => Ok(json!(time_control::can_direct_challenge(
            minutes(),
            increment()
        ))),
        "canPlayOnline" => Ok(json!(time_control::can_play_online(
            minutes(),
            increment(),
            !js::falsy(arg(args, 2))
        ))),
        "connectedAccountIndex" => list(args, 0, "accounts").map(|accounts| {
            let listed: Vec<ListedAccount> = accounts
                .iter()
                .map(|account| ListedAccount {
                    username: account.get("username").and_then(Value::as_str),
                    connected: !js::falsy(account.get("connected")),
                })
                .collect();
            json!(connected_account_index(
                &listed,
                arg(args, 1).and_then(Value::as_str)
            ))
        }),
        "validateOnlineEvent" => match arg(args, 0) {
            None => Ok(Value::Null),
            Some(raw) => online::validate(raw).map(|event| event.unwrap_or(Value::Null)),
        },
        "lichessError" => match arg(args, 0).and_then(|response| response.get("status")) {
            Some(status) => Ok(json!({
                "status": status,
                "endpoint": str_or_empty(args, 2),
                "detail": lichess::error_detail(status.as_f64(), arg(args, 1)),
            })),
            None => Err("response.status must be a number".into()),
        },
        "oauthLook" => Ok(oauth::look(arg(args, 0).unwrap_or(&Value::Null)).unwrap_or(Value::Null)),
        "oauthLookDefault" => Ok(oauth::default_look()),
        "oauthPage" => {
            let authorized = arg(args, 0).and_then(Value::as_bool).unwrap_or(false);
            let look = arg(args, 1).cloned().unwrap_or_else(oauth::default_look);
            Ok(Value::String(oauth::callback_page(authorized, &look)))
        }
        "analysisContext" => match arg(args, 1) {
            None => Ok(Value::String(format!("{}|", text_of(args, 0)))),
            Some(Value::Array(moves)) => Ok(Value::String(context::analysis_context(
                &text_of(args, 0),
                &texts_of(moves),
            ))),
            Some(_) => Err("moves must be a list".into()),
        },
        _ => return None,
    })
}

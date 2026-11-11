//! The `assert*` validators: each decodes an untrusted IPC
//! value into a typed struct, checks it, and returns the normalized value or a message.
//!
//! A value of the wrong type takes the message its validator names for that place (the field, or
//! the object for a missing property); without one, the decoder's own text, prefixed by the path
//! (`speeds[0]: unknown variant …`). Type errors come first, in the order the input lists its
//! properties; the checks after them run in field order.

use serde::de::{self, DeserializeOwned, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::constants::*;
use super::patterns;
use super::value::{self, J};
use crate::variant::VARIANTS;
use crate::{js, replay};

/// A custom message, as the TypeScript source wrote it.
type Msg = Option<&'static str>;

type Checked<T> = Result<T, String>;

/* ── Decoding and checks ── */

/// Decodes `value` as `T`. A rejection takes the message `messages` names for the keys of the
/// rejected value's path (`[]` for a list position), else the decoder's own text.
fn decode<T: DeserializeOwned>(value: J, messages: fn(&[&str]) -> Msg) -> Checked<T> {
    value::decode(value).map_err(|error| match messages(&error.keys()) {
        Some(message) => message.to_owned(),
        None => error.to_string(),
    })
}

/// Messages for a validator that names one message for every rejection.
macro_rules! always {
    ($msg:expr) => {
        (|_: &[&str]| Some($msg)) as fn(&[&str]) -> Msg
    };
}

/// No custom messages.
fn plain(_: &[&str]) -> Msg {
    None
}

/// The normalized value, as JSON (integral numbers as integers).
fn out<T: Serialize>(value: &T) -> Checked<Value> {
    serde_json::to_value(value)
        .map(value::integral)
        .map_err(|error| error.to_string())
}

/// `msg`, or a description of what the value at `path` should have been.
fn reject(msg: Msg, path: &str, expected: &str) -> String {
    match msg {
        Some(message) => message.to_owned(),
        None if path.is_empty() => format!("invalid value, expected {expected}"),
        None => format!("{path}: invalid value, expected {expected}"),
    }
}

fn ensure(ok: bool, msg: Msg, path: &str, expected: impl FnOnce() -> String) -> Checked<()> {
    if ok {
        Ok(())
    } else {
        Err(reject(msg, path, &expected()))
    }
}

/// At most `max` UTF-16 units.
fn max_chars(text: &str, max: usize, msg: Msg, path: &str) -> Checked<()> {
    ensure(js::utf16_len(text) <= max, msg, path, || {
        format!("at most {max} characters")
    })
}

/// At least `min` UTF-16 units.
fn min_chars(text: &str, min: usize, msg: Msg, path: &str) -> Checked<()> {
    ensure(js::utf16_len(text) >= min, msg, path, || match min {
        1 => "at least 1 character".into(),
        _ => format!("at least {min} characters"),
    })
}

fn max_items<T>(items: &[T], max: usize, msg: Msg, path: &str) -> Checked<()> {
    ensure(items.len() <= max, msg, path, || {
        format!("at most {max} items")
    })
}

fn min_items<T>(items: &[T], min: usize, msg: Msg, path: &str) -> Checked<()> {
    ensure(items.len() >= min, msg, path, || match min {
        1 => "at least 1 item".into(),
        _ => format!("at least {min} items"),
    })
}

fn within<N: PartialOrd + std::fmt::Display>(
    n: N,
    min: N,
    max: N,
    msg: Msg,
    path: &str,
) -> Checked<()> {
    let ok = n >= min && n <= max;
    ensure(ok, msg, path, || format!("a number from {min} to {max}"))
}

fn finite(n: f64, msg: Msg, path: &str) -> Checked<()> {
    ensure(n.is_finite(), msg, path, || "a finite number".into())
}

fn one_of(text: &str, options: &[&str], msg: Msg, path: &str) -> Checked<()> {
    ensure(options.contains(&text), msg, path, || {
        format!("one of {}", options.join(", "))
    })
}

fn matches(text: &str, test: fn(&str) -> bool, msg: Msg, path: &str, what: &str) -> Checked<()> {
    ensure(test(text), msg, path, || what.to_owned())
}

/// Each item checked by `check`, in order.
fn each<T>(items: &[T], check: impl Fn(&T) -> Checked<()>) -> Checked<()> {
    items.iter().try_for_each(check)
}

/* ── Shared fields ── */

const USERNAME: &str = "Enter a valid Lichess username.";
const NOTIFY: &str = "Invalid notification setting.";
const VOICE: &str = "Invalid voice entry.";
const ATTEMPT: &str = "Invalid puzzle attempt.";

/// A Lichess username, trimmed.
fn username(text: &str, path: &str) -> Checked<String> {
    let trimmed = js::trim(text);
    matches(trimmed, patterns::username, Some(USERNAME), path, "")?;
    Ok(trimmed.to_owned())
}

/// The account synced games are kept under: a Lichess username, or a Chess.com player's
/// `name@chess.com`. Trimmed.
fn game_account(text: &str, path: &str) -> Checked<String> {
    let trimmed = js::trim(text);
    matches(trimmed, patterns::game_account, Some(USERNAME), path, "")?;
    Ok(trimmed.to_owned())
}

/// A Chess.com username, trimmed.
fn chesscom_username(text: &str) -> Checked<String> {
    let trimmed = js::trim(text);
    matches(
        trimmed,
        patterns::chesscom_username,
        Some("Enter a valid Chess.com username."),
        "",
        "",
    )?;
    Ok(trimmed.to_owned())
}

fn game_id(text: &str) -> Checked<()> {
    matches(text, patterns::game_id, Some("Invalid game id."), "", "")
}

fn uci(text: &str) -> Checked<()> {
    matches(text, patterns::uci_move, Some("Invalid move."), "", "")
}

/// A move list: each a UCI move, and no more than a game holds.
fn move_list(moves: &[String]) -> Checked<()> {
    each(moves, |m| uci(m))?;
    max_items(moves, MAX_MOVES, Some("Invalid move list."), "")
}

fn angle(text: &str) -> Checked<()> {
    matches(
        text,
        patterns::puzzle_angle,
        Some("Invalid puzzle theme."),
        "",
        "",
    )
}

fn lichess_id(text: &str, msg: &'static str) -> Checked<()> {
    matches(text, patterns::lichess_id, Some(msg), "", "")
}

/// A position: at most 100 characters (`length` when longer), then the FEN shape (`msg`).
fn fen(text: &str, length: Msg, msg: Msg, path: &str) -> Checked<()> {
    max_chars(text, 100, length, path)?;
    matches(text, patterns::fen, msg, path, "a FEN position")
}

/// A position the engine can be given: Stockfish refuses more than 16 pieces for a side, and quits
/// rather than search it.
fn engine_position(text: &str) -> Checked<()> {
    if crate::position::fen_problem(text) == Some("tooManyPieces") {
        return Err("Each side can have at most 16 pieces.".into());
    }
    Ok(())
}

fn local_day(text: &str, msg: &'static str) -> Checked<()> {
    matches(text, patterns::local_day, Some(msg), "", "")
}

/// Messages of a move list, where the list itself has `list` and each move has its own.
fn move_list_messages(keys: &[&str], list: Msg) -> Msg {
    match keys {
        [] => list,
        _ => Some("Invalid move."),
    }
}

/* ── Single values, requests and library documents ── */

// Declared after the checks and shared fields above, so their macros (`always!`) reach them.
mod library;
mod requests;
mod values;

use library::*;
use requests::*;
use values::*;

pub use requests::{
    AnalysisRequest, BestMoveOptions, GamePageQuery, assert_analysis_request,
    assert_best_move_options, assert_game_page_query,
};
pub use values::{assert_moves, assert_username};

/* ── One check per argument ── */

macro_rules! checks {
    ($($name:ident => $check:path,)*) => {
        /// One function per validator, named for what it checks: the normalized value of one
        /// argument (`J::Undef` when it is absent), or the message it was refused with. The core's
        /// method table (`kchess_contracts::core_api`) names one per argument.
        pub mod checks {
            use super::*;

            $(pub fn $name(value: J) -> Checked<Value> {
                out(&$check(value)?)
            })*

            /// A position alone (`assertAnalysisRequest({ fen, lines: 1 })`): its FEN.
            pub fn position(value: J) -> Checked<Value> {
                position_of(value, None)
            }

            /// A position of `variant` (standard chess without one): its FEN.
            fn position_of(fen: J, variant: Option<&str>) -> Checked<Value> {
                let mut fields = vec![("fen".into(), fen), ("lines".into(), J::Num(1.0))];
                if let Some(variant) = variant {
                    fields.push(("variant".into(), J::Str(variant.into())));
                }
                Ok(Value::String(assert_analysis_request(J::Obj(fields))?.fen))
            }

            /// `reviewGet`'s variant, when it names one.
            fn review_variant_of(args: &[J]) -> Option<&str> {
                args.get(2).and_then(J::str)
            }

            /// The review request `reviewGet(fen, moves, variant)` makes, checked.
            fn review_of(args: &[J], variant: Option<&str>) -> Checked<()> {
                let at = |i: usize| args.get(i).cloned().unwrap_or(J::Undef);
                let mut fields = vec![("fen".into(), at(0)), ("moves".into(), at(1))];
                if let Some(variant) = variant {
                    fields.push(("variant".into(), J::Str(variant.into())));
                }
                assert_review_request(J::Obj(fields)).map(|_| ())
            }

            /// `reviewGet`'s position: a variant game's is checked with its moves by the
            /// variant's rules. As sent.
            pub fn review_fen(value: J, args: &[J]) -> Checked<Value> {
                match review_variant_of(args) {
                    Some(variant) => review_of(args, Some(variant))?,
                    None => {
                        position_of(value.clone(), None)?;
                    }
                }
                Ok(value.into_value())
            }

            /// `reviewGet`'s moves (checked with the position for a variant game). As sent.
            pub fn review_moves(value: J, args: &[J]) -> Checked<Value> {
                if review_variant_of(args).is_none() {
                    assert_moves(value.clone())?;
                }
                Ok(value.into_value())
            }

            /// `reviewGet`'s variant: anything; without one, the position and moves must make a
            /// review of standard chess. As sent.
            pub fn review_variant(value: J, args: &[J]) -> Checked<Value> {
                if review_variant_of(args).is_none() {
                    review_of(args, None)?;
                }
                Ok(value.into_value())
            }

            /// A played game to save: an object (`assertArchivedGameShape`).
            pub fn archived_game_shape(value: J) -> Checked<Value> {
                crate::misc::documents::assert_archived_game_shape(vec![value])
                    .map(J::into_value)
            }

            /// Anything: the method reads it itself.
            pub fn any(value: J) -> Checked<Value> {
                Ok(value.into_value())
            }
        }
    };
}

checks! {
    username => assert_username,
    game_id => assert_game_id,
    game_account => assert_game_account,
    chesscom_username => assert_chesscom_username,
    game_ids => assert_game_ids,
    moves => assert_moves,
    level => assert_level,
    friend_list => assert_friend_list,
    relation_action => assert_relation_action,
    message_text => assert_message_text,
    insights_query => assert_insights_query,
    perf_type => assert_perf_type,
    variant => assert_variant,
    lichess_id => assert_lichess_id,
    puzzle_id => assert_puzzle_id,
    optional_account => assert_optional_account,
    theme => assert_theme,
    core_settings => assert_core_settings,
    settings => assert_settings,
    puzzle_request => assert_puzzle_request,
    puzzle_solve => assert_puzzle_solve,
    days => assert_days,
    activity_max => assert_activity_max,
    local_query => assert_local_query,
    ladder_query => assert_ladder_query,
    run_kind => assert_run_kind,
    run_input => assert_run_input,
    puzzle_attempt => assert_puzzle_attempt,
    puzzle_progress_query => assert_puzzle_progress_query,
    best_move_options => assert_best_move_options,
    analysis_request => assert_analysis_request,
    review_request => assert_review_request,
    review_key => assert_review_key,
    ai_model => assert_ai_model,
    ai_effort => assert_ai_effort,
    ai_request => assert_ai_request,
    voice_attempt => assert_voice_attempt,
    voice_update => assert_voice_update,
    voice_id => assert_voice_id,
    voice_limit => assert_voice_limit,
    game_page_query => assert_game_page_query,
    study_sync_request => assert_study_sync_request,
    lines => assert_lines,
    boolean => assert_boolean,
    chapter_name => assert_chapter_name,
    pgn_text => assert_pgn_text,
    session_object => assert_session_object,
    study_id => assert_study_id,
    study_command_shape => assert_study_command_shape,
    library_id => assert_library_id,
    repertoire_key => assert_repertoire_key,
    repertoire_line => assert_repertoire_line,
    view_preference_key => assert_view_preference_key,
    session_kind => assert_session_kind,
    side => assert_side,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    fn restore_chapter(pgn: &str) -> J {
        let chapter = obj(vec![
            ("id", J::Str("c1".into())),
            ("name", J::Str("Rook endings".into())),
            ("pgn", J::Str(pgn.into())),
        ]);
        obj(vec![
            ("op", J::Str("restoreChapter".into())),
            ("id", J::Str("s1".into())),
            ("chapter", chapter),
            ("index", J::Num(1.0)),
            ("extra", J::Str("dropped".into())),
        ])
    }

    /// Undo's `restoreChapter` takes the chapter whole and its place (the core reads its PGN).
    #[test]
    fn a_chapter_to_restore_is_checked() {
        assert_eq!(
            checks::study_command_shape(restore_chapter("1. e4 *")),
            Ok(json!({
                "op": "restoreChapter",
                "id": "s1",
                "chapter": { "id": "c1", "name": "Rook endings", "pgn": "1. e4 *" },
                "index": 1,
            }))
        );
        let without_chapter = obj(vec![
            ("op", J::Str("restoreChapter".into())),
            ("id", J::Str("s1".into())),
            ("index", J::Num(0.0)),
        ]);
        assert!(checks::study_command_shape(without_chapter).is_err());
    }

    /// `moveChapter` takes a chapter and its new place.
    #[test]
    fn a_chapter_to_move_is_checked() {
        let moved = obj(vec![
            ("op", J::Str("moveChapter".into())),
            ("id", J::Str("s1".into())),
            ("chapterId", J::Str("c1".into())),
            ("index", J::Num(2.0)),
            ("extra", J::Str("dropped".into())),
        ]);
        assert_eq!(
            checks::study_command_shape(moved),
            Ok(json!({ "op": "moveChapter", "id": "s1", "chapterId": "c1", "index": 2 }))
        );
        let without_place = obj(vec![
            ("op", J::Str("moveChapter".into())),
            ("id", J::Str("s1".into())),
            ("chapterId", J::Str("c1".into())),
        ]);
        assert!(checks::study_command_shape(without_place).is_err());
    }

    fn settings() -> Vec<(&'static str, J)> {
        let mut fields = vec![
            ("appearance", J::Str("system".into())),
            ("boardTheme", J::Str("classic".into())),
            ("lightTheme", J::Str("light".into())),
            ("darkTheme", J::Str("dark".into())),
            ("pieceSet", J::Str("cburnett".into())),
            ("pieceAnimation", J::Str("normal".into())),
            ("coordinates", J::Str("inside".into())),
            ("soundVolume", J::Num(7.0)),
            ("enginePath", J::Str(String::new())),
            ("promotion", J::Str("ask".into())),
            (
                "engineLevels",
                J::Arr(vec![J::Str("gm".into()), J::Str("club".into())]),
            ),
            ("reviewAuto", J::Str("off".into())),
            ("correspondencePoll", J::Num(30.0)),
        ];
        for flag in [
            "soundEnabled",
            "premove",
            "showLegalMoves",
            "notificationsEnabled",
            "notifyActive",
            "notifyBackground",
            "notifyOpponentMove",
            "notifyLowTime",
            "notifyGameEvents",
            "notifyComputerMove",
            "notifySound",
            "voicePushToTalk",
            "voiceConfirmMoves",
            "voiceHistory",
            "updateAutoCheck",
            "updateAutoDownload",
            "updateInstallOnQuit",
            "reviewOnBattery",
            "receiveChallenges",
            "notifyChallenges",
            "onlineChat",
            "zenMode",
            "blindfold",
            "cloudEval",
            "showOpeningName",
            "swipeNavigation",
            "swipeIndicator",
            "puzzleAutoNext",
            "confirmCorrespondence",
            "figurineNotation",
        ] {
            fields.push((flag, J::Bool(true)));
        }
        fields
    }

    #[test]
    fn settings_normalize_in_the_renderers_order() {
        let normalized = checks::settings(obj(settings())).unwrap();
        assert_eq!(normalized["soundVolume"], json!(1));
        assert_eq!(normalized["engineLevels"], json!(["club", "gm"]));
        let keys: Vec<&String> = normalized.as_object().unwrap().keys().collect();
        assert_eq!(keys[..3], ["appearance", "boardTheme", "lightTheme"]);
    }

    /// A frontend from before move confirmation sends no `confirmCorrespondence`: its settings
    /// still save, with confirmation on.
    #[test]
    fn settings_without_move_confirmation_keep_it_on() {
        let older: Vec<_> = settings()
            .into_iter()
            .filter(|(key, _)| *key != "confirmCorrespondence")
            .collect();
        let normalized = checks::settings(obj(older)).unwrap();
        assert_eq!(normalized["confirmCorrespondence"], json!(true));
        let mut off = settings();
        off.retain(|(key, _)| *key != "confirmCorrespondence");
        off.push(("confirmCorrespondence", J::Bool(false)));
        let normalized = checks::settings(obj(off)).unwrap();
        assert_eq!(normalized["confirmCorrespondence"], json!(false));
        let mut wrong = settings();
        wrong.retain(|(key, _)| *key != "confirmCorrespondence");
        wrong.push(("confirmCorrespondence", J::Num(1.0)));
        assert_eq!(
            checks::settings(obj(wrong)).unwrap_err(),
            "Invalid move confirmation setting."
        );
    }

    #[test]
    fn each_setting_keeps_its_message() {
        for (key, value, message) in [
            ("notificationsEnabled", J::Num(1.0), NOTIFY),
            ("notifyLowTime", J::Null, NOTIFY),
            (
                "engineLevels",
                J::Arr(vec![J::Num(1.0)]),
                "Invalid engine level.",
            ),
            (
                "engineLevels",
                J::Arr(vec![]),
                "Keep at least one computer level.",
            ),
            ("soundVolume", J::Num(f64::NAN), "Invalid sound volume."),
            (
                "soundVolume",
                J::Num(f64::INFINITY),
                "Invalid sound volume.",
            ),
            (
                "correspondencePoll",
                J::Num(1.5),
                "Invalid correspondence check interval.",
            ),
            ("lightTheme", J::Str("Bad".into()), "Invalid color theme."),
        ] {
            let mut fields = settings();
            fields.iter_mut().find(|(k, _)| *k == key).unwrap().1 = value;
            assert_eq!(checks::settings(obj(fields)), Err(message.into()), "{key}");
        }
        let mut missing = settings();
        missing.retain(|(k, _)| *k != "zenMode");
        assert_eq!(
            checks::settings(obj(missing)),
            Err("Invalid settings.".into())
        );
    }

    #[test]
    fn nested_values_take_their_own_message() {
        let palette = obj(vec![
            ("bg", J::Num(1.0)),
            ("text", J::Str("#fff".into())),
            ("primary", J::Str("#000".into())),
        ]);
        let theme = obj(vec![
            ("id", J::Str("night".into())),
            ("name", J::Str("Night".into())),
            ("dark", palette),
        ]);
        assert_eq!(checks::theme(theme), Err(HEX.into()));
        let moves = obj(vec![
            ("fen", J::Str("x".into())),
            ("moves", J::Arr(vec![J::Num(1.0)])),
        ]);
        assert_eq!(checks::review_request(moves), Err("Invalid move.".into()));
    }

    #[test]
    fn the_engine_is_not_given_positions_stockfish_refuses() {
        // Black has a seventeenth piece: Stockfish quit on it ("More than 32 pieces").
        let crowded = "rnbqkbnr/pppppppp/8/8/3q3N/4K3/1PPPPPPP/RNBQ1BNR w kq - 0 1";
        let refused = Err("Each side can have at most 16 pieces.".into());
        let analysis = obj(vec![
            ("fen", J::Str(crowded.into())),
            ("lines", J::Num(3.0)),
        ]);
        assert_eq!(checks::analysis_request(analysis), refused);
        let search = obj(vec![("fen", J::Str(crowded.into()))]);
        assert_eq!(checks::best_move_options(search), refused);
        // Horde's thirty-six pawns are Horde's: Fairy-Stockfish plays them.
        let horde = "rnbqkbnr/pppppppp/8/1PP2PP1/PPPPPPPP/PPPPPPPP/PPPPPPPP/PPPPPPPP w kq - 0 1";
        let search = obj(vec![
            ("fen", J::Str(horde.into())),
            ("variant", J::Str("horde".into())),
        ]);
        assert_eq!(
            checks::best_move_options(search),
            Ok(json!({ "fen": horde, "variant": "horde" }))
        );
        let search = obj(vec![
            ("fen", J::Str(horde.into())),
            ("variant", J::Str("crazyhouse".into())),
        ]);
        assert_eq!(
            checks::best_move_options(search),
            Err("Invalid position.".into())
        );
        let search = obj(vec![("variant", J::Str("bughouse".into()))]);
        assert_eq!(
            checks::best_move_options(search),
            Err("Invalid variant.".into())
        );
        let review = obj(vec![
            ("fen", J::Str(crowded.into())),
            ("moves", J::Arr(vec![])),
        ]);
        assert_eq!(checks::review_request(review), refused);
    }

    #[test]
    fn lists_are_objects_of_their_indices() {
        assert_eq!(checks::best_move_options(J::Arr(vec![])), Ok(json!({})));
        assert_eq!(checks::best_move_options(J::Undef), Ok(json!({})));
    }

    #[test]
    fn run_details_keep_their_order_and_integers() {
        let run_input = obj(vec![
            ("kind", J::Str("storm".into())),
            ("variant", J::Str(String::new())),
            ("score", J::Num(12.0)),
            (
                "detail",
                obj(vec![("z", J::Num(1.0)), ("a", J::Str("x".into()))]),
            ),
        ]);
        let normalized = checks::run_input(run_input).unwrap();
        assert_eq!(normalized["score"], json!(12));
        assert_eq!(
            serde_json::to_string(&normalized["detail"]).unwrap(),
            r#"{"z":1,"a":"x"}"#
        );
    }
}

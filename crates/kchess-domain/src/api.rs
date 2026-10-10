//! One entry point for every host: `call(method, args)` with a JSON array of arguments and a
//! JSON result. The Node module and the renderer's WebAssembly module both expose it, and
//! `crates/kchess-wasm/js/engine.ts` gives it typed TypeScript wrappers.

use serde_json::{Map, Value, json};
use shakmaty::Position;
use std::collections::HashMap;

use crate::pgn::{Game, PgnNode, make_pgn, study_document_pgn};
use crate::position::{self, DestMode};
use crate::replay::{replay_positions, replay_setup};
use crate::review::analyse_review;
use crate::rules;
use crate::tree::tree_from_pgn;

const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

type Result<T> = std::result::Result<T, String>;

fn arg(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i).filter(|v| !v.is_null())
}

fn text<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a str> {
    arg(args, i)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn texts<'a>(args: &'a [Value], i: usize, name: &str) -> Result<Vec<&'a str>> {
    arg(args, i)
        .and_then(Value::as_array)
        .and_then(|items| items.iter().map(Value::as_str).collect())
        .ok_or_else(|| format!("{name} must be a list of strings"))
}

/// `{ variant, fen }`
fn setup(args: &[Value], i: usize) -> Result<(&str, &str)> {
    let setup = arg(args, i)
        .and_then(Value::as_object)
        .ok_or("setup must be an object")?;
    let field = |key| setup.get(key).and_then(Value::as_str);
    Ok((
        field("variant").ok_or("setup.variant")?,
        field("fen").ok_or("setup.fen")?,
    ))
}

/// The legal position a `{ variant, fen }` setup describes; an error when it is not one.
fn setup_position(args: &[Value]) -> Result<shakmaty::variant::VariantPosition> {
    let (variant, fen) = setup(args, 0)?;
    rules::setup_start(variant, fen).ok_or_else(|| "setup is not a legal position".into())
}

/// Dispatch one call. Unknown methods and malformed arguments are errors.
pub fn call(method: &str, args: &str) -> Result<String> {
    // Analysis trees nest deeper than serde_json's default limit of 128 levels.
    let mut reader = serde_json::Deserializer::from_str(args);
    reader.disable_recursion_limit();
    let args: Vec<Value> =
        serde::Deserialize::deserialize(&mut reader).map_err(|e| e.to_string())?;
    let result = match method {
        "treeFromPgn" => to_value(tree_from_pgn(text(&args, 0, "pgn")?))?,
        "treeToPgn" => Value::String(tree_to_pgn(arg(&args, 0).ok_or("root")?, arg(&args, 1))?),
        "replay" => to_value(replay_positions(
            text(&args, 0, "fen")?,
            &texts(&args, 1, "moves")?,
        ))?,
        "setupReplay" => {
            let (variant, fen) = setup(&args, 0)?;
            to_value(replay_setup(variant, fen, &texts(&args, 1, "moves")?))?
        }
        "analyseReview" => analyse_review(arg(&args, 0).ok_or("review")?).unwrap_or(Value::Null),
        "studyDocumentPgn" => {
            let chapters = arg(&args, 0).and_then(Value::as_array).ok_or("chapters")?;
            let pairs: Vec<(&str, &str)> = chapters
                .iter()
                .map(|c| Some((c.get("name")?.as_str()?, c.get("pgn")?.as_str()?)))
                .collect::<Option<_>>()
                .ok_or("chapters need a name and a pgn")?;
            study_document_pgn(&pairs).map_or(Value::Null, Value::String)
        }
        "pvSan" => {
            let max = arg(&args, 2).and_then(Value::as_u64).unwrap_or(12) as usize;
            pv_san(text(&args, 0, "fen")?, &texts(&args, 1, "pv")?, max)
        }
        "playMove" => play_move(text(&args, 0, "fen")?, text(&args, 1, "uci")?),
        "startNode" => start_node(text(&args, 0, "fen")?),
        "sanHistory" => to_value(san_history(&texts(&args, 0, "moves")?))?,
        "pgnFromSan" => Value::String(pgn_from_san(&texts(&args, 0, "sans")?)),
        "pgnFromUci" => Value::String(pgn_from_san(&san_history(&texts(&args, 0, "moves")?))),
        "pgnFromMoves" => Value::String(pgn_from_moves(&texts(&args, 0, "tokens")?)),
        "setupPgn" => {
            let (variant, fen) = setup(&args, 0)?;
            let headers = arg(&args, 2).and_then(Value::as_object);
            Value::String(setup_pgn(
                variant,
                fen,
                &texts(&args, 1, "moves")?,
                headers,
            )?)
        }
        "drawReason" => to_value(draw_reason(&texts(&args, 0, "moves")?))?,
        "setupDrawReason" => {
            let (variant, fen) = setup(&args, 0)?;
            to_value(setup_draw_reason(variant, fen, &texts(&args, 1, "moves")?))?
        }
        "position" => {
            let (variant, fen) = setup(&args, 0)?;
            to_value(rules::setup_start(variant, fen).map(|pos| position::info(&pos)))?
        }
        "dests" => {
            let pos = setup_position(&args)?;
            let mode = match arg(&args, 1).and_then(Value::as_str) {
                Some("board") => DestMode::Board,
                Some("board960") => DestMode::Board960,
                _ => DestMode::Rules,
            };
            Value::Array(
                position::dests(&pos, mode)
                    .into_iter()
                    .map(|(from, to)| {
                        json!([
                            from.to_string(),
                            to.iter().map(ToString::to_string).collect::<Vec<_>>()
                        ])
                    })
                    .collect(),
            )
        }
        "legalMoves" => to_value(position::legal_moves(&setup_position(&args)?))?,
        "play" => to_value(position::play_uci(
            &setup_position(&args)?,
            text(&args, 1, "uci")?,
        ))?,
        "playSan" => to_value(position::play_san(
            &setup_position(&args)?,
            text(&args, 1, "san")?,
        ))?,
        "line" => {
            let (variant, fen) = setup(&args, 0)?;
            let moves = texts(&args, 1, "moves")?;
            let options = arg(&args, 2);
            let flag = |key| {
                options
                    .and_then(|o| o.get(key))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            };
            match rules::setup_start(variant, fen) {
                None => Value::Null,
                Some(start) => {
                    let start_fen = rules::make_fen(&start);
                    let (_, played) = position::line(start, &moves, flag("san"), flag("trim"));
                    json!({ "start": start_fen, "moves": to_value(played)? })
                }
            }
        }
        "defaultFen" => {
            let variant = rules::lichess_variant(text(&args, 0, "variant")?).ok_or("variant")?;
            Value::String(rules::make_fen(&rules::default_position(variant)))
        }
        "fenSetup" => position::fen_setup(text(&args, 0, "fen")?).unwrap_or(Value::Null),
        "fenProblem" => to_value(position::fen_problem(text(&args, 0, "fen")?))?,
        "pgnGames" => position::pgn_games(text(&args, 0, "pgn")?),
        "pgnMainline" => {
            let limit = arg(&args, 1).and_then(Value::as_u64).unwrap_or(u64::MAX);
            position::pgn_mainline(text(&args, 0, "pgn")?, limit as usize)
        }
        _ => {
            // Each group of moved rules dispatches its own methods.
            let moved = [
                crate::voice::call,
                crate::training::call,
                crate::records::call,
                crate::misc::call,
                crate::games::call,
                crate::online_game::call,
                crate::trainer::call,
            ];
            return match moved.iter().find_map(|call| call(method, &args)) {
                Some(result) => serde_json::to_string(&result?).map_err(|e| e.to_string()),
                None => Err(format!("unknown rules method {method}")),
            };
        }
    };
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/* ── Analysis tree ── */

fn ply_of<P: Position>(pos: &P) -> u32 {
    (pos.fullmoves().get() - 1) * 2 + u32::from(!pos.turn().is_white())
}

fn standard_or_default(fen: &str) -> shakmaty::variant::VariantPosition {
    rules::standard_from_fen(fen)
        .unwrap_or_else(|| rules::default_position(shakmaty::variant::Variant::Chess))
}

/// `newTree(fen)`: the root's normalized FEN and ply (the usual start when the FEN is unusable).
fn start_node(fen: &str) -> Value {
    let pos = standard_or_default(fen);
    json!({ "fen": rules::make_fen(&pos), "ply": ply_of(&pos) })
}

/// `addMove`'s rules: `uci` played from `fen` (standard rules), or null when illegal.
fn play_move(fen: &str, uci: &str) -> Value {
    let Some(mut pos) = rules::standard_from_fen(fen) else {
        return Value::Null;
    };
    let Some(m) = rules::parse_uci(uci).and_then(|u| rules::legal_move(&pos, u)) else {
        return Value::Null;
    };
    let uci = rules::make_uci(&m);
    let san = rules::san_and_play(&mut pos, m);
    json!({ "uci": uci, "san": san, "fen": rules::make_fen(&pos) })
}

/// `moveNumber(ply, always)`
fn move_number(ply: u32, always: bool) -> String {
    let number = ply / 2 + 1;
    if ply.is_multiple_of(2) {
        format!("{number}.")
    } else if always {
        format!("{number}…")
    } else {
        String::new()
    }
}

/// `pvSan`: a principal variation in SAN with move numbers, stopping at the first illegal move.
fn pv_san(fen: &str, pv: &[&str], max: usize) -> Value {
    let Some(mut pos) = rules::standard_from_fen(fen) else {
        return json!([]);
    };
    let mut ply = ply_of(&pos);
    let mut moves = Vec::new();
    for &uci in pv.iter().take(max) {
        let Some(m) = rules::parse_uci(uci).and_then(|u| rules::legal_move(&pos, u)) else {
            break;
        };
        let number = move_number(ply, moves.is_empty());
        let san = rules::san_and_play(&mut pos, m);
        let label = if number.is_empty() {
            san.clone()
        } else {
            format!("{number} {san}")
        };
        moves.push(json!({ "san": san, "uci": uci, "label": label, "fen": rules::make_fen(&pos) }));
        ply += 1;
    }
    Value::Array(moves)
}

fn strings(value: Option<&Value>) -> Result<Option<Vec<String>>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|c| {
                c.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "comments must be strings".into())
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => Err("comments must be a list".into()),
    }
}

fn nags(value: Option<&Value>) -> Result<Option<Vec<u32>>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|n| {
                n.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| "nags must be whole numbers".into())
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => Err("nags must be a list".into()),
    }
}

/// `treeToPgn(root, headers)`: the analysis tree as one PGN document.
fn tree_to_pgn(root: &Value, headers: Option<&Value>) -> Result<String> {
    let mut game = Game::empty();
    if let Some(headers) = headers.and_then(Value::as_object) {
        for (key, value) in headers {
            let value = value.as_str().ok_or("header values must be strings")?;
            game.put_header(key, value.to_string());
        }
    }
    game.comments = strings(root.get("comments"))?;
    let fen = root.get("fen").and_then(Value::as_str).ok_or("root.fen")?;
    if fen != INITIAL_FEN
        && let Some(pos) = rules::standard_from_fen(fen)
    {
        // setStartingPosition: standard rules, so no Variant header; FEN unless the usual start.
        game.remove_header("Variant");
        let start = rules::make_fen(&pos);
        if start == INITIAL_FEN {
            game.remove_header("FEN");
        } else {
            game.put_header("FEN", start);
        }
    }
    let mut stack: Vec<(&Value, usize)> = vec![(root, 0)];
    while let Some((node, index)) = stack.pop() {
        let children = node
            .get("children")
            .and_then(Value::as_array)
            .ok_or("children")?;
        let mut added = Vec::with_capacity(children.len());
        for child in children {
            let san = child.get("san").and_then(Value::as_str).ok_or("san")?;
            let move_node = PgnNode::with(
                san,
                strings(child.get("comments"))?,
                strings(child.get("startingComments"))?,
                nags(child.get("nags"))?,
            );
            added.push((child, game.add_move(index, move_node)));
        }
        // Children are written in order; their subtrees are independent of each other.
        stack.extend(added.into_iter().rev());
    }
    Ok(make_pgn(&game))
}

/* ── Move lists and PGN ── */

/// `sanHistory`: SAN of UCI moves from the standard start, as far as they are legal.
fn san_history(moves: &[&str]) -> Vec<String> {
    replay_setup("standard", INITIAL_FEN, moves)
        .map(|r| r.played.into_iter().map(|m| m.san).collect())
        .unwrap_or_default()
}

/// `pgnFromSan`: SAN tokens as a PGN with default headers (not validated).
fn pgn_from_san<S: AsRef<str>>(sans: &[S]) -> String {
    let mut game = Game::empty();
    let mut parent = 0;
    for san in sans {
        parent = game.add_move(parent, PgnNode::with(san.as_ref(), None, None, None));
    }
    make_pgn(&game)
}

fn is_result(token: &str) -> bool {
    matches!(token, "1-0" | "0-1" | "1/2-1/2" | "*")
}

fn is_uci(token: &str) -> bool {
    let b = token.as_bytes();
    (b.len() == 4 || b.len() == 5)
        && (b'a'..=b'h').contains(&b[0])
        && (b'1'..=b'8').contains(&b[1])
        && (b'a'..=b'h').contains(&b[2])
        && (b'1'..=b'8').contains(&b[3])
        && (b.len() == 4 || b"qrbn".contains(&b[4]))
}

/// `pgnFromMoves`: UCI or SAN tokens (results left out) as a PGN.
fn pgn_from_moves(tokens: &[&str]) -> String {
    let cleaned: Vec<&str> = tokens
        .iter()
        .map(|t| crate::js::trim(t))
        .filter(|t| !t.is_empty() && !is_result(t))
        .collect();
    if !cleaned.is_empty() && cleaned.iter().all(|t| is_uci(&t.to_lowercase())) {
        pgn_from_san(&san_history(&cleaned))
    } else {
        pgn_from_san(&cleaned)
    }
}

/// `setupPgn`: moves from a setup, with the headers another program needs to replay them.
fn setup_pgn(
    variant: &str,
    fen: &str,
    moves: &[&str],
    headers: Option<&Map<String, Value>>,
) -> Result<String> {
    let mut game = Game::empty();
    for (key, value) in headers.into_iter().flatten() {
        game.put_header(
            key,
            value
                .as_str()
                .ok_or("header values must be strings")?
                .to_string(),
        );
    }
    if variant != "standard" {
        let name = if variant == "chess960" {
            "Chess960"
        } else {
            variant
        };
        game.put_header("Variant", name.to_string());
    }
    if fen != INITIAL_FEN || variant == "chess960" {
        game.put_header("SetUp", "1".into());
        game.put_header("FEN", fen.to_string());
    }
    let sans: Vec<String> = replay_setup(variant, fen, moves)
        .map(|r| r.played.into_iter().map(|m| m.san).collect())
        .unwrap_or_default();
    let mut parent = 0;
    for san in &sans {
        parent = game.add_move(parent, PgnNode::with(san, None, None, None));
    }
    Ok(make_pgn(&game))
}

/* ── Draws ── */

/// The first four FEN fields: what makes positions the same for repetition.
fn repetition_key(pos: &shakmaty::variant::VariantPosition) -> String {
    rules::make_fen(pos)
        .split(' ')
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
}

fn ended(pos: &shakmaty::variant::VariantPosition) -> bool {
    pos.is_variant_end() || pos.is_insufficient_material() || pos.legal_moves().is_empty()
}

/// Repetition counting over `moves`; `parse` reads each move as the TypeScript function did.
fn repetition(
    mut pos: shakmaty::variant::VariantPosition,
    moves: &[&str],
    trim: bool,
) -> Option<&'static str> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    seen.insert(repetition_key(&pos), 1);
    for &uci in moves {
        let uci = if trim { crate::js::trim(uci) } else { uci };
        let Some(m) = rules::parse_uci(uci).and_then(|u| rules::legal_move(&pos, u)) else {
            break;
        };
        pos.play_unchecked(m);
        let count = seen.entry(repetition_key(&pos)).or_insert(0);
        *count += 1;
        if *count >= 3 {
            return Some("Threefold repetition");
        }
    }
    (!ended(&pos) && pos.halfmoves() >= 100).then_some("Fifty-move rule")
}

/// `drawReason`: repetition or the fifty-move rule, from the standard start.
fn draw_reason(moves: &[&str]) -> Option<&'static str> {
    repetition(
        rules::default_position(shakmaty::variant::Variant::Chess),
        moves,
        true,
    )
}

/// `setupDrawReason`: the same from a setup, for variants where the rules apply.
fn setup_draw_reason(variant: &str, fen: &str, moves: &[&str]) -> Option<&'static str> {
    let replayed = replay_setup(variant, fen, moves)?;
    if ![
        "standard",
        "chess960",
        "kingOfTheHill",
        "threeCheck",
        "atomic",
    ]
    .contains(&variant)
    {
        return None;
    }
    let start = rules::setup_start(variant, fen)?;
    // The TypeScript rules re-read each move as given, so a padded move ends the count there.
    let played: Vec<&str> = replayed.played.iter().map(|m| m.uci.as_str()).collect();
    repetition(start, &played, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_by_name() {
        let tree = call("treeFromPgn", r#"["1. e4 *"]"#).unwrap();
        assert!(tree.contains("\"uci\":\"e2e4\""));
        assert_eq!(
            call(
                "drawReason",
                r#"[["g1f3","g8f6","f3g1","f6g8","g1f3","g8f6","f3g1","f6g8"]]"#
            )
            .unwrap(),
            "\"Threefold repetition\""
        );
        assert!(call("nope", "[]").is_err());
        assert!(call("replay", "[1]").is_err());
    }
}

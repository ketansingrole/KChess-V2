//! One entry point for every host: `call(method, args)` with a JSON array of arguments and a
//! JSON result. Every host binds this one function.

use serde_json::{Map, Value, json};

use crate::error::{DomainError, Result};
use crate::pgn::{Game, PgnNode, make_pgn, study_document_pgn};
use crate::position::{self, DestMode};
use crate::replay::{replay_positions, replay_setup};
use crate::review::analyse_review;
use crate::rules::{self, INITIAL_FEN};
use crate::tree::tree_from_pgn;

/// The deepest argument nesting `call` parses. The largest analysis tree (1024 plies, see
/// `tree.rs`) nests 2051 levels. Parsing, walking and dropping it recurse, and an overflowed
/// stack aborts the host process, which no panic guard can catch.
const MAX_JSON_DEPTH: usize = 4096;
/// Deeper arguments, and methods that build a tree, are handled on a thread with `DEEP_STACK`
/// bytes of stack: through `Value`, the largest tree overflows a 2 MiB thread (tokio's and Rust's
/// default) in a debug build.
const DEEP_JSON: usize = 256;
const TREE_METHODS: &[&str] = &["treeFromPgn"];
const DEEP_STACK: usize = 32 * 1024 * 1024;

/// How deeply `json` nests arrays and objects, counting no further than one past
/// `MAX_JSON_DEPTH`. Brackets inside strings do not count; malformed input is left to the parser.
fn nesting(json: &str) -> usize {
    let (mut depth, mut deepest, mut in_string, mut escaped) = (0usize, 0usize, false, false);
    for byte in json.bytes() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
                if deepest > MAX_JSON_DEPTH {
                    break;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

fn arg(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i).filter(|v| !v.is_null())
}

fn text<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a str> {
    arg(args, i)
        .and_then(Value::as_str)
        .ok_or_else(|| DomainError::Message(format!("{name} must be a string")))
}

fn texts<'a>(args: &'a [Value], i: usize, name: &str) -> Result<Vec<&'a str>> {
    arg(args, i)
        .and_then(Value::as_array)
        .and_then(|items| items.iter().map(Value::as_str).collect())
        .ok_or_else(|| DomainError::Message(format!("{name} must be a list of strings")))
}

/// `{ variant, fen }`
fn setup(args: &[Value], i: usize) -> Result<(&str, &str)> {
    let setup = arg(args, i)
        .and_then(Value::as_object)
        .ok_or_else(|| DomainError::Message("setup must be an object".to_string()))?;
    let field = |key| setup.get(key).and_then(Value::as_str);
    Ok((
        field("variant").ok_or_else(|| DomainError::Message("setup.variant".to_string()))?,
        field("fen").ok_or_else(|| DomainError::Message("setup.fen".to_string()))?,
    ))
}

/// The legal position a `{ variant, fen }` setup describes; an error when it is not one.
fn setup_position(args: &[Value]) -> Result<shakmaty::variant::VariantPosition> {
    let (variant, fen) = setup(args, 0)?;
    rules::setup_start(variant, fen)
        .ok_or_else(|| DomainError::Message("setup is not a legal position".to_string()))
}

/// Answer one call, its arguments a JSON array. Unknown methods and malformed arguments are
/// errors.
pub fn call(method: &str, args: &str) -> std::result::Result<String, String> {
    let depth = nesting(args);
    if depth > MAX_JSON_DEPTH {
        return Err("arguments nest too deeply".to_string());
    }
    if goes_deep(method, depth) {
        return call_deep(method, args);
    }
    call_inner(method, args).map_err(|e| e.to_string())
}

/// Whether `call` answers on the large stack: arguments `depth` deep past `DEEP_JSON`, and the
/// methods that build deep trees.
fn goes_deep(method: &str, depth: usize) -> bool {
    depth > DEEP_JSON || TREE_METHODS.contains(&method)
}

/// `call_inner` on a thread whose stack holds `MAX_JSON_DEPTH` levels, whatever thread asked.
fn call_deep(method: &str, args: &str) -> std::result::Result<String, String> {
    on_large_stack(|| call_inner(method, args).map_err(|e| e.to_string()))
}

/// `work` on a thread whose stack holds `MAX_JSON_DEPTH` levels, whatever thread asked. Callers
/// that read a deep answer (a game tree) do so here too: parsing, walking and dropping it recurse.
pub fn on_large_stack<T: Send>(
    work: impl FnOnce() -> std::result::Result<T, String> + Send,
) -> std::result::Result<T, String> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("kchess-deep-call".into())
            .stack_size(DEEP_STACK)
            .spawn_scoped(scope, work)
            .map_err(|error| format!("could not start a thread: {error}"))?
            .join()
            .map_err(|_| "internal error".to_string())?
    })
}

fn call_inner(method: &str, args: &str) -> Result<String> {
    // Analysis trees nest deeper than serde_json's default limit of 128 levels; `call` bounds
    // the depth and gives deep arguments a large stack instead.
    let mut reader = serde_json::Deserializer::from_str(args);
    reader.disable_recursion_limit();
    let args: Vec<Value> = serde::Deserialize::deserialize(&mut reader)?;
    // One JSON array, nothing after it.
    reader.end()?;
    Ok(serde_json::to_string(&dispatch(method, &args)?)?)
}

/// `call` for arguments a Rust caller already holds as values: the same rules and answers,
/// without writing the arguments out as JSON and reading the answer back.
pub fn call_value(method: &str, args: &[Value]) -> std::result::Result<Value, String> {
    if deeper_than(args, MAX_JSON_DEPTH) {
        return Err("arguments nest too deeply".to_string());
    }
    if needs_large_stack(method, args) {
        return on_large_stack(|| dispatch(method, args).map_err(|e| e.to_string()));
    }
    dispatch(method, args).map_err(|e| e.to_string())
}

/// Whether `call_value` answers this call on a thread of its own with a large stack: deep
/// arguments, and the methods that build deep trees.
pub fn needs_large_stack(method: &str, args: &[Value]) -> bool {
    TREE_METHODS.contains(&method) || deeper_than(args, DEEP_JSON)
}

/// Whether the values nest more than `limit` arrays and objects deep.
fn deeper_than(args: &[Value], limit: usize) -> bool {
    let mut stack: Vec<(&Value, usize)> = args.iter().map(|value| (value, 2)).collect();
    while let Some((value, depth)) = stack.pop() {
        let children: Box<dyn Iterator<Item = &Value>> = match value {
            Value::Array(items) => Box::new(items.iter()),
            Value::Object(map) => Box::new(map.values()),
            _ => continue,
        };
        if depth > limit {
            return true;
        }
        stack.extend(children.map(|child| (child, depth + 1)));
    }
    false
}

/// Dispatch one call. Unknown methods and malformed arguments are errors.
fn dispatch(method: &str, args: &[Value]) -> Result<Value> {
    let result = match method {
        "treeFromPgn" => to_value(tree_from_pgn(text(args, 0, "pgn")?))?,
        "treeToPgn" => Value::String(tree_to_pgn(
            arg(args, 0).ok_or_else(|| DomainError::Message("root".to_string()))?,
            arg(args, 1),
        )?),
        "replay" => to_value(replay_positions(
            text(args, 0, "fen")?,
            &texts(args, 1, "moves")?,
        ))?,
        "setupReplay" => {
            let (variant, fen) = setup(args, 0)?;
            to_value(replay_setup(variant, fen, &texts(args, 1, "moves")?))?
        }
        "reviewClassifications" => crate::review::review_classifications(
            arg(args, 0).ok_or_else(|| DomainError::Message("review".to_string()))?,
        )
        .unwrap_or(Value::Null),
        "reviewPhases" => crate::review::review_phases(
            arg(args, 0).ok_or_else(|| DomainError::Message("review".to_string()))?,
        )
        .unwrap_or(Value::Null),
        "reviewMoment" => crate::coach::review_moment(
            arg(args, 0).unwrap_or(&Value::Null),
            arg(args, 1).and_then(Value::as_u64).unwrap_or(0) as usize,
        )
        .unwrap_or(Value::Null),
        "puzzleClue" => {
            crate::tactics::puzzle_facts(text(args, 0, "fen")?, &texts(args, 1, "moves")?)
                .map_or(Value::Null, |line| json!(crate::coach::clue(&line)))
        }
        "analyseReview" => {
            analyse_review(arg(args, 0).ok_or_else(|| DomainError::Message("review".to_string()))?)
                .unwrap_or(Value::Null)
        }
        "studyDocumentPgn" => {
            let chapters = arg(args, 0)
                .and_then(Value::as_array)
                .ok_or_else(|| DomainError::Message("chapters".to_string()))?;
            let pairs: Vec<(&str, &str)> = chapters
                .iter()
                .map(|c| Some((c.get("name")?.as_str()?, c.get("pgn")?.as_str()?)))
                .collect::<Option<_>>()
                .ok_or_else(|| {
                    DomainError::Message("chapters need a name and a pgn".to_string())
                })?;
            study_document_pgn(&pairs).map_or(Value::Null, Value::String)
        }
        "pvSan" => {
            let max = arg(args, 2).and_then(Value::as_u64).unwrap_or(12) as usize;
            let variant = arg(args, 3).and_then(Value::as_str).unwrap_or_default();
            pv_san(text(args, 0, "fen")?, &texts(args, 1, "pv")?, max, variant)
        }
        "playMove" => play_move(text(args, 0, "fen")?, text(args, 1, "uci")?),
        "startNode" => start_node(text(args, 0, "fen")?),
        "sanHistory" => to_value(san_history(&texts(args, 0, "moves")?))?,
        "pgnFromSan" => Value::String(pgn_from_san(&texts(args, 0, "sans")?)),
        "pgnFromUci" => Value::String(pgn_from_san(&san_history(&texts(args, 0, "moves")?))),
        "pgnFromMoves" => Value::String(pgn_from_moves(&texts(args, 0, "tokens")?)),
        "setupPgn" => {
            let (variant, fen) = setup(args, 0)?;
            let headers = arg(args, 2).and_then(Value::as_object);
            Value::String(setup_pgn(variant, fen, &texts(args, 1, "moves")?, headers)?)
        }
        "drawReason" => to_value(crate::records::games::draw_reason(&texts(
            args, 0, "moves",
        )?))?,
        "setupDrawReason" => {
            let (variant, fen) = setup(args, 0)?;
            to_value(crate::records::games::setup_draw_reason(
                variant,
                fen,
                &texts(args, 1, "moves")?,
            ))?
        }
        "position" => {
            let (variant, fen) = setup(args, 0)?;
            to_value(rules::setup_start(variant, fen).map(|pos| position::info(&pos)))?
        }
        "dests" => {
            let pos = setup_position(args)?;
            // "board960" lists castling as the rules do.
            let mode = match arg(args, 1).and_then(Value::as_str) {
                Some("board") => DestMode::Board,
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
        "legalMoves" => to_value(position::legal_moves(&setup_position(args)?))?,
        "play" => to_value(position::play_uci(
            &setup_position(args)?,
            text(args, 1, "uci")?,
        ))?,
        "playSan" => to_value(position::play_san(
            &setup_position(args)?,
            text(args, 1, "san")?,
        ))?,
        "line" => {
            let (variant, fen) = setup(args, 0)?;
            let moves = texts(args, 1, "moves")?;
            let options = arg(args, 2);
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
        "chess960Fen" => {
            let n = arg(args, 0)
                .and_then(Value::as_f64)
                .ok_or_else(|| DomainError::Message("n must be a number".to_string()))?;
            Value::String(rules::chess960_fen(n.floor() as i64))
        }
        "defaultFen" => {
            let variant = rules::lichess_variant(text(args, 0, "variant")?)
                .ok_or_else(|| DomainError::Message("variant".to_string()))?;
            Value::String(rules::make_fen(&rules::default_position(variant)))
        }
        "fenSetup" => position::fen_setup(text(args, 0, "fen")?).unwrap_or(Value::Null),
        "fenProblem" => to_value(position::fen_problem(text(args, 0, "fen")?))?,
        "pgnGames" => position::pgn_games(text(args, 0, "pgn")?),
        "pgnMainline" => {
            let limit = arg(args, 1).and_then(Value::as_u64).unwrap_or(u64::MAX);
            position::pgn_mainline(text(args, 0, "pgn")?, limit as usize)
        }
        _ => {
            // Each group of moved rules dispatches its own methods.
            let moved = [
                crate::voice::call,
                crate::training::call,
                crate::records::call,
                crate::misc::call,
                crate::theme::call,
            ];
            return match moved.iter().find_map(|call| call(method, args)) {
                Some(result) => Ok(result.map_err(DomainError::Message)?),
                None => Err(DomainError::Message(format!(
                    "unknown rules method {method}"
                ))),
            };
        }
    };
    Ok(result)
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

/* ── Analysis tree ── */

fn standard_or_default(fen: &str) -> shakmaty::variant::VariantPosition {
    rules::standard_from_fen(fen)
        .unwrap_or_else(|| rules::default_position(shakmaty::variant::Variant::Chess))
}

/// `newTree(fen)`: the root's normalized FEN and ply (the usual start when the FEN is unusable).
fn start_node(fen: &str) -> Value {
    let pos = standard_or_default(fen);
    json!({ "fen": rules::make_fen(&pos), "ply": rules::ply_of(&pos) })
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
/// `variant` is a Lichess key, empty for standard chess.
fn pv_san(fen: &str, pv: &[&str], max: usize, variant: &str) -> Value {
    let position = match variant {
        "" => rules::standard_from_fen(fen),
        variant => rules::setup_start(variant, fen),
    };
    let Some(mut pos) = position else {
        return json!([]);
    };
    let mut moves = Vec::new();
    for (ply, &uci) in (rules::ply_of(&pos)..).zip(pv.iter().take(max)) {
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
                    .ok_or_else(|| DomainError::Message("comments must be strings".to_string()))
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => Err(DomainError::Message("comments must be a list".to_string())),
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
                    .ok_or_else(|| DomainError::Message("nags must be whole numbers".to_string()))
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => Err(DomainError::Message("nags must be a list".to_string())),
    }
}

/// `treeToPgn(root, headers)`: the analysis tree as one PGN document.
fn tree_to_pgn(root: &Value, headers: Option<&Value>) -> Result<String> {
    let mut game = Game::empty();
    if let Some(headers) = headers.and_then(Value::as_object) {
        for (key, value) in headers {
            let value = value
                .as_str()
                .ok_or_else(|| DomainError::Message("header values must be strings".to_string()))?;
            game.put_header(key, value.to_string());
        }
    }
    game.comments = strings(root.get("comments"))?;
    let fen = root
        .get("fen")
        .and_then(Value::as_str)
        .ok_or_else(|| DomainError::Message("root.fen".to_string()))?;
    // A variant game (Crazyhouse, Atomic…) keeps its Variant header, and its FEN unless it starts
    // where that variant does.
    let variant = rules::pgn_variant(game.header("Variant"))
        .filter(|variant| *variant != shakmaty::variant::Variant::Chess);
    if let Some(variant) = variant {
        if let Some(pos) =
            rules::parse_fen(fen).and_then(|setup| rules::setup_position(variant, setup))
        {
            let start = rules::make_fen(&pos);
            if start == rules::make_fen(&rules::default_position(variant)) {
                game.remove_header("FEN");
            } else {
                game.put_header("FEN", start);
            }
        }
    } else if fen != INITIAL_FEN
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
            .ok_or_else(|| DomainError::Message("children".to_string()))?;
        let mut added = Vec::with_capacity(children.len());
        for child in children {
            let san = child
                .get("san")
                .and_then(Value::as_str)
                .ok_or_else(|| DomainError::Message("san".to_string()))?;
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

/// `pgnFromMoves`: UCI or SAN tokens (results left out) as a PGN.
fn pgn_from_moves(tokens: &[&str]) -> String {
    let cleaned: Vec<&str> = tokens
        .iter()
        .map(|t| crate::js::trim(t))
        .filter(|t| !t.is_empty() && !is_result(t))
        .collect();
    let lower: Vec<String> = cleaned.iter().map(|t| t.to_lowercase()).collect();
    if !cleaned.is_empty() && lower.iter().all(|t| rules::is_uci_text(t)) {
        // Replayed as detected: the rules read UCI in lower case.
        let lower: Vec<&str> = lower.iter().map(String::as_str).collect();
        pgn_from_san(&san_history(&lower))
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
                .ok_or_else(|| DomainError::Message("header values must be strings".to_string()))?
                .to_string(),
        );
    }
    if variant != "standard" {
        // The PGN `Variant` header takes the names other PGN readers know, not the Lichess keys.
        let name = match variant {
            "chess960" => "Chess960",
            "kingOfTheHill" => "King of the Hill",
            "threeCheck" => "Three-check",
            "antichess" => "Antichess",
            "atomic" => "Atomic",
            "horde" => "Horde",
            "racingKings" => "Racing Kings",
            "crazyhouse" => "Crazyhouse",
            other => other,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_pgns_name_their_variant_as_pgn_readers_know_it() {
        let koth = crate::rules::make_fen(&crate::rules::default_position(
            shakmaty::variant::Variant::KingOfTheHill,
        ));
        let pgn = setup_pgn("kingOfTheHill", &koth, &["e2e4"], None).unwrap();
        assert!(pgn.contains("[Variant \"King of the Hill\"]"), "{pgn}");
        assert_eq!(
            crate::rules::pgn_variant(Some("King of the Hill")),
            Some(shakmaty::variant::Variant::KingOfTheHill)
        );
    }

    #[test]
    fn upper_case_uci_moves_make_a_pgn() {
        // They were detected as UCI after lower-casing, then replayed as written: no moves.
        assert_eq!(
            pgn_from_moves(&["E2E4", "E7E5"]),
            pgn_from_moves(&["e2e4", "e7e5"])
        );
        assert!(pgn_from_moves(&["E2E4"]).contains("1. e4"));
    }

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
        assert!(call("replay", r#"["8/8/8/8/8/8/8/8 w - - 0 1", []] []"#).is_err());
        assert!(call("replay", "[1]").is_err());
    }

    fn nested(depth: usize) -> String {
        format!("[{}{}]", "[".repeat(depth - 1), "]".repeat(depth - 1))
    }

    /// 50,000 nested arrays used to overflow the stack and kill the host process.
    #[test]
    fn rejects_nesting_that_would_overflow_the_stack() {
        assert_eq!(
            call("reviewMoment", &nested(50_000)).unwrap_err(),
            "arguments nest too deeply"
        );
        assert_eq!(
            call("reviewMoment", &nested(MAX_JSON_DEPTH + 1)).unwrap_err(),
            "arguments nest too deeply"
        );
        // Brackets inside strings are text, not nesting.
        let text = format!("[[\"{}\\\"\"]]", "[".repeat(MAX_JSON_DEPTH * 2));
        assert!(call("reviewMoment", &text).is_ok());
    }

    /// A board's destinations castle onto the king's square too, a Chess960 board's onto the
    /// rook only, as the rules do.
    #[test]
    fn dests_castle_as_the_board_asks() {
        let setup = json!({ "variant": "standard", "fen": "4k3/8/8/8/8/8/8/4K2R w K - 0 1" });
        let king = |mode: Value| {
            let dests: Value =
                serde_json::from_str(&call("dests", &json!([setup, mode]).to_string()).unwrap())
                    .unwrap();
            dests
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry[0] == "e1")
                .unwrap()[1]
                .clone()
        };
        assert!(
            king(json!("board"))
                .as_array()
                .unwrap()
                .contains(&json!("g1"))
        );
        for mode in [json!("board960"), Value::Null] {
            let squares = king(mode);
            assert!(squares.as_array().unwrap().contains(&json!("h1")));
            assert!(!squares.as_array().unwrap().contains(&json!("g1")));
        }
    }

    /// Closing brackets end a level; an escaped quote does not end a string.
    #[test]
    fn nesting_counts_open_levels_outside_strings() {
        assert_eq!(nesting("[]"), 1);
        assert_eq!(nesting("[[], [], {}]"), 2);
        assert_eq!(nesting("[[[]], [[]]]"), 3);
        assert_eq!(nesting(r#"["]"]"#), 1);
        assert_eq!(nesting(r#"["\"", [[]]]"#), 3);
        assert_eq!(nesting(r#"["\" [[[", 1]"#), 1);
        assert_eq!(nesting(r#"["\\", [[]]]"#), 3);
    }

    /// Runs `f` on a 256 KiB thread, far smaller than any host's, so a deep call that stayed on
    /// the caller's stack would overflow.
    fn on_small_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(256 * 1024)
                .spawn_scoped(scope, f)
                .unwrap()
                .join()
                .unwrap()
        })
    }

    /// Values nested `depth` arrays deep.
    fn nested_value(depth: usize) -> Value {
        let mut value = json!([]);
        for _ in 1..depth {
            value = Value::Array(vec![value]);
        }
        value
    }

    /// `call_value` refuses arguments past the depth limit, and takes deep arguments and the
    /// tree methods to the large stack: on a small one they would overflow it.
    #[test]
    fn call_value_bounds_and_moves_deep_work_off_the_callers_stack() {
        let too_deep = nested_value(MAX_JSON_DEPTH + 1);
        assert_eq!(
            call_value("reviewMoment", std::slice::from_ref(&too_deep)).unwrap_err(),
            "arguments nest too deeply"
        );
        on_large_stack(move || {
            drop(too_deep);
            Ok(())
        })
        .unwrap();
        let deep = nested_value(MAX_JSON_DEPTH - 8);
        assert!(needs_large_stack(
            "reviewMoment",
            std::slice::from_ref(&deep)
        ));
        // Answered, whatever the answer, without overflowing the small stack.
        on_small_stack(|| drop(call_value("reviewMoment", std::slice::from_ref(&deep))));
        on_large_stack(move || {
            drop(deep);
            Ok(())
        })
        .unwrap();
        // Objects nest as arrays do.
        let mut object = json!({});
        for _ in 0..DEEP_JSON {
            object = json!({ "a": object });
        }
        assert!(needs_large_stack(
            "reviewMoment",
            std::slice::from_ref(&object)
        ));
        assert!(!needs_large_stack(
            "reviewMoment",
            &[nested_value(DEEP_JSON - 1)]
        ));
        assert!(needs_large_stack(
            "reviewMoment",
            &[nested_value(DEEP_JSON)]
        ));
        assert!(needs_large_stack("treeFromPgn", &[json!("1. e4 *")]));
        assert!(!needs_large_stack("replay", &[json!("1. e4 *")]));

        let moves = ["Nf3", "Nf6", "Ng1", "Ng8"].repeat(256).join(" ");
        let pgn = json!(format!("{moves} *"));
        let plies = on_small_stack(|| {
            let tree = call_value("treeFromPgn", &[pgn]).unwrap();
            let (mut node, mut plies) = (&tree, 0);
            while let Some(next) = node["children"].get(0) {
                node = next;
                plies += 1;
            }
            // Dropped where it was built would be on this small stack: dropped on a large one.
            let _ = on_large_stack(move || {
                drop(tree);
                Ok(())
            });
            plies
        });
        assert_eq!(plies, 1024);
    }

    /// The deepest accepted input is parsed and dropped whatever stack the caller has.
    #[test]
    fn accepts_the_depth_limit_on_a_small_stack() {
        let result = on_small_stack(|| call("reviewMoment", &nested(MAX_JSON_DEPTH)));
        assert_ne!(result, Err("arguments nest too deeply".to_string()));
    }

    /// A variant game's tree is written with its Variant header, and its FEN only when it starts
    /// elsewhere than its variant does, so it reads back in that variant.
    #[test]
    fn a_variant_tree_is_written_in_its_variant() {
        let written = |pgn: &str| {
            let tree = call("treeFromPgn", &serde_json::to_string(&[pgn]).unwrap()).unwrap();
            let tree: Value = serde_json::from_str(&tree).unwrap();
            let headers = tree["headers"].clone();
            let pgn = call(
                "treeToPgn",
                &serde_json::to_string(&json!([tree, headers])).unwrap(),
            );
            serde_json::from_str::<String>(&pgn.unwrap()).unwrap()
        };
        let pgn = written("[Variant \"Crazyhouse\"]\n\n1. e4 d5 2. exd5 Qxd5 3. P@e3 *");
        assert!(pgn.contains("[Variant \"Crazyhouse\"]"), "{pgn}");
        assert!(!pgn.contains("[FEN"), "{pgn}");
        assert!(pgn.contains("3. P@e3"), "{pgn}");
        let held = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[Pp] w KQkq - 0 1";
        let pgn = written(&format!(
            "[Variant \"Crazyhouse\"]\n[FEN \"{held}\"]\n\n1. P@e4 *"
        ));
        assert!(pgn.contains(&format!("[FEN \"{held}\"]")), "{pgn}");
        assert!(pgn.contains("[Variant \"Crazyhouse\"]"), "{pgn}");
    }

    /// A 1024-ply tree, the deepest the rules build, round-trips through `call` on a small stack
    /// (it overflowed tokio's 2 MiB default in a debug build).
    #[test]
    fn accepts_the_deepest_analysis_tree() {
        let moves = ["Nf3", "Nf6", "Ng1", "Ng8"].repeat(256).join(" ");
        let pgn = serde_json::to_string(&[format!("{moves} *")]).unwrap();
        let tree = on_small_stack(|| call("treeFromPgn", &pgn)).unwrap();
        assert_ne!(tree, "null");
        assert!(nesting(&format!("[{tree}]")) > DEEP_JSON);
        on_small_stack(|| call("treeToPgn", &format!("[{tree}]"))).unwrap();
    }
}

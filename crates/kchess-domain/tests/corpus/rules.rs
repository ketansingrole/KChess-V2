//! Chess rules, PGN, the analysis tree and reviews.

use serde_json::{Map, Value, json};
use shakmaty::variant::VariantPosition;

use kchess_domain::position::{self, DestMode};
use kchess_domain::records::review::StoredReview;
use kchess_domain::variant::Variant;
use kchess_domain::{coach, pgn, records, replay, review, rules, tree};

use super::{Out, arg};

fn text<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a str, String> {
    arg(args, i)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn texts<'a>(args: &'a [Value], i: usize, name: &str) -> Result<Vec<&'a str>, String> {
    arg(args, i)
        .and_then(Value::as_array)
        .and_then(|items| items.iter().map(Value::as_str).collect())
        .ok_or_else(|| format!("{name} must be a list of strings"))
}

/// `{ variant, fen }`
fn setup(args: &[Value], i: usize) -> Result<(Variant, &str), String> {
    let setup = arg(args, i)
        .and_then(Value::as_object)
        .ok_or_else(|| "setup must be an object".to_string())?;
    let field = |key| setup.get(key).and_then(Value::as_str);
    Ok((
        super::variant(field("variant").ok_or_else(|| "setup.variant".to_string())?)?,
        field("fen").ok_or_else(|| "setup.fen".to_string())?,
    ))
}

/// The legal position a `{ variant, fen }` setup describes; an error when it is not one.
fn setup_position(args: &[Value]) -> Result<VariantPosition, String> {
    let (variant, fen) = setup(args, 0)?;
    rules::setup_start(variant, fen).ok_or_else(|| "setup is not a legal position".to_string())
}

fn to_value<T: serde::Serialize>(value: T) -> Out {
    serde_json::to_value(value).map_err(|e| format!("serialization: {e}"))
}

/// The review the TypeScript suites passed, the fields they left out read as a review's
/// defaults; None for one no review could be.
fn stored_review(args: &[Value]) -> Result<Option<StoredReview>, String> {
    let mut fields = arg(args, 0).ok_or_else(|| "review".to_string())?.clone();
    if let Some(map) = fields.as_object_mut() {
        let defaults =
            json!({ "key": "", "source": "local", "depth": 0, "complete": false, "updatedAt": 0 });
        for (key, value) in defaults.as_object().into_iter().flatten() {
            map.entry(key).or_insert_with(|| value.clone());
        }
    }
    Ok(serde_json::from_value(fields).ok())
}

fn header_pairs(headers: Option<&Map<String, Value>>) -> Result<Vec<(&str, &str)>, String> {
    headers
        .into_iter()
        .flatten()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.as_str(), value))
                .ok_or_else(|| "header values must be strings".to_string())
        })
        .collect()
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    answer(method, args).transpose()
}

fn answer(method: &str, args: &[Value]) -> Result<Option<Value>, String> {
    Ok(Some(match method {
        "treeFromPgn" => to_value(tree::tree_from_pgn(text(args, 0, "pgn")?))?,
        "treeToPgn" => {
            let root = arg(args, 0).ok_or_else(|| "root".to_string())?;
            let headers = arg(args, 1).and_then(Value::as_object);
            if headers.is_some_and(|headers| headers.values().any(|value| !value.is_string())) {
                return Err("header values must be strings".into());
            }
            let root = json_tree(root)?;
            Value::String(tree::tree_to_pgn(&root, headers)?)
        }
        "replay" => to_value(replay::replay_positions(
            text(args, 0, "fen")?,
            &texts(args, 1, "moves")?,
        ))?,
        "setupReplay" => {
            let (variant, fen) = setup(args, 0)?;
            to_value(replay::replay_setup(
                variant,
                fen,
                &texts(args, 1, "moves")?,
            ))?
        }
        "reviewClassifications" => {
            to_value(stored_review(args)?.and_then(|r| review::classifications(&r)))?
        }
        "reviewPhases" => to_value(stored_review(args)?.and_then(|r| review::phases(&r)))?,
        "reviewMoment" => stored_review(args)?
            .and_then(|r| {
                coach::review_moment(
                    &r,
                    arg(args, 1).and_then(Value::as_u64).unwrap_or(0) as usize,
                )
            })
            .unwrap_or(Value::Null),
        "puzzleClue" => json!(coach::puzzle_clue(
            text(args, 0, "fen")?,
            &texts(args, 1, "moves")?
        )),
        "analyseReview" => to_value(stored_review(args)?.and_then(|r| review::analyse(&r).ok()))?,
        "studyDocumentPgn" => {
            let chapters = arg(args, 0)
                .and_then(Value::as_array)
                .ok_or_else(|| "chapters".to_string())?;
            let pairs: Vec<(&str, &str)> = chapters
                .iter()
                .map(|c| Some((c.get("name")?.as_str()?, c.get("pgn")?.as_str()?)))
                .collect::<Option<_>>()
                .ok_or_else(|| "chapters need a name and a pgn".to_string())?;
            json!(pgn::study_document_pgn(&pairs))
        }
        "pvSan" => {
            let max = arg(args, 2).and_then(Value::as_u64).unwrap_or(12) as usize;
            let variant = super::variant(arg(args, 3).and_then(Value::as_str).unwrap_or_default())?;
            to_value(tree::pv_san(
                text(args, 0, "fen")?,
                &texts(args, 1, "pv")?,
                max,
                variant,
            ))?
        }
        "startNode" => {
            let (fen, ply) = tree::tree_root(text(args, 0, "fen")?);
            json!({ "fen": fen, "ply": ply })
        }
        "sanHistory" => to_value(pgn::san_history(&texts(args, 0, "moves")?))?,
        "pgnFromSan" => Value::String(pgn::pgn_from_san(&texts(args, 0, "sans")?)),
        "pgnFromUci" => Value::String(pgn::pgn_from_uci(&texts(args, 0, "moves")?)),
        "setupPgn" => {
            let (variant, fen) = setup(args, 0)?;
            let headers = header_pairs(arg(args, 2).and_then(Value::as_object))?;
            Value::String(pgn::setup_pgn(
                variant,
                fen,
                &texts(args, 1, "moves")?,
                &headers,
            )?)
        }
        "setupDrawReason" => {
            let (variant, fen) = setup(args, 0)?;
            to_value(records::games::setup_draw_reason(
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
                .ok_or_else(|| "n must be a number".to_string())?;
            Value::String(rules::chess960_fen(n.floor() as i64))
        }
        "defaultFen" => Value::String(
            Variant::from_key(text(args, 0, "variant")?)
                .ok_or_else(|| "variant".to_string())?
                .default_fen(),
        ),
        "fenSetup" => to_value(position::fen_setup(text(args, 0, "fen")?))?,
        "fenProblem" => to_value(
            position::fen_problem(text(args, 0, "fen")?).map(position::FenProblem::as_str),
        )?,
        "pgnGames" => to_value(position::pgn_games(text(args, 0, "pgn")?))?,
        "pgnMainline" => {
            let limit = arg(args, 1).and_then(Value::as_u64).unwrap_or(u64::MAX);
            to_value(position::pgn_mainline(
                text(args, 0, "pgn")?,
                limit as usize,
            ))?
        }
        _ => return Ok(None),
    }))
}

/// An analysis tree as the suites passed it (`fen`, `children` of `san`, `comments`,
/// `startingComments`, `nags`), checked in the order the JSON writer read it.
fn json_tree(root: &Value) -> Result<tree::TreeNode, String> {
    fn strings(value: Option<&Value>) -> Result<Option<Vec<String>>, String> {
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|c| {
                    c.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "comments must be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(_) => Err("comments must be a list".to_string()),
        }
    }
    fn nags(value: Option<&Value>) -> Result<Option<Vec<u32>>, String> {
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|n| {
                    n.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| "nags must be whole numbers".to_string())
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(_) => Err("nags must be a list".to_string()),
        }
    }
    let comments = strings(root.get("comments"))?;
    let fen = root
        .get("fen")
        .and_then(Value::as_str)
        .ok_or_else(|| "root.fen".to_string())?;
    // Every node checked depth first, as written, before any is built.
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let children = node
            .get("children")
            .and_then(Value::as_array)
            .ok_or_else(|| "children".to_string())?;
        for child in children {
            child
                .get("san")
                .and_then(Value::as_str)
                .ok_or_else(|| "san".to_string())?;
            strings(child.get("comments"))?;
            strings(child.get("startingComments"))?;
            nags(child.get("nags"))?;
        }
        stack.extend(children.iter().rev());
    }
    fn build(node: &Value) -> tree::TreeNode {
        let mut built = tree::TreeNode::leaf(
            String::new(),
            node["san"].as_str().unwrap_or_default().to_string(),
            String::new(),
            0,
        );
        built.comments = strings(node.get("comments")).unwrap_or_default();
        built.starting_comments = strings(node.get("startingComments")).unwrap_or_default();
        built.nags = nags(node.get("nags")).unwrap_or_default();
        built.children = node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .map(build)
            .collect();
        built
    }
    let mut tree = build(root);
    tree.fen = fen.to_string();
    tree.comments = comments;
    Ok(tree)
}

//! Chess rules, PGN, the analysis tree and reviews.

use serde_json::{Map, Value, json};
use shakmaty::variant::VariantPosition;

use kchess_domain::position::{self, DestMode};
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
fn setup(args: &[Value], i: usize) -> Result<(&str, &str), String> {
    let setup = arg(args, i)
        .and_then(Value::as_object)
        .ok_or_else(|| "setup must be an object".to_string())?;
    let field = |key| setup.get(key).and_then(Value::as_str);
    Ok((
        field("variant").ok_or_else(|| "setup.variant".to_string())?,
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
        "treeToPgn" => Value::String(tree::tree_to_pgn(
            arg(args, 0).ok_or_else(|| "root".to_string())?,
            arg(args, 1).and_then(Value::as_object),
        )?),
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
            review::review_classifications(arg(args, 0).ok_or_else(|| "review".to_string())?)
                .unwrap_or(Value::Null)
        }
        "reviewPhases" => review::review_phases(arg(args, 0).ok_or_else(|| "review".to_string())?)
            .unwrap_or(Value::Null),
        "reviewMoment" => coach::review_moment(
            arg(args, 0).unwrap_or(&Value::Null),
            arg(args, 1).and_then(Value::as_u64).unwrap_or(0) as usize,
        )
        .unwrap_or(Value::Null),
        "puzzleClue" => json!(coach::puzzle_clue(
            text(args, 0, "fen")?,
            &texts(args, 1, "moves")?
        )),
        "analyseReview" => {
            review::analyse_review(arg(args, 0).ok_or_else(|| "review".to_string())?)
                .unwrap_or(Value::Null)
        }
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
            let variant = arg(args, 3).and_then(Value::as_str).unwrap_or_default();
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
            ))
        }
        "drawReason" => to_value(records::games::draw_reason(&texts(args, 0, "moves")?))?,
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
            rules::default_fen(text(args, 0, "variant")?).ok_or_else(|| "variant".to_string())?,
        ),
        "fenSetup" => to_value(position::fen_setup(text(args, 0, "fen")?))?,
        "fenProblem" => to_value(position::fen_problem(text(args, 0, "fen")?))?,
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

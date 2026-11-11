use serde_json::{Value, json};

use kchess_domain::editor::{self, CastlingRights, EditorSetup};
use kchess_domain::rules;
use kchess_domain::voice::{self, Grammar};

use super::Out;

type Result<T> = std::result::Result<T, String>;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let grammar = |which| Ok(json!(voice::grammar(which)));
    let result = match method {
        "COORDINATE_GRAMMAR" => grammar(Grammar::Coordinate),
        "MOVE_GRAMMAR" => grammar(Grammar::Move),
        "GAME_GRAMMAR" => grammar(Grammar::Game),
        "EDITOR_GRAMMAR" => grammar(Grammar::Editor),
        "ANALYSIS_GRAMMAR" => grammar(Grammar::Analysis),
        "ALL_SQUARES" => Ok(json!(voice::all_squares())),
        "spokenCommand" => text(args, 0).map(|t| json!(voice::game_command(t))),
        "spokenChoice" => text(args, 0).map(|t| json!(voice::spoken_choice(t))),
        "spokenSquare" => text(args, 0).map(|t| json!(voice::spoken_square(t))),
        "spokenMove" => spoken_move(args),
        "spokenEdit" => text(args, 0).map(|t| {
            json!(voice::spoken_edit(
                t,
                args.get(1).and_then(Value::as_str).unwrap_or("white")
            ))
        }),
        "spokenAnalysisCommand" => text(args, 0).map(|t| json!(voice::analysis_command(t))),
        "withPiece" => with_piece(args),
        "castlingAvailable" => text(args, 0).map(|board| json!(editor::castling_available(board))),
        "enPassantSquares" => text(args, 0).and_then(|board| {
            text(args, 1).map(|turn| json!(editor::en_passant_squares(board, side(turn))))
        }),
        "setupFen" => setup_fen(args),
        "randomSquare" => random_square(args),
        "squareColor" => text(args, 0).map(|s| json!(voice::square_color(s))),
        "knightMoves" => text(args, 0).map(|s| json!(voice::knight_moves(s))),
        "knightDistance" => text(args, 0)
            .and_then(|from| text(args, 1).map(|to| json!(voice::knight_distance(from, to)))),
        "knightChallenge" => (|| {
            Ok(json!(voice::knight_challenge(
                number(args, 0)?,
                number(args, 1)?,
                &numbers(args, 2)?,
            )))
        })(),
        "knightFen" => text(args, 0).map(|s| json!(voice::knight_fen(s))),
        "materialDiff" => text(args, 0).map(|fen| json!(voice::material_diff(fen))),
        _ => return None,
    };
    Some(result)
}

fn text(args: &[Value], i: usize) -> Result<&str> {
    args.get(i)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("argument {i} must be a string"))
}

fn numbers(args: &[Value], i: usize) -> Result<Vec<f64>> {
    args.get(i)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_f64).collect())
        .ok_or_else(|| format!("argument {i} must be a list of numbers"))
}

fn number(args: &[Value], i: usize) -> Result<f64> {
    args.get(i)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("argument {i} must be a number"))
}

fn spoken_move(args: &[Value]) -> Out {
    let text = text(args, 0)?;
    let setup = args
        .get(1)
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
    let pos =
        rules::setup_start(super::variant(variant)?, fen).ok_or("setup is not a legal position")?;
    Ok(json!(voice::spoken_move(text, &pos)))
}

fn with_piece(args: &[Value]) -> Out {
    let board = text(args, 0)?;
    let square = text(args, 1)?;
    let piece = match args.get(2).filter(|v| !v.is_null()) {
        None => None,
        Some(piece) => Some((
            piece
                .get("color")
                .and_then(Value::as_str)
                .ok_or("piece.color must be a string")?,
            piece
                .get("role")
                .and_then(Value::as_str)
                .ok_or("piece.role must be a string")?,
        )),
    };
    editor::with_piece(board, square, piece).map(Value::String)
}

fn setup_fen(args: &[Value]) -> Out {
    let setup = args
        .first()
        .filter(|v| v.is_object())
        .ok_or("setup must be an object")?;
    let field = |key: &str, message: &str| {
        setup
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| message.to_string())
    };
    let ticked = |right: &str| {
        setup
            .get("castling")
            .and_then(|castling| castling.get(right))
            .and_then(Value::as_bool)
            == Some(true)
    };
    let setup = EditorSetup {
        board: field("board", "setup.board must be a string")?,
        turn: side(&field("turn", "setup.turn must be a string")?),
        castling: CastlingRights {
            white_short: ticked("K"),
            white_long: ticked("Q"),
            black_short: ticked("k"),
            black_long: ticked("q"),
        },
        ep: setup
            .get("ep")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    };
    Ok(Value::String(setup.fen()))
}

/// The side a turn names: White for `white`, Black for anything else, as the editor read it.
fn side(turn: &str) -> shakmaty::Color {
    if turn == "white" {
        shakmaty::Color::White
    } else {
        shakmaty::Color::Black
    }
}

fn random_square(args: &[Value]) -> Out {
    let previous = args.first().and_then(Value::as_str);
    let allowed: Option<Vec<String>> = match args.get(2) {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => {
            let valid = voice::all_squares();
            let mut list = Vec::new();
            for item in items {
                match item.as_str() {
                    Some(square) if valid.iter().any(|known| known == square) => {
                        list.push(square.to_string());
                    }
                    _ => return Err("argument 2 must be a list of squares".into()),
                }
            }
            Some(list)
        }
        Some(_) => return Err("argument 2 must be a list of squares".into()),
    };
    Ok(json!(voice::random_square(
        previous,
        &numbers(args, 1)?,
        allowed.as_deref(),
    )))
}

//! Training: puzzle runs, puzzles, endgame drills and lessons, openings, the client clock,
//! engine levels and UCI `info` lines.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use kchess_domain::js;
use kchess_domain::training::clock::{ClockState, SetData, format_clock};
use kchess_domain::training::data::data;
use kchess_domain::training::openings::{self, ExplorerMove};
use kchess_domain::training::puzzle::{self, PuzzleState};
use kchess_domain::training::rush::{self, RushConfig, RushState};
use kchess_domain::training::{endgames, engine_levels, lessons, uci_info};
use kchess_domain::variant::Variant;

use super::{Out, arg};

/// A puzzle's solution as sent: its moves that are strings.
fn solution_of(puzzle: &Value) -> Vec<String> {
    puzzle
        .get("solution")
        .and_then(Value::as_array)
        .map(|moves| {
            moves
                .iter()
                .filter_map(|uci| uci.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn text_arg<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a str, String> {
    arg(args, i)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

/// `Number(…)` of an argument: absent is NaN, and `"NaN"`, `"Infinity"` read back as numbers.
fn number_arg(args: &[Value], i: usize) -> f64 {
    js::to_number(arg(args, i))
}

fn value_arg<T: DeserializeOwned>(args: &[Value], i: usize, name: &str) -> Result<T, String> {
    let value = arg(args, i)
        .cloned()
        .ok_or_else(|| format!("{name} is missing"))?;
    serde_json::from_value(value).map_err(|e| format!("{name}: {e}"))
}

fn to_value<T: Serialize>(value: T) -> Out {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// A number answer: JSON has no NaN or infinity, so those go out as their text.
fn num_out(n: f64) -> Value {
    if n.is_finite() {
        Value::from(n)
    } else {
        Value::String(js::number_to_string(n))
    }
}

/// A transition's new state and its own result.
fn stepped(state: impl Serialize, result: impl Serialize) -> Out {
    to_value(json!({ "state": state, "result": result }))
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        // Puzzle runs.
        "rushConfigs" => to_value(&data().rush_configs),
        "rushChoices" => to_value(&data().rush_choices),
        "newRush" => value_arg::<RushConfig>(args, 0, "config")
            .and_then(|config| to_value(RushState::new(&config))),
        "correctMove" => state_and_config(args).and_then(|(mut state, config)| {
            let result = state.correct_move(&config);
            stepped(state, result)
        }),
        "mistake" => state_and_config(args).and_then(|(mut state, config)| {
            state.mistake(&config);
            to_value(state)
        }),
        "skip" => value_arg::<RushState>(args, 0, "state").and_then(|mut state| {
            let result = state.skip();
            stepped(state, result)
        }),
        "puzzleSolved" => value_arg::<RushState>(args, 0, "state").and_then(|mut state| {
            state.puzzle_solved(number_arg(args, 1));
            to_value(state)
        }),
        "tick" => value_arg::<RushState>(args, 0, "state").and_then(|mut state| {
            state.tick(number_arg(args, 1));
            to_value(state)
        }),
        "finish" => value_arg::<RushState>(args, 0, "state").and_then(|mut state| {
            let reason = text_arg(args, 1, "reason")?;
            state.finish(reason);
            to_value(state)
        }),
        "accuracy" => value_arg::<RushState>(args, 0, "state")
            .and_then(|state| to_value(num_out(state.accuracy()))),
        "formatRunClock" => Ok(Value::String(rush::format_run_clock(number_arg(args, 0)))),
        "comboBonusMs" => to_value(num_out(rush::combo_bonus_ms(number_arg(args, 0)))),
        "comboProgress" => to_value(num_out(rush::combo_progress(number_arg(args, 0)))),
        // Puzzles.
        "puzzleFromApi" => value_arg::<Value>(args, 0, "raw")
            .map(|raw| puzzle::from_api(&raw).unwrap_or(Value::Null)),
        "moveSquares" => value_arg::<String>(args, 0, "fen").and_then(|fen| {
            let uci = text_arg(args, 1, "uci")?;
            to_value(puzzle::move_squares(&fen, uci))
        }),
        "startPuzzle" => value_arg::<Value>(args, 0, "puzzle").and_then(|puzzle| {
            let variant = js::string(puzzle.get("variant"))
                .and_then(Variant::from_key)
                .unwrap_or_default();
            to_value(puzzle::start_puzzle(
                js::string(puzzle.get("fen")).unwrap_or(""),
                js::string(puzzle.get("lastMove")),
                variant,
            ))
        }),
        "playerMove" => (|| {
            let puzzle: Value = value_arg(args, 0, "puzzle")?;
            let state: PuzzleState = value_arg(args, 1, "state")?;
            let uci = text_arg(args, 2, "uci")?;
            to_value(puzzle::player_move(&solution_of(&puzzle), state, uci))
        })(),
        "opponentReply" => value_arg::<PuzzleState>(args, 0, "state").and_then(|state| {
            let uci = text_arg(args, 1, "uci")?;
            to_value(puzzle::opponent_reply(state, uci))
        }),
        "nextSolutionSquares" => (|| {
            let puzzle: Value = value_arg(args, 0, "puzzle")?;
            let state: PuzzleState = value_arg(args, 1, "state")?;
            to_value(puzzle::next_solution_squares(&solution_of(&puzzle), &state))
        })(),
        "playerColor" => value_arg::<Value>(args, 0, "puzzle").and_then(|puzzle| {
            let fen = js::string(puzzle.get("fen")).unwrap_or("");
            to_value(match puzzle::player_color(fen) {
                shakmaty::Color::White => "white",
                shakmaty::Color::Black => "black",
            })
        }),
        // Endgame drills.
        "endgameDrills" => to_value(&data().endgame_drills),
        "evaluateEndgame" => (|| {
            let fen: String = value_arg(args, 0, "fen")?;
            let moves: Vec<String> = value_arg(args, 1, "moves")?;
            let player: String = value_arg(args, 2, "player")?;
            let goal: String = value_arg(args, 3, "goal")?;
            to_value(endgames::evaluate(&fen, &moves, &player, &goal)?)
        })(),
        "sanFrom" => (|| {
            let fen: String = value_arg(args, 0, "fen")?;
            let moves: Vec<String> = value_arg(args, 1, "moves")?;
            to_value(endgames::san_from(&fen, &moves))
        })(),
        // Lessons.
        "lessonCourses" => to_value(&lessons::data().courses),
        "lessonProblems" => to_value(lessons::problems(lessons::data())),
        "lessonStatus" => step_args(args)
            .and_then(|(fen, task, moves)| lessons::status(&fen, &task, &moves))
            .and_then(to_value),
        "lessonMove" => step_args(args)
            .and_then(|(fen, task, moves)| {
                let uci: String = value_arg(args, 3, "uci")?;
                lessons::try_move(&fen, &task, &moves, &uci)
            })
            .and_then(to_value),
        "lessonHint" => step_args(args)
            .and_then(|(fen, task, moves)| lessons::hint(&fen, &task, &moves))
            .and_then(to_value),
        // Openings.
        "openingAt" => opening_at(args),
        "openingTree" => to_value(openings::opening_tree()),
        "openingReplies" => value_arg::<Vec<ExplorerMove>>(args, 0, "moves").and_then(|moves| {
            let yours = arg(args, 1).and_then(Value::as_bool).unwrap_or(false);
            to_value(openings::opening_replies(&moves, yours))
        }),
        "openingKey" => {
            text_arg(args, 0, "name").and_then(|name| to_value(openings::opening_key(name)))
        }
        // The client clock.
        "clockInitial" => to_value(ClockState::initial()),
        "clockSet" => clock_arg(args).and_then(|mut state| {
            let data: SetData = value_arg(args, 1, "data")?;
            state.set(&data, number_arg(args, 2));
            to_value(state)
        }),
        "clockPause" => clock_arg(args).and_then(|mut state| {
            state.pause(number_arg(args, 1));
            to_value(state)
        }),
        "clockRemaining" => clock_arg(args).and_then(|state| {
            let color = text_arg(args, 1, "color")?;
            to_value(num_out(state.remaining(color, number_arg(args, 2))))
        }),
        "clockLowTimeAlert" => clock_arg(args).and_then(|mut state| {
            let color = text_arg(args, 1, "color")?;
            let result = state.low_time_alert(color, number_arg(args, 2));
            stepped(state, result)
        }),
        "formatClock" => Ok(Value::String(format_clock(number_arg(args, 0)))),
        // Engine levels.
        "engineLadder" => to_value(engine_levels::ladder()),
        "defaultEngineLevels" => to_value(engine_levels::default_levels()),
        "engineLevelInfo" => {
            text_arg(args, 0, "id").and_then(|id| to_value(engine_levels::info(id).cloned()))
        }
        "engineLevelLabel" => text_arg(args, 0, "id")
            .and_then(engine_levels::label)
            .map(Value::String),
        "normalizeEngineLevels" => value_arg::<Vec<String>>(args, 0, "levels")
            .and_then(|levels| to_value(engine_levels::normalize(&levels))),
        // UCI `info` lines.
        "parseInfo" => text_arg(args, 0, "text").map(|text| {
            let white_to_move = arg(args, 1).and_then(Value::as_bool).unwrap_or(false);
            uci_info::parse_info(text, white_to_move).map_or(Value::Null, info_json)
        }),
        _ => return None,
    })
}

fn state_and_config(args: &[Value]) -> Result<(RushState, RushConfig), String> {
    Ok((value_arg(args, 0, "state")?, value_arg(args, 1, "config")?))
}

fn clock_arg(args: &[Value]) -> Result<ClockState, String> {
    value_arg(args, 0, "state")
}

fn step_args(args: &[Value]) -> Result<(String, lessons::Task, Vec<String>), String> {
    Ok((
        value_arg(args, 0, "fen")?,
        value_arg(args, 1, "task")?,
        value_arg(args, 2, "moves")?,
    ))
}

fn opening_at(args: &[Value]) -> Out {
    let setup: Value = value_arg(args, 0, "setup")?;
    let variant = setup.get("variant").and_then(Value::as_str).unwrap_or("");
    let fen = setup.get("fen").and_then(Value::as_str).unwrap_or("");
    let moves: Vec<String> = value_arg(args, 1, "moves")?;
    // `ply` defaults to the number of moves; a non-finite one arrives as its text.
    let ply = match arg(args, 2) {
        Some(_) => number_arg(args, 2),
        None => moves.len() as f64,
    };
    to_value(openings::opening_at(variant, fen, &moves, ply))
}

fn info_json(info: uci_info::Info) -> Value {
    let mut line = Map::new();
    line.insert("rank".into(), num_out(info.line.rank));
    line.insert("depth".into(), num_out(info.line.depth));
    line.insert("pv".into(), json!(info.line.pv));
    if let Some(mate) = info.line.mate {
        line.insert("mate".into(), num_out(mate));
    }
    if let Some(cp) = info.line.cp {
        line.insert("cp".into(), num_out(cp));
    }
    let mut out = Map::new();
    out.insert("line".into(), Value::Object(line));
    if let Some(nps) = info.nps {
        out.insert("nps".into(), num_out(nps));
    }
    Value::Object(out)
}

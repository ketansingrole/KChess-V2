//! `review <link|file|moves>`: review a game with the local engine, as the analysis board does,
//! and print each side's accuracy and the moves Stockfish faults.

use kchess_domain::pgn::parse_pgn;
use kchess_domain::review::{analyse_review, san_to_uci};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::core::{CoreClient, Event};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// A game read for review: where it starts, its main line in SAN and in UCI.
#[derive(Debug, PartialEq)]
pub struct Line {
    pub fen: String,
    pub sans: Vec<String>,
    pub moves: Vec<String>,
}

/// The main line of the first game in `pgn`. Only standard games are reviewed: the review reads
/// moves under standard rules.
pub fn main_line(pgn: &str) -> Result<Line, String> {
    let game = parse_pgn(pgn)
        .into_iter()
        .next()
        .ok_or("No game could be read from that PGN.")?;
    if let Some(variant) = game
        .header("Variant")
        .filter(|variant| !variant.eq_ignore_ascii_case("standard"))
    {
        return Err(format!("{variant} games cannot be reviewed."));
    }
    let fen = game.header("FEN").unwrap_or(START).to_string();
    let mut sans = Vec::new();
    let mut node = 0;
    while let Some(next) = game.children(node).next() {
        sans.push(game.node(next).san.as_str().to_string());
        node = next;
    }
    if sans.is_empty() {
        return Err("That game has no moves to review.".into());
    }
    let moves = san_to_uci(&fen, &sans);
    if moves.len() < sans.len() {
        return Err(format!(
            "Move {} ({}) is not legal there.",
            moves.len() / 2 + 1,
            sans[moves.len()]
        ));
    }
    Ok(Line { fen, sans, moves })
}

/// Each side's figures and the faulted moves, numbered as in the game.
pub fn report(line: &Line, review: &Value) -> Option<Value> {
    let analysis = analyse_review(review)?;
    let white_first = line.fen.split(' ').nth(1) != Some("b");
    let first_number: usize = line
        .fen
        .split(' ')
        .nth(5)
        .and_then(|number| number.parse().ok())
        .unwrap_or(1);
    let faults: Vec<Value> = analysis["moves"]
        .as_array()?
        .iter()
        .zip(&line.sans)
        .enumerate()
        .filter_map(|(ply, (entry, san))| {
            let judgment = entry["judgment"].as_str()?;
            let half = ply + usize::from(!white_first);
            Some(json!({
                "move": first_number + half / 2,
                "color": entry["color"],
                "san": san,
                "judgment": judgment,
            }))
        })
        .collect();
    Some(json!({
        "white": analysis["white"],
        "black": analysis["black"],
        "faults": faults,
    }))
}

pub async fn run(
    core: &CoreClient,
    events: &mut UnboundedReceiver<Event>,
    pgn: &str,
) -> Result<Value, String> {
    let line = main_line(pgn)?;
    core.call("resumeOnline", vec![]).await?;
    // A queued request answers null; the finished review arrives as an update. The core keys it
    // by its own reading of the start position, so it is recognised by its moves and position.
    let mut stored = core
        .call(
            "reviewRequest",
            vec![json!({ "fen": line.fen, "moves": line.moves })],
        )
        .await?;
    let start = |fen: &str| fen.split(' ').take(2).collect::<Vec<_>>().join(" ");
    let ours = |review: &Value| {
        review["moves"] == json!(line.moves)
            && review["fen"]
                .as_str()
                .is_some_and(|fen| start(fen) == start(&line.fen))
    };
    // The key the core gives the review, so a failure reported for it ends the wait.
    let key = kchess_domain::api::call_value("reviewKey", &[json!(line.fen), json!(line.moves)])
        .ok()
        .and_then(|key| key.as_str().map(str::to_string));
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    while !(ours(&stored) && stored["complete"] == json!(true)) {
        tokio::select! {
            _ = &mut interrupted => {
                if let Some(key) = stored["key"].as_str().or(key.as_deref()) {
                    let _ = core.call("reviewCancel", vec![json!(key)]).await;
                }
                return Err("Review cancelled.".into());
            }
            event = events.recv() => match event {
                None => return Err("The core closed.".into()),
                Some(event) if event.name == "core:closed" => return Err("The core closed.".into()),
                Some(event) if event.name == "host:request" => core.answer(&event.payload).await,
                Some(event) if event.name == "review:update" && ours(&event.payload["review"]) => {
                    stored = event.payload["review"].clone();
                }
                Some(event) if event.name == "review:status"
                    && event.payload["failed"]["key"].as_str().is_some_and(|failed| {
                        Some(failed) == stored["key"].as_str().or(key.as_deref())
                    }) =>
                {
                    let message = event.payload["failed"]["message"].as_str().unwrap_or_default();
                    return Err(format!("The review failed: {message}"));
                }
                Some(_) => {}
            },
        }
    }
    report(&line, &stored).ok_or_else(|| "The review could not be read.".into())
}

#[cfg(test)]
mod tests {
    use super::{main_line, report};
    use serde_json::json;

    #[test]
    fn the_main_line_is_read_and_other_variants_refused() {
        let line =
            main_line("[Event \"x\"]\n\n1. e4 e5 (1... c5) 2. Nf3 { ok } Nc6 *").expect("a game");
        assert_eq!(line.sans, ["e4", "e5", "Nf3", "Nc6"]);
        assert_eq!(line.moves, ["e2e4", "e7e5", "g1f3", "b8c6"]);
        assert_eq!(
            main_line("[Variant \"Atomic\"]\n\n1. e4 *"),
            Err("Atomic games cannot be reviewed.".into())
        );
        assert_eq!(
            main_line("1. e4 e5 2. Ke3 *"),
            Err("Move 2 (Ke3) is not legal there.".into())
        );
        assert_eq!(
            main_line("[Event \"x\"]\n\n*"),
            Err("That game has no moves to review.".into())
        );
    }

    #[test]
    fn a_blunder_is_reported_with_its_move_number() {
        // Black to move at move 10: Qg4?? hangs the queen.
        let line = main_line(
            "[FEN \"rnb1kbnr/pppp1ppp/8/4p1q1/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 10\"]\n\n10... Qg4 11. Qxg4 *",
        )
        .expect("a game");
        let review = json!({
            "fen": line.fen, "moves": line.moves, "source": "local", "depth": 18,
            "complete": true,
            "evals": [{"cp": 0, "depth": 18}, {"cp": 900, "depth": 18}, {"cp": 900, "depth": 18}],
        });
        let report = report(&line, &review).expect("a report");
        assert_eq!(
            report["faults"],
            json!([{ "move": 10, "color": "black", "san": "Qg4", "judgment": "blunder" }])
        );
        assert_eq!(report["black"]["blunder"], json!(1));
        assert_eq!(report["white"]["blunder"], json!(0));
    }
}

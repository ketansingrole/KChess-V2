//! `review <link|file|moves>`: review a game with the local engine, as the analysis board does,
//! and print each side's accuracy and the moves Stockfish faults.

use kchess_contracts::calls;
use kchess_domain::pgn::parse_pgn;
use kchess_domain::review::{analyse_review, san_to_uci};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::core::{ClientEvent, CoreClient, CoreEvent};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// A game read for review: where it starts, its main line in SAN and in UCI, and the Lichess key
/// of its variant when Fairy-Stockfish reviews it (not standard chess or Chess960).
#[derive(Debug, PartialEq)]
pub struct Line {
    pub fen: String,
    pub sans: Vec<String>,
    pub moves: Vec<String>,
    pub variant: Option<&'static str>,
}

/// The Lichess key of a PGN `Variant` header's variant.
pub fn variant_key(name: &str) -> Result<&'static str, String> {
    let variant = kchess_domain::rules::pgn_variant(Some(name))
        .ok_or_else(|| format!("Unknown variant {name}."))?;
    kchess_domain::misc::constants::VARIANTS
        .iter()
        .copied()
        .find(|key| kchess_domain::rules::lichess_variant(key) == Some(variant))
        .ok_or_else(|| format!("Unknown variant {name}."))
}

/// The main line of the first game in `pgn`, read by its variant's rules.
pub fn main_line(pgn: &str) -> Result<Line, String> {
    let game = parse_pgn(pgn)
        .into_iter()
        .next()
        .ok_or("No game could be read from that PGN.")?;
    let variant = match game.header("Variant") {
        Some(name) => {
            Some(variant_key(name)?).filter(|key| !matches!(*key, "standard" | "chess960"))
        }
        None => None,
    };
    let mut sans = Vec::new();
    let mut node = 0;
    while let Some(next) = game.children(node).next() {
        sans.push(game.node(next).san.as_str().to_string());
        node = next;
    }
    if sans.is_empty() {
        return Err("That game has no moves to review.".into());
    }
    let (fen, moves) = match variant {
        // Read by the variant's rules, up to the first move that is not legal.
        Some(_) => {
            let line = kchess_domain::position::pgn_mainline(pgn, sans.len())
                .ok_or("No game could be read from that PGN.")?;
            let fen = line.start.ok_or("No game could be read from that PGN.")?;
            let moves = line.moves.into_iter().map(|entry| entry.uci).collect();
            (fen, moves)
        }
        None => {
            let fen = game.header("FEN").unwrap_or(START).to_string();
            let moves = san_to_uci(&fen, &sans);
            (fen, moves)
        }
    };
    if moves.len() < sans.len() {
        let fields: Vec<&str> = fen.split(' ').collect();
        let first: usize = fields
            .get(5)
            .and_then(|number| number.parse().ok())
            .unwrap_or(1);
        let half = moves.len() + usize::from(fields.get(1) == Some(&"b"));
        return Err(format!(
            "Move {} ({}) is not legal there.",
            first + half / 2,
            sans[moves.len()]
        ));
    }
    Ok(Line {
        fen,
        sans,
        moves,
        variant,
    })
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
    events: &mut UnboundedReceiver<ClientEvent>,
    pgn: &str,
) -> Result<Value, String> {
    let line = main_line(pgn)?;
    core.request(calls::ResumeOnline {}).await?;
    // A queued request answers null; the finished review arrives as an update. The core keys it
    // by its own reading of the start position, so it is recognised by its moves and position.
    let mut request = json!({ "fen": line.fen, "moves": line.moves });
    // Fairy-Stockfish reviews the other variants.
    if let Some(variant) = line.variant {
        request["variant"] = json!(variant);
    }
    let mut stored = core.request(calls::ReviewRequest { request }).await?;
    let start = |fen: &str| fen.split(' ').take(2).collect::<Vec<_>>().join(" ");
    let ours = |review: &Value| {
        review["moves"] == json!(line.moves)
            && review["fen"]
                .as_str()
                .is_some_and(|fen| start(fen) == start(&line.fen))
    };
    // The key the core gives the review, so a failure reported for it ends the wait.
    let key = kchess_domain::records::review::review_key(&line.fen, &line.moves, line.variant);
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    while !(ours(&stored) && stored["complete"] == json!(true)) {
        tokio::select! {
            _ = &mut interrupted => {
                let key = stored["key"].as_str().unwrap_or(&key);
                let _ = core.request(calls::ReviewCancel { key: json!(key) }).await;
                return Err("Review cancelled.".into());
            }
            event = events.recv() => match event {
                None => return Err("The core closed.".into()),
                Some(ClientEvent::Closed) => return Err("The core closed.".into()),
                Some(ClientEvent::Event(CoreEvent::ReviewUpdate(update))) if ours(&update["review"]) => {
                    stored = update["review"].clone();
                }
                Some(ClientEvent::Event(CoreEvent::ReviewStatus(status)))
                    if status["failed"]["key"].as_str().is_some_and(|failed| {
                        failed == stored["key"].as_str().unwrap_or(&key)
                    }) =>
                {
                    let message = status["failed"]["message"].as_str().unwrap_or_default();
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
    fn the_main_line_is_read_in_its_variant() {
        let line =
            main_line("[Event \"x\"]\n\n1. e4 e5 (1... c5) 2. Nf3 { ok } Nc6 *").expect("a game");
        assert_eq!(line.sans, ["e4", "e5", "Nf3", "Nc6"]);
        assert_eq!(line.moves, ["e2e4", "e7e5", "g1f3", "b8c6"]);
        assert_eq!(line.variant, None);
        // Another variant's moves are read by its rules: a drop is a move in Crazyhouse.
        let zh = main_line("[Variant \"Crazyhouse\"]\n\n1. e4 d5 2. exd5 Qxd5 3. P@e3 *")
            .expect("a Crazyhouse game");
        assert_eq!(zh.variant, Some("crazyhouse"));
        assert_eq!(zh.moves, ["e2e4", "d7d5", "e4d5", "d8d5", "P@e3"]);
        assert!(zh.fen.contains('['), "{}", zh.fen);
        assert_eq!(
            main_line("[Variant \"Atomic\"]\n\n1. e4 d5 2. exd5 Ke7 *"),
            Err("Move 2 (Ke7) is not legal there.".into())
        );
        assert_eq!(
            main_line("[Variant \"Bughouse\"]\n\n1. e4 *"),
            Err("Unknown variant Bughouse.".into())
        );
        // Chess960 castles king onto rook's square or the king's, as the core accepts either.
        let fischer = main_line(
            "[Variant \"Chess960\"]\n[FEN \"4k3/8/8/8/8/8/8/1RK3R1 w GB - 0 1\"]\n\n1. O-O Kd7 2. Rfe1 *",
        )
        .expect("a Chess960 game");
        assert_eq!(fischer.sans, ["O-O", "Kd7", "Rfe1"]);
        assert_eq!(fischer.moves.len(), 3, "{:?}", fischer.moves);
        assert_eq!(
            main_line("1. e4 e5 2. Ke3 *"),
            Err("Move 2 (Ke3) is not legal there.".into())
        );
        // Counted from the game's own start: Black's 30th move, then White's 31st.
        assert_eq!(
            main_line(
                "[FEN \"4k3/8/8/8/8/8/8/4K3 b - - 0 30\"]\n[SetUp \"1\"]\n\n30... Kd7 31. Ke3 *"
            ),
            Err("Move 31 (Ke3) is not legal there.".into())
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

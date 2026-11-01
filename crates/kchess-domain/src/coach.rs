//! Words around the facts of `tactics.rs`: the one-line reason shown free of charge, and what a
//! language model is asked, and may answer, when someone wants more. The model only phrases: its
//! answer is checked against the positions and lines it was given, and one that names a move
//! or a tactic from outside them is thrown away.

use std::collections::HashSet;

use serde_json::{Value, json};
use shakmaty::variant::VariantPosition;
use shakmaty::{Position, Role};

use crate::rules;
use crate::tactics::{self, LineFacts, MoveFacts, Tactic};

/// The instructions every request carries.
const INSTRUCTIONS: &str = "You are a chess coach explaining a position to a club player. Use \
only the facts given: never mention a move, piece or tactic they do not contain, and never \
calculate variations of your own. Name moves exactly as written in the facts. Answer in two to \
four plain sentences, with no heading, list or markdown.";
const NUDGE_INSTRUCTIONS: &str = "You are a chess coach giving a hint for a puzzle. Point at the \
idea in one plain sentence. Never name a move, a square or the piece to move.";

/// What a model may say about a position.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Allowed {
    /// Moves and pieces, as `Nf3`, `exd5` or `Ke8`, without check marks.
    pub moves: HashSet<String>,
    pub squares: HashSet<String>,
    /// Squares and moves that must stay unsaid: a puzzle's answer.
    pub secret: HashSet<String>,
    pub fork: bool,
    pub pin: bool,
    pub mate: bool,
}

/// A request for a language model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    pub instructions: &'static str,
    pub input: String,
    pub allowed: Allowed,
}

fn bare(san: &str) -> String {
    san.trim_end_matches(['+', '#', '!', '?']).to_string()
}

fn is_square(token: &str) -> bool {
    matches!(token.as_bytes(), [b'a'..=b'h', b'1'..=b'8'])
}

/// A token written like a move or a piece on a square: `Nf3`, `Rxe8`, `exd5`, `e8=Q`, `O-O`.
fn is_move_like(token: &str) -> bool {
    if token == "O-O" || token == "O-O-O" {
        return true;
    }
    let token = token.split('=').next().unwrap_or(token);
    let Some(square) = token.len().checked_sub(2).and_then(|at| token.get(at..)) else {
        return false;
    };
    if token.len() < 3 || !is_square(square) {
        return false;
    }
    let first = token.as_bytes()[0];
    matches!(first, b'K' | b'Q' | b'R' | b'B' | b'N')
        || (first.is_ascii_lowercase() && token.contains('x'))
}

fn see(pos: &VariantPosition, allowed: &mut Allowed) {
    let board = pos.board();
    for square in board.occupied() {
        allowed.squares.insert(square.to_string());
        if let Some(piece) = board.piece_at(square)
            && piece.role != Role::Pawn
        {
            let letter = piece.role.upper_char();
            allowed.moves.insert(format!("{letter}{square}"));
        }
    }
}

fn see_line<S: AsRef<str>>(start: &VariantPosition, line: &[S], allowed: &mut Allowed) {
    see(start, allowed);
    let mut pos = start.clone();
    for text in line {
        let Some(played) =
            rules::parse_uci(text.as_ref()).and_then(|uci| rules::legal_move(&pos, uci))
        else {
            break;
        };
        allowed.squares.insert(played.to().to_string());
        let san = rules::san_and_play(&mut pos, played);
        allowed.mate |= san.ends_with('#');
        allowed.moves.insert(bare(&san));
        see(&pos, allowed);
    }
}

fn credit(line: &LineFacts, allowed: &mut Allowed) {
    for tactic in &line.tactics {
        match tactic {
            Tactic::Fork { .. } => allowed.fork = true,
            Tactic::Pin { .. } => allowed.pin = true,
            Tactic::Hanging { .. } => {}
        }
    }
}

/// Whether `reply` keeps to `allowed`. Words that claim a tactic need that tactic in the facts;
/// skewers and discovered attacks are not detected, so they are never in them.
pub fn check_reply(reply: &str, allowed: &Allowed) -> bool {
    if reply.trim().is_empty() {
        return false;
    }
    for raw in reply.split(|c: char| !(c.is_alphanumeric() || "+#=-".contains(c))) {
        let token = bare(raw.trim_matches('-'));
        if allowed.secret.contains(&token) {
            return false;
        }
        if is_square(&token) {
            if !allowed.squares.contains(&token) {
                return false;
            }
        } else if is_move_like(&token) && !allowed.moves.contains(&token) {
            return false;
        }
        let word = token.to_lowercase();
        let claims = |stems: &[&str]| stems.iter().any(|stem| word.starts_with(stem));
        if claims(&["skewer", "discover", "deflect", "decoy", "zwischen"])
            || (!allowed.fork && claims(&["fork"]))
            || (!allowed.pin && (word == "pin" || claims(&["pins", "pinn"])))
            || (!allowed.mate && (word.contains("mate") && !claims(&["material", "ultimate"])))
        {
            return false;
        }
    }
    true
}

fn tactic_words(tactic: &Tactic, missed: bool) -> String {
    match tactic {
        Tactic::Fork { san, targets, .. } => {
            let verb = if missed { "Missed" } else { "Allows" };
            format!("{verb} {san}, forking {}.", targets.join(" and "))
        }
        Tactic::Pin {
            role,
            pinned,
            behind,
            ..
        } => {
            let verb = if missed { "Missed" } else { "Allows" };
            format!("{verb} the {role}'s pin of {pinned} to {behind}.")
        }
        Tactic::Hanging { role, san, .. } if missed => format!("Missed {san}, winning the {role}."),
        Tactic::Hanging { role, san, .. } => format!("Loses the {role} to {san}."),
    }
}

fn points(material: i32) -> String {
    let plural = if material == 1 { "" } else { "s" };
    format!("{material} point{plural} of material")
}

/// What was wrong with a move, in one line, when the facts say; nothing otherwise.
pub fn reason(facts: &MoveFacts) -> Option<String> {
    if let Some(moves) = facts.mated_in {
        return Some(format!("Allows mate in {moves}."));
    }
    if let Some(moves) = facts.missed_mate {
        return Some(format!("Missed mate in {moves}."));
    }
    if let Some(line) = &facts.refutation {
        if let Some(tactic) = line.tactics.first() {
            return Some(tactic_words(tactic, false));
        }
        if line.material > 0 {
            return Some(format!("Loses {}.", points(line.material)));
        }
    }
    let line = facts.better.as_ref()?;
    if let Some(tactic) = line.tactics.first() {
        return Some(tactic_words(tactic, true));
    }
    let first = line.san.first()?;
    (line.material > 0).then(|| format!("Missed {first}, winning {}.", points(line.material)))
}

/// A puzzle's idea, named without giving its moves away.
pub fn clue(line: &LineFacts) -> &'static str {
    match line.tactics.first() {
        Some(Tactic::Fork { .. }) => "Look for a fork.",
        Some(Tactic::Pin { .. }) => "Look for a pin.",
        Some(Tactic::Hanging { .. }) => "Something is undefended.",
        None if line.san.last().is_some_and(|san| san.ends_with('#')) => "There is a forced mate.",
        None if line.material > 0 => "There is material to win.",
        None => "Look for the most forcing move.",
    }
}

fn line_text(title: &str, line: &LineFacts) -> String {
    let tactics = line
        .tactics
        .iter()
        .map(|tactic| tactic_words(tactic, false))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{title}: {}. Material won: {}. {tactics}\n",
        line.san.join(" "),
        line.material
    )
}

/// The brief for explaining what `played` did in `fen`.
pub fn move_brief<S: AsRef<str>, T: AsRef<str>>(
    fen: &str,
    played: &str,
    best: &[S],
    refutation: &[T],
    mates: tactics::Mates,
) -> Option<(MoveFacts, Brief)> {
    let facts = tactics::move_facts(fen, played, best, refutation, mates)?;
    let start = rules::standard_from_fen(fen)?;
    let mut allowed = Allowed::default();
    see_line(&start, &[played], &mut allowed);
    see_line(&start, best, &mut allowed);
    let mut after = start;
    let chosen = rules::parse_uci(played).and_then(|uci| rules::legal_move(&after, uci))?;
    rules::san_and_play(&mut after, chosen);
    see_line(&after, refutation, &mut allowed);
    allowed.mate |= facts.mated_in.is_some() || facts.missed_mate.is_some();
    let mut input = format!("Position (FEN): {fen}\nMove played: {}\n", facts.san);
    if let Some(line) = &facts.refutation {
        credit(line, &mut allowed);
        input += &line_text("The opponent's best reply line", line);
    }
    if let Some(line) = &facts.better {
        credit(line, &mut allowed);
        input += &line_text("The better line for the player", line);
    }
    if let Some(moves) = facts.mated_in {
        input += &format!("The move allows forced mate in {moves}.\n");
    }
    if let Some(moves) = facts.missed_mate {
        input += &format!("The move missed forced mate in {moves}.\n");
    }
    input += "Explain what the player missed, why it matters, and the idea behind the better move.";
    Some((
        facts,
        Brief {
            instructions: INSTRUCTIONS,
            input,
            allowed,
        },
    ))
}

/// The move `index` of a stored review: its free reason, and the request that explains it.
pub fn review_moment(review: &Value, index: usize) -> Option<Value> {
    let moves = review["moves"].as_array()?;
    let mut start = rules::standard_from_fen(review["fen"].as_str()?)?;
    for played in moves.get(..index)? {
        let chosen =
            rules::parse_uci(played.as_str()?).and_then(|uci| rules::legal_move(&start, uci))?;
        rules::san_and_play(&mut start, chosen);
    }
    let played = moves.get(index)?.as_str()?;
    let fen = rules::make_fen(&start);
    let before = &review["evals"][index];
    let after = &review["evals"][index + 1];
    let line = |eval: &Value| -> Vec<String> {
        let pv = eval["pv"].as_array().map(|pv| {
            pv.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
        pv.filter(|pv| !pv.is_empty())
            .or_else(|| eval["best"].as_str().map(|best| vec![best.to_string()]))
            .unwrap_or_default()
    };
    let (best, refutation) = (line(before), line(after));
    let mates = tactics::Mates {
        before: before["mate"].as_f64().map(|mate| mate as i32),
        after: after["mate"].as_f64().map(|mate| mate as i32),
    };
    let facts = tactics::move_facts(&fen, played, &best, &refutation, mates)?;
    Some(json!({
        "reason": reason(&facts),
        "request": {
            "kind": "move",
            "fen": fen,
            "played": played,
            "best": best,
            "refutation": refutation,
            "mateBefore": before["mate"],
            "mateAfter": after["mate"],
        },
    }))
}

/// The brief for a hint. The model gets the idea, never the moves, and the first move and its
/// squares may not appear in what it says.
pub fn nudge_brief<S: AsRef<str>>(fen: &str, solution: &[S]) -> Option<Brief> {
    let line = tactics::puzzle_facts(fen, solution)?;
    let start = rules::standard_from_fen(fen)?;
    let mut allowed = Allowed::default();
    see(&start, &mut allowed);
    allowed.mate = line.san.last().is_some_and(|san| san.ends_with('#'));
    credit(&line, &mut allowed);
    let first = rules::parse_uci(solution.first()?.as_ref())
        .and_then(|uci| rules::legal_move(&start, uci))?;
    allowed.secret.insert(first.to().to_string());
    if let Some(from) = first.from() {
        allowed.secret.insert(from.to_string());
    }
    allowed.secret.insert(bare(line.san.first()?));
    let input = format!(
        "Position (FEN): {fen}\nThe idea: {}\nGive a hint towards this idea.",
        clue(&line),
    );
    Some(Brief {
        instructions: NUDGE_INSTRUCTIONS,
        input,
        allowed,
    })
}

/// The brief for why a puzzle's solution works.
pub fn solution_brief<S: AsRef<str>>(fen: &str, solution: &[S]) -> Option<Brief> {
    let line = tactics::puzzle_facts(fen, solution)?;
    let start = rules::standard_from_fen(fen)?;
    let mut allowed = Allowed::default();
    see_line(&start, solution, &mut allowed);
    credit(&line, &mut allowed);
    let input = format!(
        "Position (FEN): {fen}\n{}Explain why this solution works.",
        line_text("The solution", &line),
    );
    Some(Brief {
        instructions: INSTRUCTIONS,
        input,
        allowed,
    })
}

/// The brief for why `played` fails in a puzzle: the solver's move, and the engine's answer.
pub fn wrong_brief<S: AsRef<str>, T: AsRef<str>>(
    fen: &str,
    solution: &[S],
    played: &str,
    refutation: &[T],
) -> Option<Brief> {
    let (_, mut brief) = move_brief(fen, played, solution, refutation, tactics::Mates::default())?;
    brief.input = brief.input.replace(
        "Explain what the player missed, why it matters, and the idea behind the better move.",
        "Explain why the move played fails, without giving the better move away.",
    );
    let better = tactics::puzzle_facts(fen, solution)?;
    brief.allowed.secret.insert(bare(better.san.first()?));
    // The input must not hand the model the answer it may not say.
    brief.input = brief
        .input
        .lines()
        .filter(|line| !line.starts_with("The better line"))
        .collect::<Vec<_>>()
        .join("\n");
    Some(brief)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactics::Mates;

    const FORK: &str = "r3k3/pp3ppp/8/1N6/8/8/PPP2PPP/4K3 w - - 0 1";

    fn fork_brief() -> Brief {
        // White plays a quiet king move, and the best line was the fork.
        move_brief(
            FORK,
            "e1e2",
            &["b5c7", "e8d7", "c7a8"],
            &["e8d7"],
            Mates::default(),
        )
        .unwrap()
        .1
    }

    #[test]
    fn the_free_reason_names_the_tactic_that_won() {
        let (facts, _) = move_brief(
            FORK,
            "e1e2",
            &["b5c7", "e8d7", "c7a8"],
            &["e8d7"],
            Mates::default(),
        )
        .unwrap();
        assert_eq!(
            reason(&facts).as_deref(),
            Some("Missed Nc7+, forking Ra8 and Ke8.")
        );
    }

    #[test]
    fn mate_comes_before_material_and_quiet_moves_get_no_reason() {
        let mated = tactics::move_facts(
            "4k3/8/8/8/8/8/8/4K2R b - - 0 1",
            "e8d8",
            &[] as &[&str],
            &[] as &[&str],
            Mates {
                before: None,
                after: Some(2),
            },
        )
        .unwrap();
        assert_eq!(reason(&mated).as_deref(), Some("Allows mate in 2."));
        let quiet = tactics::move_facts(
            "4k3/8/8/8/8/8/8/4K2R b - - 0 1",
            "e8d8",
            &["e8e7"],
            &["e1e2"],
            Mates::default(),
        )
        .unwrap();
        assert_eq!(reason(&quiet), None);
    }

    #[test]
    fn a_reply_keeping_to_the_facts_passes() {
        let brief = fork_brief();
        assert!(check_reply(
            "You missed Nc7+, which forks the king on e8 and the Ra8. Winning the rook decides it.",
            &brief.allowed
        ));
    }

    #[test]
    fn invented_moves_squares_and_tactics_are_rejected() {
        let allowed = fork_brief().allowed;
        for reply in [
            "Bxf7+ was the winning move.",
            "The knight belonged on d4.",
            "This pins the rook.",
            "A skewer wins the rook.",
            "White had mate.",
            "   ",
        ] {
            assert!(!check_reply(reply, &allowed), "{reply}");
        }
    }

    #[test]
    fn plain_chess_words_are_no_claims() {
        let allowed = fork_brief().allowed;
        assert!(check_reply(
            "The fork wins material, and your opponent's position collapses.",
            &allowed
        ));
    }

    #[test]
    fn a_nudge_never_gives_the_answer() {
        let solution = ["b5c7", "e8d7", "c7a8"];
        let brief = nudge_brief(FORK, &solution).unwrap();
        assert!(!brief.input.contains("Nc7"), "{}", brief.input);
        assert!(check_reply("A fork is in the air.", &brief.allowed));
        assert!(!check_reply("Try Nc7.", &brief.allowed));
        assert!(!check_reply("The knight wants c7.", &brief.allowed));
        assert!(!check_reply("Move from b5.", &brief.allowed));
    }

    #[test]
    fn a_review_moment_reads_the_lines_around_a_move() {
        let review = json!({
            "fen": FORK,
            "moves": ["a2a3", "e8e7", "e1e2"],
            "evals": [
                { "cp": 0 },
                { "cp": 0 },
                { "cp": 500, "pv": ["b5c7", "e7d7", "c7a8"] },
                { "cp": 0, "pv": ["e7d7"] },
            ],
        });
        let moment = review_moment(&review, 2).unwrap();
        assert_eq!(moment["request"]["played"], "e1e2");
        assert_eq!(moment["request"]["best"][0], "b5c7");
        assert_eq!(moment["request"]["refutation"][0], "e7d7");
        assert_eq!(
            moment["reason"],
            "Missed Nc7, winning 5 points of material."
        );
        assert!(review_moment(&review, 3).is_none());
    }

    #[test]
    fn a_review_without_lines_still_has_a_moment() {
        let review = json!({ "fen": FORK, "moves": ["e1e2"], "evals": [null, null] });
        let moment = review_moment(&review, 0).unwrap();
        assert_eq!(moment["reason"], Value::Null);
        assert_eq!(moment["request"]["kind"], "move");
    }

    #[test]
    fn clues_follow_the_tactic() {
        let line = |san: &[&str], material, tactics| LineFacts {
            san: san.iter().map(ToString::to_string).collect(),
            material,
            tactics,
        };
        assert_eq!(clue(&line(&["Rh8#"], 0, vec![])), "There is a forced mate.");
        assert_eq!(
            clue(&line(&["Rh8"], 3, vec![])),
            "There is material to win."
        );
        assert_eq!(
            clue(&line(&["Rh8"], 0, vec![])),
            "Look for the most forcing move."
        );
    }

    #[test]
    fn a_wrong_puzzle_move_is_explained_without_the_solution() {
        let solution = ["b5c7", "e8d7", "c7a8"];
        let brief = wrong_brief(FORK, &solution, "e1e2", &["e8d7"]).unwrap();
        assert!(brief.input.contains("without giving the better move away"));
        assert!(!brief.input.contains("Nc7"), "{}", brief.input);
        assert!(!check_reply("Nc7 was the move.", &brief.allowed));
    }
}

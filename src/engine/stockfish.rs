use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use kchess_board::{MoveRequest, PieceKind, Square};

#[derive(Debug)]
pub enum StockfishError {
    Io(std::io::Error),
    Spawn(String),
    MissingStdin,
    ProcessFailed(String),
    BestMoveMissing,
    InvalidBestMove(String),
}

impl std::fmt::Display for StockfishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Spawn(err) => write!(f, "failed to launch stockfish: {err}"),
            Self::MissingStdin => write!(f, "stockfish stdin unavailable"),
            Self::ProcessFailed(err) => write!(f, "stockfish process failed: {err}"),
            Self::BestMoveMissing => write!(f, "stockfish did not return bestmove"),
            Self::InvalidBestMove(move_text) => write!(f, "invalid bestmove returned: {move_text}"),
        }
    }
}

impl std::error::Error for StockfishError {}

impl From<std::io::Error> for StockfishError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BestMove {
    Move(MoveRequest),
    NoMove,
}

pub fn compute_best_move(
    engine_path: &Path,
    move_history: &[MoveRequest],
    elo: u16,
) -> Result<BestMove, StockfishError> {
    let mut child = Command::new(engine_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| StockfishError::Spawn(err.to_string()))?;

    let position_command = build_position_command(move_history);

    {
        let stdin = child.stdin.as_mut().ok_or(StockfishError::MissingStdin)?;
        writeln!(stdin, "uci")?;
        writeln!(stdin, "setoption name UCI_LimitStrength value true")?;
        writeln!(stdin, "setoption name UCI_Elo value {elo}")?;
        writeln!(stdin, "isready")?;
        writeln!(stdin, "{position_command}")?;
        writeln!(stdin, "go movetime 350")?;
        writeln!(stdin, "quit")?;
        stdin.flush()?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(StockfishError::ProcessFailed(stderr.trim().to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let bestmove = parse_bestmove_line(&stdout).ok_or(StockfishError::BestMoveMissing)?;
    parse_bestmove_token(bestmove)
}

pub fn build_position_command(move_history: &[MoveRequest]) -> String {
    if move_history.is_empty() {
        return "position startpos".to_string();
    }

    let moves = move_history
        .iter()
        .map(format_move_for_uci)
        .collect::<Vec<_>>()
        .join(" ");

    format!("position startpos moves {moves}")
}

fn format_move_for_uci(request: &MoveRequest) -> String {
    let mut text = format!("{}{}", request.from.algebraic(), request.to.algebraic());
    if let Some(promotion) = request.promotion {
        text.push(match promotion {
            PieceKind::Queen => 'q',
            PieceKind::Rook => 'r',
            PieceKind::Bishop => 'b',
            PieceKind::Knight => 'n',
            PieceKind::Pawn | PieceKind::King => 'q',
        });
    }
    text
}

fn parse_bestmove_line(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("bestmove "))
}

fn parse_bestmove_token(token_and_rest: &str) -> Result<BestMove, StockfishError> {
    let token = token_and_rest
        .split_whitespace()
        .next()
        .ok_or(StockfishError::BestMoveMissing)?;

    if token == "(none)" {
        return Ok(BestMove::NoMove);
    }

    if token.len() != 4 && token.len() != 5 {
        return Err(StockfishError::InvalidBestMove(token.to_string()));
    }

    let from = Square::from_algebraic(&token[0..2])
        .ok_or_else(|| StockfishError::InvalidBestMove(token.to_string()))?;
    let to = Square::from_algebraic(&token[2..4])
        .ok_or_else(|| StockfishError::InvalidBestMove(token.to_string()))?;

    let promotion = if token.len() == 5 {
        let promo = token
            .chars()
            .nth(4)
            .ok_or_else(|| StockfishError::InvalidBestMove(token.to_string()))?;
        Some(match promo.to_ascii_lowercase() {
            'q' => PieceKind::Queen,
            'r' => PieceKind::Rook,
            'b' => PieceKind::Bishop,
            'n' => PieceKind::Knight,
            _ => return Err(StockfishError::InvalidBestMove(token.to_string())),
        })
    } else {
        None
    };

    Ok(BestMove::Move(MoveRequest {
        from,
        to,
        promotion,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_move(from: &str, to: &str) -> MoveRequest {
        MoveRequest::new(
            Square::from_algebraic(from).unwrap(),
            Square::from_algebraic(to).unwrap(),
        )
    }

    #[test]
    fn position_command_formats_moves() {
        let moves = vec![parse_move("e2", "e4"), parse_move("e7", "e5")];
        assert_eq!(
            build_position_command(&moves),
            "position startpos moves e2e4 e7e5"
        );
    }

    #[test]
    fn parse_bestmove_handles_none() {
        let parsed = parse_bestmove_token("(none) ponder 0000").unwrap();
        assert_eq!(parsed, BestMove::NoMove);
    }

    #[test]
    fn parse_bestmove_handles_promotion() {
        let parsed = parse_bestmove_token("e7e8q").unwrap();
        match parsed {
            BestMove::Move(request) => {
                assert_eq!(request.from.algebraic(), "e7");
                assert_eq!(request.to.algebraic(), "e8");
                assert_eq!(request.promotion, Some(PieceKind::Queen));
            }
            BestMove::NoMove => panic!("expected move"),
        }
    }

    #[test]
    fn parse_bestmove_line_reads_last_bestmove() {
        let stdout = "info depth 1\nbestmove e2e4\ninfo depth 2\nbestmove d2d4 ponder d7d5\n";
        let bestmove = parse_bestmove_line(stdout).unwrap();
        assert_eq!(bestmove, "d2d4 ponder d7d5");
    }
}

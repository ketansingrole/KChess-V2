use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use kchess_board::{MoveRequest, PieceKind, Square};

pub const STOCKFISH_MIN_ELO: u16 = 1320;
pub const STOCKFISH_MAX_ELO: u16 = 3190;

#[cfg(not(test))]
const UCI_OK_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const UCI_OK_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg(not(test))]
const READY_OK_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const READY_OK_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg(not(test))]
const BESTMOVE_TIMEOUT_SLACK_MS: u64 = 3_000;
#[cfg(test)]
const BESTMOVE_TIMEOUT_SLACK_MS: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineProfile {
    pub limit_strength: bool,
    pub elo: Option<u16>,
    pub movetime_ms: u64,
}

impl EngineProfile {
    pub const fn new(limit_strength: bool, elo: Option<u16>, movetime_ms: u64) -> Self {
        Self {
            limit_strength,
            elo,
            movetime_ms,
        }
    }

    pub fn clamped_elo(self) -> Option<u16> {
        self.elo.map(clamp_elo)
    }
}

#[derive(Debug)]
pub enum StockfishError {
    Io(std::io::Error),
    Spawn(String),
    MissingStdin,
    MissingStdout,
    MissingStderr,
    ProcessFailed(String),
    UciTimeout,
    ReadyTimeout,
    BestmoveTimeout,
    BestMoveMissing,
    InvalidBestMove(String),
}

impl std::fmt::Display for StockfishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Spawn(err) => write!(f, "failed to launch stockfish: {err}"),
            Self::MissingStdin => write!(f, "stockfish stdin unavailable"),
            Self::MissingStdout => write!(f, "stockfish stdout unavailable"),
            Self::MissingStderr => write!(f, "stockfish stderr unavailable"),
            Self::ProcessFailed(err) => write!(f, "stockfish process failed: {err}"),
            Self::UciTimeout => write!(f, "timed out waiting for uciok from stockfish"),
            Self::ReadyTimeout => write!(f, "timed out waiting for readyok from stockfish"),
            Self::BestmoveTimeout => write!(f, "timed out waiting for bestmove from stockfish"),
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
    profile: EngineProfile,
) -> Result<BestMove, StockfishError> {
    let mut child = Command::new(engine_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| StockfishError::Spawn(err.to_string()))?;

    let mut stdin = child.stdin.take().ok_or(StockfishError::MissingStdin)?;
    let stdout = child.stdout.take().ok_or(StockfishError::MissingStdout)?;
    let mut stderr = child.stderr.take().ok_or(StockfishError::MissingStderr)?;
    let (stdout_tx, stdout_rx) = mpsc::channel::<String>();
    let stdout_reader = thread::spawn(move || -> Result<(), std::io::Error> {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = line?;
            if stdout_tx.send(line).is_err() {
                break;
            }
        }
        Ok(())
    });

    let position_command = build_position_command(move_history);
    let bestmove_line_result = (|| {
        send_command(&mut stdin, "uci")?;
        wait_for_stdout_line(&stdout_rx, UCI_OK_TIMEOUT, uci_timeout_error, |line| {
            line.trim() == "uciok"
        })?;

        send_command(
            &mut stdin,
            if profile.limit_strength {
                "setoption name UCI_LimitStrength value true"
            } else {
                "setoption name UCI_LimitStrength value false"
            },
        )?;
        if let Some(elo) = profile.clamped_elo() {
            send_command(&mut stdin, &format!("setoption name UCI_Elo value {elo}"))?;
        }

        send_command(&mut stdin, "isready")?;
        wait_for_stdout_line(&stdout_rx, READY_OK_TIMEOUT, ready_timeout_error, |line| {
            line.trim() == "readyok"
        })?;

        send_command(&mut stdin, &position_command)?;
        send_command(
            &mut stdin,
            &format!("go movetime {}", profile.movetime_ms.max(1)),
        )?;

        let bestmove_line = wait_for_stdout_line(
            &stdout_rx,
            bestmove_timeout(profile.movetime_ms),
            bestmove_timeout_error,
            |line| line.starts_with("bestmove "),
        )?;

        send_command(&mut stdin, "quit")?;
        Ok::<String, StockfishError>(bestmove_line)
    })();

    if let Err(err) = bestmove_line_result {
        terminate_process(&mut child);
        let _ = stdout_reader.join();
        return Err(err);
    }

    drop(stdin);
    let status = child.wait()?;
    let mut stderr_text = String::new();
    let _ = stderr.read_to_string(&mut stderr_text);

    match stdout_reader.join() {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(StockfishError::Io(err)),
        Err(_) => {
            return Err(StockfishError::ProcessFailed(
                "stockfish stdout reader thread panicked".to_string(),
            ));
        }
    }

    if !status.success() {
        let stderr = stderr_text.trim();
        let reason = if stderr.is_empty() {
            "unknown process failure".to_string()
        } else {
            stderr.to_string()
        };
        return Err(StockfishError::ProcessFailed(reason));
    }

    let bestmove_line = bestmove_line_result?;
    let bestmove = bestmove_line
        .strip_prefix("bestmove ")
        .ok_or(StockfishError::BestMoveMissing)?;
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

fn send_command(stdin: &mut ChildStdin, command: &str) -> Result<(), StockfishError> {
    writeln!(stdin, "{command}")?;
    stdin.flush()?;
    Ok(())
}

fn wait_for_stdout_line<F>(
    stdout_rx: &Receiver<String>,
    timeout: Duration,
    timeout_error: fn() -> StockfishError,
    mut predicate: F,
) -> Result<String, StockfishError>
where
    F: FnMut(&str) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(timeout_error());
        }

        let remaining = deadline.saturating_duration_since(now);
        match stdout_rx.recv_timeout(remaining) {
            Ok(line) => {
                if predicate(&line) {
                    return Ok(line);
                }
            }
            Err(RecvTimeoutError::Timeout) => return Err(timeout_error()),
            Err(RecvTimeoutError::Disconnected) => return Err(timeout_error()),
        }
    }
}

fn terminate_process(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn clamp_elo(elo: u16) -> u16 {
    elo.clamp(STOCKFISH_MIN_ELO, STOCKFISH_MAX_ELO)
}

fn bestmove_timeout(movetime_ms: u64) -> Duration {
    Duration::from_millis(
        movetime_ms
            .saturating_add(BESTMOVE_TIMEOUT_SLACK_MS)
            .max(250),
    )
}

fn uci_timeout_error() -> StockfishError {
    StockfishError::UciTimeout
}

fn ready_timeout_error() -> StockfishError {
    StockfishError::ReadyTimeout
}

fn bestmove_timeout_error() -> StockfishError {
    StockfishError::BestmoveTimeout
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
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn clamp_elo_bounds() {
        assert_eq!(clamp_elo(1000), STOCKFISH_MIN_ELO);
        assert_eq!(clamp_elo(1320), 1320);
        assert_eq!(clamp_elo(2200), 2200);
        assert_eq!(clamp_elo(5000), STOCKFISH_MAX_ELO);
    }

    #[cfg(unix)]
    #[test]
    fn compute_best_move_clamps_elo_and_reads_bestmove() {
        let script = TempEngineScript::new(
            "clamp-and-bestmove",
            r#"saw_clamped_elo=0
while IFS= read -r line; do
  case "$line" in
    "uci")
      echo "id name fake"
      echo "uciok"
      ;;
    "setoption name UCI_Elo value 1320")
      saw_clamped_elo=1
      ;;
    "isready")
      echo "readyok"
      ;;
    go\ movetime*)
      if [ "$saw_clamped_elo" -eq 1 ]; then
        echo "bestmove e2e4"
      else
        echo "bestmove d2d4"
      fi
      ;;
    "quit")
      exit 0
      ;;
  esac
done"#,
        );

        let result =
            compute_best_move(script.path(), &[], EngineProfile::new(true, Some(1000), 20))
                .expect("best move");

        match result {
            BestMove::Move(request) => {
                assert_eq!(request.from.algebraic(), "e2");
                assert_eq!(request.to.algebraic(), "e4");
            }
            BestMove::NoMove => panic!("expected move"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn compute_best_move_times_out_waiting_for_uciok() {
        let script = TempEngineScript::new(
            "missing-uciok",
            r#"while IFS= read -r line; do
  case "$line" in
    "quit")
      exit 0
      ;;
  esac
done"#,
        );

        let result =
            compute_best_move(script.path(), &[], EngineProfile::new(true, Some(1800), 20));
        assert!(matches!(result, Err(StockfishError::UciTimeout)));
    }

    #[cfg(unix)]
    #[test]
    fn compute_best_move_times_out_waiting_for_readyok() {
        let script = TempEngineScript::new(
            "missing-readyok",
            r#"while IFS= read -r line; do
  case "$line" in
    "uci")
      echo "uciok"
      ;;
    "quit")
      exit 0
      ;;
  esac
done"#,
        );

        let result =
            compute_best_move(script.path(), &[], EngineProfile::new(true, Some(1800), 20));
        assert!(matches!(result, Err(StockfishError::ReadyTimeout)));
    }

    #[cfg(unix)]
    #[test]
    fn compute_best_move_times_out_waiting_for_bestmove() {
        let script = TempEngineScript::new(
            "missing-bestmove",
            r#"while IFS= read -r line; do
  case "$line" in
    "uci")
      echo "uciok"
      ;;
    "isready")
      echo "readyok"
      ;;
    "quit")
      exit 0
      ;;
  esac
done"#,
        );

        let result =
            compute_best_move(script.path(), &[], EngineProfile::new(true, Some(1800), 20));
        assert!(matches!(result, Err(StockfishError::BestmoveTimeout)));
    }

    #[cfg(unix)]
    struct TempEngineScript {
        path: PathBuf,
    }

    #[cfg(unix)]
    impl TempEngineScript {
        fn new(stem: &str, body: &str) -> Self {
            let path = unique_script_path(stem);
            let script = format!("#!/bin/sh\n{body}\n");
            fs::write(&path, script).expect("write fake engine script");
            let mut perms = fs::metadata(&path).expect("script metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).expect("set fake engine script executable");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    #[cfg(unix)]
    impl Drop for TempEngineScript {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[cfg(unix)]
    fn unique_script_path(stem: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "kchess-stockfish-test-{stem}-{}-{nanos}.sh",
            std::process::id()
        ))
    }
}

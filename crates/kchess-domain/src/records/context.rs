//! The key an analysis is cached under.

/// The position a line starts from and its moves: an analysis of the same position reached
/// another way is another analysis, since repetitions differ.
pub fn analysis_context<S: AsRef<str>>(root_fen: &str, moves: &[S]) -> String {
    let moves: Vec<&str> = moves.iter().map(AsRef::as_ref).collect();
    format!("{root_fen}|{}", moves.join(" "))
}

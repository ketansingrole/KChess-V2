//! A port of chessops' `PgnParser` (non-streaming, unbounded budget, as `parsePgn` runs it):
//! the same headers, comments, NAGs, variations and game boundaries.

use crate::js;
use crate::rules;

/// A move token kept inline: tokens are at most ten bytes, so no node allocates for its move.
#[derive(Clone, Copy, Default)]
pub struct San {
    bytes: [u8; 16],
    len: u8,
}

impl San {
    fn new(text: &str) -> San {
        let mut san = San::default();
        let len = text.len().min(16);
        san.bytes[..len].copy_from_slice(&text.as_bytes()[..len]);
        san.len = len as u8;
        san
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }
}

impl std::fmt::Debug for San {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

/// One move of the game tree. Children are linked first-child/next-sibling, in the order
/// they were written (the first is the main line).
#[derive(Debug, Default, Clone)]
pub struct PgnNode {
    pub san: San,
    pub comments: Option<Vec<String>>,
    pub starting_comments: Option<Vec<String>>,
    pub nags: Option<Vec<u32>>,
    first_child: Option<usize>,
    last_child: Option<usize>,
    next_sibling: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Game {
    /// Insertion-ordered, as a JavaScript `Map`.
    pub headers: Vec<(String, String)>,
    pub comments: Option<Vec<String>>,
    /// Arena of move nodes; index 0 is the root (no move).
    pub nodes: Vec<PgnNode>,
}

impl Game {
    fn new() -> Game {
        Game {
            headers: [
                ("Event", "?"),
                ("Site", "?"),
                ("Date", "????.??.??"),
                ("Round", "?"),
                ("White", "?"),
                ("Black", "?"),
                ("Result", "*"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
            comments: None,
            nodes: vec![PgnNode::default()],
        }
    }

    /// The moves played from `node`, main line first.
    pub fn children(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.nodes[node].first_child, |&child| {
            self.nodes[child].next_sibling
        })
    }

    fn append_child(&mut self, parent: usize, child: usize) {
        match self.nodes[parent].last_child {
            Some(last) => self.nodes[last].next_sibling = Some(child),
            None => self.nodes[parent].first_child = Some(child),
        }
        self.nodes[parent].last_child = Some(child);
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// `headers.set(name, value)`: an existing header keeps its place.
    pub fn put_header(&mut self, name: &str, value: String) {
        match self.headers.iter_mut().find(|(k, _)| k == name) {
            Some(entry) => entry.1 = value,
            None => self.headers.push((name.to_string(), value)),
        }
    }

    /// `headers.delete(name)`
    pub fn remove_header(&mut self, name: &str) {
        self.headers.retain(|(k, _)| k != name);
    }

    fn set_header(&mut self, name: &str, value: String) {
        let value = if name == "Result" {
            outcome(&value).to_string()
        } else {
            value
        };
        match self.headers.iter_mut().find(|(k, _)| k == name) {
            Some(entry) => entry.1 = value,
            None => self.headers.push((name.to_string(), value)),
        }
    }
}

/// `makeOutcome(parseOutcome(value))`
fn outcome(value: &str) -> &'static str {
    match value {
        "1-0" | "1–0" | "1—0" => "1-0",
        "0-1" | "0–1" | "0—1" => "0-1",
        "1/2-1/2" | "1/2–1/2" | "1/2—1/2" => "1/2-1/2",
        _ => "*",
    }
}

struct Frame {
    parent: usize,
    root: bool,
    node: Option<usize>,
    starting_comments: Option<Vec<String>>,
}

#[derive(PartialEq, Clone, Copy)]
enum State {
    Bom,
    Pre,
    Headers,
    Moves,
    Comment,
}

/* ── Lexing: hand-written equivalents of chessops's two PGN regexes (checked against the
regexes themselves in the tests below) ── */

/// chessops's header pattern, matched at the start of `line`:
/// `^\s*\[([A-Za-z0-9][A-Za-z0-9_+#=:-]*)\s+"((?:[^"\\]|\\"|\\\\)*)"\]`.
/// Returns the name, the raw (still escaped) value and where the match ends.
fn match_header(line: &str) -> Option<(&str, &str, usize)> {
    let start = line.len() - line.trim_start_matches(js::is_space).len();
    let b = line.as_bytes();
    if b.get(start) != Some(&b'[') {
        return None;
    }
    let name_start = start + 1;
    if !b.get(name_start)?.is_ascii_alphanumeric() {
        return None;
    }
    let mut i = name_start + 1;
    while b
        .get(i)
        .is_some_and(|&c| c.is_ascii_alphanumeric() || b"_+#=:-".contains(&c))
    {
        i += 1;
    }
    let name_end = i;
    let spaces = line[i..].len() - line[i..].trim_start_matches(js::is_space).len();
    if spaces == 0 {
        return None;
    }
    i += spaces;
    if b.get(i) != Some(&b'"') {
        return None;
    }
    let value_start = i + 1;
    i = value_start;
    loop {
        match *b.get(i)? {
            b'"' => break,
            b'\\' => match b.get(i + 1) {
                Some(b'"' | b'\\') => i += 2,
                _ => return None,
            },
            _ => i += 1,
        }
    }
    (b.get(i + 1) == Some(&b']'))
        .then(|| (&line[name_start..name_end], &line[value_start..i], i + 2))
}

fn file_at(b: &[u8], i: usize) -> bool {
    b.get(i).is_some_and(|c| (b'a'..=b'h').contains(c))
}

fn rank_at(b: &[u8], i: usize) -> bool {
    b.get(i).is_some_and(|c| (b'1'..=b'8').contains(c))
}

fn byte_in(b: &[u8], i: usize, set: &[u8]) -> bool {
    b.get(i).is_some_and(|c| set.contains(c))
}

/// Length of `[-–—]` at `i`.
fn dash_at(b: &[u8], i: usize) -> Option<usize> {
    match b.get(i..) {
        Some([b'-', ..]) => Some(1),
        Some([0xE2, 0x80, 0x93 | 0x94, ..]) => Some(3),
        _ => None,
    }
}

/// `[+#]?` after a move.
fn check_suffix(b: &[u8], i: usize) -> usize {
    if byte_in(b, i, b"+#") { i + 1 } else { i }
}

/// `[NBKRQ]?[a-h]?[1-8]?[-x]?[a-h][1-8](?:=?[nbrqkNBRQK])?[+#]?`, trying optional parts in the
/// regex's backtracking order.
fn match_san(b: &[u8], p: usize) -> Option<usize> {
    const PROMOTION: &[u8] = b"nbrqkNBRQK";
    let choices = |present: bool| {
        if present {
            &[true, false][..]
        } else {
            &[false][..]
        }
    };
    for &piece in choices(byte_in(b, p, b"NBKRQ")) {
        let i0 = p + usize::from(piece);
        for &file in choices(file_at(b, i0)) {
            let i1 = i0 + usize::from(file);
            for &rank in choices(rank_at(b, i1)) {
                let i2 = i1 + usize::from(rank);
                for &separator in choices(byte_in(b, i2, b"-x")) {
                    let i3 = i2 + usize::from(separator);
                    if file_at(b, i3) && rank_at(b, i3 + 1) {
                        let mut end = i3 + 2;
                        if b.get(end) == Some(&b'=') && byte_in(b, end + 1, PROMOTION) {
                            end += 2;
                        } else if byte_in(b, end, PROMOTION) {
                            end += 1;
                        }
                        return Some(check_suffix(b, end));
                    }
                }
            }
        }
    }
    None
}

/// The next move-text token at or after `pos`, as chessops's token regex finds it.
fn next_token(line: &str, pos: usize) -> Option<(usize, usize)> {
    let b = line.as_bytes();
    let castle_letter = |i: usize| byte_in(b, i, b"O0o");
    for p in pos..b.len() {
        let c = b[p];
        if !(c.is_ascii_alphanumeric() || b"-@{;$?!()*".contains(&c)) {
            continue;
        }
        if let Some(end) = match_san(b, p) {
            return Some((p, end));
        }
        // [pnbrqkPNBRQK]?@[a-h][1-8][+#]?
        let at = if byte_in(b, p, b"pnbrqkPNBRQK") && b.get(p + 1) == Some(&b'@') {
            Some(p + 2)
        } else if c == b'@' {
            Some(p + 1)
        } else {
            None
        };
        if let Some(i) = at.filter(|&i| file_at(b, i) && rank_at(b, i + 1)) {
            return Some((p, check_suffix(b, i + 2)));
        }
        // [O0o][-–—][O0o](?:[-–—][O0o])?[+#]?
        if castle_letter(p)
            && let Some(d) = dash_at(b, p + 1)
            && castle_letter(p + 1 + d)
        {
            let mut end = p + 2 + d;
            if let Some(d2) = dash_at(b, end)
                && castle_letter(end + d2)
            {
                end += d2 + 1;
            }
            return Some((p, check_suffix(b, end)));
        }
        let rest = &b[p..];
        for literal in [&b"--"[..], b"Z0", b"0000", b"@@@@", b"{", b";"] {
            if rest.starts_with(literal) {
                return Some((p, p + literal.len()));
            }
        }
        if c == b'$' {
            let digits = rest[1..]
                .iter()
                .take(4)
                .take_while(|d| d.is_ascii_digit())
                .count();
            if digits > 0 {
                return Some((p, p + 1 + digits));
            }
        }
        if c == b'?' || c == b'!' {
            return Some((p, p + if byte_in(b, p + 1, b"?!") { 2 } else { 1 }));
        }
        if b"()*".contains(&c) {
            return Some((p, p + 1));
        }
        // 1[-–—]0 | 0[-–—]1 | 1/2[-–—]1/2
        if let Some(d) = dash_at(b, p + 1)
            && ((c == b'1' && b.get(p + 1 + d) == Some(&b'0'))
                || (c == b'0' && b.get(p + 1 + d) == Some(&b'1')))
        {
            return Some((p, p + 2 + d));
        }
        if rest.starts_with(b"1/2")
            && let Some(d) = dash_at(b, p + 3)
            && b.get(p + 3 + d..p + 6 + d) == Some(&b"1/2"[..])
        {
            return Some((p, p + 6 + d));
        }
    }
    None
}

struct Parser {
    games: Vec<Game>,
    found: bool,
    state: State,
    game: Game,
    stack: Vec<Frame>,
    comment_buf: Vec<String>,
}

impl Parser {
    fn new() -> Parser {
        Parser {
            games: Vec::new(),
            found: false,
            state: State::Bom,
            game: Game::new(),
            stack: vec![Frame {
                parent: 0,
                root: true,
                node: None,
                starting_comments: None,
            }],
            comment_buf: Vec::new(),
        }
    }

    fn reset_game(&mut self) {
        self.found = false;
        self.state = State::Pre;
        self.game = Game::new();
        self.stack = vec![Frame {
            parent: 0,
            root: true,
            node: None,
            starting_comments: None,
        }];
        self.comment_buf.clear();
    }

    fn handle_line(&mut self, line: &str) {
        let mut fresh_line = true;
        // Every rewrite below keeps a suffix of the line, so it never needs copying.
        let mut line = line;
        'continued: loop {
            if self.state == State::Bom {
                if let Some(rest) = line.strip_prefix('\u{FEFF}') {
                    line = rest;
                }
                self.state = State::Pre;
            }
            if self.state == State::Pre {
                if js::is_blank(line) || line.starts_with('%') {
                    return;
                }
                self.found = true;
                self.state = State::Headers;
            }
            if self.state == State::Headers {
                if line.starts_with('%') {
                    return;
                }
                while let Some((name, raw, end)) = match_header(line) {
                    let value = raw.replace("\\\"", "\"").replace("\\\\", "\\");
                    self.game.set_header(name, value);
                    line = &line[end..];
                    fresh_line = false;
                }
                if js::is_blank(line) {
                    return;
                }
                self.state = State::Moves;
            }
            if self.state == State::Moves {
                if fresh_line {
                    if line.starts_with('%') {
                        return;
                    }
                    if js::is_blank(line) {
                        return self.emit();
                    }
                }
                let mut pos = 0;
                while let Some((start, end)) = next_token(line, pos) {
                    pos = end;
                    let token = &line[start..end];
                    match token {
                        ";" => return,
                        "!" => self.handle_nag(1),
                        "?" => self.handle_nag(2),
                        "!!" => self.handle_nag(3),
                        "??" => self.handle_nag(4),
                        "!?" => self.handle_nag(5),
                        "?!" => self.handle_nag(6),
                        _ if token.starts_with('$') => {
                            self.handle_nag(token[1..].parse().unwrap_or(0));
                        }
                        "1-0" | "1–0" | "1—0" | "0-1" | "0–1" | "0—1" | "1/2-1/2" | "1/2–1/2"
                        | "1/2—1/2" | "*" => {
                            if self.stack.len() == 1 && token != "*" {
                                self.game.set_header("Result", token.to_string());
                            }
                        }
                        "(" => {
                            let parent = self.stack.last().map_or(0, |f| f.parent);
                            self.stack.push(Frame {
                                parent,
                                root: false,
                                node: None,
                                starting_comments: None,
                            });
                        }
                        ")" => {
                            if self.stack.len() > 1 {
                                self.stack.pop();
                            }
                        }
                        "{" => {
                            let open = end;
                            let begin = if line.as_bytes().get(open) == Some(&b' ') {
                                open + 1
                            } else {
                                open
                            };
                            line = &line[begin..];
                            self.state = State::Comment;
                            continue 'continued;
                        }
                        _ => {
                            let san = if token.starts_with(['O', '0', 'o']) {
                                San::new(&token.replace(['0', 'o'], "O").replace(['–', '—'], "-"))
                            } else if matches!(token, "Z0" | "0000" | "@@@@") {
                                San::new("--")
                            } else {
                                San::new(token)
                            };
                            let frame = self.stack.last_mut().expect("frame");
                            if let Some(node) = frame.node {
                                frame.parent = node;
                            }
                            let index = self.game.nodes.len();
                            self.game.nodes.push(PgnNode {
                                san,
                                starting_comments: frame.starting_comments.take(),
                                ..PgnNode::default()
                            });
                            frame.node = Some(index);
                            frame.root = false;
                            let parent = frame.parent;
                            self.game.append_child(parent, index);
                        }
                    }
                }
                return;
            }
            if self.state == State::Comment {
                match line.find('}') {
                    None => {
                        self.comment_buf.push(line.to_string());
                        return;
                    }
                    Some(close) => {
                        let end = if close > 0 && line.as_bytes()[close - 1] == b' ' {
                            close - 1
                        } else {
                            close
                        };
                        self.comment_buf.push(line[..end].to_string());
                        self.handle_comment();
                        line = &line[close..];
                        self.state = State::Moves;
                        fresh_line = false;
                    }
                }
            }
        }
    }

    fn handle_nag(&mut self, nag: u32) {
        if let Some(node) = self.stack.last().and_then(|f| f.node) {
            self.game.nodes[node]
                .nags
                .get_or_insert_with(Vec::new)
                .push(nag);
        }
    }

    fn handle_comment(&mut self) {
        let comment = self.comment_buf.join("\n");
        self.comment_buf.clear();
        let frame = self.stack.last_mut().expect("frame");
        if let Some(node) = frame.node {
            self.game.nodes[node]
                .comments
                .get_or_insert_with(Vec::new)
                .push(comment);
        } else if frame.root {
            self.game
                .comments
                .get_or_insert_with(Vec::new)
                .push(comment);
        } else {
            frame
                .starting_comments
                .get_or_insert_with(Vec::new)
                .push(comment);
        }
    }

    fn emit(&mut self) {
        if self.state == State::Comment {
            self.handle_comment();
        }
        if self.found {
            let game = std::mem::replace(&mut self.game, Game::new());
            self.games.push(game);
        }
        self.reset_game();
    }
}

/* ── Writing (chessops `makePgn`) ── */

/// Move text built token by token, joined by single spaces as `tokens.join(' ')` would.
struct Tokens {
    text: String,
    first: bool,
}

impl Tokens {
    fn push(&mut self, token: &str) {
        if !self.first {
            self.text.push(' ');
        }
        self.first = false;
        self.text.push_str(token);
    }

    /// `{`, the comment without `}`, `}` as three tokens.
    fn comment(&mut self, comment: &str) {
        self.push("{");
        self.push("");
        self.text.extend(comment.chars().filter(|&c| c != '}'));
        self.push("}");
    }
}

enum Step {
    Pre,
    Sidelines,
    End,
}

struct WriteFrame {
    step: Step,
    ply: u32,
    node: usize,
    /// The next sideline to write (a later sibling of `node`).
    sidelines: Option<usize>,
    in_variation: bool,
}

/// chessops `makePgn`: headers, comments, NAGs and variations exactly as chessops writes them.
pub fn make_pgn(game: &Game) -> String {
    let mut out = String::with_capacity(64 + game.nodes.len() * 8);
    if !game.headers.is_empty() {
        for (key, value) in &game.headers {
            out.push('[');
            out.push_str(key);
            out.push_str(" \"");
            for c in value.chars() {
                if c == '\\' || c == '"' {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push_str("\"]\n");
        }
        out.push('\n');
    }
    let mut tokens = Tokens {
        text: out,
        first: true,
    };
    for comment in game.comments.iter().flatten() {
        tokens.comment(comment);
    }
    let initial_ply = game
        .header("FEN")
        .filter(|fen| !fen.is_empty())
        .and_then(rules::parse_fen)
        .map_or(0, |setup| {
            (setup.fullmoves.get() - 1) * 2 + u32::from(!setup.turn.is_white())
        });
    let mut stack: Vec<WriteFrame> = Vec::new();
    if let Some(first) = game.nodes[0].first_child {
        stack.push(WriteFrame {
            step: Step::Pre,
            ply: initial_ply,
            node: first,
            sidelines: game.nodes[first].next_sibling,
            in_variation: false,
        });
    }
    let mut force_number = true;
    while let Some(frame) = stack.last_mut() {
        if frame.in_variation {
            tokens.push(")");
            frame.in_variation = false;
            force_number = true;
        }
        if let Step::Pre = frame.step {
            let data = &game.nodes[frame.node];
            for comment in data.starting_comments.iter().flatten() {
                tokens.comment(comment);
                force_number = true;
            }
            if force_number || frame.ply % 2 == 0 {
                tokens.push("");
                let number = frame.ply / 2 + 1;
                tokens.text.push_str(itoa(number).as_str());
                tokens
                    .text
                    .push_str(if frame.ply % 2 == 1 { "..." } else { "." });
                force_number = false;
            }
            tokens.push(data.san.as_str());
            for nag in data.nags.iter().flatten() {
                tokens.push("$");
                tokens.text.push_str(itoa(*nag).as_str());
                force_number = true;
            }
            for comment in data.comments.iter().flatten() {
                tokens.comment(comment);
            }
            frame.step = Step::Sidelines;
        }
        match frame.step {
            Step::Sidelines => match frame.sidelines {
                None => {
                    frame.step = Step::End;
                    let (node, ply) = (frame.node, frame.ply);
                    if let Some(first) = game.nodes[node].first_child {
                        stack.push(WriteFrame {
                            step: Step::Pre,
                            ply: ply + 1,
                            node: first,
                            sidelines: game.nodes[first].next_sibling,
                            in_variation: false,
                        });
                    }
                }
                Some(child) => {
                    frame.sidelines = game.nodes[child].next_sibling;
                    tokens.push("(");
                    force_number = true;
                    frame.in_variation = true;
                    let ply = frame.ply;
                    stack.push(WriteFrame {
                        step: Step::Pre,
                        ply,
                        node: child,
                        sidelines: None,
                        in_variation: false,
                    });
                }
            },
            Step::End => {
                stack.pop();
            }
            Step::Pre => unreachable!("handled above"),
        }
    }
    tokens.push(outcome(game.header("Result").unwrap_or("")));
    let mut out = tokens.text;
    out.push('\n');
    out
}

/// Decimal text of a small number without a formatting call.
fn itoa(mut n: u32) -> String {
    let mut digits = [0u8; 10];
    let mut at = digits.len();
    loop {
        at -= 1;
        digits[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    String::from_utf8_lossy(&digits[at..]).into_owned()
}

/// `studyDocumentPgn`: every chapter as one PGN named by a `ChapterName` header, or None when
/// a chapter holds no game (the TypeScript function throws there).
pub fn study_document_pgn<S: AsRef<str>>(chapters: &[(S, S)]) -> Option<String> {
    let mut parts = Vec::with_capacity(chapters.len());
    for (name, pgn) in chapters {
        let mut game = parse_pgn(pgn.as_ref()).into_iter().next()?;
        game.put_header("ChapterName", name.as_ref().to_string());
        parts.push(make_pgn(&game));
    }
    Some(parts.join("\n\n"))
}

/// `studyContent(studyDocumentPgn(chapters)) === studyContent(downloaded)`: whether a study
/// still matches the copy downloaded from Lichess. None when a chapter holds no game.
pub fn study_matches<S: AsRef<str>>(chapters: &[(S, S)], downloaded: &str) -> Option<bool> {
    Some(study_content(&study_document_pgn(chapters)?) == study_content(downloaded))
}

/// `studyContent`: the moves and annotations, without headers Lichess adds or rewrites.
pub fn study_content(pgn: &str) -> String {
    parse_pgn(pgn)
        .into_iter()
        .map(|mut game| {
            game.remove_header("Site");
            game.remove_header("ChapterName");
            make_pgn(&game)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// chessops `parsePgn`.
pub fn parse_pgn(pgn: &str) -> Vec<Game> {
    let mut parser = Parser::new();
    let mut rest = pgn;
    while let Some(nl) = rest.find('\n') {
        let line = &rest[..nl];
        let line = line.strip_suffix('\r').unwrap_or(line);
        parser.handle_line(line);
        rest = &rest[nl + 1..];
    }
    parser.handle_line(rest);
    parser.emit();
    parser.games
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    const SPACE: &str = r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";

    /// chessops's own patterns: the hand-written lexer must agree with them everywhere.
    fn header_regex() -> Regex {
        Regex::new(&format!(
            r#"^{SPACE}*\[([A-Za-z0-9][A-Za-z0-9_+#=:-]*){SPACE}+"((?:[^"\\]|\\"|\\\\)*)"\]"#
        ))
        .unwrap()
    }

    fn token_regex() -> Regex {
        Regex::new(concat!(
            r"(?:[NBKRQ]?[a-h]?[1-8]?[-x]?[a-h][1-8](?:=?[nbrqkNBRQK])?|[pnbrqkPNBRQK]?@[a-h][1-8]|[O0o][-–—][O0o](?:[-–—][O0o])?)[+#]?",
            r"|--|Z0|0000|@@@@|\{|;|\$[0-9]{1,4}|[?!]{1,2}|\(|\)|\*|1[-–—]0|0[-–—]1|1/2[-–—]1/2"
        ))
        .unwrap()
    }

    #[test]
    fn lexer_agrees_with_the_chessops_patterns() {
        let header = header_regex();
        let token = token_regex();
        let alphabet: Vec<char> =
            "NBKRQabcdefgh12345678xX=+#-–—O0oZ@{};$?!()*/ \"[]\u{A0}é\\tnbrqkpPEv"
                .chars()
                .collect();
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for _ in 0..200_000 {
            let len = (next() % 24) as usize;
            let text: String = (0..len)
                .map(|_| alphabet[(next() as usize) % alphabet.len()])
                .collect();
            let expected: Vec<(usize, usize)> = token
                .find_iter(&text)
                .map(|m| (m.start(), m.end()))
                .collect();
            let mut actual = Vec::new();
            let mut pos = 0;
            while let Some((start, end)) = next_token(&text, pos) {
                actual.push((start, end));
                pos = end;
            }
            assert_eq!(actual, expected, "tokens of {text:?}");
            let expected = header
                .captures(&text)
                .map(|c| (c[1].to_string(), c[2].to_string(), c.get(0).unwrap().end()));
            let actual = match_header(&text).map(|(n, v, e)| (n.to_string(), v.to_string(), e));
            assert_eq!(actual, expected, "header of {text:?}");
        }
        for text in [
            "[Event \"a\\\"b\"]",
            "  [A_1 \"x\\\\\"] rest",
            "[Event\u{A0}\"v\"]",
            "[E \"x\\\"]",
        ] {
            let expected = header.captures(text).map(|c| c.get(0).unwrap().end());
            assert_eq!(match_header(text).map(|h| h.2), expected, "{text:?}");
        }
    }

    #[test]
    fn parses_variations_comments_and_nags() {
        let games = parse_pgn(
            "[Event \"Test\"]\n\n{ start } 1. e4 $1 { best } (1. d4 { queen } d5) e5 2. Nf3 *\n",
        );
        assert_eq!(games.len(), 1);
        let game = &games[0];
        assert_eq!(game.header("Event"), Some("Test"));
        assert_eq!(game.comments.as_deref(), Some(&["start".to_string()][..]));
        let children: Vec<usize> = game.children(0).collect();
        assert_eq!(children.len(), 2);
        let e4 = &game.nodes[children[0]];
        assert_eq!(e4.san.as_str(), "e4");
        assert_eq!(e4.nags.as_deref(), Some(&[1][..]));
        assert_eq!(e4.comments.as_deref(), Some(&["best".to_string()][..]));
        assert_eq!(game.nodes[children[1]].san.as_str(), "d4");
    }

    #[test]
    fn blank_line_after_moves_ends_the_game() {
        assert_eq!(parse_pgn("1. e4 e5\n\n1. d4 d5").len(), 2);
    }
}

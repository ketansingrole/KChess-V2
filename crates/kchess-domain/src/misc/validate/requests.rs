//! Validators of request objects: each decodes one into a typed struct and checks its fields.

use super::*;
use crate::variant::Variant;

/// The variant a request names when it is not played by standard rules; an unknown one is
/// refused.
fn other_variant(key: Option<&str>) -> Checked<Option<Variant>> {
    match key.map(Variant::from_key) {
        None => Ok(None),
        Some(None) => Err("Invalid variant.".into()),
        Some(Some(variant)) => Ok((!variant.standard_rules()).then_some(variant)),
    }
}

/// A theme file: an id, a name and at least one palette.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct Theme {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    light: Option<Palette>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dark: Option<Palette>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub(super) struct Palette {
    bg: String,
    text: String,
    primary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    muted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    elevated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accented: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    border: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_muted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    success: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    info: Option<String>,
}

pub(super) const HEX: &str = "Colors must be hex, like #1a2b3c.";

impl Palette {
    fn check(&self) -> Checked<()> {
        let optional = [
            &self.muted,
            &self.elevated,
            &self.accented,
            &self.border,
            &self.text_muted,
            &self.success,
            &self.warning,
            &self.error,
            &self.info,
        ];
        [&self.bg, &self.text, &self.primary]
            .into_iter()
            .chain(optional.into_iter().flatten())
            .try_for_each(|color| matches(color, patterns::hex_color, Some(HEX), "", ""))
    }
}

pub(super) fn assert_theme(value: J) -> Checked<Theme> {
    let mut theme: Theme = decode(value, |keys| match keys {
        [] => Some("A theme file must be a JSON object."),
        ["id"] => Some("A theme needs an \"id\"."),
        ["name"] => Some("A theme needs a \"name\"."),
        ["light" | "dark", _] => Some(HEX),
        _ => None,
    })?;
    let id = Some("The \"id\" must be 1–32 characters: a–z, 0–9 and dashes.");
    matches(&theme.id, patterns::theme_id, id, "", "")?;
    theme.name = js::trim(&theme.name).to_owned();
    min_chars(&theme.name, 1, Some("A theme needs a \"name\"."), "")?;
    max_chars(&theme.name, 40, Some("The \"name\" is too long."), "")?;
    for palette in theme.light.iter().chain(&theme.dark) {
        palette.check()?;
    }
    let palette = Some("Add a \"dark\" or \"light\" palette.");
    ensure(
        theme.light.is_some() || theme.dark.is_some(),
        palette,
        "",
        String::new,
    )?;
    Ok(theme)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct PuzzleRequest {
    account: String,
    angle: String,
    difficulty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

pub(super) fn assert_puzzle_request(value: J) -> Checked<PuzzleRequest> {
    let mut request: PuzzleRequest = decode(value, |keys| match keys {
        ["account"] => Some("Invalid account."),
        ["angle"] => Some("Invalid puzzle theme."),
        ["difficulty"] => Some("Invalid difficulty."),
        ["color"] => Some("Invalid color."),
        _ => Some("Invalid puzzle request."),
    })?;
    if !request.account.is_empty() {
        request.account = username(&request.account, "")?;
    }
    angle(&request.angle)?;
    let difficulty = Some("Invalid difficulty.");
    one_of(&request.difficulty, PUZZLE_DIFFICULTIES, difficulty, "")?;
    if let Some(color) = &request.color {
        one_of(color, &["white", "black"], Some("Invalid color."), "")?;
    }
    Ok(request)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct PuzzleSolve {
    account: String,
    angle: String,
    id: String,
    win: bool,
    rated: bool,
}

pub(super) fn assert_puzzle_solve(value: J) -> Checked<PuzzleSolve> {
    let mut solve: PuzzleSolve = decode(value, |keys| match keys {
        ["account"] => Some(USERNAME),
        ["angle"] => Some("Invalid puzzle theme."),
        ["id"] => Some("Invalid puzzle."),
        _ => Some("Invalid puzzle result."),
    })?;
    solve.account = username(&solve.account, "")?;
    angle(&solve.angle)?;
    matches(
        &solve.id,
        patterns::puzzle_id,
        Some("Invalid puzzle."),
        "",
        "",
    )?;
    Ok(solve)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_rating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_rating: Option<u64>,
    count: u64,
}

pub(super) fn assert_local_query(value: J) -> Checked<LocalQuery> {
    let query: LocalQuery = decode(value, |keys| match keys {
        ["theme"] => Some("Invalid puzzle theme."),
        ["minRating" | "maxRating"] => Some("Invalid rating."),
        ["count"] => Some("Invalid puzzle count."),
        _ => Some("Invalid puzzle query."),
    })?;
    if let Some(theme) = &query.theme {
        angle(theme)?;
    }
    for rating in query.min_rating.iter().chain(&query.max_rating) {
        within(*rating, 0, 4000, Some("Invalid rating."), "")?;
    }
    within(query.count, 1, 300, Some("Invalid puzzle count."), "")?;
    Ok(query)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct LadderQuery {
    from: u64,
    to: u64,
    count: u64,
}

pub(super) fn assert_ladder_query(value: J) -> Checked<LadderQuery> {
    let query: LadderQuery = decode(value, |keys| match keys {
        ["from" | "to"] => Some("Invalid rating."),
        ["count"] => Some("Invalid puzzle count."),
        _ => Some("Invalid puzzle query."),
    })?;
    within(query.from, 0, 4000, Some("Invalid rating."), "")?;
    within(query.to, 0, 4000, Some("Invalid rating."), "")?;
    within(query.count, 1, 300, Some("Invalid puzzle count."), "")?;
    Ok(query)
}

/// A saved training run.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct RunInput {
    kind: String,
    variant: String,
    score: f64,
    detail: Detail,
}

/// A run's detail: up to 20 named strings, numbers and flags, in their order.
#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct Detail(#[serde(with = "entries")] Vec<(String, DetailValue)>);

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum DetailValue {
    Flag(bool),
    Number(f64),
    Text(String),
}

impl<'de> Deserialize<'de> for DetailValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Scalar;
        impl Visitor<'_> for Scalar {
            type Value = DetailValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string, number or boolean")
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<DetailValue, E> {
                Ok(DetailValue::Flag(b))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n as f64))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n as f64))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n))
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<DetailValue, E> {
                Ok(DetailValue::Text(text.to_owned()))
            }
        }
        deserializer.deserialize_any(Scalar)
    }
}

impl<'de> Deserialize<'de> for Detail {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        entries::deserialize(deserializer).map(Detail)
    }
}

/// A plain object as an ordered list of entries.
pub(super) mod entries {
    use super::*;

    use serde::ser::SerializeMap;

    pub fn serialize<S: serde::Serializer, T: Serialize>(
        entries: &[(String, T)],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (key, value) in entries {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }

    pub fn deserialize<'de, D: de::Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Vec<(String, T)>, D::Error> {
        struct Entries<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Entries<T> {
            type Value = Vec<(String, T)>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(entries)
            }
        }
        // A record is a plain object only; a list is not read by its indices here.
        deserializer.deserialize_any(Entries(std::marker::PhantomData))
    }
}

pub(super) fn assert_run_input(value: J) -> Checked<RunInput> {
    let run: RunInput = decode(value, |keys| match keys {
        ["kind"] => Some("Invalid kind of run."),
        ["score"] => Some("Invalid score."),
        ["detail", _] => None,
        _ => Some("Invalid run."),
    })?;
    one_of(&run.kind, RUN_KINDS, Some("Invalid kind of run."), "")?;
    matches(
        &run.variant,
        patterns::run_variant,
        Some("Invalid run."),
        "",
        "",
    )?;
    finite(run.score, Some("Invalid score."), "")?;
    within(run.score, 0.0, 1_000_000.0, None, "score")?;
    for (key, value) in &run.detail.0 {
        matches(key, patterns::run_detail_key, Some("Invalid run."), "", "")?;
        let path = format!("detail.{key}");
        match value {
            DetailValue::Text(text) => max_chars(text, 80, None, &path)?,
            DetailValue::Number(n) => finite(*n, None, &path)?,
            DetailValue::Flag(_) => {}
        }
    }
    max_items(&run.detail.0, 20, Some("Invalid run."), "")?;
    Ok(run)
}

/// A puzzle as the trainer shows it (`Puzzle`), as stored in the attempt log.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub(super) struct AttemptPuzzle {
    id: String,
    fen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_move: Option<String>,
    solution: Vec<String>,
    rating: u64,
    themes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plays: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    game_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct AttemptWrongMove {
    index: u64,
    uci: String,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct PuzzleAttempt {
    puzzle: AttemptPuzzle,
    mode: String,
    retry: bool,
    win: bool,
    revealed: bool,
    hinted: bool,
    found: u64,
    wrong: u64,
    total: u64,
    day: String,
    #[serde(rename = "wrongMoves", skip_serializing_if = "Option::is_none")]
    wrong_moves: Option<Vec<AttemptWrongMove>>,
}

pub(super) fn assert_puzzle_attempt(value: J) -> Checked<PuzzleAttempt> {
    const MSG: Msg = Some(ATTEMPT);
    let attempt: PuzzleAttempt = decode(value, |keys| match keys {
        ["puzzle", "lastMove"] | ["puzzle", "solution", "[]"] => Some("Invalid move."),
        ["puzzle", "themes", "[]"] => Some("Invalid puzzle theme."),
        ["puzzle", "gameId"] => Some("Invalid game id."),
        _ => Some(ATTEMPT),
    })?;
    let puzzle = &attempt.puzzle;
    matches(&puzzle.id, patterns::puzzle_id, MSG, "", "")?;
    fen(&puzzle.fen, MSG, MSG, "")?;
    if let Some(last) = &puzzle.last_move {
        uci(last)?;
    }
    each(&puzzle.solution, |m| uci(m))?;
    min_items(&puzzle.solution, 1, MSG, "")?;
    max_items(&puzzle.solution, 64, MSG, "")?;
    within(puzzle.rating, 0, 4000, MSG, "")?;
    each(&puzzle.themes, |theme| angle(theme))?;
    max_items(&puzzle.themes, 40, MSG, "")?;
    if let Some(plays) = puzzle.plays {
        within(plays, 0, MAX_SAFE, MSG, "")?;
    }
    if let Some(id) = &puzzle.game_id {
        game_id(id)?;
    }
    one_of(&attempt.mode, PUZZLE_TRAIN_MODES, MSG, "")?;
    within(attempt.found, 0, 32, MSG, "")?;
    within(attempt.wrong, 0, 32, MSG, "")?;
    within(attempt.total, 1, 32, MSG, "")?;
    local_day(&attempt.day, ATTEMPT)?;
    if let Some(wrong) = &attempt.wrong_moves {
        max_items(wrong, 32, MSG, "")?;
        each(wrong, |entry| {
            within(entry.index, 0, 63, MSG, "")?;
            uci(&entry.uci)
        })?;
    }
    Ok(attempt)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct PuzzleProgressQuery {
    today: String,
    limit: u64,
}

pub(super) fn assert_puzzle_progress_query(value: J) -> Checked<PuzzleProgressQuery> {
    const MSG: &str = "Invalid puzzle history query.";
    let query: PuzzleProgressQuery = decode(value, always!(MSG))?;
    local_day(&query.today, MSG)?;
    within(query.limit, 1, 500, Some(MSG), "")?;
    Ok(query)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRequest {
    pub fen: String,
    lines: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    infinite: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    moves: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
}

/// A variant game's position for analysis: its root under the variant's rules, and its moves
/// played there to the position asked about. Its FEN may carry what the variant keeps
/// (Crazyhouse's pockets, Three-check's checks) and Horde's 36 pawns.
pub(super) fn variant_analysis_position(
    request: &AnalysisRequest,
    variant: Variant,
) -> Checked<()> {
    let invalid = || "Invalid analysis position or move history.".to_string();
    for text in std::iter::once(&request.fen).chain(request.root_fen.as_ref()) {
        max_chars(text, 120, None, "fen")?;
    }
    let root = request.root_fen.as_deref().unwrap_or(&request.fen);
    let moves = request.moves.as_deref().unwrap_or_default();
    if moves.len() > 600 {
        return Err("Invalid move list.".into());
    }
    let replay = replay::replay_setup(variant, root, moves).ok_or("Invalid position.")?;
    let asked = replay::replay_setup(variant, &request.fen, &Vec::<&str>::new())
        .ok_or("Invalid position.")?;
    if replay.played.len() != moves.len() || replay.fen != asked.fen {
        return Err(invalid());
    }
    Ok(())
}

/// `assertAnalysisRequest`: the request's shape, and a position its moves reach.
pub fn assert_analysis_request(value: J) -> Checked<AnalysisRequest> {
    let request: AnalysisRequest = decode(value, |keys| match keys {
        [] | ["infinite"] => Some("Invalid engine options."),
        ["fen"] => Some("Invalid position."),
        ["lines"] => Some("Invalid number of engine lines."),
        ["rootFen"] => None,
        ["moves", rest @ ..] => move_list_messages(rest, Some("Invalid move list.")),
        ["clientId"] => Some("Invalid analysis request."),
        ["variant"] => Some("Invalid variant."),
        _ => None,
    })?;
    // Standard chess and Chess960 are checked as they always were, below.
    if let Some(variant) = other_variant(request.variant.as_deref())? {
        within(
            request.lines,
            1,
            5,
            Some("Invalid number of engine lines."),
            "",
        )?;
        if let Some(client) = request.client_id {
            within(client, 1, MAX_SAFE, Some("Invalid analysis request."), "")?;
        }
        variant_analysis_position(&request, variant)?;
        return Ok(request);
    }
    fen(&request.fen, None, Some("Invalid position."), "fen")?;
    engine_position(&request.fen)?;
    let lines = Some("Invalid number of engine lines.");
    within(request.lines, 1, 5, lines, "")?;
    if let Some(root) = &request.root_fen {
        fen(root, None, None, "rootFen")?;
    }
    if let Some(moves) = &request.moves {
        move_list(moves)?;
    }
    if let Some(client) = request.client_id {
        within(client, 1, MAX_SAFE, Some("Invalid analysis request."), "")?;
    }
    let root = request.root_fen.as_deref().unwrap_or(&request.fen);
    let moves = request.moves.as_deref().unwrap_or_default();
    // Only the final position matters: replay without a FEN and end check for every ply.
    let last = replay::fen_after(root, moves, moves.len());
    let normalized = replay::fen_after(&request.fen, &[] as &[&str], 0);
    if normalized.is_none() || last != normalized {
        return Err("Invalid analysis position or move history.".into());
    }
    Ok(request)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub(super) struct VoiceAttempt {
    source: String,
    heard: String,
    confidence: f64,
    words: Vec<VoiceWord>,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parsed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_of: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct VoiceWord {
    word: String,
    conf: f64,
}

/// A recognizer's confidence, from 0 to 1.
pub(super) fn score(n: f64, path: &str) -> Checked<()> {
    within(n, 0.0, 1.0, None, path)
}

pub(super) fn assert_voice_attempt(value: J) -> Checked<VoiceAttempt> {
    let attempt: VoiceAttempt = decode(value, |keys| match keys {
        ["words"] | ["words", "[]"] => None,
        _ => Some(VOICE),
    })?;
    one_of(&attempt.source, VOICE_SOURCES, Some(VOICE), "")?;
    max_chars(&attempt.heard, 200, None, "heard")?;
    score(attempt.confidence, "confidence")?;
    for word in &attempt.words {
        max_chars(&word.word, 40, None, "words.word")?;
        score(word.conf, "words.conf")?;
    }
    max_items(&attempt.words, 40, Some(VOICE), "")?;
    one_of(&attempt.outcome, VOICE_OUTCOMES, Some(VOICE), "")?;
    if let Some(parsed) = &attempt.parsed {
        max_chars(parsed, 120, None, "parsed")?;
    }
    if let Some(expected) = &attempt.expected {
        max_chars(expected, 20, None, "expected")?;
    }
    if let Some(position) = &attempt.fen {
        fen(position, None, Some("Invalid position."), "fen")?;
    }
    if let Some(retry) = attempt.retry_of {
        within(retry, 1, MAX_SAFE, Some(VOICE), "")?;
    }
    Ok(attempt)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct VoiceUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<String>,
}

pub(super) fn assert_voice_update(value: J) -> Checked<VoiceUpdate> {
    let update: VoiceUpdate = decode(value, always!(VOICE))?;
    if let Some(outcome) = &update.outcome {
        one_of(outcome, VOICE_OUTCOMES, Some(VOICE), "")?;
    }
    if let Some(expected) = &update.expected {
        max_chars(expected, 20, None, "expected")?;
    }
    Ok(update)
}

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

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct InsightsQuery {
    account: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    days: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
}

pub(super) fn assert_insights_query(value: J) -> Checked<InsightsQuery> {
    let mut query: InsightsQuery = decode(value, |keys| match keys {
        [] => Some("Invalid insights filter."),
        ["account"] => Some(USERNAME),
        ["days"] => Some("Invalid period."),
        _ => None,
    })?;
    query.account = game_account(&query.account, "account")?;
    if let Some(speed) = &query.speed {
        one_of(speed, EXPLORER_SPEEDS, None, "speed")?;
    }
    if let Some(days) = query.days {
        within(days, 1, 3650, Some("Invalid period."), "days")?;
    }
    if let Some(variant) = &query.variant {
        one_of(variant, VARIANTS, Some("Invalid variant."), "variant")?;
    }
    Ok(query)
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

/// The settings the core keeps, in its order.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub(super) struct CoreSettings {
    engine_path: String,
    engine_levels: Vec<String>,
    review_auto: String,
    review_on_battery: bool,
    receive_challenges: bool,
    online_chat: bool,
    correspondence_poll: u64,
    cloud_eval: bool,
    voice_history: bool,
}

/// Every setting, in the frontends' order.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub(super) struct Settings {
    appearance: String,
    board_theme: String,
    light_theme: String,
    dark_theme: String,
    piece_set: String,
    piece_animation: String,
    coordinates: String,
    sound_enabled: bool,
    sound_volume: f64,
    engine_path: String,
    premove: bool,
    promotion: String,
    show_legal_moves: bool,
    notifications_enabled: bool,
    notify_active: bool,
    notify_background: bool,
    notify_opponent_move: bool,
    notify_low_time: bool,
    notify_game_events: bool,
    notify_computer_move: bool,
    notify_sound: bool,
    voice_push_to_talk: bool,
    voice_confirm_moves: bool,
    voice_history: bool,
    update_auto_check: bool,
    update_auto_download: bool,
    update_install_on_quit: bool,
    engine_levels: Vec<String>,
    review_auto: String,
    review_on_battery: bool,
    receive_challenges: bool,
    notify_challenges: bool,
    online_chat: bool,
    correspondence_poll: u64,
    zen_mode: bool,
    blindfold: bool,
    cloud_eval: bool,
    show_opening_name: bool,
    swipe_navigation: bool,
    swipe_indicator: bool,
    puzzle_auto_next: bool,
    /// Newer than the other settings: a frontend that does not send it keeps the default, on.
    #[serde(default = "on")]
    confirm_correspondence: bool,
    #[serde(default)]
    figurine_notation: bool,
}

pub(super) fn on() -> bool {
    true
}

/// The message of each setting.
pub(super) fn settings_messages(keys: &[&str]) -> Msg {
    Some(match keys {
        ["appearance"] => "Invalid appearance.",
        ["boardTheme"] => "Invalid board theme.",
        ["lightTheme" | "darkTheme"] => "Invalid color theme.",
        ["pieceSet"] => "Invalid piece set.",
        ["pieceAnimation"] => "Invalid piece animation.",
        ["coordinates"] => "Invalid coordinates mode.",
        ["soundEnabled"] => "Invalid sound setting.",
        ["soundVolume"] => "Invalid sound volume.",
        ["enginePath"] => "Invalid engine path.",
        ["premove"] => "Invalid premove setting.",
        ["promotion"] => "Invalid promotion setting.",
        ["showLegalMoves"] => "Invalid legal moves setting.",
        ["voicePushToTalk" | "voiceConfirmMoves" | "voiceHistory"] => "Invalid voice setting.",
        ["updateAutoCheck" | "updateAutoDownload" | "updateInstallOnQuit"] => {
            "Invalid update setting."
        }
        ["engineLevels"] => "Invalid computer levels.",
        ["engineLevels", "[]"] => "Invalid engine level.",
        ["reviewAuto" | "reviewOnBattery"] => "Invalid review setting.",
        ["receiveChallenges"] => "Invalid challenge setting.",
        ["onlineChat"] => "Invalid chat setting.",
        ["correspondencePoll"] => "Invalid correspondence check interval.",
        ["zenMode"] => "Invalid zen mode setting.",
        ["blindfold"] => "Invalid blindfold setting.",
        ["cloudEval"] => "Invalid cloud evaluation setting.",
        ["showOpeningName"] => "Invalid opening name setting.",
        ["swipeNavigation" | "swipeIndicator"] => "Invalid swipe setting.",
        ["puzzleAutoNext"] => "Invalid puzzle setting.",
        ["confirmCorrespondence"] => "Invalid move confirmation setting.",
        ["figurineNotation"] => "Invalid notation setting.",
        ["notificationsEnabled"] => NOTIFY,
        [key] if key.starts_with("notify") => NOTIFY,
        _ => "Invalid settings.",
    })
}

pub(super) fn theme_setting(id: &str, msg: &'static str) -> Checked<()> {
    matches(id, patterns::theme_id, Some(msg), "", "")
}

/// The core's settings after the engine path, checked in their order. The computer levels are
/// kept in their own order, without repeats; at least one must be chosen.
pub(super) fn check_core_settings(
    engine_levels: &mut Vec<String>,
    review_auto: &str,
    correspondence_poll: u64,
) -> Checked<()> {
    each(engine_levels, |level| {
        one_of(level, ENGINE_LEVELS, Some("Invalid engine level."), "")
    })?;
    let keep = Some("Keep at least one computer level.");
    min_items(engine_levels, 1, keep, "")?;
    *engine_levels = ENGINE_LEVELS
        .iter()
        .filter(|level| engine_levels.iter().any(|chosen| chosen == *level))
        .map(|level| (*level).to_owned())
        .collect();
    one_of(
        review_auto,
        REVIEW_AUTO,
        Some("Invalid review setting."),
        "",
    )?;
    let poll = Some("Invalid correspondence check interval.");
    within(correspondence_poll, 0, 120, poll, "")
}

pub(super) fn assert_core_settings(value: J) -> Checked<CoreSettings> {
    let mut s: CoreSettings = decode(value, settings_messages)?;
    max_chars(&s.engine_path, 4096, Some("Invalid engine path."), "")?;
    check_core_settings(&mut s.engine_levels, &s.review_auto, s.correspondence_poll)?;
    Ok(s)
}

pub(super) fn assert_settings(value: J) -> Checked<Settings> {
    let mut s: Settings = decode(value, settings_messages)?;
    one_of(&s.appearance, APPEARANCES, Some("Invalid appearance."), "")?;
    theme_setting(&s.board_theme, "Invalid board theme.")?;
    theme_setting(&s.light_theme, "Invalid color theme.")?;
    theme_setting(&s.dark_theme, "Invalid color theme.")?;
    theme_setting(&s.piece_set, "Invalid piece set.")?;
    let animation = Some("Invalid piece animation.");
    one_of(&s.piece_animation, PIECE_ANIMATIONS, animation, "")?;
    let coordinates = Some("Invalid coordinates mode.");
    one_of(&s.coordinates, COORDINATE_MODES, coordinates, "")?;
    finite(s.sound_volume, Some("Invalid sound volume."), "")?;
    s.sound_volume = s.sound_volume.clamp(0.0, 1.0);
    max_chars(&s.engine_path, 4096, Some("Invalid engine path."), "")?;
    let promotion = Some("Invalid promotion setting.");
    one_of(&s.promotion, PROMOTION_MODES, promotion, "")?;
    check_core_settings(&mut s.engine_levels, &s.review_auto, s.correspondence_poll)?;
    Ok(s)
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

#[derive(Deserialize, Serialize, Default)]
#[serde(expecting = "an object")]
pub struct BestMoveOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    movetime: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chess960: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
}

pub fn assert_best_move_options(value: J) -> Checked<BestMoveOptions> {
    if matches!(value, J::Undef) {
        return Ok(BestMoveOptions::default());
    }
    let mut options: BestMoveOptions = decode(value, |keys| match keys {
        ["fen"] => Some("Invalid position."),
        ["movetime"] => Some("Invalid think time."),
        ["variant"] => Some("Invalid variant."),
        _ => Some("Invalid engine options."),
    })?;
    // Another variant (Fairy-Stockfish plays it) starts from a position legal by its own rules.
    if let Some(variant) = other_variant(options.variant.as_deref())? {
        let position = options.fen.as_deref().ok_or("Invalid position.")?;
        max_chars(position, 120, Some("Invalid position."), "fen")?;
        if crate::rules::setup_start(variant, position).is_none() {
            return Err("Invalid position.".into());
        }
        if let Some(movetime) = options.movetime {
            within(movetime, 50, 5000, Some("Invalid think time."), "")?;
        }
        return Ok(options);
    }
    options.variant = None;
    if let Some(position) = &options.fen {
        fen(position, None, Some("Invalid position."), "fen")?;
        engine_position(position)?;
    }
    if let Some(movetime) = options.movetime {
        within(movetime, 50, 5000, Some("Invalid think time."), "")?;
    }
    Ok(options)
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
pub(super) struct ReviewRequest {
    fen: String,
    moves: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    game_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
}

/// `assertReviewRequest`: the request's shape, and moves that replay from its position.
pub(super) fn assert_review_request(value: J) -> Checked<ReviewRequest> {
    let mut request: ReviewRequest = decode(value, |keys| match keys {
        ["fen"] => Some("Invalid position."),
        ["moves", rest @ ..] => move_list_messages(rest, Some("Invalid move list.")),
        ["gameId"] => Some("Invalid game id."),
        ["account"] => Some(USERNAME),
        ["variant"] => Some("Invalid variant."),
        _ => Some("Invalid review request."),
    })?;
    // A variant game (Fairy-Stockfish reviews it) is replayed by its variant's rules.
    if let Some(variant) = other_variant(request.variant.as_deref())? {
        max_chars(&request.fen, 120, Some("Invalid position."), "fen")?;
        move_list(&request.moves)?;
        if let Some(id) = &request.game_id {
            game_id(id)?;
        }
        if let Some(account) = &request.account {
            request.account = Some(game_account(account, "account")?);
        }
        if replay::replay_setup_count(variant, &request.fen, &request.moves)
            != Some(request.moves.len())
        {
            return Err("Invalid game position or move history.".into());
        }
        return Ok(request);
    }
    request.variant = None;
    fen(&request.fen, None, Some("Invalid position."), "fen")?;
    engine_position(&request.fen)?;
    move_list(&request.moves)?;
    if let Some(id) = &request.game_id {
        game_id(id)?;
    }
    if let Some(account) = &request.account {
        request.account = Some(game_account(account, "account")?);
    }
    if replay::replay_positions_count(&request.fen, &request.moves) != request.moves.len() + 1 {
        return Err("Invalid game position or move history.".into());
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

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub struct GamePageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<GameResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rated: Option<bool>,
    /// Part of an opponent's name or an opening's; blank is no search.
    #[serde(skip_serializing_if = "Option::is_none")]
    search: Option<String>,
    offset: u64,
    limit: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum GameResult {
    Win,
    Loss,
    Draw,
}

pub(super) const GAME_SEARCH: &str = "Invalid game search.";

pub fn assert_game_page_query(value: J) -> Checked<GamePageQuery> {
    let mut query: GamePageQuery = decode(value, |keys| match keys {
        ["account"] => Some(USERNAME),
        ["offset"] => Some("Invalid game offset."),
        ["limit"] => Some("Invalid game page size."),
        ["search"] => Some(GAME_SEARCH),
        _ => None,
    })?;
    if let Some(account) = &query.account {
        query.account = Some(game_account(account, "account")?);
    }
    if let Some(search) = query.search.take() {
        let search = search.trim();
        if search.chars().count() > 100 {
            return Err(GAME_SEARCH.into());
        }
        query.search = (!search.is_empty()).then(|| search.to_string());
    }
    within(query.offset, 0, MAX_SAFE, Some("Invalid game offset."), "")?;
    within(query.limit, 1, 100, Some("Invalid game page size."), "")?;
    Ok(query)
}

/// `assertStudySyncRequest`: a study's account, id and both PGN copies, bounded.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub(super) struct StudySyncRequest {
    account: Option<String>,
    study_id: Option<String>,
    baseline: Option<String>,
    pgn: Option<String>,
}

pub(super) const STUDY_TEXT: &str = "The study is empty or too large to upload.";

pub(super) fn assert_study_sync_request(value: J) -> Checked<StudySyncRequest> {
    if !value.is_object() {
        return Err("Invalid study synchronization.".into());
    }
    let mut request: StudySyncRequest = decode(value, |keys| match keys {
        ["account"] => Some(USERNAME),
        ["studyId"] => Some("Invalid id."),
        ["baseline" | "pgn"] => Some(STUDY_TEXT),
        _ => Some("Invalid study synchronization."),
    })?;
    let account = request.account.as_deref().ok_or(USERNAME)?;
    request.account = Some(username(account, "")?);
    lichess_id(
        request.study_id.as_deref().ok_or("Invalid id.")?,
        "Invalid id.",
    )?;
    let usable = |text: &Option<String>| {
        text.as_deref()
            .is_some_and(|t| !js::trim(t).is_empty() && js::utf16_len(t) <= 500_000)
    };
    ensure(
        usable(&request.baseline) && usable(&request.pgn),
        Some(STUDY_TEXT),
        "",
        String::new,
    )?;
    Ok(request)
}

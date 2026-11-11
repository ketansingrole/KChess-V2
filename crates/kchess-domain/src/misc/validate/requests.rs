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

/// `assertNewArena`: the arena's clock, schedule and options, and no other keys.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NewArena {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    clock_time: f64,
    clock_increment: f64,
    minutes: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_date: Option<f64>,
    variant: String,
    rated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

pub(super) fn assert_new_arena(value: J) -> Checked<NewArena> {
    let mut arena: NewArena = decode(value, |keys| match keys {
        ["clockTime"] => Some("Invalid clock."),
        ["clockIncrement"] => Some("Invalid increment."),
        ["minutes"] => Some("Invalid duration."),
        ["waitMinutes"] => Some("Invalid start."),
        ["variant"] => Some("Invalid variant."),
        _ => None,
    })?;
    if let Some(name) = &mut arena.name {
        *name = js::trim(name).to_owned();
        let too_long = Some("Arena names are limited to 30 characters.");
        max_chars(name, 30, too_long, "name")?;
        let punctuation = Some("Use letters, numbers and simple punctuation.");
        matches(name, patterns::arena_name, punctuation, "name", "")?;
    }
    let clock = Some("Invalid clock.");
    one_of_numbers(arena.clock_time, ARENA_CLOCK_MINUTES, clock, "clockTime")?;
    let increment = Some("Invalid increment.");
    one_of_numbers(
        arena.clock_increment,
        ARENA_INCREMENTS,
        increment,
        "clockIncrement",
    )?;
    let duration = Some("Invalid duration.");
    one_of_numbers(arena.minutes, ARENA_DURATIONS, duration, "minutes")?;
    if let Some(wait) = arena.wait_minutes {
        let start = Some("Invalid start.");
        one_of_numbers(wait, ARENA_WAIT_MINUTES, start, "waitMinutes")?;
    }
    if let Some(start) = arena.start_date {
        let ok = start.is_finite() && start.fract() == 0.0 && start >= 0.0;
        ensure(ok, None, "startDate", || "a time in milliseconds".into())?;
    }
    one_of(
        &arena.variant,
        VARIANTS,
        Some("Invalid variant."),
        "variant",
    )?;
    if let Some(password) = &arena.password {
        max_chars(password, 60, None, "password")?;
    }
    if let Some(description) = &arena.description {
        max_chars(description, 2000, None, "description")?;
    }
    Ok(arena)
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

/// `assertWatchTarget(value, channels)`: a channel from `channels`, or a game id. A valid
/// channel wins; otherwise a game id is checked; anything else is no game to watch.
pub fn assert_watch_target(value: J, channels: J) -> Checked<Value> {
    let invalid = || "Invalid game to watch.".to_string();
    if !value.is_object() {
        return Err(invalid());
    }
    let channels: Vec<String> = decode(channels, plain).unwrap_or_default();
    if let Some(channel) = value.get("channel").and_then(J::str)
        && channels.iter().any(|c| c == channel)
    {
        return Ok(serde_json::json!({ "channel": channel }));
    }
    match value.get("gameId").and_then(J::str) {
        Some(id) => {
            game_id(id)?;
            Ok(serde_json::json!({ "gameId": id }))
        }
        None => Err(invalid()),
    }
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
pub(super) struct OnlineOptions {
    minutes: u64,
    increment: u64,
    color: String,
    rated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    days: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
}

pub(super) fn assert_online_options(value: J) -> Checked<OnlineOptions> {
    let mut options: OnlineOptions = decode(value, |keys| match keys {
        [] => Some("Invalid game options."),
        ["minutes"] => Some("Invalid time control."),
        ["increment"] => Some("Invalid increment."),
        ["color"] => Some("Invalid color."),
        ["rated"] => Some("Invalid rated option."),
        ["days"] => Some("Invalid days per move."),
        ["variant"] => Some("Invalid variant."),
        ["fen"] => Some("Invalid position."),
        ["account"] => Some(USERNAME),
        ["target"] => Some("Invalid opponent."),
        _ => None,
    })?;
    within(options.minutes, 1, 180, Some("Invalid time control."), "")?;
    within(options.increment, 0, 180, Some("Invalid increment."), "")?;
    one_of(&options.color, CHALLENGE_COLORS, Some("Invalid color."), "")?;
    if let Some(days) = options.days {
        let msg = Some("Invalid days per move.");
        one_of_numbers(days as f64, &CORRESPONDENCE_DAYS, msg, "")?;
    }
    if let Some(variant) = &options.variant {
        one_of(variant, VARIANTS, Some("Invalid variant."), "")?;
    }
    if let Some(position) = &options.fen {
        fen(position, None, Some("Invalid position."), "fen")?;
    }
    if let Some(account) = &options.account {
        options.account = Some(username(account, "account")?);
    }
    if let Some(target) = &options.target {
        let target = js::trim(target);
        let valid = target.is_empty() || patterns::username(target);
        ensure(valid, Some(USERNAME), "", String::new)?;
        options.target = (!target.is_empty()).then(|| target.to_owned());
    }
    Ok(options)
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

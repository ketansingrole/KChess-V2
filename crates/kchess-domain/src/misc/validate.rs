//! The `assert*` validators of `core/src/domain/validate.ts`: each parses an untrusted IPC value
//! with the same schema the TypeScript used (see `engine.rs`) and returns the normalized value,
//! or the message the TypeScript threw.

use std::sync::LazyLock;

use super::constants::*;
use super::engine::{
    Action, J, Msg, Schema, array, literal, object, optional, parse, picklist_num, picklist_str,
    pipe, strict_object, union,
};
use super::patterns;
use crate::replay;

type Validator = fn(Vec<J>) -> Result<J, String>;

/// The argument `i`, moved out of the list (`undefined` when it is absent).
fn take(args: &mut [J], i: usize) -> J {
    args.get_mut(i)
        .map_or(J::Undef, |slot| std::mem::replace(slot, J::Undef))
}

/// Defines a validator that parses its only argument with a schema built once.
macro_rules! schema_validator {
    ($name:ident, $build:expr) => {
        fn $name(mut args: Vec<J>) -> Result<J, String> {
            static SCHEMA: LazyLock<Schema> = LazyLock::new(|| $build);
            parse(&SCHEMA, take(&mut args, 0))
        }
    };
}

const USERNAME_MESSAGE: &str = "Enter a valid Lichess username.";
const USERNAME_SOURCE: &str = "/^[a-zA-Z0-9_-]{2,30}$/";
const FEN_SOURCE: &str = r"/^[1-8pnbrqkPNBRQK]+(?:\/[1-8pnbrqkPNBRQK]+){7} [wb] (?:[KQkqA-Ha-h]{1,4}|-) (?:[a-h][36]|-)(?: \d{1,3} \d{1,4})?$/";
const SINCE_SOURCE: &str = r"/^\d{4}(-\d{2})?$/";

/// A string checked by one pattern; `msg` is the custom message, if any.
fn text(msg: Msg, source: &'static str, test: fn(&str) -> bool, pattern_msg: Msg) -> Schema {
    pipe(
        Schema::Str(msg),
        vec![Action::Regex {
            source,
            test,
            msg: pattern_msg,
        }],
    )
}

fn int_in_range(min: f64, max: f64, msg: Msg) -> Schema {
    pipe(
        Schema::Num(msg),
        vec![
            Action::Integer(msg),
            Action::MinValue(min, msg),
            Action::MaxValue(max, msg),
        ],
    )
}

fn max_len(max: usize, msg: Msg) -> Action {
    Action::MaxLength(max, msg)
}

fn username_schema() -> Schema {
    pipe(
        Schema::Str(Some(USERNAME_MESSAGE)),
        vec![
            Action::Trim,
            Action::Regex {
                source: USERNAME_SOURCE,
                test: patterns::username,
                msg: Some(USERNAME_MESSAGE),
            },
        ],
    )
}

fn game_id_schema() -> Schema {
    text(
        Some("Invalid game id."),
        "/^[a-zA-Z0-9]{8,12}$/",
        patterns::game_id,
        Some("Invalid game id."),
    )
}

fn uci_schema() -> Schema {
    text(
        Some("Invalid move."),
        "/^[a-h][1-8][a-h][1-8][qrbn]?$/",
        patterns::uci_move,
        Some("Invalid move."),
    )
}

fn moves_schema() -> Schema {
    pipe(
        array(uci_schema(), Some("Invalid move list.")),
        vec![max_len(MAX_MOVES, Some("Invalid move list."))],
    )
}

fn level_schema() -> Schema {
    picklist_str(ENGINE_LEVELS, Some("Invalid engine level."))
}

fn angle_schema() -> Schema {
    text(
        Some("Invalid puzzle theme."),
        "/^[a-zA-Z0-9_-]{1,60}$/",
        patterns::puzzle_angle,
        Some("Invalid puzzle theme."),
    )
}

/// `optionalAccountSchema`: a connected account, or empty for none.
fn optional_account_schema() -> Schema {
    union(
        vec![literal(""), username_schema()],
        Some("Invalid account."),
    )
}

fn hex_schema() -> Schema {
    text(
        Some("Colors must be hex, like #1a2b3c."),
        "/^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/",
        patterns::hex_color,
        Some("Colors must be hex, like #1a2b3c."),
    )
}

/// A theme identifier setting: 1–32 characters of a–z, 0–9 and dashes.
fn theme_setting(msg: &'static str) -> Schema {
    text(
        Some(msg),
        "/^[a-z0-9-]{1,32}$/",
        patterns::theme_id,
        Some(msg),
    )
}

fn voice_text(max: usize) -> Schema {
    pipe(
        Schema::Str(Some("Invalid voice entry.")),
        vec![max_len(max, None)],
    )
}

fn voice_pattern_score() -> Schema {
    pipe(
        Schema::Num(Some("Invalid voice entry.")),
        vec![Action::MinValue(0.0, None), Action::MaxValue(1.0, None)],
    )
}

fn palette_schema() -> Schema {
    let mut entries = vec![
        ("bg", hex_schema()),
        ("text", hex_schema()),
        ("primary", hex_schema()),
    ];
    for key in [
        "muted",
        "elevated",
        "accented",
        "border",
        "textMuted",
        "success",
        "warning",
        "error",
        "info",
    ] {
        entries.push((key, optional(hex_schema())));
    }
    object(entries, None)
}

fn theme_schema() -> Schema {
    let body = object(
        vec![
            (
                "id",
                text(
                    Some("A theme needs an \"id\"."),
                    "/^[a-z0-9-]{1,32}$/",
                    patterns::theme_id,
                    Some("The \"id\" must be 1–32 characters: a–z, 0–9 and dashes."),
                ),
            ),
            (
                "name",
                pipe(
                    Schema::Str(Some("A theme needs a \"name\".")),
                    vec![
                        Action::Trim,
                        Action::MinLength(1, Some("A theme needs a \"name\".")),
                        max_len(40, Some("The \"name\" is too long.")),
                    ],
                ),
            ),
            ("light", optional(palette_schema())),
            ("dark", optional(palette_schema())),
        ],
        Some("A theme file must be a JSON object."),
    );
    pipe(
        body,
        vec![Action::Check {
            check: |theme| {
                theme.get("light").is_some_and(J::truthy)
                    || theme.get("dark").is_some_and(J::truthy)
            },
            msg: Some("Add a \"dark\" or \"light\" palette."),
        }],
    )
}

fn notification_schema() -> Schema {
    object(
        vec![
            (
                "kind",
                picklist_str(NOTIFICATION_KINDS, Some("Invalid notification.")),
            ),
            (
                "title",
                pipe(
                    Schema::Str(Some("Invalid notification.")),
                    vec![max_len(120, Some("Invalid notification."))],
                ),
            ),
            (
                "body",
                pipe(
                    Schema::Str(Some("Invalid notification.")),
                    vec![max_len(400, Some("Invalid notification."))],
                ),
            ),
        ],
        Some("Invalid notification."),
    )
}

/// The notification switches share one message.
const NOTIFY: &str = "Invalid notification setting.";

fn settings_entries() -> Vec<(&'static str, Schema)> {
    let boolean = |msg: &'static str| Schema::Bool(Some(msg));
    vec![
        (
            "appearance",
            picklist_str(APPEARANCES, Some("Invalid appearance.")),
        ),
        ("boardTheme", theme_setting("Invalid board theme.")),
        ("lightTheme", theme_setting("Invalid color theme.")),
        ("darkTheme", theme_setting("Invalid color theme.")),
        ("pieceSet", theme_setting("Invalid piece set.")),
        (
            "pieceAnimation",
            picklist_str(PIECE_ANIMATIONS, Some("Invalid piece animation.")),
        ),
        (
            "coordinates",
            picklist_str(COORDINATE_MODES, Some("Invalid coordinates mode.")),
        ),
        ("soundEnabled", boolean("Invalid sound setting.")),
        (
            "soundVolume",
            pipe(
                Schema::Num(Some("Invalid sound volume.")),
                vec![
                    Action::Finite(Some("Invalid sound volume.")),
                    Action::Transform(clamp_unit),
                ],
            ),
        ),
        (
            "enginePath",
            pipe(
                Schema::Str(Some("Invalid engine path.")),
                vec![max_len(4096, Some("Invalid engine path."))],
            ),
        ),
        ("premove", boolean("Invalid premove setting.")),
        (
            "promotion",
            picklist_str(PROMOTION_MODES, Some("Invalid promotion setting.")),
        ),
        ("showLegalMoves", boolean("Invalid legal moves setting.")),
        ("notificationsEnabled", boolean(NOTIFY)),
        ("notifyActive", boolean(NOTIFY)),
        ("notifyBackground", boolean(NOTIFY)),
        ("notifyOpponentMove", boolean(NOTIFY)),
        ("notifyLowTime", boolean(NOTIFY)),
        ("notifyGameEvents", boolean(NOTIFY)),
        ("notifyComputerMove", boolean(NOTIFY)),
        ("notifySound", boolean(NOTIFY)),
        ("voicePushToTalk", boolean("Invalid voice setting.")),
        ("voiceConfirmMoves", boolean("Invalid voice setting.")),
        ("voiceHistory", boolean("Invalid voice setting.")),
        ("updateAutoCheck", boolean("Invalid update setting.")),
        ("updateAutoDownload", boolean("Invalid update setting.")),
        ("updateInstallOnQuit", boolean("Invalid update setting.")),
        (
            "engineLevels",
            pipe(
                array(level_schema(), Some("Invalid computer levels.")),
                vec![
                    Action::MinLength(1, Some("Keep at least one computer level.")),
                    Action::Transform(engine_levels_filter),
                ],
            ),
        ),
        (
            "reviewAuto",
            picklist_str(REVIEW_AUTO, Some("Invalid review setting.")),
        ),
        ("reviewOnBattery", boolean("Invalid review setting.")),
        ("receiveChallenges", boolean("Invalid challenge setting.")),
        ("notifyChallenges", boolean(NOTIFY)),
        ("onlineChat", boolean("Invalid chat setting.")),
        (
            "correspondencePoll",
            int_in_range(0.0, 120.0, Some("Invalid correspondence check interval.")),
        ),
        ("zenMode", boolean("Invalid zen mode setting.")),
        ("blindfold", boolean("Invalid blindfold setting.")),
        ("cloudEval", boolean("Invalid cloud evaluation setting.")),
        ("showOpeningName", boolean("Invalid opening name setting.")),
        ("swipeNavigation", boolean("Invalid swipe setting.")),
        ("swipeIndicator", boolean("Invalid swipe setting.")),
    ]
}

/// `Math.max(0, Math.min(1, n))`
fn clamp_unit(v: J) -> J {
    match v {
        // Typed numbers are never NaN, so `clamp` is `Math.max(0, Math.min(1, n))`.
        J::Num(n) => J::Num(n.clamp(0.0, 1.0)),
        other => other,
    }
}

/// `ENGINE_LEVELS.filter((level) => levels.includes(level))`
fn engine_levels_filter(v: J) -> J {
    let J::Arr(items) = &v else {
        return v;
    };
    J::Arr(
        ENGINE_LEVELS
            .iter()
            .filter(|level| items.iter().any(|item| item.str() == Some(level)))
            .map(|level| J::Str((*level).to_string()))
            .collect(),
    )
}

/// `s || undefined`
fn empty_to_undefined(v: J) -> J {
    match v {
        J::Str(s) if s.is_empty() => J::Undef,
        other => other,
    }
}

fn online_options_schema() -> Schema {
    object(
        vec![
            ("minutes", clock_field(1.0, "Invalid time control.")),
            ("increment", clock_field(0.0, "Invalid increment.")),
            (
                "color",
                picklist_str(CHALLENGE_COLORS, Some("Invalid color.")),
            ),
            ("rated", Schema::Bool(Some("Invalid rated option."))),
            (
                "days",
                optional(picklist_num(
                    CORRESPONDENCE_DAYS,
                    Some("Invalid days per move."),
                )),
            ),
            (
                "variant",
                optional(picklist_str(VARIANTS, Some("Invalid variant."))),
            ),
            (
                "fen",
                optional(pipe(
                    Schema::Str(Some("Invalid position.")),
                    vec![
                        max_len(100, None),
                        Action::Regex {
                            source: FEN_SOURCE,
                            test: patterns::fen,
                            msg: Some("Invalid position."),
                        },
                    ],
                )),
            ),
            ("account", optional(username_schema())),
            (
                "target",
                optional(pipe(
                    Schema::Str(Some("Invalid opponent.")),
                    vec![
                        Action::Trim,
                        Action::Check {
                            check: |s| match s {
                                J::Str(s) => s.is_empty() || patterns::username(s),
                                _ => true,
                            },
                            msg: Some(USERNAME_MESSAGE),
                        },
                        Action::Transform(empty_to_undefined),
                    ],
                )),
            ),
        ],
        Some("Invalid game options."),
    )
}

fn clock_field(min: f64, msg: &'static str) -> Schema {
    pipe(
        Schema::Num(Some(msg)),
        vec![
            Action::Integer(Some(msg)),
            Action::MinValue(min, Some(msg)),
            Action::MaxValue(180.0, Some(msg)),
        ],
    )
}

fn voice_attempt_schema() -> Schema {
    object(
        vec![
            (
                "source",
                picklist_str(VOICE_SOURCES, Some("Invalid voice entry.")),
            ),
            ("heard", voice_text(200)),
            ("confidence", voice_pattern_score()),
            (
                "words",
                pipe(
                    array(
                        object(
                            vec![("word", voice_text(40)), ("conf", voice_pattern_score())],
                            None,
                        ),
                        None,
                    ),
                    vec![max_len(40, Some("Invalid voice entry."))],
                ),
            ),
            (
                "outcome",
                picklist_str(VOICE_OUTCOMES, Some("Invalid voice entry.")),
            ),
            ("parsed", optional(voice_text(120))),
            ("expected", optional(voice_text(20))),
            (
                "fen",
                optional(pipe(
                    voice_text(100),
                    vec![Action::Regex {
                        source: FEN_SOURCE,
                        test: patterns::fen,
                        msg: Some("Invalid position."),
                    }],
                )),
            ),
            (
                "retryOf",
                optional(int_in_range(
                    1.0,
                    MAX_SAFE_INTEGER,
                    Some("Invalid voice entry."),
                )),
            ),
        ],
        Some("Invalid voice entry."),
    )
}

fn analysis_request_schema() -> Schema {
    object(
        vec![
            (
                "fen",
                pipe(
                    Schema::Str(Some("Invalid position.")),
                    vec![
                        max_len(100, None),
                        Action::Regex {
                            source: FEN_SOURCE,
                            test: patterns::fen,
                            msg: Some("Invalid position."),
                        },
                    ],
                ),
            ),
            (
                "lines",
                int_in_range(1.0, 5.0, Some("Invalid number of engine lines.")),
            ),
            (
                "infinite",
                optional(Schema::Bool(Some("Invalid engine options."))),
            ),
            (
                "rootFen",
                optional(pipe(
                    Schema::Str(None),
                    vec![
                        max_len(100, None),
                        Action::Regex {
                            source: FEN_SOURCE,
                            test: patterns::fen,
                            msg: None,
                        },
                    ],
                )),
            ),
            ("moves", optional(moves_schema())),
            (
                "clientId",
                optional(int_in_range(
                    1.0,
                    MAX_SAFE_INTEGER,
                    Some("Invalid analysis request."),
                )),
            ),
        ],
        Some("Invalid engine options."),
    )
}

fn review_request_schema() -> Schema {
    object(
        vec![
            (
                "fen",
                pipe(
                    Schema::Str(Some("Invalid position.")),
                    vec![
                        max_len(100, None),
                        Action::Regex {
                            source: FEN_SOURCE,
                            test: patterns::fen,
                            msg: Some("Invalid position."),
                        },
                    ],
                ),
            ),
            ("moves", moves_schema()),
            ("gameId", optional(game_id_schema())),
            ("account", optional(username_schema())),
        ],
        Some("Invalid review request."),
    )
}

fn lichess_id_schema(msg: &'static str) -> Schema {
    text(
        Some(msg),
        "/^[a-zA-Z0-9]{8}$/",
        patterns::lichess_id,
        Some(msg),
    )
}

fn puzzle_request_schema() -> Schema {
    object(
        vec![
            ("account", optional_account_schema()),
            ("angle", angle_schema()),
            (
                "difficulty",
                picklist_str(PUZZLE_DIFFICULTIES, Some("Invalid difficulty.")),
            ),
            (
                "color",
                optional(picklist_str(&["white", "black"], Some("Invalid color."))),
            ),
        ],
        Some("Invalid puzzle request."),
    )
}

fn puzzle_solve_schema() -> Schema {
    object(
        vec![
            ("account", username_schema()),
            ("angle", angle_schema()),
            (
                "id",
                text(
                    Some("Invalid puzzle."),
                    "/^[a-zA-Z0-9]{3,12}$/",
                    patterns::puzzle_id,
                    Some("Invalid puzzle."),
                ),
            ),
            ("win", Schema::Bool(Some("Invalid puzzle result."))),
            ("rated", Schema::Bool(Some("Invalid puzzle result."))),
        ],
        Some("Invalid puzzle result."),
    )
}

fn run_input_schema() -> Schema {
    object(
        vec![
            (
                "kind",
                picklist_str(RUN_KINDS, Some("Invalid kind of run.")),
            ),
            (
                "variant",
                text(
                    Some("Invalid run."),
                    "/^[a-zA-Z0-9_-]{0,40}$/",
                    patterns::run_variant,
                    Some("Invalid run."),
                ),
            ),
            (
                "score",
                pipe(
                    Schema::Num(Some("Invalid score.")),
                    vec![
                        Action::Finite(Some("Invalid score.")),
                        Action::MinValue(0.0, None),
                        Action::MaxValue(1_000_000.0, None),
                    ],
                ),
            ),
            (
                "detail",
                pipe(
                    Schema::Record {
                        key: Box::new(text(
                            None,
                            "/^[a-zA-Z0-9_]{1,30}$/",
                            patterns::run_detail_key,
                            Some("Invalid run."),
                        )),
                        value: Box::new(union(
                            vec![
                                pipe(Schema::Str(None), vec![max_len(80, None)]),
                                pipe(Schema::Num(None), vec![Action::Finite(None)]),
                                Schema::Bool(None),
                            ],
                            None,
                        )),
                        msg: Some("Invalid run."),
                    },
                    vec![Action::Check {
                        check: |detail| match detail {
                            J::Obj(fields) => fields.len() <= 20,
                            _ => true,
                        },
                        msg: Some("Invalid run."),
                    }],
                ),
            ),
        ],
        Some("Invalid run."),
    )
}

fn voice_outcome_schema() -> Schema {
    picklist_str(VOICE_OUTCOMES, Some("Invalid voice entry."))
}

fn lookup_options_schema() -> Schema {
    Schema::Optional {
        wrapped: Box::new(object(
            vec![
                (
                    "speeds",
                    optional(pipe(
                        array(picklist_str(EXPLORER_SPEEDS, None), None),
                        vec![max_len(6, None)],
                    )),
                ),
                (
                    "ratings",
                    optional(pipe(
                        array(picklist_num(EXPLORER_RATINGS, None), None),
                        vec![max_len(9, None)],
                    )),
                ),
                (
                    "player",
                    optional(pipe(
                        Schema::Str(None),
                        vec![Action::Regex {
                            source: USERNAME_SOURCE,
                            test: patterns::username,
                            msg: None,
                        }],
                    )),
                ),
                ("color", optional(picklist_str(&["white", "black"], None))),
                (
                    "modes",
                    optional(pipe(
                        array(picklist_str(&["rated", "casual"], None), None),
                        vec![max_len(2, None)],
                    )),
                ),
                (
                    "since",
                    optional(pipe(
                        Schema::Str(None),
                        vec![Action::Regex {
                            source: SINCE_SOURCE,
                            test: patterns::since,
                            msg: None,
                        }],
                    )),
                ),
            ],
            Some("Invalid explorer filters."),
        )),
        default: Some(J::Obj(Vec::new())),
    }
}

/* ── Validators ── */

schema_validator!(assert_username_v, username_schema());
schema_validator!(assert_game_id_v, game_id_schema());
schema_validator!(
    assert_game_ids_v,
    pipe(
        array(game_id_schema(), Some("Invalid game list.")),
        vec![max_len(100, Some("Too many games."))],
    )
);
schema_validator!(assert_uci_v, uci_schema());
schema_validator!(assert_moves_v, moves_schema());
schema_validator!(assert_level_v, level_schema());
schema_validator!(
    assert_usernames_v,
    pipe(
        array(username_schema(), Some("Invalid users.")),
        vec![max_len(50, Some("Invalid users."))],
    )
);
schema_validator!(
    assert_friend_list_v,
    pipe(
        array(username_schema(), Some("Invalid users.")),
        vec![
            Action::MinLength(1, Some("Choose at least one player.")),
            max_len(1000, Some("Invalid users.")),
        ],
    )
);
schema_validator!(
    assert_action_v,
    picklist_str(ONLINE_ACTIONS, Some("Invalid game action."))
);
schema_validator!(
    assert_chat_room_v,
    picklist_str(CHAT_ROOMS, Some("Invalid chat room."))
);
schema_validator!(
    assert_chat_text_v,
    pipe(
        Schema::Str(Some("Invalid message.")),
        vec![
            Action::Trim,
            Action::MinLength(1, Some("Type a message first.")),
            max_len(140, Some("Chat messages are limited to 140 characters.")),
        ],
    )
);
schema_validator!(
    assert_message_text_v,
    pipe(
        Schema::Str(Some("Invalid message.")),
        vec![
            Action::Trim,
            Action::MinLength(1, Some("Type a message first.")),
            max_len(8000, Some("Messages are limited to 8,000 characters.")),
        ],
    )
);
schema_validator!(
    assert_decline_reason_v,
    picklist_str(DECLINE_REASONS, Some("Invalid reason."))
);
schema_validator!(
    assert_tournament_system_v,
    picklist_str(&["arena", "swiss"], Some("Invalid tournament."))
);
schema_validator!(
    assert_tournament_id_v,
    text(
        Some("Invalid tournament."),
        "/^[a-zA-Z0-9]{8}$/",
        patterns::lichess_id,
        Some("Invalid tournament."),
    )
);
schema_validator!(
    assert_insights_query_v,
    object(
        vec![
            ("account", username_schema()),
            (
                "speed",
                optional(picklist_str(
                    &[
                        "ultraBullet",
                        "bullet",
                        "blitz",
                        "rapid",
                        "classical",
                        "correspondence",
                    ],
                    None,
                )),
            ),
            ("rated", optional(Schema::Bool(None))),
            (
                "days",
                optional(int_in_range(1.0, 3650.0, Some("Invalid period."))),
            ),
        ],
        Some("Invalid insights filter."),
    )
);
schema_validator!(
    assert_perf_type_v,
    picklist_str(PERF_TYPES, Some("Invalid rating category."))
);
schema_validator!(assert_lichess_id_v, lichess_id_schema("Invalid id."));
schema_validator!(assert_optional_account_v, optional_account_schema());
schema_validator!(assert_online_options_v, online_options_schema());
schema_validator!(assert_theme_v, theme_schema());
schema_validator!(assert_notification_v, notification_schema());
schema_validator!(
    assert_settings_v,
    object(settings_entries(), Some("Invalid settings."))
);
schema_validator!(assert_puzzle_request_v, puzzle_request_schema());
schema_validator!(assert_puzzle_solve_v, puzzle_solve_schema());
schema_validator!(
    assert_days_v,
    int_in_range(1.0, 365.0, Some("Invalid number of days."))
);
schema_validator!(
    assert_activity_max_v,
    int_in_range(1.0, 200.0, Some("Invalid number of puzzles."))
);
schema_validator!(
    assert_local_query_v,
    object(
        vec![
            ("theme", optional(angle_schema())),
            (
                "minRating",
                optional(int_in_range(0.0, 4000.0, Some("Invalid rating."))),
            ),
            (
                "maxRating",
                optional(int_in_range(0.0, 4000.0, Some("Invalid rating."))),
            ),
            (
                "count",
                int_in_range(1.0, 300.0, Some("Invalid puzzle count.")),
            ),
        ],
        Some("Invalid puzzle query."),
    )
);
schema_validator!(
    assert_ladder_query_v,
    object(
        vec![
            ("from", int_in_range(0.0, 4000.0, Some("Invalid rating."))),
            ("to", int_in_range(0.0, 4000.0, Some("Invalid rating."))),
            (
                "count",
                int_in_range(1.0, 300.0, Some("Invalid puzzle count.")),
            ),
        ],
        Some("Invalid puzzle query."),
    )
);
schema_validator!(
    assert_run_kind_v,
    picklist_str(RUN_KINDS, Some("Invalid kind of run."))
);
schema_validator!(assert_run_input_v, run_input_schema());
schema_validator!(
    assert_best_move_options_v,
    Schema::Optional {
        wrapped: Box::new(object(
            vec![
                (
                    "fen",
                    optional(pipe(
                        Schema::Str(Some("Invalid position.")),
                        vec![
                            max_len(100, None),
                            Action::Regex {
                                source: FEN_SOURCE,
                                test: patterns::fen,
                                msg: Some("Invalid position."),
                            },
                        ],
                    )),
                ),
                (
                    "movetime",
                    optional(int_in_range(50.0, 5000.0, Some("Invalid think time."))),
                ),
                (
                    "chess960",
                    optional(Schema::Bool(Some("Invalid engine options."))),
                ),
            ],
            Some("Invalid engine options."),
        )),
        default: Some(J::Obj(Vec::new())),
    }
);
schema_validator!(
    assert_review_key_v,
    text(
        Some("Invalid review."),
        "/^[0-9a-f]{16}$/",
        patterns::review_key,
        Some("Invalid review."),
    )
);
schema_validator!(assert_voice_attempt_v, voice_attempt_schema());
schema_validator!(
    assert_voice_update_v,
    object(
        vec![
            ("outcome", optional(voice_outcome_schema())),
            ("expected", optional(voice_text(20))),
        ],
        Some("Invalid voice entry."),
    )
);
schema_validator!(
    assert_voice_id_v,
    int_in_range(1.0, MAX_SAFE_INTEGER, Some("Invalid voice entry."))
);
schema_validator!(
    assert_voice_limit_v,
    int_in_range(1.0, 5000.0, Some("Invalid number of entries."))
);
schema_validator!(
    assert_game_page_query_v,
    object(
        vec![
            ("account", optional(username_schema())),
            (
                "result",
                optional(picklist_str(&["win", "loss", "draw"], None)),
            ),
            ("rated", optional(Schema::Bool(None))),
            (
                "offset",
                int_in_range(0.0, MAX_SAFE_INTEGER, Some("Invalid game offset.")),
            ),
            (
                "limit",
                int_in_range(1.0, 100.0, Some("Invalid game page size.")),
            ),
        ],
        None,
    )
);
schema_validator!(
    assert_broadcast_query_v,
    Schema::Optional {
        wrapped: Box::new(pipe(
            Schema::Str(None),
            vec![Action::Trim, max_len(100, None)],
        )),
        default: None,
    }
);
schema_validator!(assert_lookup_options_v, lookup_options_schema());

/// `assertTournamentPassword`: nothing or an empty password means none.
fn assert_tournament_password_v(mut args: Vec<J>) -> Result<J, String> {
    static SCHEMA: LazyLock<Schema> = LazyLock::new(|| {
        pipe(
            Schema::Str(Some("Invalid password.")),
            vec![max_len(100, Some("Invalid password."))],
        )
    });
    match take(&mut args, 0) {
        J::Undef => Ok(J::Null),
        J::Str(s) if s.is_empty() => Ok(J::Null),
        value => parse(&SCHEMA, value),
    }
}

/// `assertWatchTarget(value, channels)`: a channel from `channels`, or a game id.
fn assert_watch_target_v(mut args: Vec<J>) -> Result<J, String> {
    let value = take(&mut args, 0);
    let channels = match take(&mut args, 1) {
        J::Arr(items) => items,
        _ => Vec::new(),
    };
    let schema = union(
        vec![
            object(
                vec![(
                    "channel",
                    Schema::Picklist {
                        options: channels,
                        msg: None,
                    },
                )],
                None,
            ),
            object(vec![("gameId", game_id_schema())], None),
        ],
        Some("Invalid game to watch."),
    );
    parse(&schema, value)
}

/// `assertExport`: the request's shape, then that its bytes are the file they claim to be.
fn assert_export_v(mut args: Vec<J>) -> Result<J, String> {
    static SCHEMA: LazyLock<Schema> = LazyLock::new(|| {
        object(
            vec![
                (
                    "name",
                    pipe(
                        Schema::Str(None),
                        vec![Action::Regex {
                            source: r"/^[\w .()+-]{1,80}$/",
                            test: patterns::file_name,
                            msg: Some("Invalid file name."),
                        }],
                    ),
                ),
                (
                    "kind",
                    picklist_str(&["gif", "png", "pgn"], Some("Invalid export.")),
                ),
                (
                    "data",
                    union(
                        vec![
                            Schema::Bytes(None),
                            pipe(Schema::Str(None), vec![max_len(2_000_000, None)]),
                        ],
                        None,
                    ),
                ),
            ],
            None,
        )
    });
    let request = parse(&SCHEMA, take(&mut args, 0))?;
    let invalid = || Err("Invalid export.".to_string());
    let kind = request.get("kind").and_then(J::str).unwrap_or_default();
    let data = request.get("data");
    if kind == "pgn" {
        if !matches!(data, Some(J::Str(_))) {
            return invalid();
        }
        return Ok(request);
    }
    let Some(J::Bytes { len, head }) = data else {
        return invalid();
    };
    if *len > EXPORT_MAX {
        return invalid();
    }
    let magic: &[u8] = if kind == "gif" {
        b"GIF8"
    } else {
        &[137, 80, 78, 71]
    };
    if head.as_slice() != magic {
        return invalid();
    }
    Ok(request)
}

/// `assertStudySyncRequest`: a study's account, id and both PGN copies, bounded.
fn assert_study_sync_request_v(mut args: Vec<J>) -> Result<J, String> {
    static LICHESS: LazyLock<Schema> = LazyLock::new(|| lichess_id_schema("Invalid id."));
    static USER: LazyLock<Schema> = LazyLock::new(username_schema);
    let value = take(&mut args, 0);
    if !value.is_object() {
        return Err("Invalid study synchronization.".into());
    }
    let field = |key: &str| value.get(key).cloned().unwrap_or(J::Undef);
    let account = parse(&USER, field("account"))?;
    let study_id = parse(&LICHESS, field("studyId"))?;
    let (baseline, pgn) = (field("baseline"), field("pgn"));
    let usable = |v: &J| match v {
        J::Str(s) => !crate::js::trim(s).is_empty() && crate::js::utf16_len(s) <= 500_000,
        _ => false,
    };
    if !usable(&baseline) || !usable(&pgn) {
        return Err("The study is empty or too large to upload.".into());
    }
    Ok(J::Obj(vec![
        ("account".into(), account),
        ("studyId".into(), study_id),
        ("baseline".into(), baseline),
        ("pgn".into(), pgn),
    ]))
}

/// `assertAnalysisRequest`: the request's shape, and a position its moves reach.
fn assert_analysis_request_v(mut args: Vec<J>) -> Result<J, String> {
    static SCHEMA: LazyLock<Schema> = LazyLock::new(analysis_request_schema);
    let request = parse(&SCHEMA, take(&mut args, 0))?;
    let fen = request.get("fen").and_then(J::str).unwrap_or_default();
    let root = request.get("rootFen").and_then(J::str).unwrap_or(fen);
    let moves = str_list(request.get("moves"));
    let positions = replay::replay_positions(root, &moves);
    let normalized = replay::replay_positions(fen, &Vec::<&str>::new())
        .first()
        .map(|p| p.fen.clone());
    let last = positions.last().map(|p| p.fen.as_str());
    if normalized.is_none() || positions.len() != moves.len() + 1 || last != normalized.as_deref() {
        return Err("Invalid analysis position or move history.".into());
    }
    Ok(request)
}

/// `assertReviewRequest`: the request's shape, and moves that replay from its position.
fn assert_review_request_v(mut args: Vec<J>) -> Result<J, String> {
    static SCHEMA: LazyLock<Schema> = LazyLock::new(review_request_schema);
    let request = parse(&SCHEMA, take(&mut args, 0))?;
    let fen = request.get("fen").and_then(J::str).unwrap_or_default();
    let moves = str_list(request.get("moves"));
    if replay::replay_positions_count(fen, &moves) != moves.len() + 1 {
        return Err("Invalid game position or move history.".into());
    }
    Ok(request)
}

/// The strings of a list (absent or not a list gives none).
fn str_list(v: Option<&J>) -> Vec<&str> {
    match v {
        Some(J::Arr(items)) => items.iter().filter_map(J::str).collect(),
        _ => Vec::new(),
    }
}

/// The validators by their TypeScript names.
pub fn validator(method: &str) -> Option<Validator> {
    Some(match method {
        "assertUsername" => assert_username_v,
        "assertGameId" => assert_game_id_v,
        "assertGameIds" => assert_game_ids_v,
        "assertUci" => assert_uci_v,
        "assertMoves" => assert_moves_v,
        "assertLevel" => assert_level_v,
        "assertUsernames" => assert_usernames_v,
        "assertFriendList" => assert_friend_list_v,
        "assertAction" => assert_action_v,
        "assertChatRoom" => assert_chat_room_v,
        "assertChatText" => assert_chat_text_v,
        "assertNewArena" => assert_new_arena_v,
        "assertMessageText" => assert_message_text_v,
        "assertDeclineReason" => assert_decline_reason_v,
        "assertTournamentSystem" => assert_tournament_system_v,
        "assertTournamentId" => assert_tournament_id_v,
        "assertTournamentPassword" => assert_tournament_password_v,
        "assertExport" => assert_export_v,
        "assertInsightsQuery" => assert_insights_query_v,
        "assertPerfType" => assert_perf_type_v,
        "assertLichessId" => assert_lichess_id_v,
        "assertWatchTarget" => assert_watch_target_v,
        "assertOptionalAccount" => assert_optional_account_v,
        "assertOnlineOptions" => assert_online_options_v,
        "assertTheme" => assert_theme_v,
        "assertNotification" => assert_notification_v,
        "assertCoreSettings" => assert_core_settings_v,
        "assertSettings" => assert_settings_v,
        "assertPuzzleRequest" => assert_puzzle_request_v,
        "assertPuzzleSolve" => assert_puzzle_solve_v,
        "assertDays" => assert_days_v,
        "assertActivityMax" => assert_activity_max_v,
        "assertLocalQuery" => assert_local_query_v,
        "assertLadderQuery" => assert_ladder_query_v,
        "assertRunKind" => assert_run_kind_v,
        "assertRunInput" => assert_run_input_v,
        "assertBestMoveOptions" => assert_best_move_options_v,
        "assertAnalysisRequest" => assert_analysis_request_v,
        "assertReviewRequest" => assert_review_request_v,
        "assertReviewKey" => assert_review_key_v,
        "assertVoiceAttempt" => assert_voice_attempt_v,
        "assertVoiceUpdate" => assert_voice_update_v,
        "assertVoiceId" => assert_voice_id_v,
        "assertVoiceLimit" => assert_voice_limit_v,
        "assertGamePageQuery" => assert_game_page_query_v,
        "assertStudySyncRequest" => assert_study_sync_request_v,
        "assertBroadcastQuery" => assert_broadcast_query_v,
        "assertLookupOptions" => assert_lookup_options_v,
        _ => return None,
    })
}

/// `assertCoreSettings`: `pick(settingsSchema, CORE_SETTINGS_KEYS)`, keeping the settings message.
fn assert_core_settings_v(mut args: Vec<J>) -> Result<J, String> {
    static SCHEMA: LazyLock<Schema> = LazyLock::new(|| {
        let mut all = settings_entries();
        let picked = CORE_SETTINGS_KEYS
            .iter()
            .filter_map(|key| {
                let at = all.iter().position(|(name, _)| name == key)?;
                Some(all.remove(at))
            })
            .collect();
        object(picked, Some("Invalid settings."))
    });
    parse(&SCHEMA, take(&mut args, 0))
}

/// `assertNewArena`: a strict object of the arena's clock, schedule and options.
fn assert_new_arena_v(mut args: Vec<J>) -> Result<J, String> {
    static SCHEMA: LazyLock<Schema> = LazyLock::new(|| {
        strict_object(
            vec![
                (
                    "name",
                    optional(pipe(
                        Schema::Str(None),
                        vec![
                            Action::Trim,
                            max_len(30, Some("Arena names are limited to 30 characters.")),
                            Action::Regex {
                                source: r"/^[\p{L}\p{N} ,.'&()-]*$/u",
                                test: patterns::arena_name,
                                msg: Some("Use letters, numbers and simple punctuation."),
                            },
                        ],
                    )),
                ),
                (
                    "clockTime",
                    picklist_num(ARENA_CLOCK_MINUTES, Some("Invalid clock.")),
                ),
                (
                    "clockIncrement",
                    picklist_num(ARENA_INCREMENTS, Some("Invalid increment.")),
                ),
                (
                    "minutes",
                    picklist_num(ARENA_DURATIONS, Some("Invalid duration.")),
                ),
                (
                    "waitMinutes",
                    optional(picklist_num(ARENA_WAIT_MINUTES, Some("Invalid start."))),
                ),
                (
                    "startDate",
                    optional(pipe(
                        Schema::Num(None),
                        vec![Action::Integer(None), Action::MinValue(0.0, None)],
                    )),
                ),
                (
                    "variant",
                    picklist_str(ARENA_VARIANTS, Some("Invalid variant.")),
                ),
                ("rated", Schema::Bool(None)),
                (
                    "password",
                    optional(pipe(Schema::Str(None), vec![max_len(60, None)])),
                ),
                (
                    "description",
                    optional(pipe(Schema::Str(None), vec![max_len(2000, None)])),
                ),
            ],
            None,
        )
    });
    parse(&SCHEMA, take(&mut args, 0))
}

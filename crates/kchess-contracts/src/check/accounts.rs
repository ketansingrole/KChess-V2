//! The arguments of the settings, accounts, Chess.com, synced games and player methods.

use serde::{Deserialize, Serialize};

use kchess_domain::js;
use kchess_domain::misc::patterns;
use kchess_domain::misc::value::J;
use kchess_domain::variant::VARIANTS;

use super::*;
use crate::types::{
    APPEARANCES, COORDINATE_MODES, CoreSettings, ENGINE_LEVELS, EXPLORER_SPEEDS,
    FrontendPreferences, GamePageQuery, InsightsQuery, PERF_TYPES, PIECE_ANIMATIONS,
    PROMOTION_MODES, PerfType, RELATION_ACTIONS, REVIEW_AUTO, RelationAction, Settings,
};

/* ── Names and ids ── */

/// The account synced games are kept under: a Lichess username, or a Chess.com player's
/// `name@chess.com`. Trimmed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct GameAccount(pub String);

impl Validate for GameAccount {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text: String = decode(value, |_| Some(USERNAME))?;
        game_account(&text, "").map(GameAccount)
    }
}

fn game_account(text: &str, path: &str) -> Result<String, Refusal> {
    let trimmed = js::trim(text);
    matches(trimmed, patterns::game_account, Some(USERNAME), path, "")?;
    Ok(trimmed.to_owned())
}

const CHESSCOM_USERNAME: &str = "Enter a valid Chess.com username.";

/// A Chess.com username, trimmed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct ChesscomUsername(pub String);

impl Validate for ChesscomUsername {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text: String = decode(value, |_| Some(CHESSCOM_USERNAME))?;
        let trimmed = js::trim(&text);
        matches(
            trimmed,
            patterns::chesscom_username,
            Some(CHESSCOM_USERNAME),
            "",
            "",
        )?;
        Ok(ChesscomUsername(trimmed.to_owned()))
    }
}

/// Players to add or look up at once: 1 to 1,000 Lichess usernames, each trimmed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct FriendList(pub Vec<String>);

impl Validate for FriendList {
    fn validate(value: J) -> Result<Self, Refusal> {
        let names: Vec<String> = decode(value, |keys| match keys {
            [] => Some("Invalid users."),
            _ => Some(USERNAME),
        })?;
        let names = names
            .into_iter()
            .map(|name| Username::validate(J::Str(name)).map(|Username(name)| name))
            .collect::<Result<Vec<_>, _>>()?;
        ensure(
            !names.is_empty(),
            Some("Choose at least one player."),
            "",
            String::new,
        )?;
        max_items(&names, 1000, Some("Invalid users."), "")?;
        Ok(FriendList(names))
    }
}

/// A private message to send, trimmed: 1 to 8,000 characters.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct MessageText(pub String);

impl Validate for MessageText {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text: String = decode(value, |_| Some("Invalid message."))?;
        let text = js::trim(&text);
        min_chars(text, 1, Some("Type a message first."), "")?;
        max_chars(
            text,
            8000,
            Some("Messages are limited to 8,000 characters."),
            "",
        )?;
        Ok(MessageText(text.to_owned()))
    }
}

impl Validate for PerfType {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, PERF_TYPES, Some("Invalid rating category."))
    }
}

impl Validate for RelationAction {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, RELATION_ACTIONS, Some("Invalid action."))
    }
}

/* ── Settings ── */

const NOTIFY: &str = "Invalid notification setting.";

/// The message of each setting.
fn settings_messages(keys: &[&str]) -> Msg {
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

/// A wire value of a settings choice the checks passed, as its enum.
fn chosen<T: serde::de::DeserializeOwned>(text: String) -> Result<T, Refusal> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(|error| error.to_string())
}

/// The core's settings as sent: their strings checked one by one, for their messages.
#[derive(Deserialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct SentCoreSettings {
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

/// The core's settings after the engine path, checked in their order. The computer levels are
/// kept in the ladder's order, without repeats; at least one must be chosen.
fn core_settings(sent: SentCoreSettings) -> Result<CoreSettings, Refusal> {
    max_chars(&sent.engine_path, 4096, Some("Invalid engine path."), "")?;
    for level in &sent.engine_levels {
        one_of(level, ENGINE_LEVELS, Some("Invalid engine level."), "")?;
    }
    ensure(
        !sent.engine_levels.is_empty(),
        Some("Keep at least one computer level."),
        "",
        String::new,
    )?;
    let engine_levels = ENGINE_LEVELS
        .iter()
        .filter(|level| sent.engine_levels.iter().any(|chosen| chosen == *level))
        .map(|level| chosen((*level).to_owned()))
        .collect::<Result<_, _>>()?;
    one_of(
        &sent.review_auto,
        REVIEW_AUTO,
        Some("Invalid review setting."),
        "",
    )?;
    let poll = Some("Invalid correspondence check interval.");
    within(sent.correspondence_poll, 0, 120, poll, "")?;
    Ok(CoreSettings {
        engine_path: sent.engine_path,
        voice_history: sent.voice_history,
        engine_levels,
        review_auto: chosen(sent.review_auto)?,
        review_on_battery: sent.review_on_battery,
        receive_challenges: sent.receive_challenges,
        online_chat: sent.online_chat,
        correspondence_poll: sent.correspondence_poll as f64,
        cloud_eval: sent.cloud_eval,
    })
}

impl Validate for CoreSettings {
    fn validate(value: J) -> Result<Self, Refusal> {
        core_settings(decode(value, settings_messages)?)
    }
}

/// Every setting as sent, in the frontends' order.
#[derive(Deserialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct SentSettings {
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

fn on() -> bool {
    true
}

fn theme_setting(id: &str, msg: &'static str) -> Result<(), Refusal> {
    matches(id, patterns::theme_id, Some(msg), "", "")
}

impl Validate for Settings {
    fn validate(value: J) -> Result<Self, Refusal> {
        let s: SentSettings = decode(value, settings_messages)?;
        one_of(&s.appearance, APPEARANCES, Some("Invalid appearance."), "")?;
        theme_setting(&s.board_theme, "Invalid board theme.")?;
        theme_setting(&s.light_theme, "Invalid color theme.")?;
        theme_setting(&s.dark_theme, "Invalid color theme.")?;
        theme_setting(&s.piece_set, "Invalid piece set.")?;
        let animation = Some("Invalid piece animation.");
        one_of(&s.piece_animation, PIECE_ANIMATIONS, animation, "")?;
        let coordinates = Some("Invalid coordinates mode.");
        one_of(&s.coordinates, COORDINATE_MODES, coordinates, "")?;
        ensure(
            s.sound_volume.is_finite(),
            Some("Invalid sound volume."),
            "",
            String::new,
        )?;
        let promotion = Some("Invalid promotion setting.");
        let core = SentCoreSettings {
            engine_path: s.engine_path,
            engine_levels: s.engine_levels,
            review_auto: s.review_auto,
            review_on_battery: s.review_on_battery,
            receive_challenges: s.receive_challenges,
            online_chat: s.online_chat,
            correspondence_poll: s.correspondence_poll,
            cloud_eval: s.cloud_eval,
            voice_history: s.voice_history,
        };
        max_chars(&core.engine_path, 4096, Some("Invalid engine path."), "")?;
        one_of(&s.promotion, PROMOTION_MODES, promotion, "")?;
        Ok(Settings {
            core: core_settings(core)?,
            frontend: FrontendPreferences {
                appearance: chosen(s.appearance)?,
                board_theme: s.board_theme,
                light_theme: s.light_theme,
                dark_theme: s.dark_theme,
                piece_set: s.piece_set,
                piece_animation: chosen(s.piece_animation)?,
                coordinates: chosen(s.coordinates)?,
                sound_enabled: s.sound_enabled,
                sound_volume: s.sound_volume.clamp(0.0, 1.0),
                premove: s.premove,
                promotion: chosen(s.promotion)?,
                show_legal_moves: s.show_legal_moves,
                notifications_enabled: s.notifications_enabled,
                notify_active: s.notify_active,
                notify_background: s.notify_background,
                notify_opponent_move: s.notify_opponent_move,
                notify_low_time: s.notify_low_time,
                notify_game_events: s.notify_game_events,
                notify_computer_move: s.notify_computer_move,
                notify_sound: s.notify_sound,
                voice_push_to_talk: s.voice_push_to_talk,
                voice_confirm_moves: s.voice_confirm_moves,
                update_auto_check: s.update_auto_check,
                update_auto_download: s.update_auto_download,
                update_install_on_quit: s.update_install_on_quit,
                notify_challenges: s.notify_challenges,
                zen_mode: s.zen_mode,
                blindfold: s.blindfold,
                show_opening_name: s.show_opening_name,
                swipe_navigation: s.swipe_navigation,
                swipe_indicator: s.swipe_indicator,
                puzzle_auto_next: s.puzzle_auto_next,
                confirm_correspondence: s.confirm_correspondence,
                figurine_notation: s.figurine_notation,
            },
        })
    }
}

/* ── Synced games ── */

const GAME_SEARCH: &str = "Invalid game search.";
const MAX_SAFE: u64 = 9_007_199_254_740_991;

impl Validate for GamePageQuery {
    fn validate(value: J) -> Result<Self, Refusal> {
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
}

/// The insights filter as sent: its strings checked one by one, for their messages.
#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentInsightsQuery {
    account: String,
    speed: Option<String>,
    rated: Option<bool>,
    days: Option<u64>,
    variant: Option<String>,
}

impl Validate for InsightsQuery {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentInsightsQuery = decode(value, |keys| match keys {
            [] => Some("Invalid insights filter."),
            ["account"] => Some(USERNAME),
            ["days"] => Some("Invalid period."),
            _ => None,
        })?;
        let account = game_account(&sent.account, "account")?;
        let speed = match sent.speed {
            Some(speed) => {
                one_of(&speed, EXPLORER_SPEEDS, None, "speed")?;
                Some(chosen(speed)?)
            }
            None => None,
        };
        if let Some(days) = sent.days {
            within(days, 1, 3650, Some("Invalid period."), "days")?;
        }
        let variant = match sent.variant {
            Some(key) => {
                one_of(&key, VARIANTS, Some("Invalid variant."), "variant")?;
                kchess_domain::variant::Variant::from_key(&key)
            }
            None => None,
        };
        Ok(InsightsQuery {
            account,
            speed,
            rated: sent.rated,
            days: sent.days,
            variant,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_and_messages_are_trimmed() {
        assert_eq!(checked::<GameAccount>(json!(" Alice ")), Ok(json!("Alice")));
        assert_eq!(
            checked::<GameAccount>(json!(" hikaru@chess.com ")),
            Ok(json!("hikaru@chess.com"))
        );
        assert_eq!(checked::<GameAccount>(json!("a b")), Err(USERNAME.into()));
        assert_eq!(
            checked::<ChesscomUsername>(json!(" hikaru_n ")),
            Ok(json!("hikaru_n"))
        );
        assert_eq!(
            checked::<ChesscomUsername>(json!("a")),
            Err(CHESSCOM_USERNAME.into())
        );
        assert_eq!(
            checked::<FriendList>(json!([" Alice ", "bob42"])),
            Ok(json!(["Alice", "bob42"]))
        );
        assert_eq!(
            checked::<FriendList>(json!([])),
            Err("Choose at least one player.".into())
        );
        assert_eq!(checked::<MessageText>(json!("  hi  ")), Ok(json!("hi")));
        assert_eq!(
            checked::<MessageText>(json!("   ")),
            Err("Type a message first.".into())
        );
    }

    #[test]
    fn a_game_result_filter_is_one_of_three() {
        let query = J::Obj(vec![
            ("result".into(), J::Str("won".into())),
            ("offset".into(), J::Num(0.0)),
            ("limit".into(), J::Num(10.0)),
        ]);
        assert_eq!(
            check::<GamePageQuery>(query),
            Err("result: unknown variant `won`, expected one of `win`, `loss`, `draw`".into())
        );
    }

    #[test]
    fn a_relation_is_one_of_four_actions() {
        assert_eq!(
            check::<RelationAction>(J::Str("block".into())),
            Ok(RelationAction::Block)
        );
        assert_eq!(
            check::<RelationAction>(J::Str("mute".into())),
            Err("Invalid action.".into())
        );
    }

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    fn settings() -> Vec<(&'static str, J)> {
        let mut fields = vec![
            ("appearance", J::Str("system".into())),
            ("boardTheme", J::Str("classic".into())),
            ("lightTheme", J::Str("light".into())),
            ("darkTheme", J::Str("dark".into())),
            ("pieceSet", J::Str("cburnett".into())),
            ("pieceAnimation", J::Str("normal".into())),
            ("coordinates", J::Str("inside".into())),
            ("soundVolume", J::Num(7.0)),
            ("enginePath", J::Str(String::new())),
            ("promotion", J::Str("ask".into())),
            (
                "engineLevels",
                J::Arr(vec![J::Str("gm".into()), J::Str("club".into())]),
            ),
            ("reviewAuto", J::Str("off".into())),
            ("correspondencePoll", J::Num(30.0)),
        ];
        for flag in [
            "soundEnabled",
            "premove",
            "showLegalMoves",
            "notificationsEnabled",
            "notifyActive",
            "notifyBackground",
            "notifyOpponentMove",
            "notifyLowTime",
            "notifyGameEvents",
            "notifyComputerMove",
            "notifySound",
            "voicePushToTalk",
            "voiceConfirmMoves",
            "voiceHistory",
            "updateAutoCheck",
            "updateAutoDownload",
            "updateInstallOnQuit",
            "reviewOnBattery",
            "receiveChallenges",
            "notifyChallenges",
            "onlineChat",
            "zenMode",
            "blindfold",
            "cloudEval",
            "showOpeningName",
            "swipeNavigation",
            "swipeIndicator",
            "puzzleAutoNext",
            "confirmCorrespondence",
            "figurineNotation",
        ] {
            fields.push((flag, J::Bool(true)));
        }
        fields
    }

    #[test]
    fn settings_are_normalized() {
        let normalized = check::<Settings>(obj(settings())).unwrap();
        assert_eq!(normalized.frontend.sound_volume, 1.0);
        assert_eq!(
            normalized.core.engine_levels,
            [
                crate::types::EngineLevel::Club,
                crate::types::EngineLevel::Gm
            ]
        );
    }

    /// A frontend from before move confirmation sends no `confirmCorrespondence`: its settings
    /// still save, with confirmation on.
    #[test]
    fn settings_without_move_confirmation_keep_it_on() {
        let older: Vec<_> = settings()
            .into_iter()
            .filter(|(key, _)| *key != "confirmCorrespondence")
            .collect();
        let normalized = check::<Settings>(obj(older)).unwrap();
        assert!(normalized.frontend.confirm_correspondence);
        let mut off = settings();
        off.retain(|(key, _)| *key != "confirmCorrespondence");
        off.push(("confirmCorrespondence", J::Bool(false)));
        let normalized = check::<Settings>(obj(off)).unwrap();
        assert!(!normalized.frontend.confirm_correspondence);
        let mut wrong = settings();
        wrong.retain(|(key, _)| *key != "confirmCorrespondence");
        wrong.push(("confirmCorrespondence", J::Num(1.0)));
        assert_eq!(
            check::<Settings>(obj(wrong)).unwrap_err(),
            "Invalid move confirmation setting."
        );
    }

    #[test]
    fn each_setting_keeps_its_message() {
        for (key, value, message) in [
            ("notificationsEnabled", J::Num(1.0), NOTIFY),
            ("notifyLowTime", J::Null, NOTIFY),
            (
                "engineLevels",
                J::Arr(vec![J::Num(1.0)]),
                "Invalid engine level.",
            ),
            (
                "engineLevels",
                J::Arr(vec![]),
                "Keep at least one computer level.",
            ),
            ("soundVolume", J::Num(f64::NAN), "Invalid sound volume."),
            (
                "soundVolume",
                J::Num(f64::INFINITY),
                "Invalid sound volume.",
            ),
            (
                "correspondencePoll",
                J::Num(1.5),
                "Invalid correspondence check interval.",
            ),
            ("lightTheme", J::Str("Bad".into()), "Invalid color theme."),
        ] {
            let mut fields = settings();
            fields.iter_mut().find(|(k, _)| *k == key).unwrap().1 = value;
            assert_eq!(
                check::<Settings>(obj(fields)).map(|_| ()),
                Err(message.into()),
                "{key}"
            );
        }
        let mut missing = settings();
        missing.retain(|(k, _)| *k != "zenMode");
        assert_eq!(
            check::<Settings>(obj(missing)).map(|_| ()),
            Err("Invalid settings.".into())
        );
    }

    #[test]
    fn an_optional_account_is_empty_or_a_name() {
        assert_eq!(
            check::<OptionalAccount>(J::Str(String::new())),
            Ok(OptionalAccount(String::new()))
        );
        assert_eq!(
            check::<OptionalAccount>(J::Str(" Alice ".into())),
            Ok(OptionalAccount("Alice".into()))
        );
        assert_eq!(
            check::<OptionalAccount>(J::Null),
            Err("Invalid account.".into())
        );
    }
}

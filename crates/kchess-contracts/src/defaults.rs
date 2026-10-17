//! `crates/kchess-contracts/ts/defaultSettings.ts`: the default preferences, as data.

use kchess_domain::training;
use ts_rs::Config;

use crate::generate::Item;
use crate::types::{
    Appearance, CoordinateMode, CoreSettings, EngineLevel, FrontendPreferences, PieceAnimation,
    PromotionMode, ReviewAuto, Settings,
};

/// Minutes between correspondence-game polls.
const DEFAULT_CORRESPONDENCE_POLL_MINUTES: f64 = 5.0;
/// Default sound volume (0.0–1.0).
const DEFAULT_SOUND_VOLUME: f64 = 0.7;

/// The engine levels in `Settings`, from the domain's training data.
fn default_engine_levels() -> Result<Vec<EngineLevel>, String> {
    let Some(levels) = training::call("defaultEngineLevels", &[]) else {
        return Err("defaultEngineLevels is not a training method".into());
    };
    let levels =
        levels.map_err(|cause| format!("the bundled engine levels are unreadable: {cause}"))?;
    serde_json::from_value(levels)
        .map_err(|cause| format!("the bundled engine levels are not known levels: {cause}"))
}

/// The default preferences. The engine levels are the domain's data (`training` data).
pub fn default_settings() -> Result<Settings, String> {
    let engine_levels = default_engine_levels()?;
    Ok(Settings {
        core: CoreSettings {
            engine_path: String::new(),
            voice_history: true,
            engine_levels,
            review_auto: ReviewAuto::Recent,
            review_on_battery: false,
            receive_challenges: true,
            online_chat: true,
            correspondence_poll: DEFAULT_CORRESPONDENCE_POLL_MINUTES,
            cloud_eval: false,
        },
        frontend: FrontendPreferences {
            appearance: Appearance::System,
            board_theme: "brown".into(),
            light_theme: "kchess".into(),
            dark_theme: "kchess".into(),
            piece_set: "cburnett".into(),
            piece_animation: PieceAnimation::Normal,
            coordinates: CoordinateMode::Inside,
            sound_enabled: true,
            sound_volume: DEFAULT_SOUND_VOLUME,
            premove: true,
            promotion: PromotionMode::Ask,
            show_legal_moves: true,
            notifications_enabled: true,
            notify_active: false,
            notify_background: true,
            notify_opponent_move: true,
            notify_low_time: true,
            notify_game_events: true,
            notify_computer_move: true,
            notify_sound: false,
            voice_push_to_talk: false,
            voice_confirm_moves: true,
            update_auto_check: true,
            update_auto_download: true,
            update_install_on_quit: true,
            notify_challenges: true,
            zen_mode: false,
            blindfold: false,
            show_opening_name: true,
            swipe_navigation: true,
            swipe_indicator: true,
            puzzle_auto_next: true,
        },
    })
}

pub fn items(_cfg: &Config) -> Result<Vec<Item>, String> {
    let settings = default_settings()?;
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|cause| format!("the default settings do not serialize: {cause}"))?;
    Ok(vec![Item::Raw {
        name: "DEFAULT_SETTINGS".into(),
        text: format!("export const DEFAULT_SETTINGS: Settings = {json};"),
    }])
}

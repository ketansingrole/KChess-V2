//! `crates/kchess-contracts/ts/defaultSettings.ts`: the default preferences, as data.

use kchess_domain::training;
use ts_rs::Config;

use crate::generate::Item;
use crate::types::{
    Appearance, CoordinateMode, CoreSettings, EngineLevel, FrontendPreferences, PieceAnimation,
    PromotionMode, ReviewAuto, Settings,
};

/// The default preferences. The engine levels are the domain's data (`training` data).
pub fn default_settings() -> Settings {
    let levels = training::call("defaultEngineLevels", &[])
        .expect("defaultEngineLevels is a training method")
        .expect("the bundled engine levels are readable");
    let engine_levels: Vec<EngineLevel> =
        serde_json::from_value(levels).expect("default engine levels are known levels");
    Settings {
        core: CoreSettings {
            engine_path: String::new(),
            voice_history: true,
            engine_levels,
            review_auto: ReviewAuto::Recent,
            review_on_battery: false,
            receive_challenges: true,
            online_chat: true,
            correspondence_poll: 5.0,
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
            sound_volume: 0.7,
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
        },
    }
}

pub fn items(_cfg: &Config) -> Vec<Item> {
    let json = serde_json::to_string_pretty(&default_settings()).expect("settings serialize");
    vec![Item::Raw {
        name: "DEFAULT_SETTINGS".into(),
        text: format!("export const DEFAULT_SETTINGS: Settings = {json};"),
    }]
}

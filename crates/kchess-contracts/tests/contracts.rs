//! The contract shapes as frontends rely on them: wire values, defaults and the method table.

use kchess_contracts::defaults::default_settings;
use kchess_contracts::types::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

/// Every wire value reads as a variant and writes back as itself, the variants are as many as the
/// values, and a value outside the list is refused.
fn wire_values_round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    name: &str,
    values: &[&str],
) {
    let mut seen = Vec::new();
    for value in values {
        let parsed: T = serde_json::from_value(json!(value))
            .unwrap_or_else(|cause| panic!("{name}: {value} does not read: {cause}"));
        assert_eq!(
            serde_json::to_value(&parsed).ok(),
            Some(json!(value)),
            "{name}"
        );
        assert!(!seen.contains(&parsed), "{name}: {value} repeats a variant");
        seen.push(parsed);
    }
    assert!(
        serde_json::from_value::<T>(json!("not-a-wire-value")).is_err(),
        "{name} accepts an unknown value"
    );
}

#[test]
fn every_wire_enum_round_trips_its_listed_values() {
    macro_rules! check {
        ($($ty:ty: $values:ident),+ $(,)?) => {
            $(wire_values_round_trip::<$ty>(stringify!($ty), $values);)+
        };
    }
    check!(
        Appearance: APPEARANCES,
        ChallengeColor: CHALLENGE_COLORS,
        ChatRoom: CHAT_ROOMS,
        CoordinateMode: COORDINATE_MODES,
        DeclineReason: DECLINE_REASONS,
        EngineLevel: ENGINE_LEVELS,
        ExplorerSpeed: EXPLORER_SPEEDS,
        OnlineAction: ONLINE_ACTIONS,
        PerfType: PERF_TYPES,
        PieceAnimation: PIECE_ANIMATIONS,
        PositionLookupKind: POSITION_LOOKUP_KINDS,
        PromotionMode: PROMOTION_MODES,
        PuzzleDifficulty: PUZZLE_DIFFICULTIES,
        PuzzleTrainMode: PUZZLE_TRAIN_MODES,
        ReviewAuto: REVIEW_AUTO,
        RunKind: RUN_KINDS,
        VoiceOutcome: VOICE_OUTCOMES,
        VoiceSource: VOICE_SOURCES,
    );
}

#[test]
fn the_default_settings_round_trip_through_json() {
    let settings = default_settings().expect("the bundled defaults read");
    assert!(!settings.core.engine_levels.is_empty());
    let wire = serde_json::to_value(&settings).expect("settings write");
    // One flat object: the core's preferences and the frontend's side by side.
    assert!(wire.get("engineLevels").is_some() && wire.get("appearance").is_some());
    let back: Settings = serde_json::from_value(wire).expect("settings read back");
    assert_eq!(back, settings);
}

#[test]
fn an_unknown_setting_value_is_refused() {
    let mut wire = serde_json::to_value(default_settings().expect("defaults")).expect("write");
    wire["appearance"] = json!("sepia");
    assert!(serde_json::from_value::<Settings>(wire).is_err());
}

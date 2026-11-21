//! Each stored setting's name and category, by its stored name (`soundVolume`): the Undo notice
//! names a changed setting by it, and a category's Restore defaults puts its settings back as a
//! new profile has them, with that notice's Undo instead of a question first.

use std::sync::LazyLock;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use kchess_contracts::types::Settings;

use super::{Section, TOGGLES, Toggle};
use crate::assets::icon;

/// The stored settings that are not yes/no toggles (`TOGGLES`), named as their Settings rows.
const OTHERS: [(&str, &str, Section); 14] = [
    ("appearance", "Appearance", Section::Themes),
    ("lightTheme", "Light mode theme", Section::Themes),
    ("darkTheme", "Dark mode theme", Section::Themes),
    ("boardTheme", "Board theme", Section::Appearance),
    ("pieceSet", "Piece set", Section::Appearance),
    ("pieceAnimation", "Piece animation", Section::Appearance),
    ("coordinates", "Board coordinates", Section::Appearance),
    ("figurineNotation", "Piece notation", Section::Appearance),
    ("soundVolume", "Volume", Section::Sound),
    ("promotion", "Pawn promotion", Section::Gameplay),
    ("engineLevels", "Computer levels", Section::Gameplay),
    (
        "correspondencePoll",
        "Check correspondence games",
        Section::Online,
    ),
    ("enginePath", "Engine", Section::Engine),
    (
        "reviewAuto",
        "Review my games automatically",
        Section::Engine,
    ),
];

/// Settings Restore defaults leaves: the engine is a file the person chose, not a preference.
const KEPT: [&str; 1] = ["enginePath"];

/// The settings a new profile starts with.
static DEFAULTS: LazyLock<Option<Settings>> = LazyLock::new(|| {
    kchess_contracts::defaults::default_settings()
        .map_err(|cause| {
            crate::log::write(
                kchess_contracts::Level::Warn,
                "settings",
                &format!("The default settings could not be read: {cause}"),
            )
        })
        .ok()
});

/// The settings a new profile starts with, by stored name.
static DEFAULT_VALUES: LazyLock<serde_json::Map<String, serde_json::Value>> =
    LazyLock::new(|| match DEFAULTS.as_ref().map(serde_json::to_value) {
        Some(Ok(serde_json::Value::Object(values))) => values,
        _ => serde_json::Map::new(),
    });

/// The stored setting each of `TOGGLES` turns on and off, in their order.
static TOGGLE_KEYS: LazyLock<Vec<Option<String>>> =
    LazyLock::new(|| TOGGLES.iter().map(flipped_key).collect());

/// The stored setting `toggle` turns on and off.
fn flipped_key(toggle: &Toggle) -> Option<String> {
    let defaults = DEFAULTS.as_ref()?;
    let mut flipped = defaults.clone();
    (toggle.set)(&mut flipped, !(toggle.on)(defaults));
    match crate::settings::changes(defaults, &flipped).as_slice() {
        [change] => Some(change.key.clone()),
        _ => None,
    }
}

/// The toggles, each with the stored setting it turns on and off.
fn toggles() -> impl Iterator<Item = (&'static Toggle, Option<&'static str>)> {
    TOGGLES.iter().zip(TOGGLE_KEYS.iter().map(Option::as_deref))
}

/// What a stored setting is called, as its row or Quick Pick names it: with whether it is a
/// yes/no toggle.
pub(crate) fn name(key: &str) -> Option<(&'static str, bool)> {
    toggles()
        .find(|(_, of)| *of == Some(key))
        .map(|(toggle, _)| (toggle.label(), true))
        .or_else(|| {
            OTHERS
                .iter()
                .find(|(other, _, _)| *other == key)
                .map(|(_, name, _)| (*name, false))
        })
}

/// The stored settings of `section` that Restore defaults puts back.
fn section_keys(section: Section) -> Vec<String> {
    toggles()
        .filter(|(toggle, _)| toggle.section == section)
        .filter_map(|(_, key)| key.map(str::to_string))
        .chain(
            OTHERS
                .iter()
                .filter(|(_, _, of)| *of == section)
                .map(|(key, _, _)| key.to_string()),
        )
        .filter(|key| !KEPT.contains(&key.as_str()))
        .collect()
}

/// Whether `value` is what a new profile has for `key`.
pub(crate) fn is_default(key: &str, value: &serde_json::Value) -> bool {
    DEFAULT_VALUES.get(key) == Some(value)
}

/// Restore defaults for `section` (Settings' header), while any of its settings differs from a
/// new profile's.
pub(crate) fn restore(section: Section, settings: &Settings) -> Option<impl IntoElement> {
    let keys = section_keys(section);
    let Ok(serde_json::Value::Object(shown)) = serde_json::to_value(settings) else {
        return None;
    };
    if keys.iter().all(|key| is_default(key, &shown[key.as_str()])) {
        return None;
    }
    let defaults = DEFAULTS.clone()?;
    Some(
        Button::new("restore-defaults")
            .ghost()
            .small()
            .icon(icon("rotate-ccw"))
            .label("Restore defaults")
            .accessibility_label(SharedString::from(format!(
                "Restore {} defaults",
                section.label()
            )))
            .on_click(move |_, _, cx| {
                let (defaults, keys) = (defaults.clone(), keys.clone());
                crate::settings::change(cx, move |settings| {
                    *settings = crate::settings::reset(settings, &defaults, &keys);
                });
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::{DEFAULTS, KEPT, OTHERS, Section, TOGGLES, flipped_key, name, section_keys};
    use std::collections::BTreeSet;

    /// Every stored setting has one name, from its toggle or `OTHERS`, and one category whose
    /// Restore defaults puts it back (but the engine chosen).
    #[test]
    fn every_setting_is_named_and_restored_with_one_category() {
        let defaults = DEFAULTS.as_ref().expect("the default settings");
        let toggled: Vec<String> = TOGGLES
            .iter()
            .map(|toggle| flipped_key(toggle).unwrap_or_else(|| panic!("{} sets one", toggle.id)))
            .collect();
        let others: Vec<String> = OTHERS.iter().map(|(key, _, _)| key.to_string()).collect();
        let all: BTreeSet<String> = toggled.iter().chain(&others).cloned().collect();
        assert_eq!(
            all.len(),
            toggled.len() + others.len(),
            "a setting named twice"
        );
        let serde_json::Value::Object(stored) =
            serde_json::to_value(defaults).expect("settings as JSON")
        else {
            unreachable!("settings are an object")
        };
        let stored: BTreeSet<String> = stored.keys().cloned().collect();
        assert_eq!(all, stored);
        for key in &stored {
            assert!(name(key).is_some(), "{key} has no name");
        }
        let restored: Vec<String> = Section::ALL.into_iter().flat_map(section_keys).collect();
        let expected: Vec<&String> = stored
            .iter()
            .filter(|key| !KEPT.contains(&key.as_str()))
            .collect();
        assert_eq!(restored.len(), expected.len(), "{restored:?}");
    }
}

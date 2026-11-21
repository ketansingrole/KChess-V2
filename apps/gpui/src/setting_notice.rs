//! The notice a settings change brings up, with Undo: a setting changed anywhere (Settings,
//! Quick Pick, a key, a page's own settings) can be tried and taken back without remembering
//! what it was. A newer change's notice replaces the one before.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, SharedString, Window};
use serde_json::Value;

use crate::pages::settings::{TOGGLES, Toggle};
use crate::settings::{Edit, SettingsHandle};

/// The notices' id: one shows at a time.
struct SettingNotice;

/// The Undo button's element id.
pub const UNDO: &str = "undo-setting";

/// The stored settings that are not yes/no toggles (`TOGGLES`), named as their Settings rows.
const NAMES: [(&str, &str); 14] = [
    ("appearance", "Appearance"),
    ("lightTheme", "Light mode theme"),
    ("darkTheme", "Dark mode theme"),
    ("boardTheme", "Board theme"),
    ("pieceSet", "Piece set"),
    ("pieceAnimation", "Piece animation"),
    ("coordinates", "Board coordinates"),
    ("figurineNotation", "Piece notation"),
    ("soundVolume", "Volume"),
    ("promotion", "Pawn promotion"),
    ("engineLevels", "Computer levels"),
    ("correspondencePoll", "Check correspondence games"),
    ("enginePath", "Engine"),
    ("reviewAuto", "Review my games automatically"),
];

/// Show that `title` changed, with Undo running `undo`.
pub fn offer(
    title: impl Into<SharedString>,
    undo: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let note = Notification::new().title(title).id::<SettingNotice>();
    let note = crate::widgets::action_notification(note, UNDO, "Undo", undo)
        // Unlike a deletion's, a setting's Undo goes as other notices do: the setting itself
        // stays where it can be changed back.
        .autohide(true);
    window.push_notification(note, cx);
}

/// Show the person's settings change `edit`, whose Undo takes it back.
pub fn edited(edit: &Edit, window: &mut Window, cx: &mut App) {
    let id = edit.id;
    offer(
        title(edit, cx),
        move |_, cx| crate::settings::undo(cx, id),
        window,
        cx,
    );
}

/// "Premoves on", "Premoves off".
pub fn switched(name: &str, on: bool) -> String {
    format!("{name} {}", if on { "on" } else { "off" })
}

/// "Premoves off", "Board theme changed"; "Settings changed" for several at once.
fn title(edit: &Edit, cx: &App) -> String {
    let [change] = edit.changes.as_slice() else {
        return "Settings changed".into();
    };
    if let Some(toggle) = toggle_of(&change.key, cx) {
        return switched(toggle.label(), matches!(change.after, Value::Bool(true)));
    }
    match NAMES.iter().find(|(key, _)| *key == change.key) {
        Some((_, name)) => format!("{name} changed"),
        None => "Setting changed".into(),
    }
}

/// The toggle that changes the stored setting `key` alone.
fn toggle_of(key: &str, cx: &App) -> Option<&'static Toggle> {
    let shown = &cx.global::<SettingsHandle>().0.read(cx).settings;
    TOGGLES.iter().find(|toggle| {
        let mut flipped = shown.clone();
        (toggle.set)(&mut flipped, !(toggle.on)(shown));
        matches!(
            crate::settings::changes(shown, &flipped).as_slice(),
            [change] if change.key == key
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{NAMES, TOGGLES};
    use kchess_contracts::defaults::default_settings;
    use std::collections::BTreeSet;

    /// Every stored setting is named in its notice: by its toggle or in `NAMES`, never both.
    #[test]
    fn every_setting_has_a_name() {
        let defaults = default_settings().expect("the default settings");
        let toggled: BTreeSet<String> = TOGGLES
            .iter()
            .flat_map(|toggle| {
                let mut flipped = defaults.clone();
                (toggle.set)(&mut flipped, !(toggle.on)(&defaults));
                crate::settings::changes(&defaults, &flipped)
                    .into_iter()
                    .map(|change| change.key)
            })
            .collect();
        let named: BTreeSet<String> = NAMES.iter().map(|(key, _)| key.to_string()).collect();
        assert!(
            toggled.is_disjoint(&named),
            "{:?}",
            toggled.intersection(&named)
        );
        let serde_json::Value::Object(stored) =
            serde_json::to_value(&defaults).expect("settings as JSON")
        else {
            unreachable!("settings are an object")
        };
        let unnamed: Vec<&String> = stored
            .keys()
            .filter(|key| !toggled.contains(*key) && !named.contains(*key))
            .collect();
        assert!(unnamed.is_empty(), "no notice name for {unnamed:?}");
    }
}

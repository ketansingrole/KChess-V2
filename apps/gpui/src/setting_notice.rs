//! The notice a settings change brings up, with Undo: a setting changed anywhere (Settings,
//! Quick Pick, a key, a page's own settings) can be tried and taken back without remembering
//! what it was. A newer change's notice replaces the one before.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, SharedString, Window};
use serde_json::Value;

use crate::pages::settings::defaults;
use crate::settings::Edit;

/// The notices' id: one shows at a time.
struct SettingNotice;

/// The Undo button's element id.
pub const UNDO: &str = "undo-setting";

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
        title(edit),
        move |_, cx| crate::settings::undo(cx, id),
        window,
        cx,
    );
}

/// "Premoves on", "Premoves off".
pub fn switched(name: &str, on: bool) -> String {
    format!("{name} {}", if on { "on" } else { "off" })
}

/// "Premoves off", "Board theme changed"; "Defaults restored" or "Settings changed" for
/// several at once.
fn title(edit: &Edit) -> String {
    let [change] = edit.changes.as_slice() else {
        let restored = edit
            .changes
            .iter()
            .all(|change| defaults::is_default(&change.key, &change.after));
        return if restored {
            "Defaults restored"
        } else {
            "Settings changed"
        }
        .into();
    };
    match defaults::name(&change.key) {
        Some((name, true)) => switched(name, matches!(change.after, Value::Bool(true))),
        Some((name, false)) => format!("{name} changed"),
        None => "Setting changed".into(),
    }
}

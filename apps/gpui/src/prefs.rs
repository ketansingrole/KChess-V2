//! View preferences (tabs, layouts, toggles): kept by the core (`viewPreferences`), shared with the
//! Electron app under the same `kchess:` names (`useViewPreference`), so a page opens as it was left
//! in either app.

use gpui_kit::{App, Global};
use kchess_core::Level;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::core::AppCore;

#[derive(Default)]
pub struct ViewPreferences {
    values: Map<String, Value>,
}

impl Global for ViewPreferences {}

/// Read the saved preferences before the window opens.
pub fn init(cx: &mut App) {
    let values = match cx
        .global::<AppCore>()
        .bridge
        .call_now("viewPreferences", Vec::new())
    {
        Ok(Value::Object(values)) => values,
        Ok(_) => Map::new(),
        Err(cause) => {
            crate::log::write(
                Level::Warn,
                "view-preferences",
                &format!("Loading saved view preferences failed: {}", cause.message),
            );
            Map::new()
        }
    };
    cx.set_global(ViewPreferences { values });
}

/// The preference `key`, or `fallback` until it is changed or when it is unreadable.
pub fn get<T: DeserializeOwned>(cx: &App, key: &str, fallback: T) -> T {
    cx.try_global::<ViewPreferences>()
        .and_then(|prefs| prefs.values.get(key))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or(fallback)
}

/// Remember a preference for every KChess app on this profile.
pub fn set<T: Serialize>(cx: &mut App, key: &str, value: T) {
    let value = json!(value);
    let prefs = cx.default_global::<ViewPreferences>();
    if prefs.values.get(key) == Some(&value) {
        return;
    }
    prefs.values.insert(key.to_string(), value.clone());
    let save = cx
        .global::<AppCore>()
        .call("setViewPreference", vec![json!(key), value]);
    let key = key.to_string();
    cx.background_executor()
        .spawn(async move {
            if let Err(cause) = save.await {
                crate::log::write(
                    Level::Warn,
                    "view-preferences",
                    &format!("Saving a view preference failed: {key} {}", cause.message),
                );
            }
        })
        .detach();
}

/// Forget a preference (`forgetViewPreference`): its default applies again in both apps.
pub fn forget(cx: &mut App, key: &str) {
    let prefs = cx.default_global::<ViewPreferences>();
    if prefs.values.remove(key).is_none() {
        return;
    }
    let save = cx
        .global::<AppCore>()
        .call("setViewPreference", vec![json!(key), Value::Null]);
    cx.background_executor()
        .spawn(async move {
            let _ = save.await;
        })
        .detach();
}

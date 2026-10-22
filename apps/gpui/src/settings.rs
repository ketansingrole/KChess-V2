//! The profile's settings as the app holds them: the core's and the desktop's together
//! (`desktopSettings`), shared with the Electron app. Views read `AppSettings` and save through it.

use gpui_kit::{App, AppContext as _, Entity, EventEmitter, Global};
use kchess_contracts::types::{AppTheme, CustomThemeReport, Settings};
use kchess_core::Level;

use crate::core::{AppCore, CoreEvent};

/// The settings and the user's own themes, as last read from the core.
pub struct AppSettings {
    pub settings: Settings,
    pub custom_themes: Vec<AppTheme>,
}

/// Sent when the settings or themes change.
pub struct SettingsChanged;

impl EventEmitter<SettingsChanged> for AppSettings {}

/// How views reach the settings: `cx.global::<SettingsHandle>().0`.
#[derive(Clone)]
pub struct SettingsHandle(pub Entity<AppSettings>);

impl Global for SettingsHandle {}

/// What a fresh profile stores (the contract defaults, `defaultSettings.ts`).
fn defaults() -> Settings {
    match kchess_contracts::defaults::default_settings() {
        Ok(settings) => settings,
        // The contracts crate's own tests decode them; this cannot fail at run time.
        Err(cause) => panic!("the contract defaults do not decode as Settings: {cause}"),
    }
}

/// Starts with the defaults and loads the saved settings and themes from the core.
pub fn init(cx: &mut App) {
    let entity = cx.new(|_| AppSettings {
        settings: defaults(),
        custom_themes: Vec::new(),
    });
    cx.set_global(SettingsHandle(entity.clone()));
    let events = cx.global::<AppCore>().events.clone();
    cx.subscribe(&events, {
        let entity = entity.clone();
        move |_, event: &CoreEvent, cx| {
            if event.name == "settings:saved" {
                reload(&entity, cx);
            }
        }
    })
    .detach();
    reload(&entity, cx);
}

/// Read the settings and themes again.
pub fn reload(entity: &Entity<AppSettings>, cx: &mut App) {
    let core = cx.global::<AppCore>().clone();
    let settings = core.call("desktopSettings", Vec::new());
    let themes = core.call("customThemes", Vec::new());
    let entity = entity.downgrade();
    cx.spawn(async move |cx| {
        let settings = settings.await;
        let themes = themes.await;
        let _ = entity.update(cx, |this, cx| {
            match settings.map(serde_json::from_value::<Settings>) {
                Ok(Ok(settings)) => this.settings = settings,
                Ok(Err(cause)) => crate::log::write(
                    Level::Warn,
                    "settings",
                    &format!("The saved settings could not be read: {cause}"),
                ),
                Err(cause) => crate::log::write(
                    Level::Warn,
                    "settings",
                    &format!("Loading settings failed: {}", cause.message),
                ),
            }
            match themes.map(serde_json::from_value::<CustomThemeReport>) {
                Ok(Ok(report)) => this.custom_themes = report.themes,
                Ok(Err(cause)) => crate::log::write(
                    Level::Warn,
                    "themes",
                    &format!("The custom themes could not be read: {cause}"),
                ),
                Err(cause) => crate::log::write(
                    Level::Warn,
                    "themes",
                    &format!("Loading custom themes failed: {}", cause.message),
                ),
            }
            cx.emit(SettingsChanged);
            cx.notify();
        });
    })
    .detach();
}

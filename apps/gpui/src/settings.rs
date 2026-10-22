//! The profile's settings as the app holds them: the core's and the desktop's together
//! (`desktopSettings`), shared with the Electron app. Views read `AppSettings` and save through it;
//! saves wait for a pause in editing and are written one at a time (`settingsPersistence.ts`).

use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, AppContext as _, Entity, EventEmitter, Global, Task, WeakEntity};
use kchess_contracts::types::{AppTheme, CustomThemeReport, Settings};
use kchess_core::{CoreError, Level};
use serde_json::Value;

use crate::core::{AppCore, BoxedSave, CoreEvent};

/// How long edits settle before they are saved, as in the Electron app.
const SAVE_DELAY: Duration = Duration::from_millis(400);

/// The settings and the user's own themes, as last read from the core.
pub struct AppSettings {
    pub settings: Settings,
    pub custom_themes: Vec<AppTheme>,
    /// The folder the user's themes are read from, and the files that could not be used.
    pub themes_dir: String,
    pub theme_problems: Vec<String>,
    /// Edits not yet stored: the core's copy is not reloaded over them.
    dirty: bool,
    save_soon: Option<Task<()>>,
    writes: mpsc::UnboundedSender<Write>,
}

enum Write {
    Save(Settings),
    Flushed(oneshot::Sender<()>),
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
/// Read the settings and themes before the window opens, so it opens in the chosen theme.
pub fn init(cx: &mut App) {
    let bridge = std::sync::Arc::clone(&cx.global::<AppCore>().bridge);
    let settings = bridge
        .call_now("desktopSettings", Vec::new())
        .ok()
        .and_then(|settings| serde_json::from_value::<Settings>(settings).ok())
        .unwrap_or_else(defaults);
    let report = bridge
        .call_now("customThemes", Vec::new())
        .ok()
        .and_then(|report| serde_json::from_value::<CustomThemeReport>(report).ok())
        .unwrap_or_else(|| CustomThemeReport {
            themes: Vec::new(),
            dir: String::new(),
            problems: Vec::new(),
        });
    let (writes, queue) = mpsc::unbounded();
    let entity = cx.new(|_| AppSettings {
        settings,
        custom_themes: report.themes,
        themes_dir: report.dir,
        theme_problems: report.problems,
        dirty: false,
        save_soon: None,
        writes,
    });
    write_in_order(entity.downgrade(), queue, cx);
    crate::core::on_quit(cx, {
        let entity = entity.clone();
        move |cx| Some(entity.update(cx, |settings, _| settings.flushed()))
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
                // Edits not yet stored stay; their own save brings the rest.
                Ok(Ok(_)) if this.dirty => {}
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
                Ok(Ok(report)) => {
                    this.custom_themes = report.themes;
                    this.themes_dir = report.dir;
                    this.theme_problems = report.problems;
                }
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

/// Write saves one at a time, in the order they were made, off the UI thread so they also finish
/// while the app quits. A save that the core stored as sent leaves nothing unsaved; one that
/// failed shows what is stored again.
fn write_in_order(
    entity: WeakEntity<AppSettings>,
    mut queue: mpsc::UnboundedReceiver<Write>,
    cx: &mut App,
) {
    let core = cx.global::<AppCore>().clone();
    let (stored, mut results) = mpsc::unbounded::<(Settings, Result<Value, CoreError>)>();
    cx.background_executor()
        .spawn(async move {
            while let Some(write) = queue.next().await {
                match write {
                    Write::Save(sent) => {
                        let saved = core
                            .call("saveDesktopSettings", vec![serde_json::json!(sent)])
                            .await;
                        let _ = stored.unbounded_send((sent, saved));
                    }
                    Write::Flushed(done) => {
                        let _ = done.send(());
                    }
                }
            }
        })
        .detach();
    cx.spawn(async move |cx| {
        while let Some((sent, saved)) = results.next().await {
            let _ = entity.update(cx, |this, cx| {
                let unchanged = this.settings == sent;
                match saved.map(serde_json::from_value::<Settings>) {
                    Ok(Ok(stored)) => {
                        if unchanged {
                            this.dirty = this.save_soon.is_some();
                            this.settings = stored;
                        }
                    }
                    Ok(Err(cause)) => crate::log::write(
                        Level::Warn,
                        "settings",
                        &format!("The saved settings could not be read: {cause}"),
                    ),
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "settings",
                            &format!("Saving settings failed: {}", cause.message),
                        );
                        if unchanged {
                            this.dirty = false;
                            reload(&cx.entity(), cx);
                        }
                    }
                }
                cx.emit(SettingsChanged);
                cx.notify();
            });
        }
    })
    .detach();
}

impl AppSettings {
    /// Change settings; the views see the change at once and it is saved after a pause.
    pub fn update(&mut self, change: impl FnOnce(&mut Settings), cx: &mut gpui_kit::Context<Self>) {
        let before = self.settings.clone();
        change(&mut self.settings);
        if self.settings == before {
            return;
        }
        self.dirty = true;
        cx.emit(SettingsChanged);
        cx.notify();
        self.save_soon = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let _ = this.update(cx, |this, _| this.save_now());
        }));
    }

    /// Send the edits to be saved now.
    fn save_now(&mut self) {
        self.save_soon = None;
        let _ = self
            .writes
            .unbounded_send(Write::Save(self.settings.clone()));
    }

    /// Save waiting edits now and wait for every save so far (before the app quits).
    fn flushed(&mut self) -> BoxedSave {
        if self.save_soon.is_some() {
            self.save_now();
        }
        let (done, wait) = oneshot::channel();
        let _ = self.writes.unbounded_send(Write::Flushed(done));
        Box::pin(async move {
            let _ = wait.await;
        })
    }
}

/// Change settings from a view (`toggleSetting`, the Settings page); they save themselves.
pub fn change(cx: &mut App, change: impl FnOnce(&mut Settings) + 'static) {
    let handle = cx.global::<SettingsHandle>().0.clone();
    handle.update(cx, |settings, cx| settings.update(change, cx));
}

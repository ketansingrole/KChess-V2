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

use crate::core::{Answer, AppCore, BoxedSave, CoreEvent, Reads};

/// How long edits settle before they are saved, as in the Electron app.
const SAVE_DELAY: Duration = Duration::from_millis(400);

/// The settings and the user's own themes, as last read from the core.
pub struct AppSettings {
    pub settings: Settings,
    /// The settings as the core last stored them: what this window's edits started from.
    stored: Settings,
    pub custom_themes: Vec<AppTheme>,
    /// The folder the user's themes are read from, and the files that could not be used.
    pub themes_dir: String,
    pub theme_problems: Vec<String>,
    /// Edits not yet stored: the core's copy is not reloaded over them.
    dirty: bool,
    save_soon: Option<Task<()>>,
    writes: mpsc::UnboundedSender<Write>,
    reads: Reads,
}

enum Write {
    /// The settings shown, and those they were edited from: only what changed is written.
    Save(Box<(Settings, Settings)>),
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
        stored: settings.clone(),
        settings,
        custom_themes: report.themes,
        themes_dir: report.dir,
        theme_problems: report.problems,
        dirty: false,
        save_soon: None,
        writes,
        reads: Reads::default(),
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
            // Another window saved, or this one reconnected and may have missed a save.
            if event.name == "settings:saved" || event.name == crate::core::RECONNECTED {
                reload(&entity, cx);
            }
        }
    })
    .detach();
}

/// Read the settings and themes again: after this window's saves already sent, and only the
/// newest read, so an older answer never becomes the base of this window's next save.
pub fn reload(entity: &Entity<AppSettings>, cx: &mut App) {
    let (read, sent) = entity.update(cx, |this, _| (this.reads.start(), this.sent()));
    let entity = entity.downgrade();
    cx.spawn(async move |cx| {
        sent.await;
        let (settings, themes) = cx.update(|cx| {
            let core = cx.global::<AppCore>().clone();
            (
                core.call("desktopSettings", Vec::new()),
                core.call("customThemes", Vec::new()),
            )
        });
        let settings = settings.await;
        let themes = themes.await;
        let _ = entity.update(cx, |this, cx| {
            match this.reads.answer(read) {
                Answer::Show => {}
                Answer::Again => return reload(&cx.entity(), cx),
                Answer::Superseded => return,
            }
            match settings.map(serde_json::from_value::<Settings>) {
                // Edits not yet stored stay; every other setting shows what is stored.
                Ok(Ok(settings)) => {
                    this.settings = if this.dirty {
                        adopt(&this.settings, &this.stored, &settings)
                    } else {
                        settings.clone()
                    };
                    this.stored = settings;
                }
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
                    Write::Save(save) => {
                        let (sent, base) = *save;
                        let saved = core
                            .call(
                                "saveDesktopSettings",
                                vec![serde_json::json!(sent), serde_json::json!(base)],
                            )
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
                            this.settings = stored.clone();
                        } else {
                            // Edited meanwhile: only those edits stay; every other setting takes
                            // the stored copy, so the next save does not write back what another
                            // window changed.
                            this.settings = adopt(&this.settings, &sent, &stored);
                        }
                        this.stored = stored;
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
        self.reads.edited();
        let _ = self.writes.unbounded_send(Write::Save(Box::new((
            self.settings.clone(),
            self.stored.clone(),
        ))));
    }

    /// Save waiting edits now and wait for every save so far (before the app quits).
    fn flushed(&mut self) -> BoxedSave {
        if self.save_soon.is_some() {
            self.save_now();
        }
        self.sent()
    }

    /// Wait for the saves already sent.
    fn sent(&self) -> BoxedSave {
        let (done, wait) = oneshot::channel();
        let _ = self.writes.unbounded_send(Write::Flushed(done));
        Box::pin(async move {
            let _ = wait.await;
        })
    }
}

/// `shown`, with every setting it has not changed from `stored` taken from `next` (another
/// window's save): this window's own edits stay, still to be saved.
fn adopt(shown: &Settings, stored: &Settings, next: &Settings) -> Settings {
    let values = (
        serde_json::to_value(shown),
        serde_json::to_value(stored),
        serde_json::to_value(next),
    );
    let (Ok(Value::Object(mut merged)), Ok(Value::Object(stored)), Ok(Value::Object(next))) =
        values
    else {
        return shown.clone();
    };
    for (key, value) in next {
        if merged.get(&key) == stored.get(&key) {
            merged.insert(key, value);
        }
    }
    serde_json::from_value(Value::Object(merged)).unwrap_or_else(|_| shown.clone())
}

/// Change settings from a view (`toggleSetting`, the Settings page); they save themselves.
pub fn change(cx: &mut App, change: impl FnOnce(&mut Settings) + 'static) {
    let handle = cx.global::<SettingsHandle>().0.clone();
    handle.update(cx, |settings, cx| settings.update(change, cx));
}

/// Save waiting edits now; done once the core has them (before asking it about what they set).
pub fn flush(cx: &mut App) -> BoxedSave {
    let handle = cx.global::<SettingsHandle>().0.clone();
    handle.update(cx, |settings, _| settings.flushed())
}

#[cfg(test)]
mod tests {
    use super::{adopt, defaults};

    #[test]
    fn another_windows_settings_show_beside_this_windows_edits() {
        let stored = defaults();
        let mut shown = stored.clone();
        shown.frontend.zen_mode = !stored.frontend.zen_mode;
        let mut next = stored.clone();
        next.frontend.sound_enabled = !stored.frontend.sound_enabled;
        next.frontend.zen_mode = stored.frontend.zen_mode;
        let merged = adopt(&shown, &stored, &next);
        assert_eq!(
            merged.frontend.zen_mode, shown.frontend.zen_mode,
            "this window's edit stays"
        );
        assert_eq!(
            merged.frontend.sound_enabled, next.frontend.sound_enabled,
            "the other window's shows"
        );
    }

    #[test]
    fn an_edit_during_a_save_keeps_what_the_core_merged_from_another_window() {
        // This window shows volume 0.2 and saves an appearance change; the core keeps another
        // window's volume (0.8) in what it stores. Zen mode was edited before the answer came.
        let mut sent = defaults();
        sent.frontend.sound_volume = 0.2;
        sent.frontend.zen_mode = false;
        let mut stored = sent.clone();
        stored.frontend.sound_volume = 0.8;
        let mut shown = sent.clone();
        shown.frontend.zen_mode = true;
        let rebased = adopt(&shown, &sent, &stored);
        assert_eq!(rebased.frontend.sound_volume, 0.8, "not written back");
        assert!(rebased.frontend.zen_mode, "the later edit stays");
    }
}

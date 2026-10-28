//! The profile's settings as the app holds them: the core's and the desktop's together
//! (`desktopSettings`), shared with the Electron app. Views read `AppSettings` and save through it;
//! saves wait for a pause in editing and are written one at a time.

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
    /// Saves sent and not yet answered, and the newest of them: the next save is based on it,
    /// so a change undone while an earlier save is still writing is written too.
    pending: usize,
    queued: Settings,
    /// The last save failed: its edits stay shown and are sent again with the next save.
    failed: bool,
    save_soon: Option<Task<()>>,
    writes: mpsc::UnboundedSender<Write>,
    reads: Reads,
}

enum Write {
    /// The settings shown, and those they were edited from: only what changed is written.
    Save(Box<(Settings, Settings)>),
    /// Done once every save before it was answered: with why the last of them failed, if it did.
    Flushed(oneshot::Sender<crate::core::Saved>),
}

/// Sent when the settings or themes change.
pub struct SettingsChanged;

impl EventEmitter<SettingsChanged> for AppSettings {}

/// How views reach the settings: `cx.global::<SettingsHandle>().0`.
#[derive(Clone)]
pub struct SettingsHandle(pub Entity<AppSettings>);

impl Global for SettingsHandle {}

/// What a fresh profile stores (the contract defaults).
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
        queued: settings.clone(),
        settings,
        custom_themes: report.themes,
        themes_dir: report.dir,
        theme_problems: report.problems,
        dirty: false,
        pending: 0,
        failed: false,
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
        let _ = sent.await;
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
/// failed keeps its edits shown, to be sent again.
fn write_in_order(
    entity: WeakEntity<AppSettings>,
    mut queue: mpsc::UnboundedReceiver<Write>,
    cx: &mut App,
) {
    let core = cx.global::<AppCore>().clone();
    let (stored, mut results) = mpsc::unbounded::<(Settings, Result<Value, CoreError>)>();
    cx.background_executor()
        .spawn(async move {
            // The last save, if it failed: with the settings it was based on, and why.
            let mut failed: Option<(Settings, Settings, String)> = None;
            while let Some(write) = queue.next().await {
                match write {
                    Write::Save(save) => {
                        let (sent, base) = *save;
                        let base = rebased(base, failed.as_ref());
                        let saved = core
                            .call(
                                "saveDesktopSettings",
                                vec![serde_json::json!(sent), serde_json::json!(base)],
                            )
                            .await;
                        failed = saved
                            .as_ref()
                            .err()
                            .map(|cause| (sent.clone(), base, cause.message.clone()));
                        let _ = stored.unbounded_send((sent, saved));
                    }
                    Write::Flushed(done) => {
                        let _ = done.send(match &failed {
                            Some((_, _, cause)) => {
                                Err(format!("Settings could not be saved: {cause}"))
                            }
                            None => Ok(()),
                        });
                    }
                }
            }
        })
        .detach();
    cx.spawn(async move |cx| {
        while let Some((sent, saved)) = results.next().await {
            let _ = entity.update(cx, |this, cx| {
                this.pending = this.pending.saturating_sub(1);
                let unchanged = this.settings == sent;
                this.failed = saved.is_err();
                match saved.map(serde_json::from_value::<Settings>) {
                    Ok(Ok(stored)) => {
                        if unchanged {
                            this.dirty = this.save_soon.is_some() || this.pending > 0;
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
                        // The edits stay, unsaved: what another window stored shows beside them.
                        this.dirty = true;
                        reload(&cx.entity(), cx);
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

    /// Send the edits to be saved now: what changed since the newest save still writing, or
    /// else since what is stored.
    fn save_now(&mut self) {
        self.save_soon = None;
        self.reads.edited();
        let base = if self.pending > 0 {
            self.queued.clone()
        } else {
            self.stored.clone()
        };
        self.pending += 1;
        self.queued = self.settings.clone();
        let _ = self
            .writes
            .unbounded_send(Write::Save(Box::new((self.settings.clone(), base))));
    }

    /// Save waiting edits (and those a failed save left) now, and wait for every save so far:
    /// with why the last failed, if it did (before the app quits).
    fn flushed(&mut self) -> BoxedSave {
        if self.save_soon.is_some() || (self.failed && self.dirty) {
            self.save_now();
        }
        self.sent()
    }

    /// Wait for the saves already sent.
    fn sent(&self) -> BoxedSave {
        let (done, wait) = oneshot::channel();
        let _ = self.writes.unbounded_send(Write::Flushed(done));
        Box::pin(async move {
            wait.await
                .unwrap_or_else(|_| Err("The settings writer stopped.".to_string()))
        })
    }
}

/// The base to write `base`'s save from: when it was based on a save that failed, that save's own
/// base, so the failed save's edits are written with this one rather than lost.
fn rebased(base: Settings, failed: Option<&(Settings, Settings, String)>) -> Settings {
    match failed {
        Some((sent, failed_base, _)) if *sent == base => failed_base.clone(),
        _ => base,
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
    use super::{AppSettings, Write, adopt, defaults, rebased};
    use crate::core::Reads;
    use futures::channel::mpsc;
    use kchess_contracts::types::Settings;
    use serde_json::Value;

    fn settings(writes: mpsc::UnboundedSender<Write>) -> AppSettings {
        let stored = defaults();
        AppSettings {
            settings: stored.clone(),
            queued: stored.clone(),
            stored,
            custom_themes: Vec::new(),
            themes_dir: String::new(),
            theme_problems: Vec::new(),
            dirty: false,
            pending: 0,
            failed: false,
            save_soon: None,
            writes,
            reads: Reads::default(),
        }
    }

    /// The core's `saveDesktopSettings`: what `sent` changed from `base`, over `stored`.
    fn store(stored: &Settings, sent: &Settings, base: &Settings) -> Settings {
        let (Value::Object(mut merged), Value::Object(sent), Value::Object(base)) = (
            serde_json::to_value(stored).unwrap(),
            serde_json::to_value(sent).unwrap(),
            serde_json::to_value(base).unwrap(),
        ) else {
            unreachable!()
        };
        for (key, value) in sent {
            if base.get(&key) != Some(&value) {
                merged.insert(key, value);
            }
        }
        serde_json::from_value(Value::Object(merged)).unwrap()
    }

    fn saves(queue: &mut mpsc::UnboundedReceiver<Write>) -> Vec<(Settings, Settings)> {
        let mut saves = Vec::new();
        while let Ok(write) = queue.try_recv() {
            if let Write::Save(save) = write {
                saves.push(*save);
            }
        }
        saves
    }

    /// Sound on; off (save A); on again (save B) before A is answered. Both are written, so the
    /// profile ends with sound on, as shown.
    #[test]
    fn a_change_undone_while_a_save_is_writing_is_saved() {
        let (writes, mut queue) = mpsc::unbounded();
        let mut this = settings(writes);
        let on = this.settings.frontend.sound_enabled;
        this.settings.frontend.sound_enabled = !on;
        this.save_now();
        this.settings.frontend.sound_enabled = on;
        this.save_now();
        let mut profile = this.stored.clone();
        for (sent, base) in saves(&mut queue) {
            profile = store(&profile, &sent, &base);
        }
        assert_eq!(profile.frontend.sound_enabled, on);
    }

    /// Save A fails; save B, made before A was answered, writes A's edits too.
    #[test]
    fn a_save_based_on_a_failed_one_writes_its_edits() {
        let (writes, mut queue) = mpsc::unbounded();
        let mut this = settings(writes);
        let stored = this.stored.clone();
        this.settings.frontend.zen_mode = !stored.frontend.zen_mode;
        this.save_now();
        this.settings.frontend.sound_enabled = !stored.frontend.sound_enabled;
        this.save_now();
        let saves = saves(&mut queue);
        let (failed_sent, failed_base) = saves[0].clone();
        let (sent, base) = saves[1].clone();
        let base = rebased(base, Some(&(failed_sent, failed_base, "disk full".into())));
        let profile = store(&stored, &sent, &base);
        assert_eq!(profile.frontend.zen_mode, !stored.frontend.zen_mode);
        assert_eq!(
            profile.frontend.sound_enabled,
            !stored.frontend.sound_enabled
        );
    }

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

//! The profile's settings as the app holds them: the core's and the desktop's together
//! (`desktopSettings`). Views read `AppSettings` and save through it;
//! saves wait for a pause in editing and are written one at a time.

use crate::log::ResultExt as _;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{App, AppContext as _, Entity, EventEmitter, Global, Task, WeakEntity};
use kchess_contracts::calls::{CustomThemes, DesktopSettings, SaveDesktopSettings};
use kchess_contracts::types::{AppTheme, CustomThemeReport, Settings};
use kchess_contracts::{CoreError, Level};
use serde_json::Value;

use crate::core::{Answer, AppCore, ClientEvent, CoreEvent, Reads};
use crate::quit::{BoxedSave, Saved};
use crate::writer::{Store, Writer};

/// How long edits settle before they are saved.
const SAVE_DELAY: Duration = Duration::from_millis(400);

/// How long further changes to the same settings count as one change that Undo takes back
/// whole (a slider dragged, choices tried in turn): about as long as the Undo notice shows.
const EDIT_WINDOW: Duration = Duration::from_secs(5);

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
    writes: Writer<Save>,
    reads: Reads,
    /// The person's latest change, which Undo takes back, and how many changes were recorded.
    last_edit: Option<Edit>,
    edits: u64,
}

/// A change a person made to their settings, which Undo takes back.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    /// Counts the changes, so the notice of an older one cannot take back a newer one.
    pub id: u64,
    /// The settings it changed, each with its value before and after.
    pub changes: Vec<Change>,
    /// When it was last changed.
    at: Instant,
}

/// One setting an edit changed, by its stored name (`soundVolume`).
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub key: String,
    pub before: Value,
    pub after: Value,
}

impl Edit {
    fn same_settings(&self, changes: &[Change]) -> bool {
        self.changes.len() == changes.len()
            && self
                .changes
                .iter()
                .zip(changes)
                .all(|(this, other)| this.key == other.key)
    }
}

/// Sent when a person changes a setting (`change`): the change Undo would take back.
pub struct SettingsEdited(pub Edit);

impl EventEmitter<SettingsEdited> for AppSettings {}

/// A save: the settings shown, and those they were edited from, as only what changed is written.
struct Save {
    sent: Settings,
    base: Settings,
}

/// What the writer is called once it has stopped.
const CLOSED: &str = "The settings writer stopped";

/// Sent when the settings or themes change.
pub struct SettingsChanged;

impl EventEmitter<SettingsChanged> for AppSettings {}

/// How views reach the settings: `cx.global::<SettingsHandle>().0`.
#[derive(Clone)]
pub struct SettingsHandle(pub Entity<AppSettings>);

impl Global for SettingsHandle {}

/// Settings at their defaults with no core behind them, for the tests of the crates the app is
/// built from (`test-support`): their saves go nowhere.
#[cfg(any(test, feature = "test-support"))]
pub fn init_for_tests(cx: &mut App) -> Entity<AppSettings> {
    let (writes, _) = Writer::queued(CLOSED);
    let stored = defaults();
    let entity = cx.new(|_| AppSettings {
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
        last_edit: None,
        edits: 0,
    });
    cx.set_global(SettingsHandle(entity.clone()));
    entity
}

/// What a fresh profile stores (the contract defaults): only tests start from them, since the
/// app starts from what its startup read (`init`).
#[cfg(any(test, feature = "test-support"))]
fn defaults() -> Settings {
    match kchess_contracts::defaults::default_settings() {
        Ok(settings) => settings,
        // The contracts crate's own tests decode them; this cannot fail at run time.
        Err(cause) => panic!("the contract defaults do not decode as Settings: {cause}"),
    }
}

/// Starts from `settings` and the custom themes, as the app's startup read them from the core
/// (`startup::check`, which refuses to start without the settings). Both are in before the window
/// opens, so it opens in the chosen theme. The themes are read with the rest: read here, they kept
/// the main thread waiting after the startup screen's Try again (2026-11-24). Themes that could
/// not be read leave only the built-in ones.
pub fn init(settings: Settings, themes: Result<CustomThemeReport, CoreError>, cx: &mut App) {
    let report = themes.warn_on_err().unwrap_or_else(|| CustomThemeReport {
        themes: Vec::new(),
        dir: String::new(),
        problems: Vec::new(),
    });
    let (stored, results) = mpsc::unbounded();
    let saving = Saving {
        core: cx.global::<AppCore>().clone(),
        failed: None,
        stored,
    };
    let writes = Writer::start(saving, CLOSED, cx);
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
        last_edit: None,
        edits: 0,
    });
    show_saves(entity.downgrade(), results, cx);
    crate::quit::on_quit(cx, {
        let entity = entity.clone();
        move |cx| Some(entity.update(cx, |settings, _| settings.flushed()))
    });
    cx.set_global(SettingsHandle(entity.clone()));
    let events = cx.global::<AppCore>().events.clone();
    cx.subscribe(&events, {
        let entity = entity.clone();
        move |_, event: &ClientEvent, cx| {
            // Another window saved, or this one reconnected and may have missed a save.
            if matches!(
                event,
                ClientEvent::Event(CoreEvent::SettingsSaved(_)) | ClientEvent::Reconnected
            ) {
                reload(&entity, cx);
            }
        }
    })
    .detach();
}

/// Read the settings and themes again: after this window's saves already sent, and only the
/// newest read, so an older answer never becomes the base of this window's next save.
pub fn reload(entity: &Entity<AppSettings>, cx: &mut App) {
    entity.update(cx, |this, cx| this.reload(cx));
}

impl AppSettings {
    /// `reload`, on the settings already being updated: a read answered after a local change
    /// reads again from here, since updating the entity again inside its own update panics.
    fn reload(&mut self, cx: &mut gpui_kit::Context<Self>) {
        let (read, sent) = (self.reads.start(), self.sent());
        cx.spawn(async move |entity, cx| {
            let _ = sent.await;
            let (settings, themes) = cx.update(|cx| {
                let core = cx.global::<AppCore>().clone();
                (
                    core.request(DesktopSettings {}),
                    core.request(CustomThemes {}),
                )
            });
            let settings = settings.await;
            let themes = themes.await;
            let _ = entity.update(cx, |this, cx| {
                match this.reads.answer(read) {
                    Answer::Show => {}
                    Answer::Again => return this.reload(cx),
                    Answer::Superseded => return,
                }
                match settings {
                    // Edits not yet stored stay; every other setting shows what is stored.
                    Ok(settings) => {
                        this.settings = if this.dirty {
                            adopt(&this.settings, &this.stored, &settings)
                        } else {
                            settings.clone()
                        };
                        this.stored = settings;
                    }
                    Err(cause) => crate::log::write(
                        Level::Warn,
                        "settings",
                        &format!("Loading settings failed: {}", cause.message),
                    ),
                }
                match themes {
                    Ok(report) => {
                        this.custom_themes = report.themes;
                        this.themes_dir = report.dir;
                        this.theme_problems = report.problems;
                    }
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
}

/// The settings' saves (`Writer`), one at a time in the order they were made. A save that
/// failed keeps its edits shown, to be sent again with the next save (`rebased`); its answer, or
/// why it failed, goes to `show_saves`.
struct Saving {
    core: AppCore,
    /// The last save, if it failed: with the settings it was based on, and why.
    failed: Option<(Settings, Settings, String)>,
    stored: mpsc::UnboundedSender<(Settings, Result<Settings, CoreError>)>,
}

impl Store for Saving {
    type Write = Save;

    async fn write(&mut self, Save { sent, base }: Save) {
        let base = rebased(base, self.failed.as_ref());
        let saved = self
            .core
            .request(SaveDesktopSettings {
                settings: sent.clone(),
                base: Some(base.clone()),
            })
            .await;
        self.failed = saved
            .as_ref()
            .err()
            .map(|cause| (sent.clone(), base, cause.message.clone()));
        let _ = self.stored.unbounded_send((sent, saved));
    }

    /// A failed save is sent again from the window, with its edits and any since
    /// (`AppSettings::flushed`), so there is nothing here to retry.
    async fn flush(&mut self, _retry: bool) -> Saved {
        match &self.failed {
            Some((_, _, cause)) => Err(format!("Settings could not be saved: {cause}")),
            None => Ok(()),
        }
    }
}

/// Show each save's answer: what the core stored, or why it failed.
fn show_saves(
    entity: WeakEntity<AppSettings>,
    mut results: mpsc::UnboundedReceiver<(Settings, Result<Settings, CoreError>)>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Some((sent, saved)) = results.next().await {
            let _ = entity.update(cx, |this, cx| {
                this.pending = this.pending.saturating_sub(1);
                let unchanged = this.settings == sent;
                this.failed = saved.is_err();
                match saved {
                    Ok(stored) => {
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
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "settings",
                            &format!("Saving settings failed: {}", cause.message),
                        );
                        // The edits stay, unsaved: what another window stored shows beside them.
                        this.dirty = true;
                        this.reload(cx);
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
        let edit = self.record(
            changes(&before, &self.settings),
            cx.background_executor().now(),
        );
        cx.emit(SettingsEdited(edit));
        self.edited(cx);
    }

    /// The person's latest change, which Undo would take back.
    pub fn last_edit(&self) -> Option<&Edit> {
        self.last_edit.as_ref()
    }

    /// Take back the change `id` (`undo`): each setting it changed that still shows what it
    /// set goes back to what it was; a setting changed since, here or in another window, stays.
    fn undo(&mut self, id: u64, cx: &mut gpui_kit::Context<Self>) {
        let Some(edit) = self.last_edit.take_if(|edit| edit.id == id) else {
            return;
        };
        let restored = restored(&self.settings, &edit.changes);
        if restored != self.settings {
            self.settings = restored;
            self.edited(cx);
        }
    }

    /// Note the person's change: one more change to the same settings soon after the last
    /// joins it, so Undo goes back to before the first; any other is a change of its own.
    fn record(&mut self, changes: Vec<Change>, now: Instant) -> Edit {
        self.edits += 1;
        let joined = self.last_edit.take().and_then(|last| {
            if !last.same_settings(&changes) || now.duration_since(last.at) >= EDIT_WINDOW {
                return None;
            }
            let joined: Vec<Change> = last
                .changes
                .into_iter()
                .zip(&changes)
                .map(|(first, next)| Change {
                    after: next.after.clone(),
                    ..first
                })
                .collect();
            // Back where it started (a switch turned on and off again): that is a change of
            // its own, which Undo takes back.
            joined
                .iter()
                .any(|change| change.before != change.after)
                .then_some(joined)
        });
        let edit = Edit {
            id: self.edits,
            changes: joined.unwrap_or(changes),
            at: now,
        };
        self.last_edit = Some(edit.clone());
        edit
    }

    /// The settings shown changed: the views redraw and it is saved after a pause.
    fn edited(&mut self, cx: &mut gpui_kit::Context<Self>) {
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
        self.writes.send(Save {
            sent: self.settings.clone(),
            base,
        });
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
        self.writes.written()
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

/// Each setting `after` holds differently from `before`, by its stored name, in stored order.
pub fn changes(before: &Settings, after: &Settings) -> Vec<Change> {
    let (Ok(Value::Object(before)), Ok(Value::Object(after))) =
        (serde_json::to_value(before), serde_json::to_value(after))
    else {
        return Vec::new();
    };
    after
        .into_iter()
        .filter_map(|(key, after)| {
            let before = before.get(&key).cloned().unwrap_or(Value::Null);
            (before != after).then_some(Change { key, before, after })
        })
        .collect()
}

/// `settings` with each of `changes` taken back where it still holds its `after` value.
fn restored(settings: &Settings, changes: &[Change]) -> Settings {
    let Ok(Value::Object(mut shown)) = serde_json::to_value(settings) else {
        return settings.clone();
    };
    for change in changes {
        if shown.get(&change.key) == Some(&change.after) {
            shown.insert(change.key.clone(), change.before.clone());
        }
    }
    serde_json::from_value(Value::Object(shown)).unwrap_or_else(|_| settings.clone())
}

/// `settings` with each of `keys` (stored names) as `defaults` has it: Restore defaults.
pub fn reset(settings: &Settings, defaults: &Settings, keys: &[String]) -> Settings {
    let (Ok(Value::Object(mut shown)), Ok(Value::Object(defaults))) = (
        serde_json::to_value(settings),
        serde_json::to_value(defaults),
    ) else {
        return settings.clone();
    };
    for key in keys {
        if let Some(value) = defaults.get(key) {
            shown.insert(key.clone(), value.clone());
        }
    }
    serde_json::from_value(Value::Object(shown)).unwrap_or_else(|_| settings.clone())
}

/// The app's own preferences, as shown now.
pub fn frontend(cx: &App) -> &kchess_contracts::types::FrontendPreferences {
    &cx.global::<SettingsHandle>().0.read(cx).settings.frontend
}

/// Change settings from a view; they save themselves.
pub fn change(cx: &mut App, change: impl FnOnce(&mut Settings) + 'static) {
    let handle = cx.global::<SettingsHandle>().0.clone();
    handle.update(cx, |settings, cx| settings.update(change, cx));
}

/// Take back the person's change `id` (the Undo of its notice), unless they changed settings
/// again since; it saves itself.
pub fn undo(cx: &mut App, id: u64) {
    let handle = cx.global::<SettingsHandle>().0.clone();
    handle.update(cx, |settings, cx| settings.undo(id, cx));
}

/// Save waiting edits now; done once the core has them (before asking it about what they set).
pub fn flush(cx: &mut App) -> BoxedSave {
    let handle = cx.global::<SettingsHandle>().0.clone();
    handle.update(cx, |settings, _| settings.flushed())
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, EDIT_WINDOW, Save, adopt, changes, defaults, rebased, restored};
    use crate::core::Reads;
    use crate::writer::{Queued, Writer};
    use futures::channel::mpsc;
    use kchess_contracts::types::Settings;
    use serde_json::Value;
    use std::time::{Duration, Instant};

    #[gpui_kit::test]
    fn tests_get_the_default_settings_without_a_core(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            let entity = super::init_for_tests(cx);
            assert_eq!(*super::frontend(cx), defaults().frontend);
            assert_eq!(
                cx.global::<super::SettingsHandle>().0.entity_id(),
                entity.entity_id()
            );
        });
    }

    /// Another window's save is being read when this window saves an edit: the read is answered
    /// after the edit, so it is read again. That retry updated the settings inside their own
    /// update, which GPUI refuses with a panic.
    #[gpui_kit::test]
    fn a_read_answered_after_a_local_edit_reads_again(cx: &mut gpui_kit::TestAppContext) {
        use kchess_contracts::calls::SaveDesktopSettings;
        cx.executor().allow_parking();
        let _core = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::tests::start_core(cx)
        });
        let entity = cx.update(|cx| {
            let themes = kchess_contracts::types::CustomThemeReport {
                themes: Vec::new(),
                dir: String::new(),
                problems: Vec::new(),
            };
            super::init(defaults(), Ok(themes), cx);
            cx.global::<super::SettingsHandle>().0.clone()
        });
        let base = defaults();
        let mut elsewhere = base.clone();
        elsewhere.frontend.sound_enabled = !base.frontend.sound_enabled;
        let bridge =
            cx.update(|cx| std::sync::Arc::clone(&cx.global::<crate::core::AppCore>().bridge));
        bridge
            .request_now(SaveDesktopSettings {
                settings: elsewhere.clone(),
                base: Some(base),
            })
            .expect("another window's save");
        cx.update(|cx| {
            super::reload(&entity, cx);
            // This window's edit is sent after that read started.
            entity.update(cx, |this, _| this.reads.edited());
        });
        for _ in 0..3000 {
            cx.run_until_parked();
            if cx.update(|cx| entity.read(cx).stored.frontend.sound_enabled)
                == elsewhere.frontend.sound_enabled
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for the settings to be read again");
    }

    /// A save the core refused reads the settings again from inside the settings' own update,
    /// which updated them again there and panicked (a full disk).
    #[gpui_kit::test]
    fn a_refused_save_reads_the_settings_again(cx: &mut gpui_kit::TestAppContext) {
        cx.executor().allow_parking();
        let _core = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::tests::start_core(cx)
        });
        let (sent, results) = mpsc::unbounded();
        let entity = cx.update(|cx| {
            let entity = super::init_for_tests(cx);
            super::show_saves(entity.downgrade(), results, cx);
            entity
        });
        sent.unbounded_send((
            defaults(),
            Err(kchess_contracts::CoreError::new("The disk is full")),
        ))
        .expect("the answer");
        for _ in 0..300 {
            cx.run_until_parked();
            if cx.update(|cx| entity.read(cx).failed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for the refused save");
    }

    fn settings(writes: Writer<Save>) -> AppSettings {
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
            last_edit: None,
            edits: 0,
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

    fn saves(queue: &mut mpsc::UnboundedReceiver<Queued<Save>>) -> Vec<(Settings, Settings)> {
        let mut saves = Vec::new();
        while let Ok(queued) = queue.try_recv() {
            if let Queued::Write(Save { sent, base }) = queued {
                saves.push((sent, base));
            }
        }
        saves
    }

    /// Sound on; off (save A); on again (save B) before A is answered. Both are written, so the
    /// profile ends with sound on, as shown.
    #[test]
    fn a_change_undone_while_a_save_is_writing_is_saved() {
        let (writes, mut queue) = Writer::queued("closed");
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
        let (writes, mut queue) = Writer::queued("closed");
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

    /// Record a change to `this` made by `change` at `at`, as `update` does.
    fn change(
        this: &mut AppSettings,
        at: Instant,
        change: impl FnOnce(&mut Settings),
    ) -> super::Edit {
        let before = this.settings.clone();
        change(&mut this.settings);
        this.record(changes(&before, &this.settings), at)
    }

    /// A volume dragged from 0.7 to 0.3 is one change: Undo goes back to 0.7.
    #[test]
    fn changes_to_one_setting_soon_after_each_other_are_one_edit() {
        let (writes, _queue) = Writer::queued("closed");
        let mut this = settings(writes);
        let start = Instant::now();
        let first = change(&mut this, start, |s| s.frontend.sound_volume = 0.5);
        let last = change(&mut this, start + Duration::from_secs(1), |s| {
            s.frontend.sound_volume = 0.3
        });
        assert_ne!(first.id, last.id, "the newer notice replaces the older");
        assert_eq!(last.changes.len(), 1);
        assert_eq!(last.changes[0].key, "soundVolume");
        assert_eq!(last.changes[0].before, serde_json::json!(0.7));
        assert_eq!(last.changes[0].after, serde_json::json!(0.3));
    }

    #[test]
    fn a_change_after_a_pause_or_to_another_setting_is_an_edit_of_its_own() {
        let (writes, _queue) = Writer::queued("closed");
        let mut this = settings(writes);
        let start = Instant::now();
        change(&mut this, start, |s| s.frontend.sound_volume = 0.5);
        let later = change(&mut this, start + EDIT_WINDOW, |s| {
            s.frontend.sound_volume = 0.3
        });
        assert_eq!(later.changes[0].before, serde_json::json!(0.5));
        let other = change(&mut this, start + EDIT_WINDOW, |s| {
            s.frontend.zen_mode = true
        });
        assert_eq!(other.changes.len(), 1);
        assert_eq!(other.changes[0].key, "zenMode");
    }

    /// A switch turned on and straight off again: Undo turns it back on, not nothing.
    #[test]
    fn a_setting_changed_back_is_an_edit_undo_takes_back() {
        let (writes, _queue) = Writer::queued("closed");
        let mut this = settings(writes);
        let start = Instant::now();
        change(&mut this, start, |s| s.frontend.zen_mode = true);
        let back = change(&mut this, start, |s| s.frontend.zen_mode = false);
        assert_eq!(back.changes[0].before, serde_json::json!(true));
        assert_eq!(back.changes[0].after, serde_json::json!(false));
    }

    #[test]
    fn undo_keeps_a_setting_changed_since() {
        let stored = defaults();
        let mut changed = stored.clone();
        changed.frontend.sound_volume = 0.2;
        changed.frontend.zen_mode = true;
        let edit = changes(&stored, &changed);
        // Another window set the volume meanwhile.
        changed.frontend.sound_volume = 0.9;
        let undone = restored(&changed, &edit);
        assert_eq!(undone.frontend.zen_mode, stored.frontend.zen_mode);
        assert_eq!(undone.frontend.sound_volume, 0.9);
    }
}

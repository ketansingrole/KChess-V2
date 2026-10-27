//! View preferences (tabs, layouts, toggles): kept by the core (`viewPreferences`), shared with the
//! Electron app under the same `kchess:` names (`useViewPreference`), so a page opens as it was left
//! in either app. Another app's change is taken when it is reported (`profile:changed`), unless
//! this app changed that preference since.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, Global};
use kchess_core::Level;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::core::{AppCore, BoxedSave, Saved};

#[derive(Default)]
pub struct ViewPreferences {
    values: Map<String, Value>,
    /// Keys whose latest write the core refused, with that write: retried as the app quits.
    failed: Failed,
    /// Writes, sent in order by one writer off the UI thread (`write_in_order`).
    writes: Option<mpsc::UnboundedSender<Write>>,
    /// Writes of each key the core has not answered yet.
    sending: Sending,
    /// Local changes made so far, and the latest one of each key, so an older read never undoes it.
    changes: u64,
    changed_at: HashMap<String, u64>,
    /// Reads of the saved preferences started; only the newest one is taken.
    reads: u64,
}

type Failed = Arc<Mutex<HashMap<String, (Value, String)>>>;
type Sending = Arc<Mutex<HashMap<String, usize>>>;

enum Write {
    /// Store `key` as `value`; `null` forgets it.
    Set(String, Value),
    /// Done once every write before it was answered, and refused ones were tried again.
    Flushed(oneshot::Sender<Saved>),
}

impl Global for ViewPreferences {}

impl ViewPreferences {
    /// The core refused the latest write of `key`: the shown value is not what the profile keeps.
    fn unsaved(&self, key: &str) -> bool {
        self.failed
            .lock()
            .is_ok_and(|failed| failed.contains_key(key))
    }

    fn write(&mut self, key: &str, value: Value) {
        self.changes += 1;
        self.changed_at.insert(key.to_string(), self.changes);
        if let Some(writes) = &self.writes
            && writes
                .unbounded_send(Write::Set(key.to_string(), value))
                .is_ok()
            && let Ok(mut sending) = self.sending.lock()
        {
            *sending.entry(key.to_string()).or_default() += 1;
        }
    }

    /// Take the core's preferences read after `since` changes, leaving those this app changed
    /// since then or has still to save. Returns whether any shown preference changed.
    fn adopt(&mut self, saved: Map<String, Value>, since: u64) -> bool {
        let keys: Vec<String> = self.values.keys().chain(saved.keys()).cloned().collect();
        let mut changed = false;
        for key in keys {
            let sending = self
                .sending
                .lock()
                .is_ok_and(|sending| sending.contains_key(&key));
            if sending
                || self.unsaved(&key)
                || self.changed_at.get(&key).is_some_and(|at| *at > since)
            {
                continue;
            }
            match saved.get(&key) {
                Some(value) if self.values.get(&key) != Some(value) => {
                    self.values.insert(key, value.clone());
                    changed = true;
                }
                Some(_) => {}
                None => changed |= self.values.remove(&key).is_some(),
            }
        }
        changed
    }

    /// Every write so far reached the profile, or why one did not.
    fn flushed(&self) -> BoxedSave {
        let (done, wait) = oneshot::channel();
        let sent = self
            .writes
            .as_ref()
            .is_some_and(|writes| writes.unbounded_send(Write::Flushed(done)).is_ok());
        Box::pin(async move {
            if !sent {
                return Ok(());
            }
            wait.await
                .unwrap_or_else(|_| Err("View preferences are closed.".to_string()))
        })
    }
}

/// Write each preference in order. A refused write is kept until a later write of its key
/// succeeds, and tried again when the app quits, which reports it if it fails again.
async fn write_in_order<F, Fut>(
    mut queue: mpsc::UnboundedReceiver<Write>,
    failed: Failed,
    sending: Sending,
    save: F,
) where
    F: Fn(String, Value) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let record = |key: String, value: Value, result: Result<(), String>| {
        if let Ok(mut failed) = failed.lock() {
            match result {
                Ok(()) => failed.remove(&key),
                Err(cause) => failed.insert(key, (value, cause)),
            };
        }
    };
    while let Some(write) = queue.next().await {
        match write {
            Write::Set(key, value) => {
                let result = save(key.clone(), value.clone()).await;
                if let Ok(mut sending) = sending.lock()
                    && let Some(count) = sending.get_mut(&key)
                {
                    *count -= 1;
                    if *count == 0 {
                        sending.remove(&key);
                    }
                }
                record(key, value, result);
            }
            Write::Flushed(done) => {
                let retry: Vec<(String, Value)> = failed
                    .lock()
                    .map(|failed| {
                        failed
                            .iter()
                            .map(|(key, (value, _))| (key.clone(), value.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                for (key, value) in retry {
                    let result = save(key.clone(), value.clone()).await;
                    record(key, value, result);
                }
                let cause = failed
                    .lock()
                    .ok()
                    .and_then(|failed| failed.values().next().map(|(_, cause)| cause.clone()));
                let _ = done.send(match cause {
                    Some(cause) => Err(format!("A view preference could not be saved: {cause}")),
                    None => Ok(()),
                });
            }
        }
    }
}

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
    let (writes, queue) = mpsc::unbounded();
    let failed = Failed::default();
    let sending = Sending::default();
    let core = cx.global::<AppCore>().clone();
    cx.background_executor()
        .spawn(write_in_order(
            queue,
            Arc::clone(&failed),
            Arc::clone(&sending),
            move |key, value| {
                let save = core.call("setViewPreference", vec![json!(key), value]);
                async move {
                    save.await.map(|_| ()).map_err(|cause| {
                        crate::log::write(
                            Level::Warn,
                            "view-preferences",
                            &format!("Saving a view preference failed: {key} {}", cause.message),
                        );
                        cause.message
                    })
                }
            },
        ))
        .detach();
    cx.set_global(ViewPreferences {
        values,
        failed,
        writes: Some(writes),
        sending,
        ..ViewPreferences::default()
    });
    crate::core::on_quit(cx, |cx| {
        cx.try_global::<ViewPreferences>()
            .map(ViewPreferences::flushed)
    });
    crate::core::on_profile_change(cx, &["viewPreferences"], reload);
}

/// Read the saved preferences again: another app changed them, or this one reconnected.
fn reload(cx: &mut App) {
    let prefs = cx.default_global::<ViewPreferences>();
    prefs.reads += 1;
    let (read, since) = (prefs.reads, prefs.changes);
    let saved = cx.global::<AppCore>().call("viewPreferences", Vec::new());
    cx.spawn(async move |cx| {
        let saved = match saved.await {
            Ok(Value::Object(saved)) => saved,
            Ok(_) => return,
            Err(cause) => {
                return crate::log::write(
                    Level::Warn,
                    "view-preferences",
                    &format!("Reading view preferences failed: {}", cause.message),
                );
            }
        };
        cx.update(|cx| {
            let prefs = cx.default_global::<ViewPreferences>();
            if prefs.reads == read && prefs.adopt(saved, since) {
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// The preference `key`, or `fallback` until it is changed or when it is unreadable.
pub fn get<T: DeserializeOwned>(cx: &App, key: &str, fallback: T) -> T {
    cx.try_global::<ViewPreferences>()
        .and_then(|prefs| prefs.values.get(key))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or(fallback)
}

/// Remember a preference for every KChess app on this profile. Choosing the shown value again
/// writes it again when the core refused it.
pub fn set<T: Serialize>(cx: &mut App, key: &str, value: T) {
    let value = json!(value);
    let prefs = cx.default_global::<ViewPreferences>();
    if prefs.values.get(key) == Some(&value) && !prefs.unsaved(key) {
        return;
    }
    prefs.values.insert(key.to_string(), value.clone());
    prefs.write(key, value);
    // Views read preferences while rendering without observing them; a cached one shows the
    // change only when the windows redraw past their caches.
    cx.refresh_windows();
}

/// Forget a preference (`forgetViewPreference`): its default applies again in both apps.
pub fn forget(cx: &mut App, key: &str) {
    let prefs = cx.default_global::<ViewPreferences>();
    if prefs.values.remove(key).is_none() && !prefs.unsaved(key) {
        return;
    }
    prefs.write(key, Value::Null);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::LocalPool;
    use futures::task::LocalSpawnExt as _;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A refused write (a set, then a forget) was dropped: the core never heard of it again, and
    /// quitting reported success. It is kept, tried again at quit once the core recovers, and a
    /// later write of the key replaces it.
    #[test]
    fn refused_writes_are_retried_at_quit_until_the_profile_keeps_them() {
        let refuse = Arc::new(AtomicBool::new(true));
        let stored = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let failed = Failed::default();
        let (writes, queue) = mpsc::unbounded();
        let mut pool = LocalPool::new();
        pool.spawner()
            .spawn_local(write_in_order(
                queue,
                Arc::clone(&failed),
                Sending::default(),
                {
                    let refuse = Arc::clone(&refuse);
                    let stored = Arc::clone(&stored);
                    move |key, value| {
                        let result = if refuse.load(Ordering::SeqCst) {
                            Err("disk full".to_string())
                        } else {
                            stored.lock().unwrap().push((key, value));
                            Ok(())
                        };
                        async move { result }
                    }
                },
            ))
            .unwrap();
        let mut prefs = ViewPreferences {
            failed: Arc::clone(&failed),
            writes: Some(writes),
            ..ViewPreferences::default()
        };
        prefs.write("kchess:tab", json!("games"));
        prefs.write("kchess:mode", Value::Null);
        prefs.write("kchess:size", json!(1));
        pool.run_until_stalled();
        assert!(prefs.unsaved("kchess:tab") && prefs.unsaved("kchess:mode"));
        let quit = pool.run_until(prefs.flushed());
        assert!(quit.unwrap_err().contains("disk full"));

        refuse.store(false, Ordering::SeqCst);
        // A later write of a key replaces its refused one.
        prefs.write("kchess:size", json!(2));
        pool.run_until_stalled();
        assert!(!prefs.unsaved("kchess:size"));
        assert_eq!(pool.run_until(prefs.flushed()), Ok(()));
        let mut kept = stored.lock().unwrap().clone();
        kept.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            kept,
            [
                ("kchess:mode".to_string(), Value::Null),
                ("kchess:size".to_string(), json!(2)),
                ("kchess:tab".to_string(), json!("games")),
            ]
        );
        assert!(failed.lock().unwrap().is_empty());
        prefs.writes = None;
    }

    /// Another app's change stayed unseen until restart. It is taken, except where this app
    /// changed the preference after the read began or has a write still unanswered.
    #[test]
    fn takes_another_apps_preferences_but_keeps_newer_local_changes() {
        let (writes, _queue) = mpsc::unbounded();
        let mut prefs = ViewPreferences {
            writes: Some(writes),
            ..ViewPreferences::default()
        };
        prefs.values.insert("kchess:tab".into(), json!("games"));
        prefs.values.insert("kchess:gone".into(), json!(1));
        let since = prefs.changes;
        prefs.values.insert("kchess:mine".into(), json!("local"));
        prefs.write("kchess:mine", json!("local"));
        let saved = Map::from_iter([
            ("kchess:tab".to_string(), json!("studies")),
            ("kchess:mine".to_string(), json!("older")),
            ("kchess:new".to_string(), json!(true)),
        ]);
        assert!(prefs.adopt(saved.clone(), since));
        assert_eq!(prefs.values["kchess:tab"], json!("studies"));
        assert_eq!(prefs.values["kchess:new"], json!(true));
        assert_eq!(prefs.values["kchess:mine"], json!("local"));
        assert!(!prefs.values.contains_key("kchess:gone"));
        // A read begun after the change still waits for the write's answer.
        assert!(!prefs.adopt(saved, prefs.changes));
        assert_eq!(prefs.values["kchess:mine"], json!("local"));
        prefs.writes = None;
    }

    /// A view that reads a preference while rendering, counting its renders.
    struct Reader {
        renders: std::rc::Rc<std::cell::Cell<u32>>,
        shown: std::rc::Rc<std::cell::Cell<i64>>,
    }

    impl gpui_kit::Render for Reader {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            cx: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            self.renders.set(self.renders.get() + 1);
            self.shown.set(get(cx, "kchess:test-size", 0));
            gpui_kit::div()
        }
    }

    /// Holds the reader as a cached view, as the shell holds the page and pages hold the board.
    struct Holder(gpui_kit::Entity<Reader>);

    impl gpui_kit::Render for Holder {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            use gpui_kit::{ParentElement as _, Styled as _};
            let mut style = gpui_kit::StyleRefinement::default();
            style.size.width = Some(gpui_kit::relative(1.).into());
            style.size.height = Some(gpui_kit::relative(1.).into());
            gpui_kit::div()
                .size_full()
                .child(self.0.clone().cached(style))
        }
    }

    /// Views read preferences without observing them; once they are cached, a changed
    /// preference showed only after something else redrew them (a page tab stayed on the old one).
    #[gpui_kit::test]
    fn a_changed_preference_reaches_cached_views(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(|cx| cx.set_global(ViewPreferences::default()));
        let renders = std::rc::Rc::new(std::cell::Cell::new(0));
        let shown = std::rc::Rc::new(std::cell::Cell::new(-1));
        let reader = cx.new(|_| Reader {
            renders: renders.clone(),
            shown: shown.clone(),
        });
        let window = cx.add_window(|_, _| Holder(reader));
        let draw = |cx: &mut gpui_kit::TestAppContext| {
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        };
        draw(cx);
        assert_eq!(shown.get(), 0);

        // The holder redrawing alone reuses the cached reader.
        let before = renders.get();
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        draw(cx);
        assert_eq!(renders.get(), before, "the cached reader drew again");

        cx.update(|cx| set(cx, "kchess:test-size", 5));
        draw(cx);
        assert_eq!(shown.get(), 5);

        cx.update(|cx| forget(cx, "kchess:test-size"));
        draw(cx);
        assert_eq!(shown.get(), 0);
    }
}

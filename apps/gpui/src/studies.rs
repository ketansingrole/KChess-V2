//! The study library as the core keeps it (`stores/studies.ts`): every change is a command the core
//! makes (`studyCommand`), sent in order by one writer off the UI thread, so the app can wait for
//! them while it quits. Each command's answer comes back to its caller; the library shows the
//! studies the answers name. A command the core refuses shows its error and what the profile keeps.

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, AppContext as _, Context, Entity, Global, WeakEntity};
use kchess_core::Level;
use serde_json::{Map, Value, json};

use crate::core::{AppCore, BoxedSave};

/// Studies a library may hold.
pub const MAX_STUDIES: usize = 50;
/// Chapters a study may hold.
pub const MAX_CHAPTERS: usize = 64;

enum Write {
    Command {
        command: Value,
        reply: Option<oneshot::Sender<Result<Value, String>>>,
    },
    Flushed(oneshot::Sender<()>),
}

pub struct StudyLibrary {
    items: Vec<Value>,
    /// Why the last command failed, until one succeeds; or why the stored studies could not be read.
    pub error: Option<String>,
    writes: mpsc::UnboundedSender<Write>,
}

/// How views reach the library: `cx.global::<StudyHandle>().0`.
#[derive(Clone)]
pub struct StudyHandle(pub Entity<StudyLibrary>);

impl Global for StudyHandle {}

/// Read the studies before the first page opens, and start the writer.
pub fn init(cx: &mut App) {
    let library = cx
        .global::<AppCore>()
        .bridge
        .call_now("library", Vec::new())
        .unwrap_or(Value::Null);
    let items = library["studies"].as_array().cloned().unwrap_or_default();
    let error = library["studyError"].as_str().map(str::to_string);
    let (writes, queue) = mpsc::unbounded();
    let entity = cx.new(|_| StudyLibrary {
        items,
        error,
        writes,
    });
    write_in_order(entity.downgrade(), queue, cx);
    crate::core::on_quit(cx, {
        let entity = entity.clone();
        move |cx| Some(entity.read(cx).flushed())
    });
    cx.set_global(StudyHandle(entity));
}

fn write_in_order(
    entity: WeakEntity<StudyLibrary>,
    mut queue: mpsc::UnboundedReceiver<Write>,
    cx: &mut App,
) {
    let core = cx.global::<AppCore>().clone();
    // Each answer, for the library to show.
    let (answers, mut answered) = mpsc::unbounded::<Result<Value, String>>();
    cx.background_executor()
        .spawn(async move {
            while let Some(write) = queue.next().await {
                let (command, reply) = match write {
                    Write::Command { command, reply } => (command, reply),
                    Write::Flushed(done) => {
                        let _ = done.send(());
                        continue;
                    }
                };
                let op = command["op"].as_str().unwrap_or_default().to_string();
                let result = core
                    .call("studyCommand", vec![command, json!(true)])
                    .await
                    .map_err(|cause| {
                        crate::log::write(
                            Level::Warn,
                            "studies",
                            &format!("saving the study library failed: {op} {}", cause.message),
                        );
                        cause.message
                    });
                // The caller hears first, from here: the UI thread may not run while the app quits.
                if let Some(reply) = reply {
                    let _ = reply.send(result.clone());
                }
                let _ = answers.unbounded_send(result);
            }
        })
        .detach();
    cx.spawn(async move |cx| {
        while let Some(answer) = answered.next().await {
            let _ = entity.update(cx, |this, cx| {
                match answer {
                    Ok(result) => {
                        this.apply(&result);
                        this.error = None;
                    }
                    Err(cause) => {
                        this.error = Some(cause);
                        this.reload(cx);
                    }
                }
                cx.notify();
            });
        }
    })
    .detach();
}

/// `timeAgo`: "just now", "5 min ago", "3 h ago", "2 days ago", else the date.
pub fn time_ago(timestamp: f64, now: f64) -> String {
    if timestamp <= 0. {
        return "never".into();
    }
    let seconds = ((now - timestamp) / 1000.).round().max(0.);
    if seconds < 45. {
        return "just now".into();
    }
    let minutes = (seconds / 60.).round();
    if minutes < 60. {
        return format!("{minutes} min ago");
    }
    let hours = (minutes / 60.).round();
    if hours < 24. {
        return format!("{hours} h ago");
    }
    let days = (hours / 24.).round();
    if days < 31. {
        return format!("{days} day{} ago", if days == 1. { "" } else { "s" });
    }
    crate::game::game_date(timestamp)
}

/// Milliseconds since the epoch.
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |since| since.as_millis() as f64)
}

/// A study's chapters as select items: their ids, and their names numbered as the study orders them.
pub fn chapter_items(study: &Value) -> Vec<(String, String)> {
    study["chapters"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, chapter)| {
            (
                chapter["id"].as_str().unwrap_or_default().to_string(),
                format!(
                    "{}. {}",
                    index + 1,
                    chapter["name"].as_str().unwrap_or_default()
                ),
            )
        })
        .collect()
}

/// The whole study as one PGN, a game per chapter (`studyDocumentPgn`).
pub fn document_pgn(study: &Value) -> String {
    crate::game::rules("studyDocumentPgn", &[study["chapters"].clone()])
        .ok()
        .and_then(|pgn| pgn.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl StudyLibrary {
    pub fn items(&self) -> &[Value] {
        &self.items
    }

    pub fn study(&self, id: &str) -> Option<&Value> {
        self.items.iter().find(|study| study["id"] == json!(id))
    }

    /// A name no study has yet (`freshName`).
    pub fn fresh_name(&self, base: &str) -> String {
        (1..)
            .map(|n| {
                if n == 1 {
                    base.to_string()
                } else {
                    format!("{base} {n}")
                }
            })
            .find(|name| !self.items.iter().any(|study| study["name"] == json!(name)))
            .unwrap_or_else(|| base.to_string())
    }

    /// Send a command; its answer (the changed studies, and an `id` where one was made).
    pub fn command(
        &self,
        command: Value,
    ) -> impl std::future::Future<Output = Result<Value, String>> + 'static + use<> {
        let (reply, answer) = oneshot::channel();
        let sent = self.writes.unbounded_send(Write::Command {
            command,
            reply: Some(reply),
        });
        async move {
            if sent.is_err() {
                return Err("The study library is closed.".to_string());
            }
            answer
                .await
                .unwrap_or_else(|_| Err("The study library is closed.".to_string()))
        }
    }

    /// Wait for every command sent so far.
    pub fn flushed(&self) -> BoxedSave {
        let (done, wait) = oneshot::channel();
        let _ = self.writes.unbounded_send(Write::Flushed(done));
        Box::pin(async move {
            let _ = wait.await;
        })
    }

    /// Show the studies an answer names: all of them, or the changed ones in the library's order.
    pub fn apply(&mut self, result: &Value) {
        let studies = result["studies"].as_array().cloned().unwrap_or_default();
        let Some(order) = result["order"].as_array() else {
            if result.get("studies").is_some() {
                self.items = studies;
            }
            return;
        };
        let mut by_id: Map<String, Value> = Map::new();
        for study in self.items.drain(..).chain(studies) {
            if let Some(id) = study["id"].as_str() {
                by_id.insert(id.to_string(), study);
            }
        }
        self.items = order
            .iter()
            .filter_map(|id| id.as_str().and_then(|id| by_id.remove(id)))
            .collect();
    }

    /// Show the studies the profile keeps (after a command failed).
    fn reload(&mut self, cx: &mut Context<Self>) {
        let library = cx.global::<AppCore>().call("library", Vec::new());
        cx.spawn(async move |this, cx| {
            if let Ok(library) = library.await
                && let Some(items) = library["studies"].as_array()
            {
                let items = items.clone();
                let _ = this.update(cx, |this, cx| {
                    this.items = items;
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{StudyLibrary, chapter_items, time_ago};
    use futures::channel::mpsc;
    use serde_json::json;

    #[test]
    fn answers_show_the_changed_studies_in_the_library_order() {
        let (writes, _queue) = mpsc::unbounded();
        let mut library = StudyLibrary {
            items: vec![
                json!({ "id": "a", "name": "A" }),
                json!({ "id": "b", "name": "B" }),
            ],
            error: None,
            writes,
        };
        // An incremental answer: the changed study, and every id in order.
        library.apply(&json!({
            "studies": [{ "id": "c", "name": "C" }, { "id": "a", "name": "A2" }],
            "order": ["c", "a", "b"],
        }));
        let names: Vec<_> = library
            .items
            .iter()
            .map(|study| study["name"].clone())
            .collect();
        assert_eq!(names, [json!("C"), json!("A2"), json!("B")]);
        // A removal leaves the id out of the order.
        library.apply(&json!({ "studies": [], "order": ["a"] }));
        assert_eq!(library.items.len(), 1);
        // A whole answer replaces the list.
        library.apply(&json!({ "studies": [{ "id": "z" }] }));
        assert_eq!(library.items, [json!({ "id": "z" })]);
        assert_eq!(library.fresh_name("Z"), "Z");
    }

    #[test]
    fn edits_read_as_how_long_ago() {
        let now = 1_000_000_000_000.;
        assert_eq!(time_ago(now - 10_000., now), "just now");
        assert_eq!(time_ago(now - 5. * 60_000., now), "5 min ago");
        assert_eq!(time_ago(now - 3. * 3_600_000., now), "3 h ago");
        assert_eq!(time_ago(now - 86_400_000., now), "1 day ago");
        assert_eq!(time_ago(0., now), "never");
    }

    #[test]
    fn chapters_are_numbered_in_order() {
        let study =
            json!({ "chapters": [{ "id": "a", "name": "Main" }, { "id": "b", "name": "Side" }] });
        assert_eq!(
            chapter_items(&study),
            [
                ("a".to_string(), "1. Main".to_string()),
                ("b".to_string(), "2. Side".to_string())
            ]
        );
    }
}

//! The study library as the core keeps it: every change is a command the core
//! makes (`studyCommand`), sent in order by one writer off the UI thread, so the app can wait for
//! them while it quits. Each command's answer comes back to its caller; the library shows the
//! studies the answers name. A command the core refuses shows its error and what the profile keeps.

use kchess_contracts::calls;
use std::collections::HashMap;

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, AppContext as _, Context, Entity, Global, WeakEntity};
use kchess_contracts::check::PgnText;
use kchess_contracts::types::{Library, ProfileArea, StudyCommand, StudyCommandResult};
use kchess_contracts::{CoreError, ErrorKind, Level};
use serde_json::{Map, Value, json};

use crate::core::{Answer, AppCore, Reads};
use crate::quit::BoxedSave;

const CLOSED: &str = "The study library is closed";

/// Studies a library may hold.
pub const MAX_STUDIES: usize = 50;
/// Chapters a study may hold.
pub const MAX_CHAPTERS: usize = 64;

enum Write {
    Command {
        command: StudyCommand,
        reply: Option<oneshot::Sender<Result<StudyCommandResult, CoreError>>>,
    },
    /// Done once every command before it was answered: with the chapters whose last content save
    /// failed, if any did.
    Flushed(oneshot::Sender<crate::quit::Saved>),
    /// The board now shows this chapter with these moves (`None`: the library has no such
    /// chapter). Queued like the saves, so a save sent before it keeps the base it was made on.
    Track {
        key: (String, String),
        pgn: Option<String>,
    },
}

/// The moves each chapter held when this window last opened or saved it, by study and chapter.
/// Chapter saves are based on them, so they never overwrite another window's newer
/// moves; a library refresh leaves them. Only the writer keeps them, in the order of its queue.
type Bases = HashMap<(String, String), String>;

/// The core refused a chapter save: another window changed the chapter since this one read it.
pub fn is_stale(cause: &CoreError) -> bool {
    cause.kind == Some(ErrorKind::Conflict)
}

pub struct StudyLibrary {
    items: Vec<Value>,
    /// Why the last command failed, until one succeeds; or why the stored studies could not be read.
    pub error: Option<String>,
    writes: mpsc::UnboundedSender<Write>,
    reads: Reads,
}

/// How views reach the library: `cx.global::<StudyHandle>().0`.
#[derive(Clone)]
pub struct StudyHandle(pub Entity<StudyLibrary>);

impl Global for StudyHandle {}

/// Start from the studies of the library read before the first page opens, and start the writer.
pub fn init(library: &Library, cx: &mut App) {
    let items = library.studies.clone();
    let error = library.study_error.clone();
    let (writes, queue) = mpsc::unbounded();
    let entity = cx.new(|_| StudyLibrary {
        items,
        error,
        writes,
        reads: Reads::default(),
    });
    write_in_order(entity.downgrade(), queue, cx);
    crate::quit::on_quit(cx, {
        let entity = entity.clone();
        move |cx| Some(entity.read(cx).flushed())
    });
    // Another window saved a study: show the profile's.
    crate::core::on_profile_change(cx, &[ProfileArea::Studies], {
        let entity = entity.downgrade();
        move |cx| {
            let _ = entity.update(cx, |library, cx| library.reload(cx));
        }
    });
    cx.set_global(StudyHandle(entity));
}

/// A stored study's id.
pub fn study_id(study: &Value) -> String {
    study["id"].as_str().unwrap_or_default().to_string()
}

/// The chapter a content save writes to an existing study.
fn saved_chapter(command: &StudyCommand) -> Option<(String, String)> {
    let StudyCommand::Save {
        id: Some(id),
        chapter_id: Some(chapter),
        ..
    } = command
    else {
        return None;
    };
    (!id.is_empty() && !chapter.is_empty()).then(|| (id.clone(), chapter.clone()))
}

/// After a content save: the chapter it wrote, whose moves are now the base, and those moves.
fn written_chapter(
    command: &StudyCommand,
    result: &StudyCommandResult,
) -> Option<((String, String), String)> {
    let StudyCommand::Save {
        id,
        chapter_id,
        pgn,
        ..
    } = command
    else {
        return None;
    };
    let id = result.id.as_deref().or(id.as_deref())?;
    let chapter = chapter_id
        .as_deref()
        .filter(|chapter| !chapter.is_empty())
        .or_else(|| {
            result
                .studies
                .iter()
                .find(|study| study["id"] == json!(id))?["chapters"][0]["id"]
                .as_str()
        })?;
    Some(((id.to_string(), chapter.to_string()), pgn.clone()))
}

/// The study, and the chapter when only one goes, a removal takes out of the library.
fn removed_by(command: &StudyCommand) -> Option<(&str, Option<&str>)> {
    match command {
        StudyCommand::Remove { id } => Some((id, None)),
        StudyCommand::RemoveChapter { id, chapter_id } => Some((id, Some(chapter_id))),
        _ => None,
    }
}

/// Forgets what is kept for a removed study (`chapter` absent) or one of its chapters.
fn forget_removed<V>(kept: &mut HashMap<(String, String), V>, study: &str, chapter: Option<&str>) {
    kept.retain(|(id, at), _| id != study || chapter.is_some_and(|chapter| chapter != at));
}

fn write_in_order(
    entity: WeakEntity<StudyLibrary>,
    mut queue: mpsc::UnboundedReceiver<Write>,
    cx: &mut App,
) {
    let core = cx.global::<AppCore>().clone();
    // Each answer, for the library to show.
    let (answers, mut answered) = mpsc::unbounded::<Result<StudyCommandResult, CoreError>>();
    cx.background_executor()
        .spawn(async move {
            // Chapters whose last content save failed, until one succeeds: their moves are not
            // in the profile (a chapter another window changed keeps its moves as a copy).
            let mut unsaved: HashMap<(String, String), String> = HashMap::new();
            let mut bases = Bases::new();
            while let Some(write) = queue.next().await {
                let (command, reply) = match write {
                    Write::Command { command, reply } => (command, reply),
                    Write::Track { key, pgn } => {
                        match pgn {
                            Some(pgn) => bases.insert(key, pgn),
                            None => bases.remove(&key),
                        };
                        continue;
                    }
                    Write::Flushed(done) => {
                        let _ = done.send(match unsaved.values().next() {
                            Some(cause) => Err(format!("A study could not be saved: {cause}")),
                            None => Ok(()),
                        });
                        continue;
                    }
                };
                let based_on = saved_chapter(&command).and_then(|key| bases.get(&key).cloned());
                let call = calls::StudyCommand {
                    command: command.clone(),
                    incremental: Some(true),
                    based_on: based_on.map(PgnText),
                };
                let result = core.request(call).await.inspect_err(|cause| {
                    let op = json!(command)["op"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    crate::log::write(
                        Level::Warn,
                        "studies",
                        &format!("saving the study library failed: {op} {}", cause.message),
                    );
                });
                if let Some(key) = saved_chapter(&command) {
                    match &result {
                        Err(cause) if !is_stale(cause) => {
                            unsaved.insert(key, cause.message.clone());
                        }
                        _ => {
                            unsaved.remove(&key);
                        }
                    }
                }
                // A removed study or chapter has nothing left to save.
                if result.is_ok()
                    && let Some((study, chapter)) = removed_by(&command)
                {
                    forget_removed(&mut unsaved, study, chapter);
                    forget_removed(&mut bases, study, chapter);
                }
                if let Ok(result) = &result
                    && let Some((key, pgn)) = written_chapter(&command, result)
                {
                    bases.insert(key, pgn);
                }
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
                        this.reads.edited();
                        this.error = None;
                    }
                    Err(cause) => {
                        this.error = Some(cause.message);
                        this.reload(cx);
                    }
                }
                cx.notify();
            });
        }
    })
    .detach();
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

/// The whole study as one PGN, a game per chapter.
pub fn document_pgn(study: &Value) -> String {
    let chapters: Option<Vec<(&str, &str)>> = study["chapters"].as_array().and_then(|chapters| {
        chapters
            .iter()
            .map(|chapter| Some((chapter["name"].as_str()?, chapter["pgn"].as_str()?)))
            .collect()
    });
    chapters
        .and_then(|chapters| kchess_domain::pgn::study_document_pgn(&chapters))
        .unwrap_or_default()
}

impl StudyLibrary {
    pub fn items(&self) -> &[Value] {
        &self.items
    }

    pub fn study(&self, id: &str) -> Option<&Value> {
        self.items.iter().find(|study| study["id"] == json!(id))
    }

    /// The board shows this chapter as the library has it: its saves are based on that.
    pub fn track(&self, study: &str, chapter: &str) {
        let pgn = self
            .study(study)
            .and_then(|study| study["chapters"].as_array())
            .and_then(|chapters| chapters.iter().find(|c| c["id"] == json!(chapter)))
            .and_then(|chapter| chapter["pgn"].as_str())
            .map(str::to_string);
        let key = (study.to_string(), chapter.to_string());
        let _ = self.writes.unbounded_send(Write::Track { key, pgn });
    }

    /// A name no study has yet.
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

    /// Send a command; its answer (the changed studies, and an `id` where one was made), or why
    /// the core refused it.
    pub fn command(
        &self,
        command: StudyCommand,
    ) -> impl std::future::Future<Output = Result<StudyCommandResult, String>> + 'static + use<>
    {
        let answer = self.command_answer(command);
        async move { answer.await.map_err(|cause| cause.message) }
    }

    /// `command`, with the core's error as it came: a stale chapter save is `ErrorKind::Conflict`.
    pub fn command_answer(
        &self,
        command: StudyCommand,
    ) -> impl std::future::Future<Output = Result<StudyCommandResult, CoreError>> + 'static + use<>
    {
        let (reply, answer) = oneshot::channel();
        let sent = self.writes.unbounded_send(Write::Command {
            command,
            reply: Some(reply),
        });
        async move {
            if sent.is_err() {
                return Err(CoreError::new(CLOSED));
            }
            answer.await.unwrap_or_else(|_| Err(CoreError::new(CLOSED)))
        }
    }

    /// Wait for every command sent so far: with why a chapter's moves are not saved, if any.
    pub fn flushed(&self) -> BoxedSave {
        let (done, wait) = oneshot::channel();
        let _ = self.writes.unbounded_send(Write::Flushed(done));
        Box::pin(async move { wait.await.unwrap_or_else(|_| Err(CLOSED.to_string())) })
    }

    /// Show the studies an answer names: all of them, or the changed ones in the library's order.
    pub fn apply(&mut self, result: &StudyCommandResult) {
        let studies = result.studies.clone();
        let Some(order) = &result.order else {
            self.items = studies;
            return;
        };
        let mut by_id: Map<String, Value> = Map::new();
        for study in self.items.drain(..).chain(studies) {
            if let Some(id) = study["id"].as_str() {
                by_id.insert(id.to_string(), study);
            }
        }
        self.items = order.iter().filter_map(|id| by_id.remove(id)).collect();
    }

    /// Show the studies the profile keeps (after a command failed, or another window changed
    /// them): read once this window's commands are answered, and never over a newer answer.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let read = self.reads.start();
        let sent = self.flushed();
        cx.spawn(async move |this, cx| {
            let _ = sent.await;
            let Ok(library) = this.update(cx, |_, cx| {
                cx.global::<AppCore>().request(calls::Library {})
            }) else {
                return;
            };
            if let Ok(library) = library.await {
                let items = library.studies;
                let _ = this.update(cx, |this, cx| match this.reads.answer(read) {
                    Answer::Show => {
                        this.items = items;
                        cx.notify();
                    }
                    Answer::Again => this.reload(cx),
                    Answer::Superseded => {}
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Bases, StudyLibrary, Write, chapter_items, forget_removed, saved_chapter, written_chapter,
    };
    use futures::channel::mpsc;
    use kchess_contracts::types::{StudyCommand, StudyCommandResult};
    use serde_json::json;

    fn answer(value: serde_json::Value) -> StudyCommandResult {
        serde_json::from_value(value).expect("a study command answer")
    }

    fn chapter_save(id: Option<&str>, chapter: Option<&str>, pgn: &str) -> StudyCommand {
        StudyCommand::Save {
            name: "A".into(),
            pgn: pgn.into(),
            id: id.map(str::to_string),
            chapter_id: chapter.map(str::to_string),
        }
    }

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
            reads: Default::default(),
        };
        // An incremental answer: the changed study, and every id in order.
        library.apply(&answer(json!({
            "studies": [{ "id": "c", "name": "C" }, { "id": "a", "name": "A2" }],
            "order": ["c", "a", "b"],
        })));
        let names: Vec<_> = library
            .items
            .iter()
            .map(|study| study["name"].clone())
            .collect();
        assert_eq!(names, [json!("C"), json!("A2"), json!("B")]);
        // A removal leaves the id out of the order.
        library.apply(&answer(json!({ "studies": [], "order": ["a"] })));
        assert_eq!(library.items.len(), 1);
        // A whole answer replaces the list.
        library.apply(&answer(json!({ "studies": [{ "id": "z" }] })));
        assert_eq!(library.items, [json!({ "id": "z" })]);
        assert_eq!(library.fresh_name("Z"), "Z");
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

    /// A save queued before the board reopens a chapter another window changed keeps the base it
    /// was made on: the new base reaches the writer after it, so the core refuses it as stale
    /// instead of letting it overwrite the other window's moves.
    #[test]
    fn a_reopened_chapter_takes_its_base_after_the_saves_queued_before() {
        let (writes, mut queue) = mpsc::unbounded();
        let library = StudyLibrary {
            items: vec![json!({ "id": "a", "chapters": [{ "id": "c", "pgn": "1. d4 *" }] })],
            error: None,
            writes,
            reads: Default::default(),
        };
        let save = chapter_save(Some("a"), Some("c"), "1. e4 e5 *");
        drop(library.command(save.clone()));
        library.track("a", "c");
        let Ok(Write::Command { command, .. }) = queue.try_recv() else {
            panic!("the save comes first");
        };
        assert_eq!(command, save);
        let Ok(Write::Track { key, pgn }) = queue.try_recv() else {
            panic!("then the new base");
        };
        assert_eq!(key, ("a".to_string(), "c".to_string()));
        assert_eq!(pgn.as_deref(), Some("1. d4 *"));
    }

    /// Regression: the base of every chapter opened or saved stayed after its study or chapter
    /// was removed, a copy of its moves for the session.
    #[test]
    fn a_removed_study_or_chapter_is_forgotten() {
        let key = |study: &str, chapter: &str| (study.to_string(), chapter.to_string());
        let mut kept: Bases = [key("a", "1"), key("a", "2"), key("b", "1")]
            .into_iter()
            .map(|key| (key, String::new()))
            .collect();
        forget_removed(&mut kept, "a", Some("1"));
        assert!(!kept.contains_key(&key("a", "1")));
        assert!(kept.contains_key(&key("a", "2")) && kept.contains_key(&key("b", "1")));
        forget_removed(&mut kept, "a", None);
        assert_eq!(kept.keys().collect::<Vec<_>>(), vec![&key("b", "1")]);
    }

    #[test]
    fn chapter_saves_are_based_on_the_chapter_they_write() {
        let save = chapter_save(Some("a"), Some("c"), "1. e4 *");
        let key = ("a".to_string(), "c".to_string());
        assert_eq!(saved_chapter(&save), Some(key.clone()));
        assert_eq!(
            written_chapter(&save, &answer(json!({ "id": "a", "studies": [] }))),
            Some((key, "1. e4 *".to_string()))
        );
        let rename = StudyCommand::Rename {
            id: "a".into(),
            name: "B".into(),
        };
        assert_eq!(saved_chapter(&rename), None);
        // A new study: its first chapter, once the core made it.
        let new = chapter_save(None, None, "1. d4 *");
        assert_eq!(saved_chapter(&new), None);
        let made =
            answer(json!({ "id": "n", "studies": [{ "id": "n", "chapters": [{ "id": "n1" }] }] }));
        assert_eq!(
            written_chapter(&new, &made),
            Some((("n".to_string(), "n1".to_string()), "1. d4 *".to_string()))
        );
    }
}

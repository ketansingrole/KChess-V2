//! The study library as the core keeps it: every change is a command the core
//! makes (`studyCommand`), sent in order by one writer off the UI thread, so the app can wait for
//! them while it quits. Each command's answer comes back to its caller; the library shows the
//! studies the answers name. A command the core refuses shows its error and what the profile keeps.

use crate::log::ResultExt as _;
use kchess_contracts::calls;
use std::collections::HashMap;

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, AppContext as _, Context, Entity, Global};
use kchess_contracts::types::{Library, ProfileArea, Study, StudyCommand, StudyCommandResult};
use kchess_contracts::{CoreError, ErrorKind, Level};
use serde_json::json;

use crate::core::{Answer, AppCore, Reads};
use crate::quit::{BoxedSave, Saved};
use crate::writer::{Store, Writer};

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
    items: Vec<Study>,
    /// Why the last command failed, until one succeeds; or why the stored studies could not be read.
    pub error: Option<String>,
    writes: Writer<Write>,
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
    // Each answer, for the library to show.
    let (answers, answered) = mpsc::unbounded();
    let core = cx.global::<AppCore>().clone();
    let saving = Saving {
        send: Box::new(move |call| Box::pin(core.request(call))),
        unsaved: HashMap::new(),
        bases: Bases::new(),
        answers,
    };
    let writes = Writer::start(saving, CLOSED, cx);
    let entity = cx.new(|_| StudyLibrary {
        items,
        error,
        writes,
        reads: Reads::default(),
    });
    show_answers(&entity, answered, cx);
    crate::quit::on_quit(cx, {
        let entity = entity.clone();
        move |cx| Some(entity.read(cx).flushed())
    });
    // Another window saved a study: show the profile's.
    crate::core::on_profile_change(cx, &[ProfileArea::Studies], {
        let entity = entity.downgrade();
        move |cx| {
            entity.update(cx, |library, cx| library.reload(cx)).ok();
        }
    });
    cx.set_global(StudyHandle(entity));
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
            let study = result.studies.iter().find(|study| study.id == id)?;
            Some(study.chapters.first()?.id.as_str())
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

/// The library's commands (`Writer`), each sent with the moves its chapter was based on. A
/// chapter save the core refused is kept, the newest of each chapter, and tried again before the
/// app quits, unless a later save of the chapter went through or the board took the chapter
/// again as the library has it (`Write::Track`).
struct Saving {
    send: SendCommand,
    /// Chapter saves the core refused, with why: their moves are not in the profile (a chapter
    /// another window changed is not among them: the board keeps its moves as a copy).
    unsaved: HashMap<(String, String), (StudyCommand, String)>,
    bases: Bases,
    answers: mpsc::UnboundedSender<Result<StudyCommandResult, CoreError>>,
}

/// How the writer reaches the core: `AppCore::request`, or a test's stand-in.
type SendCommand = Box<
    dyn Fn(
            calls::StudyCommand,
        )
            -> std::pin::Pin<Box<dyn Future<Output = Result<StudyCommandResult, CoreError>> + Send>>
        + Send,
>;

impl Saving {
    /// Send `command`, note how it went, and answer its caller and the library.
    async fn run(
        &mut self,
        command: StudyCommand,
        reply: Option<oneshot::Sender<Result<StudyCommandResult, CoreError>>>,
    ) {
        let based_on = saved_chapter(&command).and_then(|key| self.bases.get(&key).cloned());
        let call = calls::StudyCommand {
            command: command.clone(),
            incremental: Some(true),
            based_on: based_on.map(kchess_contracts::check::ChapterPgn),
        };
        let result = (self.send)(call).await.inspect_err(|cause| {
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
                    // gpui-lints: allow(unlogged_failure) logged above, with its command
                    let cause = cause.message.clone();
                    self.unsaved.insert(key, (command.clone(), cause));
                }
                _ => {
                    self.unsaved.remove(&key);
                }
            }
        }
        // A removed study or chapter has nothing left to save.
        if result.is_ok()
            && let Some((study, chapter)) = removed_by(&command)
        {
            forget_removed(&mut self.unsaved, study, chapter);
            forget_removed(&mut self.bases, study, chapter);
        }
        if let Ok(result) = &result
            && let Some((key, pgn)) = written_chapter(&command, result)
        {
            self.bases.insert(key, pgn);
        }
        // The caller hears first, from here: the UI thread may not run while the app quits.
        if let Some(reply) = reply {
            let _ = reply.send(result.clone());
        }
        let _ = self.answers.unbounded_send(result);
    }
}

impl Store for Saving {
    type Write = Write;

    async fn write(&mut self, write: Write) {
        match write {
            Write::Command { command, reply } => self.run(command, reply).await,
            Write::Track { key, pgn } => {
                // The board shows the chapter as the library has it now: a save of it refused
                // earlier is not what it shows, and would overwrite it.
                self.unsaved.remove(&key);
                match pgn {
                    Some(pgn) => self.bases.insert(key, pgn),
                    None => self.bases.remove(&key),
                };
            }
        }
    }

    async fn flush(&mut self, retry: bool) -> Saved {
        if retry {
            let again: Vec<StudyCommand> = self
                .unsaved
                .drain()
                .map(|(_, (command, _))| command)
                .collect();
            for command in again {
                self.run(command, None).await;
            }
        }
        match self.unsaved.values().next() {
            Some((_, cause)) => Err(format!("A study could not be saved: {cause}")),
            None => Ok(()),
        }
    }
}

/// Show each answer: the studies it names, or why the core refused the command and what the
/// profile keeps.
fn show_answers(
    entity: &Entity<StudyLibrary>,
    mut answered: mpsc::UnboundedReceiver<Result<StudyCommandResult, CoreError>>,
    cx: &mut App,
) {
    let entity = entity.downgrade();
    cx.spawn(async move |cx| {
        while let Some(answer) = answered.next().await {
            entity
                .update(cx, |this, cx| {
                    match answer {
                        Ok(result) => {
                            this.apply(&result);
                            this.reads.edited();
                            this.error = None;
                        }
                        Err(cause) => {
                            this.error = Some(crate::loading::failure("studies", &cause));
                            this.reload(cx);
                        }
                    }
                    cx.notify();
                })
                .ok();
        }
    })
    .detach();
}

/// A study's chapters as select items: their ids, and their names numbered as the study orders them.
pub fn chapter_items(study: &Study) -> Vec<(String, String)> {
    study
        .chapters
        .iter()
        .enumerate()
        .map(|(index, chapter)| {
            (
                chapter.id.clone(),
                format!("{}. {}", index + 1, chapter.name),
            )
        })
        .collect()
}

/// The whole study as one PGN, a game per chapter.
pub fn document_pgn(study: &Study) -> String {
    let chapters: Vec<(&str, &str)> = study
        .chapters
        .iter()
        .map(|chapter| (chapter.name.as_str(), chapter.pgn.as_str()))
        .collect();
    kchess_domain::pgn::study_document_pgn(&chapters).unwrap_or_default()
}

impl StudyLibrary {
    pub fn items(&self) -> &[Study] {
        &self.items
    }

    pub fn study(&self, id: &str) -> Option<&Study> {
        self.items.iter().find(|study| study.id == id)
    }

    /// The board shows this chapter as the library has it: its saves are based on that.
    pub fn track(&self, study: &str, chapter: &str) {
        let pgn = self
            .study(study)
            .and_then(|study| study.chapter(chapter))
            .map(|chapter| chapter.pgn.clone());
        let key = (study.to_string(), chapter.to_string());
        self.writes.send(Write::Track { key, pgn });
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
            .find(|name| !self.items.iter().any(|study| study.name == *name))
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
        let sent = self.writes.send(Write::Command {
            command,
            reply: Some(reply),
        });
        async move {
            if !sent {
                return Err(CoreError::new(CLOSED));
            }
            answer.await.unwrap_or_else(|_| Err(CoreError::new(CLOSED)))
        }
    }

    /// Wait for every command sent so far, the refused chapter saves tried again: with why a
    /// chapter's moves are still not saved, if any.
    pub fn flushed(&self) -> BoxedSave {
        self.writes.flushed()
    }

    /// Show the studies an answer names: all of them, or the changed ones in the library's order.
    pub fn apply(&mut self, result: &StudyCommandResult) {
        let studies = result.studies.clone();
        let Some(order) = &result.order else {
            self.items = studies;
            return;
        };
        let mut by_id: HashMap<String, Study> = HashMap::new();
        for study in self.items.drain(..).chain(studies) {
            by_id.insert(study.id.clone(), study);
        }
        self.items = order.iter().filter_map(|id| by_id.remove(id)).collect();
    }

    /// Show the studies the profile keeps (after a command failed, or another window changed
    /// them): read once this window's commands are answered, and never over a newer answer.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let read = self.reads.start();
        // Not retried: a failure's own reload would retry it in a loop.
        let sent = self.writes.written();
        cx.spawn(async move |this, cx| {
            let _ = sent.await;
            let Ok(library) = this.update(cx, |_, cx| {
                cx.global::<AppCore>().request(calls::Library {})
            }) else {
                return;
            };
            if let Some(library) = library.await.warn_on_err() {
                let items = library.studies;
                this.update(cx, |this, cx| match this.reads.answer(read) {
                    Answer::Show => {
                        this.items = items;
                        cx.notify();
                    }
                    Answer::Again => this.reload(cx),
                    Answer::Superseded => {}
                })
                .ok();
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Bases, CLOSED, Saving, StudyLibrary, Write, chapter_items, forget_removed, saved_chapter,
        written_chapter,
    };
    use crate::writer::{Queued, Writer, write_in_order};
    use futures::channel::mpsc;
    use futures::executor::LocalPool;
    use futures::task::LocalSpawnExt as _;
    use kchess_contracts::CoreError;
    use kchess_contracts::types::{Study, StudyChapter, StudyCommand, StudyCommandResult};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// A library whose writer sends to a stand-in core: it refuses every command while `refuse`
    /// is set, and keeps the PGN of each save it takes. Run its writer on the pool.
    fn refusing_library(
        pool: &LocalPool,
        refuse: &Arc<Mutex<bool>>,
        saved: &Arc<Mutex<Vec<String>>>,
    ) -> StudyLibrary {
        let (refuse, saved) = (Arc::clone(refuse), Arc::clone(saved));
        let (answers, _answered) = mpsc::unbounded();
        let saving = Saving {
            send: Box::new(move |call| {
                let result = if *refuse.lock().unwrap() {
                    Err(CoreError::new("disk full"))
                } else {
                    if let StudyCommand::Save { pgn, .. } = &call.command {
                        saved.lock().unwrap().push(pgn.clone());
                    }
                    Ok(StudyCommandResult::default())
                };
                Box::pin(async move { result })
            }),
            unsaved: HashMap::new(),
            bases: Bases::new(),
            answers,
        };
        let (writes, queue) = Writer::queued(CLOSED);
        pool.spawner()
            .spawn_local(write_in_order(saving, queue))
            .unwrap();
        StudyLibrary {
            items: vec![study("a", "A", &[("c", "C", "1. d4 *")])],
            error: None,
            writes,
            reads: Default::default(),
        }
    }

    /// Regression: a chapter save the core refused was never sent again. Quitting reported it,
    /// and Retry in the quit dialog reported it again without sending it, so the moves could
    /// only be given up. Before quitting it is sent again, and Retry saves it once the core
    /// takes it.
    #[test]
    fn a_refused_chapter_save_is_sent_again_before_quitting() {
        let mut pool = LocalPool::new();
        let (refuse, saved) = (Arc::new(Mutex::new(true)), Arc::default());
        let library = refusing_library(&pool, &refuse, &saved);
        let save = library.command(chapter_save(Some("a"), Some("c"), "1. e4 *"));
        assert_eq!(pool.run_until(save), Err("disk full".to_string()));
        // A read of the profile waits for the writes without retrying them.
        assert!(pool.run_until(library.writes.written()).is_err());
        let quit = pool.run_until(library.flushed());
        assert!(quit.unwrap_err().contains("disk full"));

        *refuse.lock().unwrap() = false;
        assert_eq!(pool.run_until(library.flushed()), Ok(()));
        assert_eq!(*saved.lock().unwrap(), ["1. e4 *"]);
        // Nothing is left to send again.
        assert_eq!(pool.run_until(library.flushed()), Ok(()));
        assert_eq!(saved.lock().unwrap().len(), 1);
    }

    /// A refused chapter save is dropped once the board takes the chapter again as the library
    /// has it: sent at quit, it would overwrite the moves the board shows.
    #[test]
    fn a_refused_save_of_a_chapter_taken_again_is_not_sent() {
        let mut pool = LocalPool::new();
        let (refuse, saved) = (Arc::new(Mutex::new(true)), Arc::default());
        let library = refusing_library(&pool, &refuse, &saved);
        let save = library.command(chapter_save(Some("a"), Some("c"), "1. e4 *"));
        assert!(pool.run_until(save).is_err());
        library.track("a", "c");
        *refuse.lock().unwrap() = false;
        assert_eq!(pool.run_until(library.flushed()), Ok(()));
        assert!(saved.lock().unwrap().is_empty());
    }

    /// A study of these chapters (id, name, PGN).
    fn study(id: &str, name: &str, chapters: &[(&str, &str, &str)]) -> Study {
        let chapters: Vec<StudyChapter> = chapters
            .iter()
            .map(|(id, name, pgn)| StudyChapter {
                id: id.to_string(),
                name: name.to_string(),
                pgn: pgn.to_string(),
            })
            .collect();
        Study {
            id: id.into(),
            name: name.into(),
            pgn: chapters.first().map(|c| c.pgn.clone()).unwrap_or_default(),
            chapters,
            updated_at: 0.into(),
            cloud: None,
        }
    }

    fn answer(studies: Vec<Study>, order: Option<&[&str]>) -> StudyCommandResult {
        StudyCommandResult {
            studies,
            order: order.map(|order| order.iter().map(|id| id.to_string()).collect()),
            ..Default::default()
        }
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
        let (writes, _queue) = Writer::queued(CLOSED);
        let mut library = StudyLibrary {
            items: vec![study("a", "A", &[]), study("b", "B", &[])],
            error: None,
            writes,
            reads: Default::default(),
        };
        // An incremental answer: the changed study, and every id in order.
        library.apply(&answer(
            vec![study("c", "C", &[]), study("a", "A2", &[])],
            Some(&["c", "a", "b"]),
        ));
        let names: Vec<_> = library
            .items
            .iter()
            .map(|study| study.name.as_str())
            .collect();
        assert_eq!(names, ["C", "A2", "B"]);
        // A removal leaves the id out of the order.
        library.apply(&answer(vec![], Some(&["a"])));
        assert_eq!(library.items.len(), 1);
        // A whole answer replaces the list.
        library.apply(&answer(vec![study("z", "Z2", &[])], None));
        assert_eq!(library.items, [study("z", "Z2", &[])]);
        assert_eq!(library.fresh_name("Z"), "Z");
    }

    /// A change the core refuses is shown on the library and logged; one that works clears the
    /// failure.
    #[gpui_kit::test]
    fn a_refused_change_is_shown_and_logged(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.executor().allow_parking();
        let _core = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::tests::start_core(cx)
        });
        // The writer and the core answer on their own threads: step the app until `ready` holds.
        let wait = |cx: &mut gpui_kit::TestAppContext,
                    what: &str,
                    ready: &dyn Fn(&mut gpui_kit::App) -> bool| {
            for _ in 0..3000 {
                cx.run_until_parked();
                if cx.update(|cx| ready(cx)) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("timed out waiting for {what}");
        };
        cx.update(|cx| super::init(&kchess_contracts::types::Library::default(), cx));
        let shown =
            |cx: &mut gpui_kit::App| cx.global::<super::StudyHandle>().0.read(cx).error.clone();
        let before = crate::tests::logged_in("studies");
        cx.update(|cx| {
            let answer =
                cx.global::<super::StudyHandle>()
                    .0
                    .read(cx)
                    .command(StudyCommand::AddChapter {
                        id: "no-such-study".into(),
                        name: "Chapter".into(),
                    });
            cx.background_spawn(answer).detach();
        });
        wait(cx, "the failure shown", &|cx| shown(cx).is_some());
        assert!(crate::tests::logged_in("studies") > before, "logged");
        cx.update(|cx| {
            let answer = cx
                .global::<super::StudyHandle>()
                .0
                .read(cx)
                .command(chapter_save(None, None, "*"));
            cx.background_spawn(answer).detach();
        });
        wait(cx, "the failure cleared", &|cx| shown(cx).is_none());
    }

    #[test]
    fn chapters_are_numbered_in_order() {
        let study = study("s", "S", &[("a", "Main", "*"), ("b", "Side", "*")]);
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
        let (writes, mut queue) = Writer::queued(CLOSED);
        let library = StudyLibrary {
            items: vec![study("a", "A", &[("c", "C", "1. d4 *")])],
            error: None,
            writes,
            reads: Default::default(),
        };
        let save = chapter_save(Some("a"), Some("c"), "1. e4 e5 *");
        drop(library.command(save.clone()));
        library.track("a", "c");
        let Ok(Queued::Write(Write::Command { command, .. })) = queue.try_recv() else {
            panic!("the save comes first");
        };
        assert_eq!(command, save);
        let Ok(Queued::Write(Write::Track { key, pgn })) = queue.try_recv() else {
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
            written_chapter(
                &save,
                &StudyCommandResult {
                    id: Some("a".into()),
                    ..Default::default()
                }
            ),
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
        let made = StudyCommandResult {
            id: Some("n".into()),
            studies: vec![study("n", "N", &[("n1", "N", "1. d4 *")])],
            ..Default::default()
        };
        assert_eq!(
            written_chapter(&new, &made),
            Some((("n".to_string(), "n1".to_string()), "1. d4 *".to_string()))
        );
    }
}

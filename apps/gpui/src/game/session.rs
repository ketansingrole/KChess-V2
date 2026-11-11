//! A page's unfinished session (Play with computer, Over the board, the analysis board): read
//! off the UI thread to resume it, then saved shortly after each change, and before the app
//! quits, through the played games' ordered writer (`played.rs`).
//!
//! Nothing is saved before the session is read, so a slow read never has the page's fresh start
//! replace the saved game. A read that fails is said on the page and keeps the profile's session
//! as it is, until the player starts a game of their own (`SessionSaver::open`).

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::*;
use kchess_contracts::{CoreError, Level, calls};
use serde_json::{Value, json};

use crate::core::AppCore;
use crate::played::PlayedHandle;

/// Saves wait this long for more changes.
pub const SAVE_DELAY: Duration = Duration::from_millis(150);

/// A page that keeps an unfinished session.
pub trait Resumes: Sized + 'static {
    fn saver(&mut self) -> &mut SessionSaver<Self>;

    /// The session to save now.
    fn snapshot(&mut self, cx: &mut Context<Self>) -> Value;

    /// The saved sessions were read (each null when there is none); `None` when the read
    /// failed, which the saver says on the page.
    fn resume(&mut self, saved: Option<Sessions>, window: &mut Window, cx: &mut Context<Self>);
}

/// The sessions a page resumes from: its own, and its places in Game history.
pub struct Sessions(HashMap<String, Value>);

impl Sessions {
    /// The `kind` session, null when there is none.
    pub fn take(&mut self, kind: &str) -> Value {
        self.0.remove(kind).unwrap_or(Value::Null)
    }
}

/// Where the read of the saved session is.
#[derive(Clone, Debug, PartialEq)]
pub enum Resume {
    Reading,
    /// Read (or replaced by a game the player started): changes are saved.
    Ready,
    /// The read failed, for this reason: nothing is saved over the profile's session.
    Failed(String),
}

/// What the reads' answers mean for the page: the sessions to resume, or why there are none to
/// trust. A failed read is never taken for an empty profile.
pub fn resumed(reads: Vec<(String, Result<Value, CoreError>)>) -> Result<Sessions, String> {
    let mut sessions = HashMap::new();
    for (kind, read) in reads {
        let session = read.map_err(|cause| {
            format!("Your unfinished game could not be read: {}", cause.message)
        })?;
        sessions.insert(kind, session);
    }
    Ok(Sessions(sessions))
}

type Waiting<V> = Box<dyn FnOnce(&mut V, &mut Window, &mut Context<V>)>;

pub struct SessionSaver<V> {
    kind: &'static str,
    state: Resume,
    save: Option<Task<()>>,
    /// Work that needs the saved session, run once it is read.
    waiting: Vec<Waiting<V>>,
}

impl<V: Resumes> SessionSaver<V> {
    /// Read the `kind` session for page `V`, and the `also` sessions it resumes with (its places
    /// in Game history); the page resumes them once read. Its changes are saved as `kind` from
    /// then on, and before the app quits.
    pub fn start(
        kind: &'static str,
        also: &[String],
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        let core = cx.global::<AppCore>();
        let reads: Vec<_> = std::iter::once(kind.to_string())
            .chain(also.iter().cloned())
            .map(|kind| {
                let read = core.request(calls::Session { kind: json!(kind) });
                async move { (kind, read.await) }
            })
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            let read = resumed(futures::future::join_all(reads).await);
            let _ = this.update_in(cx, |this, window, cx| {
                let saved = match read {
                    Ok(saved) => {
                        this.saver().state = Resume::Ready;
                        Some(saved)
                    }
                    Err(cause) => {
                        crate::log::write(Level::Warn, "library", &format!("{kind}: {cause}"));
                        this.saver().state = Resume::Failed(cause);
                        None
                    }
                };
                this.resume(saved, window, cx);
                let waiting = std::mem::take(&mut this.saver().waiting);
                for work in waiting {
                    work(this, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        let page = cx.entity().downgrade();
        crate::quit::on_quit(cx, move |cx| {
            let page = page.upgrade()?;
            page.update(cx, |page, cx| Self::write_now(page, cx));
            Some(cx.global::<PlayedHandle>().0.read(cx).flushed())
        });
        SessionSaver {
            kind,
            state: Resume::Reading,
            save: None,
            waiting: Vec::new(),
        }
    }

    /// The saved session has been read, and changes are saved.
    pub fn loaded(&self) -> bool {
        self.state == Resume::Ready
    }

    /// Back to before the read, as on a profile slow to answer.
    #[cfg(test)]
    pub fn forget(&mut self) {
        self.state = Resume::Reading;
    }

    /// Why the saved session could not be read, while that stands.
    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            Resume::Failed(cause) => Some(cause),
            _ => None,
        }
    }

    /// What the page says of its saving: a session it could not read, a save that failed.
    pub fn problems(&self, cx: &App) -> Vec<SharedString> {
        let saving = cx
            .global::<PlayedHandle>()
            .0
            .read(cx)
            .error
            .as_ref()
            .map(|cause| SharedString::from(format!("Automatic saving: {cause}")));
        self.failure()
            .map(|cause| SharedString::from(cause.to_string()))
            .into_iter()
            .chain(saving)
            .collect()
    }

    /// The player starts a game of their own: after a failed read, it is saved from now on.
    pub fn open(&mut self) {
        if matches!(self.state, Resume::Failed(_)) {
            self.state = Resume::Ready;
        }
    }

    /// Run `work` once the saved session is read: now, if it has been.
    pub fn when_read(
        this: &mut V,
        work: impl FnOnce(&mut V, &mut Window, &mut Context<V>) + 'static,
        window: &mut Window,
        cx: &mut Context<V>,
    ) {
        if this.saver().state == Resume::Reading {
            this.saver().waiting.push(Box::new(work));
        } else {
            work(this, window, cx);
        }
    }

    /// The page changed: save it shortly.
    pub fn changed(this: &mut V, cx: &mut Context<V>) {
        if !this.saver().loaded() {
            return;
        }
        this.saver().save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| Self::write_now(this, cx));
        }));
    }

    /// Save the page now, after the writes sent before it.
    pub fn write_now(this: &mut V, cx: &mut Context<V>) {
        if !this.saver().loaded() {
            return;
        }
        this.saver().save = None;
        let kind = this.saver().kind;
        let snapshot = this.snapshot(cx);
        cx.global::<PlayedHandle>()
            .0
            .read(cx)
            .save_session(kind.to_string(), snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::resumed;
    use kchess_contracts::CoreError;
    use serde_json::{Value, json};

    /// A session that could not be read is not an empty profile: the page says so, rather than
    /// starting a new game over the saved one.
    #[test]
    fn a_failed_read_is_not_a_fresh_start() {
        let mut read = resumed(vec![
            ("local".into(), Ok(json!({ "moves": ["e2e4"] }))),
            ("archive:board".into(), Ok(Value::Null)),
        ])
        .expect("both read");
        assert_eq!(read.take("local"), json!({ "moves": ["e2e4"] }));
        assert_eq!(read.take("archive:board"), Value::Null);
        assert_eq!(read.take("archive:clock"), Value::Null);
        let failed = resumed(vec![
            ("local".into(), Ok(Value::Null)),
            (
                "archive:board".into(),
                Err(CoreError::new("the profile is locked")),
            ),
        ]);
        assert_eq!(
            failed.err().as_deref(),
            Some("Your unfinished game could not be read: the profile is locked")
        );
    }
}

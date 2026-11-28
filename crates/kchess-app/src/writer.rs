//! One service's writes to the core, made one at a time in the order they were sent, by a task
//! off the UI thread so they also finish while the app quits. View preferences, Settings, the
//! games played and the study library each say what a write is and how its answer is kept
//! (`Store`); this keeps their order and what a flush is, the same for all of them: done once
//! every write sent before it is answered, and before the app quits (`flushed`) once the refused
//! ones were tried again, with why one still failed.

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::App;

use crate::quit::{BoxedSave, Saved};

/// What a service's writes are, and how it keeps their answers. Runs on the writer's task.
pub trait Store: Send + 'static {
    type Write: Send + 'static;

    /// Send `write` to the core, and keep how it went: a refused write is kept to try again,
    /// until a later write replaces it.
    fn write(&mut self, write: Self::Write) -> impl Future<Output = ()> + Send + '_;

    /// Every write before this is answered. Send the refused ones again first when `retry`;
    /// then why one is still unsaved, if any is.
    fn flush(&mut self, retry: bool) -> impl Future<Output = Saved> + Send + '_;

    /// Whether `newer`, queued behind `older` with no flush between them, leaves nothing of
    /// `older` worth writing: a game saved again before its last save went out. Only the newer
    /// is sent then.
    fn replaces(_newer: &Self::Write, _older: &Self::Write) -> bool {
        false
    }
}

/// What the writer's queue carries.
pub enum Queued<W> {
    Write(W),
    Flush {
        retry: bool,
        done: oneshot::Sender<Saved>,
    },
}

/// Sends a service's writes to its writer's task.
pub struct Writer<W> {
    queue: mpsc::UnboundedSender<Queued<W>>,
    /// What a flush answers once the task has stopped.
    closed: &'static str,
}

impl<W> Clone for Writer<W> {
    fn clone(&self) -> Self {
        Writer {
            queue: self.queue.clone(),
            closed: self.closed,
        }
    }
}

impl<W: Send + 'static> Writer<W> {
    /// Start writing for `store` on a background task; `closed` is what a flush answers once
    /// the task has stopped.
    pub fn start<S: Store<Write = W>>(store: S, closed: &'static str, cx: &App) -> Self {
        let (writer, queue) = Self::queued(closed);
        cx.background_executor()
            .spawn(write_in_order(store, queue))
            .detach();
        writer
    }

    /// A writer and its queue, nothing reading it yet: for `write_in_order`, or a test that
    /// reads the queue itself. Dropping the queue closes the writer.
    pub fn queued(closed: &'static str) -> (Self, mpsc::UnboundedReceiver<Queued<W>>) {
        let (queue, receiver) = mpsc::unbounded();
        (Writer { queue, closed }, receiver)
    }

    /// Queue `write` after every write sent before it; false once the writer has stopped.
    pub fn send(&self, write: W) -> bool {
        self.queue.unbounded_send(Queued::Write(write)).is_ok()
    }

    /// Done once every write sent so far is answered, the refused ones tried again: with why
    /// one is still unsaved. What the app waits for before it quits.
    pub fn flushed(&self) -> BoxedSave {
        self.flush(true)
    }

    /// Done once every write sent so far is answered, refused ones left as they are: what a
    /// read of the profile waits for, so it never shows the profile from before this app's
    /// writes. A failure's own reload waits for this, and so never retries in a loop.
    pub fn written(&self) -> BoxedSave {
        self.flush(false)
    }

    fn flush(&self, retry: bool) -> BoxedSave {
        let (done, answer) = oneshot::channel();
        let sent = self
            .queue
            .unbounded_send(Queued::Flush { retry, done })
            .is_ok();
        let closed = self.closed;
        Box::pin(async move {
            if !sent {
                return Err(closed.to_string());
            }
            answer.await.unwrap_or_else(|_| Err(closed.to_string()))
        })
    }
}

/// Write each queued write in turn, until every `Writer` is dropped. A write that one queued
/// behind it replaces before the next flush is left out (`Store::replaces`).
pub async fn write_in_order<S: Store>(
    mut store: S,
    mut queue: mpsc::UnboundedReceiver<Queued<S::Write>>,
) {
    while let Some(first) = queue.next().await {
        let mut ready = vec![first];
        while let Ok(Some(next)) = queue.try_next() {
            ready.push(next);
        }
        let replaced = replaced::<S>(&ready);
        for (queued, replaced) in ready.into_iter().zip(replaced) {
            match queued {
                Queued::Write(_) if replaced => {}
                Queued::Write(write) => store.write(write).await,
                Queued::Flush { retry, done } => {
                    let saved = store.flush(retry).await;
                    // The caller stopped waiting (the app quit without it): nothing to tell.
                    let _ = done.send(saved);
                }
            }
        }
    }
}

/// Which of the queued writes a later one replaces before the next flush.
fn replaced<S: Store>(ready: &[Queued<S::Write>]) -> Vec<bool> {
    ready
        .iter()
        .enumerate()
        .map(|(index, queued)| {
            let Queued::Write(older) = queued else {
                return false;
            };
            ready[index + 1..]
                .iter()
                .map_while(|later| match later {
                    Queued::Write(newer) => Some(newer),
                    Queued::Flush { .. } => None,
                })
                .any(|newer| S::replaces(newer, older))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Store, Writer, write_in_order};
    use crate::quit::Saved;
    use futures::executor::LocalPool;
    use futures::task::LocalSpawnExt as _;
    use std::sync::{Arc, Mutex};

    /// Keeps what it was sent, refusing every write while `refuse` is set; retried at a flush.
    #[derive(Default)]
    struct Kept {
        refuse: Arc<Mutex<bool>>,
        stored: Arc<Mutex<Vec<u32>>>,
        refused: Vec<u32>,
    }

    impl Kept {
        fn send(&mut self, write: u32) {
            if *self.refuse.lock().unwrap() {
                self.refused.push(write);
            } else {
                self.stored.lock().unwrap().push(write);
            }
        }
    }

    impl Store for Kept {
        type Write = u32;

        async fn write(&mut self, write: u32) {
            self.send(write);
        }

        async fn flush(&mut self, retry: bool) -> Saved {
            if retry {
                for write in std::mem::take(&mut self.refused) {
                    self.send(write);
                }
            }
            match self.refused.first() {
                Some(write) => Err(format!("{write} refused")),
                None => Ok(()),
            }
        }
    }

    /// Writes land in the order they were sent; a flush answers after every write before it,
    /// retries the refused ones only before quitting (`flushed`), and a stopped writer says so.
    #[test]
    fn writes_land_in_order_and_a_flush_waits_for_them() {
        let store = Kept::default();
        let (refuse, stored) = (Arc::clone(&store.refuse), Arc::clone(&store.stored));
        let (writer, queue) = Writer::queued("closed");
        let mut pool = LocalPool::new();
        pool.spawner()
            .spawn_local(write_in_order(store, queue))
            .unwrap();
        for write in 0..50 {
            assert!(writer.send(write));
        }
        assert_eq!(pool.run_until(writer.written()), Ok(()));
        assert_eq!(*stored.lock().unwrap(), (0..50).collect::<Vec<_>>());

        *refuse.lock().unwrap() = true;
        writer.send(50);
        assert_eq!(pool.run_until(writer.written()), Err("50 refused".into()));
        *refuse.lock().unwrap() = false;
        // A read's wait leaves the refused write as it is; the quit's tries it again.
        assert_eq!(pool.run_until(writer.written()), Err("50 refused".into()));
        assert_eq!(pool.run_until(writer.flushed()), Ok(()));
        assert_eq!(stored.lock().unwrap().last(), Some(&50));

        let closed: Writer<u32> = Writer::queued("The writer stopped").0;
        assert!(!closed.send(1));
        assert_eq!(
            pool.run_until(closed.flushed()),
            Err("The writer stopped".into())
        );
    }

    /// Keeps what it was sent; a write replaces a queued one of the same game.
    struct Latest(Arc<Mutex<Vec<(char, u32)>>>);

    impl Store for Latest {
        type Write = (char, u32);

        async fn write(&mut self, write: (char, u32)) {
            self.0.lock().unwrap().push(write);
        }

        async fn flush(&mut self, _: bool) -> Saved {
            Ok(())
        }

        fn replaces(newer: &(char, u32), older: &(char, u32)) -> bool {
            newer.0 == older.0
        }
    }

    /// Saves of one game queued while the one before them is still out reach the core as the
    /// newest alone, in the place of the last; a flush between two keeps both.
    #[test]
    fn a_write_replaced_while_queued_is_left_out() {
        let stored = Arc::new(Mutex::new(Vec::new()));
        let (writer, queue) = Writer::queued("closed");
        for write in [('a', 1), ('b', 1), ('a', 2), ('a', 3)] {
            writer.send(write);
        }
        let first = writer.written();
        writer.send(('a', 4));
        let mut pool = LocalPool::new();
        pool.spawner()
            .spawn_local(write_in_order(Latest(Arc::clone(&stored)), queue))
            .unwrap();
        assert_eq!(pool.run_until(first), Ok(()));
        assert_eq!(pool.run_until(writer.written()), Ok(()));
        assert_eq!(*stored.lock().unwrap(), [('b', 1), ('a', 3), ('a', 4)],);
    }
}

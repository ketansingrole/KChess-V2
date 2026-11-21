//! The core in a frontend's own process (`Mode::Embedded`): what the programs that host the core
//! (the app, the command line) hand `CoreHandle::open` as `Options::embed`, so the handle and the
//! crates a frontend is built from never build the core themselves.

use std::sync::{Arc, Weak};

use kchess_client::{
    BoxFuture, ClientEvent, EMBEDDED_CONNECTION, Embed, EmbeddedCore, Embedding, Log,
};
use kchess_contracts::{CoreError, CoreEvent, HostReply, Level};
use kchess_core::connection::{APP, ClientInfo};
use kchess_core::{Core, Host};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// Opens a core in this process for the frontend that asks.
pub fn embed() -> Embed {
    Arc::new(|embedding| open(embedding).map(|core| core as Arc<dyn EmbeddedCore>))
}

/// The core's host in an embedded core: its events go to the frontend's channel.
struct Forward {
    events: mpsc::UnboundedSender<ClientEvent>,
    log: Log,
}

impl Host for Forward {
    fn log(&self, level: Level, scope: &str, message: &str) {
        (self.log)(level, scope, message);
    }

    fn emit(&self, event: CoreEvent) {
        // The frontend has gone when the receiver is dropped; late events have nowhere to go.
        let _ = self.events.send(ClientEvent::Event(event));
    }
}

/// A core embedded by one frontend, whose connection is the core's own app (`APP`).
struct Embedded {
    core: Arc<Core>,
    /// The frontend's calls in the order it made them, for `start_in_order`.
    calls: mpsc::UnboundedSender<Call>,
}

/// A call waiting for its turn to start, and where its answer goes.
struct Call {
    method: String,
    args: Vec<Value>,
    answer: oneshot::Sender<Result<Value, CoreError>>,
}

/// What a call whose core has gone answers.
const STOPPED: &str = "The KChess core stopped.";

/// Starts each call in the order the frontend made it, then lets it run on beside the others. A
/// call spawned straight from the frontend started whenever a worker picked it up, so a stop
/// made before a start could stop nothing and leave the start running.
async fn start_in_order(core: Weak<Core>, mut calls: mpsc::UnboundedReceiver<Call>) {
    while let Some(Call {
        method,
        args,
        mut answer,
    }) = calls.recv().await
    {
        // Dropped before its turn: nobody waits for it.
        if answer.is_closed() {
            continue;
        }
        let Some(core) = core.upgrade() else {
            let _ = answer.send(Err(CoreError::new(STOPPED)));
            continue;
        };
        let call = kchess_core::tasks::started(async move { core.call(&method, args).await });
        tokio::spawn(async move {
            // Its caller dropped it: the core stops the work, not only the reply.
            let answered = tokio::select! {
                biased;
                answered = call => Some(answered),
                () = answer.closed() => None,
            };
            if let Some(answered) = answered {
                let _ = answer.send(answered);
            }
        });
    }
}

fn open(embedding: Embedding) -> Result<Arc<Embedded>, String> {
    let Embedding {
        config,
        hello,
        events,
        log,
    } = embedding;
    let core = Core::open(config, Arc::new(Forward { events, log })).map_err(|e| e.message)?;
    core.embedded_by(ClientInfo {
        kind: hello.kind,
        release: kchess_client::protocol::release(),
        capabilities: hello.capabilities,
    });
    let core = Arc::new(core);
    let (calls, queued) = mpsc::unbounded_channel();
    tokio::spawn(start_in_order(Arc::downgrade(&core), queued));
    Ok(Arc::new(Embedded { core, calls }))
}

impl EmbeddedCore for Embedded {
    fn call(
        self: Arc<Self>,
        method: String,
        args: Vec<Value>,
    ) -> BoxFuture<Result<Value, CoreError>> {
        let (answer, answered) = oneshot::channel();
        let queued = self
            .calls
            .send(Call {
                method,
                args,
                answer,
            })
            .is_ok();
        Box::pin(async move {
            if !queued {
                return Err(CoreError::new(STOPPED));
            }
            answered
                .await
                .unwrap_or_else(|_| Err(CoreError::new(STOPPED)))
        })
    }

    fn host_reply(&self, id: u64, reply: HostReply) -> Result<(), CoreError> {
        self.core.host_reply(id, reply)
    }

    fn focus(&self, focused: bool) {
        self.core.focus(APP, focused);
    }

    fn suspend(&self) {
        self.core.suspend_connection(APP);
    }

    fn close(self: Arc<Self>) -> BoxFuture<()> {
        Box::pin(async move { self.core.close().await })
    }
}

// The handle names the embedding frontend's connection without the core.
const _: () = assert!(EMBEDDED_CONNECTION == APP);

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use kchess_client::{Answer, CoreHandle, Hello, Mode, Options};
    use kchess_contracts::{Capability, HostRequest};
    use kchess_core::Config;
    use serde_json::json;

    use super::*;

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn quiet() -> Log {
        Arc::new(|_, _, _| {})
    }

    /// The core a test's handle opened, kept where the test can reach it.
    type Opened = Arc<Mutex<Option<Arc<Embedded>>>>;

    fn embedded(profile: &std::path::Path, answer: Option<Answer>) -> (Options, Opened) {
        embedded_as(
            profile,
            answer,
            vec![Capability::Browser, Capability::Focus],
        )
    }

    fn embedded_as(
        profile: &std::path::Path,
        answer: Option<Answer>,
        capabilities: Vec<Capability>,
    ) -> (Options, Opened) {
        let opened: Opened = Arc::default();
        let embed: Embed = {
            let opened = Arc::clone(&opened);
            Arc::new(move |embedding| {
                let core = open(embedding)?;
                *lock(&opened) = Some(Arc::clone(&core));
                Ok(core as Arc<dyn EmbeddedCore>)
            })
        };
        let options = Options {
            config: Config::new(profile.to_path_buf()),
            hello: Hello {
                kind: "test".into(),
                capabilities,
            },
            mode: Mode::Embedded,
            embed,
            reconnect: false,
            log: quiet(),
            answer,
        };
        (options, opened)
    }

    fn inner(opened: &Opened) -> Arc<Embedded> {
        lock(opened).clone().expect("the embedded core")
    }

    /// A frontend that answers where requests arrive is never handed one, and its answer
    /// reaches the core.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_requests_are_answered_where_they_arrive() {
        let profile = tempfile::tempdir().expect("profile");
        let asked: Arc<Mutex<Vec<HostRequest>>> = Arc::default();
        let answer: Answer = {
            let asked = Arc::clone(&asked);
            Arc::new(move |request| {
                lock(&asked).push(request);
                HostReply::Value(json!(true))
            })
        };
        let (options, opened) = embedded(profile.path(), Some(answer));
        let (core, mut events) = CoreHandle::open(options).await.expect("opens");
        let answer = inner(&opened)
            .core
            .capabilities()
            .request(HostRequest::Focus)
            .await
            .expect("answered");
        assert_eq!(answer, json!(true));
        assert_eq!(lock(&asked).clone(), [HostRequest::Focus]);
        while let Ok(event) = events.try_recv() {
            assert!(
                !matches!(event, ClientEvent::Event(CoreEvent::HostRequest(_))),
                "{event:?}"
            );
        }
        core.close().await;
    }

    /// A frontend's calls take effect in the order it made them, though each runs on whichever
    /// worker takes it: of two writes made one after the other, the first could land last.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn calls_take_effect_in_the_order_they_were_made() {
        let profile = tempfile::tempdir().expect("profile");
        let (options, _) = embedded(profile.path(), None);
        let (core, _events) = CoreHandle::open(options).await.expect("opens");
        let writes: Vec<_> = (0..200)
            .map(|n| {
                tokio::spawn(core.call("setViewPreference", vec![json!("kchess:order"), json!(n)]))
            })
            .collect();
        for write in writes {
            write.await.expect("ran").expect("written");
        }
        let preferences = core.call("viewPreferences", vec![]).await.expect("read");
        assert_eq!(preferences["kchess:order"], json!(199));
        core.close().await;
    }

    /// A call its frontend drops stops in the core: the sign-in it began stops listening.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dropped_call_stops_its_work() {
        let profile = tempfile::tempdir().expect("profile");
        let (options, _) = embedded(profile.path(), None);
        let (core, mut events) = CoreHandle::open(options).await.expect("opens");
        let signing_in = tokio::spawn(core.call("connectLichess", vec![]));
        let page = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Some(ClientEvent::Event(CoreEvent::HostRequest(request)))
                        if matches!(request.request, HostRequest::OpenExternal(_)) =>
                    {
                        break request;
                    }
                    Some(_) => {}
                    None => panic!("the events ended"),
                }
            }
        })
        .await
        .expect("the sign-in page opens");
        let HostRequest::OpenExternal(page) = page.request else {
            unreachable!()
        };
        let port: u16 = page
            .split("redirect_uri=http%3A%2F%2F127.0.0.1%3A")
            .nth(1)
            .and_then(|rest| rest.split('%').next())
            .and_then(|port| port.parse().ok())
            .unwrap_or_else(|| panic!("a loopback redirect in {page}"));
        signing_in.abort();
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(
            stopped.is_ok(),
            "the sign-in still listens after its call was dropped"
        );
        core.close().await;
    }

    /// An embedded core's log lines reach the frontend's log.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_embedded_core_logs_through_its_frontend() {
        let profile = tempfile::tempdir().expect("profile");
        let lines: Arc<Mutex<Vec<String>>> = Arc::default();
        let (mut options, _) = embedded(profile.path(), None);
        options.log = {
            let lines = Arc::clone(&lines);
            Arc::new(move |_, scope, message| lock(&lines).push(format!("{scope}: {message}")))
        };
        let (core, _events) = CoreHandle::open(options).await.expect("opens");
        let refused = core.call("library", vec![json!("extra")]).await;
        assert!(refused.is_err());
        let lines = lock(&lines).clone();
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("core: Invalid method arguments: library")),
            "{lines:?}"
        );
        core.close().await;
    }

    /// Without an answer, a host request is an event, and `reply` answers it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_requests_are_events_without_an_answer() {
        let profile = tempfile::tempdir().expect("profile");
        let (options, opened) = embedded(profile.path(), None);
        let (core, mut events) = CoreHandle::open(options).await.expect("opens");
        let capabilities = inner(&opened).core.capabilities();
        let mut asking =
            tokio::spawn(async move { capabilities.request(HostRequest::Focus).await });
        let answer = loop {
            tokio::select! {
                asked = &mut asking => break asked.expect("asked").expect("answered"),
                Some(event) = events.recv() => {
                    if let ClientEvent::Event(CoreEvent::HostRequest(ask)) = event {
                        assert_eq!(ask.request, HostRequest::Focus);
                        core.reply(ask.id, HostReply::Value(json!(true)))
                            .await
                            .expect("replied");
                    }
                }
            }
        };
        assert_eq!(answer, json!(true));
        core.close().await;
        let refused = core.call("library", Vec::new()).await;
        assert!(refused.is_err(), "a closed core answers nothing");
    }

    /// 2026-11-11: an embedded core sent every host request to its frontend, whatever the
    /// frontend declared, so an embedded command line was asked to focus a window it does not
    /// have. It hears only what it said it can answer, as the shared core's clients do.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_embedded_core_asks_only_what_its_frontend_can_answer() {
        let profile = tempfile::tempdir().expect("profile");
        let (options, opened) = embedded_as(profile.path(), None, vec![Capability::Browser]);
        let (core, _events) = CoreHandle::open(options).await.expect("opens");
        let embedded = inner(&opened);
        let connections = embedded.core.connections();
        let info = connections.info(APP).expect("the frontend");
        assert_eq!(info.kind, "test");
        assert_eq!(info.capabilities, [Capability::Browser]);
        assert_eq!(connections.route(Capability::Focus, Some(APP)), None);
        assert_eq!(connections.alert_owner(), None);
        assert_eq!(connections.route(Capability::Browser, Some(APP)), Some(APP));
        core.close().await;
    }
}

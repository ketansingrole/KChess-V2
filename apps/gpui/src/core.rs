//! Opening the profile's core (`kchess_app::core` holds it, `docs/shared-core.md`) and answering
//! its host requests on the UI thread, which owns the window and the browser (`answer_host`).

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Global};
use kchess_contracts::{Capability, HostAsk, HostReply, HostRequest, Level};
use kchess_core::TestSwitches;
use kchess_service::{ConnectError, CoreHandle, Hello, Layout, Mode};

pub use kchess_app::core::*;

/// The test switches the app was started with (`Config::test`), when it was.
pub struct TestSwitchesGlobal(pub Arc<TestSwitches>);

impl Global for TestSwitchesGlobal {}

/// The test switches the app was started with, when it was.
pub fn test_switches(cx: &App) -> Option<&TestSwitches> {
    cx.try_global::<TestSwitchesGlobal>().map(|test| &*test.0)
}

/// What the person reads when another KChess release has this profile open
/// (`docs/shared-core.md`, "Version check").
fn refusal(error: &ConnectError) -> String {
    let ConnectError::Refused { clients, .. } = error else {
        return error.to_string();
    };
    let ours = kchess_service::protocol::release();
    let Some(other) = clients.first() else {
        return error.to_string();
    };
    let app = match other.kind.as_str() {
        "gpui" => "GPUI",
        kind => kind,
    };
    if kchess_delta::Version::parse(&other.release) > kchess_delta::Version::parse(&ours) {
        format!(
            "KChess {} ({app}) is open on this profile. Update this app to {} to use them together.",
            other.release, other.release
        )
    } else {
        format!(
            "KChess {} ({app}) is open on this profile. Update it to {ours}, or quit it, to open this app.",
            other.release
        )
    }
}

/// Opens the profile's core in `mode`; a shared one that stops is started again, and its
/// users start again what they had started there on `ClientEvent::Reconnected`.
fn open(
    config: kchess_core::Config,
    mode: Mode,
) -> Result<
    (
        CoreBridge,
        tokio::sync::mpsc::UnboundedReceiver<ClientEvent>,
    ),
    String,
> {
    let runtime =
        kchess_service::core_runtime().map_err(|e| format!("The core could not start: {e}"))?;
    let options = kchess_service::Options {
        config,
        hello: Hello {
            kind: "gpui".into(),
            capabilities: vec![Capability::Browser, Capability::Focus],
        },
        mode,
        reconnect: true,
        log: Arc::new(crate::log::write),
        answer: None,
    };
    let (handle, events) = runtime
        .block_on(CoreHandle::open(options))
        .map_err(|error| refusal(&error))?;
    Ok((CoreBridge::new(runtime, handle), events))
}

/// Opens the core for `data_dir` and delivers its events to `CoreEvents` on the UI thread.
pub fn start(data_dir: PathBuf, cx: &mut App) -> Result<(), String> {
    let layout = Layout::current();
    // Tokens are sealed by the core, so this app is never asked to seal or open one.
    let config = layout.config(&data_dir, true);
    let test = config.test.clone().map(Arc::new);
    let (bridge, mut receiver) = open(config, layout.mode(false)?)?;
    if let Some(test) = test {
        cx.set_global(TestSwitchesGlobal(test));
    }
    let core = AppCore {
        bridge: Arc::new(bridge),
        events: cx.new(|_| CoreEvents),
    };
    cx.set_global(core.clone());
    cx.spawn(async move |cx| {
        while let Some(event) = receiver.recv().await {
            if let ClientEvent::Event(CoreEvent::HostRequest(ask)) = event {
                cx.update(|cx| answer_host(ask, cx));
                continue;
            }
            if let ClientEvent::Event(CoreEvent::AlertsOwner(owner)) = &event {
                let owner = owner.connection;
                cx.update(|cx| cx.set_global(AlertsOwnerGlobal(owner)));
            }
            core.events.update(cx, |_, cx| cx.emit(event));
        }
    })
    .detach();
    Ok(())
}

/// The answer to a host request, given on the UI thread.
fn answer_host(ask: HostAsk, cx: &mut App) {
    let reply = match ask.request {
        // With a local stand-in for Lichess (the test switches), the page a sign-in opens is
        // written to the profile for the test to follow, instead of the person's browser.
        HostRequest::OpenExternal(url)
            if test_switches(cx).is_some_and(|test| test.lichess_origin.is_some()) =>
        {
            let line = format!("{url}\n");
            let written = crate::profile::data_dir().and_then(|dir| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join("opened-urls.txt"))
                    .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))
            });
            match written {
                Ok(()) => HostReply::done(),
                Err(cause) => HostReply::Error(cause.to_string()),
            }
        }
        HostRequest::OpenExternal(url) if kchess_host::opens_in_browser(&url) => {
            cx.open_url(&url);
            HostReply::done()
        }
        HostRequest::OpenExternal(_) => HostReply::Error(kchess_host::NOT_A_WEB_PAGE.into()),
        HostRequest::Focus => {
            cx.activate(true);
            HostReply::done()
        }
        request => HostReply::Error(format!("Unknown host capability {}", request.kind())),
    };
    let delivery = cx.global::<AppCore>().bridge.reply(ask.id, reply);
    cx.background_executor()
        .spawn(async move {
            if let Err(cause) = delivery.await {
                crate::log::write(
                    Level::Debug,
                    "native-core",
                    &format!("Host reply was not delivered: {}", cause.message),
                );
            }
        })
        .detach();
}

//! The profile's core, as the CLI reaches it (`kchess_service::CoreHandle`): the shared core
//! service the app also uses (started when none runs), or, with `--embedded`, a core in this
//! process. Its host requests are answered here as they arrive (`answer`), and its errors are
//! told with the command line's own fix (`told`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kchess_contracts::calls::CoreCall;
use kchess_contracts::{Capability, CoreError, ErrorKind, HostReply, HostRequest, Level};
use kchess_service::{CoreHandle, Hello, Layout, Mode};
use serde_json::Value;
use tokio::sync::mpsc;

pub use kchess_contracts::CoreEvent;
pub use kchess_service::ClientEvent;

/// How the login page reaches the person.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Browser {
    Open,
    Print,
}

#[cfg_attr(test, derive(Debug, PartialEq))]
pub struct Options {
    pub data_dir: PathBuf,
    pub browser: Browser,
    pub credentials: bool,
    pub embedded: bool,
}

pub struct CoreClient {
    handle: CoreHandle,
}

/// The app's profile, which the CLI shares: `KCHESS_USER_DATA_DIR`, else the app's own folder.
pub fn default_data_dir() -> PathBuf {
    Layout::current().profile()
}

/// Diagnostics go to stderr, so JSON on stdout stays usable in pipelines.
fn log(level: Level, scope: &str, message: &str) {
    if matches!(level, Level::Warn | Level::Error) {
        eprintln!("[{scope}] {}", kchess_core::redact::redact(message));
    }
}

impl CoreClient {
    /// Opens the profile's core and starts delivering its events.
    pub async fn open(
        options: &Options,
    ) -> Result<(CoreClient, mpsc::UnboundedReceiver<ClientEvent>), String> {
        std::fs::create_dir_all(&options.data_dir)
            .map_err(|cause| format!("The profile folder is unavailable: {cause}"))?;
        let layout = Layout::current();
        let mode = match layout.service_binary() {
            Some(binary) if !options.embedded => Mode::Service { binary },
            _ => Mode::Embedded,
        };
        let browser = options.browser;
        let (handle, events) = CoreHandle::open(kchess_service::Options {
            config: layout.config(&options.data_dir, options.credentials),
            hello: Hello {
                kind: "cli".into(),
                capabilities: vec![Capability::Browser],
            },
            mode,
            reconnect: false,
            log: Arc::new(log),
            answer: Some(Arc::new(move |request| {
                answer(request, browser, open_browser)
            })),
        })
        .await
        .map_err(|error| error.to_string())?;
        Ok((CoreClient { handle }, events))
    }

    /// A `CoreApi` method by name: only for `call`, which takes any method; every other command
    /// makes a typed call (`request`).
    pub async fn call_by_name(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        self.handle
            .call(method, args)
            .await
            .map_err(|cause| told(&cause))
    }

    /// A typed core call (`kchess_contracts::calls`): its method with its arguments, and its
    /// answer read in its shape.
    pub async fn request<C: CoreCall>(&self, call: C) -> Result<C::Answer, String> {
        self.handle
            .request(call)
            .await
            .map_err(|cause| told(&cause))
    }

    pub async fn close(&self) {
        self.handle.close().await;
    }
}

/// A core error as the command line tells it: the core's advice names the app's Settings, which
/// share this profile; the command line's own way to the same fix follows it.
fn told(error: &CoreError) -> String {
    let fix = match error.kind {
        Some(ErrorKind::Login) => "From the command line: `kchess-cli login`.",
        Some(ErrorKind::NoEngine) => "From the command line: --engine <path>.",
        _ => return error.message.clone(),
    };
    format!("{} {fix}", error.message)
}

/// The answer to a host request, opening a web page with `open` when `browser` says so and
/// telling the person on stderr.
fn answer(
    request: HostRequest,
    browser: Browser,
    open: impl FnOnce(&str) -> std::io::Result<()>,
) -> HostReply {
    match request {
        HostRequest::OpenExternal(url) if kchess_core::capabilities::opens_in_browser(&url) => {
            if browser == Browser::Open && open(&url).is_ok() {
                eprintln!("Opened {url}");
            } else {
                eprintln!("Open this page to continue: {url}");
            }
            HostReply::done()
        }
        HostRequest::OpenExternal(_) => {
            HostReply::Error("Only web addresses open in the browser.".into())
        }
        HostRequest::OnBattery => HostReply::Value(Value::Bool(false)),
        request => HostReply::Error(format!(
            "The command line cannot answer {}.",
            request.kind()
        )),
    }
}

fn open_browser(url: &str) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        std::process::Command::new("xdg-open")
    };
    command
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

pub fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A web page opens in the browser only when asked to, and nothing else opens at all.
    #[test]
    fn host_requests_are_answered() {
        let page = || HostRequest::OpenExternal("https://lichess.org/".into());
        let opened = std::cell::RefCell::new(Vec::new());
        let open = |url: &str| {
            opened.borrow_mut().push(url.to_string());
            Ok(())
        };
        assert_eq!(answer(page(), Browser::Print, open), HostReply::done());
        assert!(opened.borrow().is_empty());
        assert_eq!(answer(page(), Browser::Open, open), HostReply::done());
        assert_eq!(*opened.borrow(), ["https://lichess.org/"]);
        let file = HostRequest::OpenExternal("file:///etc/passwd".into());
        assert_eq!(
            answer(file, Browser::Open, open),
            HostReply::Error("Only web addresses open in the browser.".into())
        );
        assert_eq!(opened.borrow().len(), 1);
        assert_eq!(
            answer(HostRequest::OnBattery, Browser::Open, open),
            HostReply::Value(json!(false))
        );
        assert_eq!(
            answer(HostRequest::Focus, Browser::Open, open),
            HostReply::Error("The command line cannot answer focus.".into())
        );
    }

    /// The fix is chosen by the error's kind, never by its words: a reworded message keeps it.
    #[test]
    fn a_login_or_engine_failure_names_the_command_line_fix() {
        let login =
            CoreError::new("@ann's Lichess login is unavailable. Reconnect it in Settings.")
                .with_kind(ErrorKind::Login);
        assert_eq!(
            told(&login),
            "@ann's Lichess login is unavailable. Reconnect it in Settings. From the command line: `kchess-cli login`."
        );
        let engine = CoreError::new("No engine.").with_kind(ErrorKind::NoEngine);
        assert_eq!(
            told(&engine),
            "No engine. From the command line: --engine <path>."
        );
        let offline = CoreError::new("Lichess is unreachable.").with_kind(ErrorKind::Offline);
        assert_eq!(told(&offline), "Lichess is unreachable.");
        assert_eq!(
            told(&CoreError::new("Move 2 is not legal.")),
            "Move 2 is not legal."
        );
    }
}

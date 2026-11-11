//! Lichess sign-in: PKCE with S256, a
//! random state, a loopback callback server on 127.0.0.1 that serves only `/callback` with the
//! right `state` and only once, the page the browser lands on, the token exchange, the account
//! check and the save of the login.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use super::accounts::Lichess;
use crate::host::Level;
use kchess_contracts::error::{CoreError, Result as CoreResult};
use kchess_contracts::types::UsageKind;

/// The OAuth client id KChess sends to Lichess.
pub const OAUTH_CLIENT_ID: &str = "kchess-desktop";

/// Permissions asked for at login. Studies, tournaments, team Swiss events and messages need
/// their own; accounts connected earlier are asked to reconnect the first time one is used.
pub const OAUTH_SCOPES: [&str; 12] = [
    "board:play",
    "challenge:read",
    "challenge:write",
    "follow:read",
    "follow:write",
    "msg:write",
    "puzzle:read",
    "puzzle:write",
    "study:read",
    "study:write",
    "tournament:write",
    "team:read",
];

/// How long the browser has to come back to the callback (`Lichess login timed out.`).
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a connection may take to send its request line before it is dropped.
const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
/// The most of a request head that is read.
const HEAD_LIMIT: usize = 8192;

/// The PKCE pair: `verifier` stays in the core, `challenge` goes to Lichess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

fn random_bytes<const N: usize>() -> CoreResult<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|cause| CoreError::new(format!("Random bytes are unavailable: {cause}")))?;
    Ok(bytes)
}

/// 32 random bytes in unpadded base64url as the verifier, and its SHA-256 as the challenge.
pub fn new_pkce() -> CoreResult<Pkce> {
    let verifier = URL_SAFE_NO_PAD.encode(random_bytes::<32>()?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok(Pkce {
        verifier,
        challenge,
    })
}

/// The state that ties the callback to this login.
pub fn new_state() -> CoreResult<String> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<24>()?))
}

/// The address the browser opens: `{base}/oauth` with the parameters in this order.
pub fn authorize_url(
    base: &str,
    redirect: &str,
    challenge: &str,
    state: &str,
) -> CoreResult<String> {
    let mut url = reqwest::Url::parse(&format!("{base}/oauth"))
        .map_err(|cause| CoreError::new(format!("Invalid Lichess URL: {cause}")))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", OAUTH_CLIENT_ID)
        .append_pair("redirect_uri", redirect)
        .append_pair("scope", &OAUTH_SCOPES.join(" "))
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", challenge)
        .append_pair("state", state);
    Ok(url.into())
}

pub use kchess_domain::records::oauth::{callback_page, default_look};

/// A callback request's target, parsed: its path and first value of `state` and `code`.
pub struct Callback {
    path: String,
    state: Option<String>,
    pub code: Option<String>,
    /// The client id a registering sign-in was issued.
    pub client_id: Option<String>,
}

fn parse_target(target: &str) -> Option<Callback> {
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    let first = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    Some(Callback {
        path: url.path().to_string(),
        state: first("state"),
        code: first("code").filter(|code| !code.is_empty()),
        client_id: first("client_id").filter(|id| !id.is_empty()),
    })
}

/// The request line's target of a request head, or None.
fn request_target(head: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(head);
    let line = text.lines().next()?;
    line.split_whitespace().nth(1).map(str::to_string)
}

async fn read_head(socket: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
        if head.windows(4).any(|window| window == b"\r\n\r\n") || head.len() > HEAD_LIMIT {
            break;
        }
    }
    Ok(head)
}

fn not_found() -> &'static [u8] {
    b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
}

/// The callback response: the page, with its headers.
fn page_response(status: u16, body: &str) -> Vec<u8> {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Cache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Leaves an answered callback connection open until the browser hangs up, so the page is not
/// lost to a reset (`loopback::close_after_client`); the sign-in goes on meanwhile.
fn hand_back(mut socket: TcpStream) {
    tokio::spawn(async move { crate::loopback::close_after_client(&mut socket).await });
}

/// The callback a sign-in waits for (`wait_for_redirect`): its `path` and `state`, the page the
/// browser is shown (for an authorized sign-in or not), and how long to wait before failing with
/// `timed_out`.
pub struct Redirect<'a> {
    pub path: &'a str,
    pub state: &'a str,
    pub page: &'a (dyn Fn(bool) -> String + Sync),
    pub timed_out: &'a str,
    pub timeout: Duration,
}

/// Waits for the one callback that may end a sign-in: `path` with this `state`. Favicon
/// requests, port probes and forged callbacks get a 404 and do not end it. `on_return` runs as
/// soon as the browser hands the sign-in back, before the token exchange.
pub async fn wait_for_redirect(
    listener: &TcpListener,
    redirect: Redirect<'_>,
    cancel: &CancellationToken,
    on_return: &(dyn Fn() + Sync),
) -> CoreResult<Callback> {
    let Redirect {
        path,
        state,
        page,
        timed_out,
        timeout,
    } = redirect;
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    // Each connection's request head is read on its own, so one that connects and sends nothing
    // cannot hold up the browser's callback behind it; dropped (and aborted) on return.
    let mut heads = tokio::task::JoinSet::new();
    loop {
        let (mut socket, head) = tokio::select! {
            _ = cancel.cancelled() => return Err(CoreError::aborted("Core closed.")),
            _ = &mut deadline => return Err(CoreError::new(timed_out)),
            accepted = listener.accept(), if heads.len() < PENDING_HEADS => {
                let (mut socket, _) = accepted.map_err(|cause| {
                    CoreError::new(format!("Could not open OAuth callback: {cause}"))
                })?;
                heads.spawn(async move {
                    let head = tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut socket)).await;
                    (socket, head)
                });
                continue;
            }
            Some(read) = heads.join_next() => match read {
                Ok(read) => read,
                Err(_) => continue,
            },
        };
        let Ok(Ok(head)) = head else {
            let _ = socket.shutdown().await;
            continue;
        };
        let callback = request_target(&head)
            .as_deref()
            .and_then(parse_target)
            .filter(|callback| {
                callback.path == path
                    && callback
                        .state
                        .as_deref()
                        .is_some_and(|given| same_secret(given, state))
            });
        let Some(callback) = callback else {
            let _ = socket.write_all(not_found()).await;
            hand_back(socket);
            continue;
        };
        on_return();
        let authorized = callback.code.is_some();
        let status = if authorized { 200 } else { 400 };
        let _ = socket
            .write_all(&page_response(status, &page(authorized)))
            .await;
        hand_back(socket);
        return Ok(callback);
    }
}

/// Request heads read at once; a connection past this waits for one of them to end (at most
/// `HEAD_TIMEOUT`).
const PENDING_HEADS: usize = 16;

/// `a == b` in time that depends on their lengths only.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |difference, (x, y)| difference | (x ^ y))
            == 0
}

/// `wait_for_redirect` for a Lichess login, answering its code.
pub async fn wait_for_callback(
    listener: &TcpListener,
    state: &str,
    look: &Value,
    timeout: Duration,
    cancel: &CancellationToken,
    on_return: &(dyn Fn() + Sync),
) -> CoreResult<String> {
    let page = |authorized| callback_page(authorized, look);
    let redirect = Redirect {
        path: "/callback",
        state,
        page: &page,
        timed_out: "Lichess login timed out.",
        timeout,
    };
    let callback = wait_for_redirect(listener, redirect, cancel, on_return).await?;
    callback
        .code
        .ok_or_else(|| CoreError::new("Lichess login was not completed."))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct AccountResponse {
    username: String,
}

/// The account a login was made for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connected {
    pub username: String,
}

impl Lichess {
    /// Signs an account in through the browser.
    pub async fn connect_lichess(
        &self,
        look: Option<&Value>,
        on_return: Option<&(dyn Fn() + Sync)>,
    ) -> CoreResult<Connected> {
        self.connect_lichess_within(look, on_return, CALLBACK_TIMEOUT)
            .await
    }

    /// Signs an account in with a callback deadline of `timeout`.
    pub async fn connect_lichess_within(
        &self,
        look: Option<&Value>,
        on_return: Option<&(dyn Fn() + Sync)>,
        timeout: Duration,
    ) -> CoreResult<Connected> {
        let epoch = self.login_epoch();
        let look = look.cloned().unwrap_or_else(default_look);
        let pkce = new_pkce()?;
        let state = new_state()?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|cause| CoreError::new(format!("Could not open OAuth callback: {cause}")))?;
        let port = listener
            .local_addr()
            .map_err(|cause| CoreError::new(format!("Could not open OAuth callback: {cause}")))?
            .port();
        let redirect = format!("http://127.0.0.1:{port}/callback");
        let url = authorize_url(self.client().base(), &redirect, &pkce.challenge, &state)?;
        let noop = || {};
        let on_return: &(dyn Fn() + Sync) = on_return.unwrap_or(&noop);
        let cancel = self.lifetime().clone();
        let opening = async {
            match self.capabilities().open_external(&url).await {
                Ok(()) => std::future::pending::<CoreResult<String>>().await,
                Err(cause) => {
                    self.host().log(
                        Level::Warn,
                        "lichess",
                        &format!("Could not open Lichess login page: {}", cause.message),
                    );
                    Err(cause)
                }
            }
        };
        let code = tokio::select! {
            answer = wait_for_callback(&listener, &state, &look, timeout, &cancel, on_return) => answer?,
            answer = opening => answer?,
        };
        if let Err(cause) = self.capabilities().focus().await {
            self.host().log(
                Level::Warn,
                "lichess",
                &format!("Could not bring KChess forward: {}", cause.message),
            );
        }
        let anonymous = self.anonymous(UsageKind::Other);
        let token: TokenResponse = self
            .client()
            .post("/api/token", &anonymous)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("client_id", OAUTH_CLIENT_ID),
                ("code_verifier", pkce.verifier.as_str()),
            ])
            .cancel(&cancel)
            .read_json()
            .await?;
        let account: AccountResponse = self
            .client()
            .get(
                "/api/account",
                &anonymous.with_token(token.access_token.as_str()),
            )
            .cancel(&cancel)
            .read_json()
            .await?;
        self.save_login(&account.username, &token.access_token, epoch)
            .await?;
        Ok(Connected {
            username: account.username,
        })
    }

    /// The token is encrypted by the host's credential store and saved with the
    /// account, unless a logout ran since the login began.
    async fn save_login(&self, username: &str, token: &str, epoch: u64) -> CoreResult<()> {
        if epoch != self.login_epoch() {
            return Err(CoreError::new("Login was cancelled by logout."));
        }
        if !self.capabilities().secrets_available().await? {
            return Err(CoreError::new("OS credential encryption is unavailable."));
        }
        let ciphertext = self.capabilities().encrypt(token).await?;
        let current = || epoch == self.login_epoch();
        self.store()
            .save_login(username, &ciphertext, &current)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_the_s256_of_the_verifier() {
        let pkce = new_pkce().unwrap();
        assert_eq!(pkce.verifier.len(), 43);
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes()));
        assert_eq!(pkce.challenge, expected);
        assert_ne!(new_pkce().unwrap().verifier, pkce.verifier);
    }

    #[test]
    fn the_authorize_url_carries_the_login_parameters_in_order() {
        let url = authorize_url(
            "https://lichess.org",
            "http://127.0.0.1:4000/callback",
            "CH",
            "ST",
        )
        .unwrap();
        assert!(
            url.starts_with(
                "https://lichess.org/oauth?response_type=code&client_id=kchess-desktop"
            )
        );
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=ST"));
        assert!(
            url.contains("follow%3Aread")
                || url.contains("follow:read")
                || url.contains("follow%3aread")
        );
    }

    #[test]
    fn the_page_is_self_contained_and_escapes_nothing_it_was_not_given() {
        let page = callback_page(true, &default_look());
        assert!(page.contains("Signed in to Lichess"));
        assert!(!page.contains("__"));
        assert!(!page.contains("<script"));
        assert!(!page.contains("http://") && !page.contains("https://"));
        let cancelled = callback_page(false, &default_look());
        assert!(cancelled.contains("Connection cancelled"));
    }

    #[test]
    fn callback_targets_are_parsed() {
        let callback = parse_target("/callback?code=abc&state=xyz").unwrap();
        assert_eq!(callback.path, "/callback");
        assert_eq!(callback.state.as_deref(), Some("xyz"));
        assert_eq!(callback.code.as_deref(), Some("abc"));
        assert_eq!(parse_target("/callback?state=x&code=").unwrap().code, None);
    }

    /// A local process that connects to the callback port and sends nothing held up the real
    /// callback behind it for five seconds each time; now heads are read side by side.
    #[tokio::test]
    async fn a_silent_connection_does_not_hold_up_the_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let silent: Vec<TcpStream> =
            futures_util::future::join_all((0..4).map(|_| TcpStream::connect(address)))
                .await
                .into_iter()
                .map(Result::unwrap)
                .collect();
        let browser = tokio::spawn(async move {
            let mut socket = TcpStream::connect(address).await.unwrap();
            socket
                .write_all(b"GET /callback?code=abc&state=ST HTTP/1.1\r\nHost: x\r\n\r\n")
                .await
                .unwrap();
            let mut answer = Vec::new();
            let _ = socket.read_to_end(&mut answer).await;
        });
        let started = std::time::Instant::now();
        let code = wait_for_callback(
            &listener,
            "ST",
            &default_look(),
            Duration::from_secs(30),
            &CancellationToken::new(),
            &|| {},
        )
        .await
        .unwrap();
        assert_eq!(code, "abc");
        assert!(started.elapsed() < HEAD_TIMEOUT, "{:?}", started.elapsed());
        drop(silent);
        browser.abort();
    }

    #[test]
    fn the_state_compares_whole() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "ab"));
    }
}

//! Lichess sign-in (`connectLichess` and `crates/kchess-node/js/oauthPage.ts`): PKCE with S256, a
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
use super::client::authorize;
use crate::error::{CoreError, Result as CoreResult};
use crate::host::Level;

/// `OAUTH_CLIENT_ID` of `lichess.ts`.
pub const OAUTH_CLIENT_ID: &str = "kchess-desktop";

/// Permissions asked for at login. Studies, tournaments, team Swiss events and messages need
/// their own; accounts connected earlier are asked to reconnect the first time one is used.
pub const OAUTH_SCOPES: [&str; 11] = [
    "board:play",
    "challenge:read",
    "challenge:write",
    "follow:read",
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
    getrandom::getrandom(&mut bytes)
        .map_err(|cause| CoreError::new(format!("Random bytes are unavailable: {cause}")))?;
    Ok(bytes)
}

/// `randomBytes(32).toString('base64url')` as the verifier, and its SHA-256 as the challenge.
pub fn new_pkce() -> CoreResult<Pkce> {
    let verifier = URL_SAFE_NO_PAD.encode(random_bytes::<32>()?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok(Pkce {
        verifier,
        challenge,
    })
}

/// The state that ties the callback to this login (`randomBytes(24)`).
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

/// The OAuth look used until the app sends its own colours (`DEFAULT_OAUTH_LOOK`).
pub fn default_look() -> Value {
    match kchess_domain::records::call("oauthLookDefault", &[]) {
        Some(Ok(look)) => look,
        _ => Value::Null,
    }
}

pub use kchess_domain::records::oauth::callback_page;

/// A callback request's target, parsed: its path and first value of `state` and `code`.
struct Callback {
    path: String,
    state: Option<String>,
    code: Option<String>,
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

/// The callback response: the page, with the headers `connectLichess` sets.
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

/// Waits for the one callback that may end the login: `/callback` with this `state`. Favicon
/// requests, port probes and forged callbacks get a 404 and do not end it. `on_return` runs as
/// soon as the browser hands the login back, before the token exchange.
pub async fn wait_for_callback(
    listener: &TcpListener,
    state: &str,
    look: &Value,
    timeout: Duration,
    cancel: &CancellationToken,
    on_return: &(dyn Fn() + Sync),
) -> CoreResult<String> {
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        let accepted = tokio::select! {
            _ = cancel.cancelled() => return Err(CoreError::aborted("Core closed.")),
            _ = &mut deadline => return Err(CoreError::new("Lichess login timed out.")),
            accepted = listener.accept() => accepted,
        };
        let (mut socket, _) = match accepted {
            Ok(accepted) => accepted,
            Err(cause) => {
                return Err(CoreError::new(format!(
                    "Could not open OAuth callback: {cause}"
                )));
            }
        };
        let head = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut socket)).await {
            Ok(Ok(head)) => head,
            _ => {
                let _ = socket.shutdown().await;
                continue;
            }
        };
        let callback = request_target(&head).as_deref().and_then(parse_target);
        let matches = callback.as_ref().is_some_and(|callback| {
            callback.path == "/callback" && callback.state.as_deref() == Some(state)
        });
        if !matches {
            let _ = socket.write_all(not_found()).await;
            let _ = socket.shutdown().await;
            continue;
        }
        let code = callback.and_then(|callback| callback.code);
        on_return();
        let (status, body) = match &code {
            Some(_) => (200, callback_page(true, look)),
            None => (400, callback_page(false, look)),
        };
        let _ = socket.write_all(&page_response(status, &body)).await;
        let _ = socket.shutdown().await;
        return match code {
            Some(code) => Ok(code),
            None => Err(CoreError::new("Lichess login was not completed.")),
        };
    }
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
    /// `connectLichess`: signs an account in through the browser.
    pub async fn connect_lichess(
        &self,
        look: Option<&Value>,
        on_return: Option<&(dyn Fn() + Sync)>,
    ) -> CoreResult<Connected> {
        self.connect_lichess_within(look, on_return, CALLBACK_TIMEOUT)
            .await
    }

    /// `connectLichess` with a callback deadline of `timeout` (`CALLBACK_TIMEOUT` in the app).
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
        let token: TokenResponse = self
            .client()
            .post_form(
                "/api/token",
                &[
                    ("grant_type", "authorization_code"),
                    ("code", code.as_str()),
                    ("redirect_uri", redirect.as_str()),
                    ("client_id", OAUTH_CLIENT_ID),
                    ("code_verifier", pkce.verifier.as_str()),
                ],
                None,
                &cancel,
            )
            .await?;
        let account: AccountResponse = self
            .client()
            .get_json(
                "/api/account",
                &[],
                Some(&authorize(&token.access_token)),
                &cancel,
            )
            .await?;
        self.save_login(&account.username, &token.access_token, epoch)
            .await?;
        Ok(Connected {
            username: account.username,
        })
    }

    /// `saveLogin`: the token is encrypted by the host's credential store and saved with the
    /// account, unless a logout ran since the login began.
    async fn save_login(&self, username: &str, token: &str, epoch: u64) -> CoreResult<()> {
        if epoch != self.login_epoch() {
            return Err(CoreError::new("Login was cancelled by logout."));
        }
        if !self.capabilities().secrets_available().await? {
            return Err(CoreError::new("OS credential encryption is unavailable."));
        }
        let ciphertext = self.capabilities().encrypt(token).await?;
        self.store().save_login(username, &ciphertext)
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
}

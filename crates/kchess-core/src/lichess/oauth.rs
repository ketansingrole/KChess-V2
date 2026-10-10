//! Lichess sign-in (`connectLichess` and `core/src/services/oauthPage.ts`): PKCE with S256, a
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

const APP_ICON_SVG: &str = r##"<svg viewBox="0 0 1024 1024"><defs><linearGradient id="tile" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F7DC90"/><stop offset="1" stop-color="#D8A23F"/></linearGradient><linearGradient id="rook" gradientUnits="userSpaceOnUse" x1="0" y1="204" x2="0" y2="820"><stop offset="0" stop-color="#3B2D20"/><stop offset="1" stop-color="#1E1610"/></linearGradient><filter id="shadow" x="-10%" y="-10%" width="120%" height="125%"><feDropShadow dx="0" dy="14" stdDeviation="12" flood-color="#140a00" flood-opacity="0.38"/></filter></defs><polygon points="716.10,0.00 733.82,0.07 746.29,0.28 757.33,0.64 767.51,1.14 777.10,1.78 786.22,2.56 794.97,3.48 803.40,4.55 811.56,5.75 819.46,7.10 827.14,8.59 834.62,10.22 841.90,11.99 849.00,13.90 855.93,15.94 862.69,18.13 869.29,20.46 875.74,22.93 882.04,25.53 888.19,28.28 894.20,31.16 900.06,34.17 905.79,37.33 911.37,40.62 916.82,44.05 922.13,47.61 927.31,51.31 932.35,55.14 937.26,59.11 942.04,63.21 946.67,67.45 951.18,71.82 955.55,76.33 959.79,80.96 963.89,85.74 967.86,90.65 971.69,95.69 975.39,100.87 978.95,106.18 982.38,111.63 985.67,117.21 988.83,122.94 991.84,128.80 994.72,134.81 997.47,140.96 1000.07,147.26 1002.54,153.71 1004.87,160.31 1007.06,167.07 1009.10,174.00 1011.01,181.10 1012.78,188.38 1014.41,195.86 1015.90,203.54 1017.25,211.44 1018.45,219.60 1019.52,228.03 1020.44,236.78 1021.22,245.90 1021.86,255.49 1022.36,265.67 1022.72,276.71 1022.93,289.18 1023.00,306.90 1023.00,716.10 1022.93,733.82 1022.72,746.29 1022.36,757.33 1021.86,767.51 1021.22,777.10 1020.44,786.22 1019.52,794.97 1018.45,803.40 1017.25,811.56 1015.90,819.46 1014.41,827.14 1012.78,834.62 1011.01,841.90 1009.10,849.00 1007.06,855.93 1004.87,862.69 1002.54,869.29 1000.07,875.74 997.47,882.04 994.72,888.19 991.84,894.20 988.83,900.06 985.67,905.79 982.38,911.37 978.95,916.82 975.39,922.13 971.69,927.31 967.86,932.35 963.89,937.26 959.79,942.04 955.55,946.67 951.18,951.18 946.67,955.55 942.04,959.79 937.26,963.89 932.35,967.86 927.31,971.69 922.13,975.39 916.82,978.95 911.37,982.38 905.79,985.67 900.06,988.83 894.20,991.84 888.19,994.72 882.04,997.47 875.74,1000.07 869.29,1002.54 862.69,1004.87 855.93,1007.06 849.00,1009.10 841.90,1011.01 834.62,1012.78 827.14,1014.41 819.46,1015.90 811.56,1017.25 803.40,1018.45 794.97,1019.52 786.22,1020.44 777.10,1021.22 767.51,1021.86 757.33,1022.36 746.29,1022.72 733.82,1022.93 716.10,1023.00 306.90,1023.00 289.18,1022.93 276.71,1022.72 265.67,1022.36 255.49,1021.86 245.90,1021.22 236.78,1020.44 228.03,1019.52 219.60,1018.45 211.44,1017.25 203.54,1015.90 195.86,1014.41 188.38,1012.78 181.10,1011.01 174.00,1009.10 167.07,1007.06 160.31,1004.87 153.71,1002.54 147.26,1000.07 140.96,997.47 134.81,994.72 128.80,991.84 122.94,988.83 117.21,985.67 111.63,982.38 106.18,978.95 100.87,975.39 95.69,971.69 90.65,967.86 85.74,963.89 80.96,959.79 76.33,955.55 71.82,951.18 67.45,946.67 63.21,942.04 59.11,937.26 55.14,932.35 51.31,927.31 47.61,922.13 44.05,916.82 40.62,911.37 37.33,905.79 34.17,900.06 31.16,894.20 28.28,888.19 25.53,882.04 22.93,875.74 20.46,869.29 18.13,862.69 15.94,855.93 13.90,849.00 11.99,841.90 10.22,834.62 8.59,827.14 7.10,819.46 5.75,811.56 4.55,803.40 3.48,794.97 2.56,786.22 1.78,777.10 1.14,767.51 0.64,757.33 0.28,746.29 0.07,733.82 0.00,716.10 0.00,306.90 0.07,289.18 0.28,276.71 0.64,265.67 1.14,255.49 1.78,245.90 2.56,236.78 3.48,228.03 4.55,219.60 5.75,211.44 7.10,203.54 8.59,195.86 10.22,188.38 11.99,181.10 13.90,174.00 15.94,167.07 18.13,160.31 20.46,153.71 22.93,147.26 25.53,140.96 28.28,134.81 31.16,128.80 34.17,122.94 37.33,117.21 40.62,111.63 44.05,106.18 47.61,100.87 51.31,95.69 55.14,90.65 59.11,85.74 63.21,80.96 67.45,76.33 71.82,71.82 76.33,67.45 80.96,63.21 85.74,59.11 90.65,55.14 95.69,51.31 100.87,47.61 106.18,44.05 111.63,40.62 117.21,37.33 122.94,34.17 128.80,31.16 134.81,28.28 140.96,25.53 147.26,22.93 153.71,20.46 160.31,18.13 167.07,15.94 174.00,13.90 181.10,11.99 188.38,10.22 195.86,8.59 203.54,7.10 211.44,5.75 219.60,4.55 228.03,3.48 236.78,2.56 245.90,1.78 255.49,1.14 265.67,0.64 276.71,0.28 289.18,0.07 306.90,0.00" fill="url(#tile)"/><g fill="url(#rook)" filter="url(#shadow)"><rect x="254.4" y="758.4" width="515.2" height="61.6" rx="20.2"/><rect x="288.0" y="696.8" width="448.0" height="69.4" rx="17.9"/><polygon points="366.4,366.4 657.6,366.4 702.4,708.0 321.6,708.0"/><rect x="299.2" y="316.0" width="425.6" height="58.2" rx="13.4"/><rect x="310.4" y="260.0" width="403.2" height="58.2" rx="0.0"/><rect x="310.4" y="204.0" width="100.8" height="114.2" rx="7.8"/><rect x="461.6" y="204.0" width="100.8" height="114.2" rx="7.8"/><rect x="612.8" y="204.0" width="100.8" height="114.2" rx="7.8"/></g></svg>"##;

fn colours(look: &Value) -> String {
    let field = |key: &str| look[key].as_str().unwrap_or("").to_string();
    format!(
        "--bg:{};--card:{};--text:{};--muted:{};--primary:{};--border:{};",
        field("bg"),
        field("elevated"),
        field("text"),
        field("textMuted"),
        field("primary"),
        field("border"),
    )
}

/// `oauthPage(authorized, look)`: a self-contained callback page. It loads nothing external and
/// carries no credentials or callback values.
pub fn callback_page(authorized: bool, look: &Value) -> String {
    let title = if authorized {
        "Signed in to Lichess"
    } else {
        "Connection cancelled"
    };
    let message = if authorized {
        "KChess is finishing the connection and has been brought back to the front."
    } else {
        "Your Lichess account was not connected. Return to KChess to try again when you’re ready."
    };
    let hint = if authorized {
        "You can close this browser tab and continue in KChess."
    } else {
        "You can close this browser tab."
    };
    let appearance = look["appearance"].as_str().unwrap_or("system");
    let theme = if appearance == "system" {
        format!(
            ":root{{color-scheme:light;{}}}@media(prefers-color-scheme:dark){{:root{{color-scheme:dark;{}}}}}",
            colours(&look["light"]),
            colours(&look["dark"]),
        )
    } else {
        format!(
            ":root{{color-scheme:{appearance};{}}}",
            colours(&look[appearance]),
        )
    };
    let icon = APP_ICON_SVG.replacen("<svg ", "<svg aria-hidden=\"true\" ", 1);
    let mark = if authorized { "✓" } else { "↩" };
    PAGE_TEMPLATE
        .replace("__THEME__", &theme)
        .replace("__TITLE__", title)
        .replace("__ICON__", &icon)
        .replace("__MARK__", mark)
        .replace("__MESSAGE__", message)
        .replace("__HINT__", hint)
}

const PAGE_TEMPLATE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="referrer" content="no-referrer"><title>__TITLE__ · KChess</title>
<style>
__THEME__
*{box-sizing:border-box}html{font-family:Inter,ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;-webkit-font-smoothing:antialiased}body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px;background:radial-gradient(ellipse at top,color-mix(in srgb,var(--primary) 14%,transparent) 0,transparent 60%),var(--bg);color:var(--text)}main{width:100%;max-width:440px;padding:36px;border:1px solid var(--border);border-radius:16px;background:var(--card)}.brand{display:flex;align-items:center;gap:8px;font-size:15px;font-weight:600;color:var(--muted)}.brand svg{width:24px;height:24px;flex:none}.mark{display:grid;place-items:center;width:56px;height:56px;margin:28px 0 20px;border-radius:50%;background:color-mix(in srgb,var(--primary) 16%,transparent);color:var(--primary);font-size:26px;font-weight:700}h1{font-size:24px;line-height:1.25;margin:0 0 12px;letter-spacing:-.02em}p{font-size:15px;line-height:1.6;color:color-mix(in srgb,var(--text) 80%,var(--muted));margin:0 0 20px}.hint{border-top:1px solid var(--border);padding-top:20px;margin:0;font-size:13px;color:var(--muted)}@media(max-width:480px){main{padding:24px}h1{font-size:22px}}
</style></head><body><main><div class="brand">__ICON__KChess</div><div class="mark" aria-hidden="true">__MARK__</div><h1>__TITLE__</h1><p>__MESSAGE__</p><p class="hint">__HINT__</p></main></body></html>"##;

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

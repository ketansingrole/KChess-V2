use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::random;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

const AUTH_URL: &str = "https://lichess.org/oauth";
const TOKEN_URL: &str = "https://lichess.org/api/token";
const ACCOUNT_URL: &str = "https://lichess.org/api/account";
const CLIENT_ID: &str = "kchess-desktop";
const OAUTH_SCOPES: &str = "board:play challenge:write";
const CALLBACK_TIMEOUT_SECS: u64 = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectedLichessAccount {
    pub username: String,
    pub access_token: String,
    pub scopes: String,
}

#[derive(Debug)]
pub enum OAuthError {
    Io(std::io::Error),
    Url(url::ParseError),
    Http(reqwest::Error),
    Json(serde_json::Error),
    MissingCode,
    InvalidState,
    InvalidResponse(String),
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "oauth callback failed: {err}"),
            Self::Url(err) => write!(f, "oauth url error: {err}"),
            Self::Http(err) => write!(f, "oauth request failed: {err}"),
            Self::Json(err) => write!(f, "oauth response parse failed: {err}"),
            Self::MissingCode => write!(f, "oauth callback did not include an authorization code"),
            Self::InvalidState => write!(f, "oauth callback state did not match the login request"),
            Self::InvalidResponse(err) => write!(f, "oauth response was invalid: {err}"),
        }
    }
}

impl std::error::Error for OAuthError {}

impl From<std::io::Error> for OAuthError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<url::ParseError> for OAuthError {
    fn from(value: url::ParseError) -> Self {
        Self::Url(value)
    }
}

impl From<reqwest::Error> for OAuthError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value)
    }
}

impl From<serde_json::Error> for OAuthError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub fn begin_oauth_login() -> Result<(String, OAuthPendingLogin), OAuthError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;

    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let verifier = random_string();
    let state = random_string();
    let challenge = code_challenge(&verifier);

    let mut url = Url::parse(AUTH_URL)?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", OAUTH_SCOPES)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &challenge)
        .append_pair("state", &state);

    Ok((
        url.into(),
        OAuthPendingLogin {
            listener,
            redirect_uri,
            verifier,
            state,
        },
    ))
}

pub fn complete_oauth_login(
    pending: OAuthPendingLogin,
) -> Result<ConnectedLichessAccount, OAuthError> {
    let callback = wait_for_callback(pending.listener)?;
    let callback_url = Url::parse(&callback)?;
    let mut code = None;
    let mut returned_state = None;

    for (key, value) in callback_url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => returned_state = Some(value.into_owned()),
            _ => {}
        }
    }

    let code = code.ok_or(OAuthError::MissingCode)?;
    if returned_state.as_deref() != Some(pending.state.as_str()) {
        return Err(OAuthError::InvalidState);
    }

    let client = Client::builder().timeout(Duration::from_secs(20)).build()?;

    let token: TokenResponse = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("client_id", CLIENT_ID),
            ("code_verifier", pending.verifier.as_str()),
        ])
        .send()?
        .error_for_status()?
        .json()?;

    let account: AccountResponse = client
        .get(ACCOUNT_URL)
        .bearer_auth(&token.access_token)
        .send()?
        .error_for_status()?
        .json()?;

    Ok(ConnectedLichessAccount {
        username: account.username,
        access_token: token.access_token,
        scopes: token.scope.unwrap_or_else(|| OAUTH_SCOPES.to_string()),
    })
}

pub fn credential_url(username: &str) -> String {
    format!("kchess-lichess-{username}")
}

pub struct OAuthPendingLogin {
    listener: TcpListener,
    redirect_uri: String,
    verifier: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccountResponse {
    #[serde(rename = "username")]
    username: String,
}

fn random_string() -> String {
    let bytes: [u8; 32] = random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn wait_for_callback(listener: TcpListener) -> Result<String, OAuthError> {
    let deadline = Instant::now() + Duration::from_secs(CALLBACK_TIMEOUT_SECS);
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(stream) => break stream,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(OAuthError::InvalidResponse(
                        "oauth login timed out waiting for browser callback".to_string(),
                    ));
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(OAuthError::Io(err)),
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut buffer = [0_u8; 4096];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| OAuthError::InvalidResponse("empty callback request".to_string()))?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| OAuthError::InvalidResponse("malformed callback request".to_string()))?;

    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><h3>KChess connected to Lichess.</h3><p>You can return to the app.</p></body></html>";
    stream.write_all(response.as_bytes())?;
    stream.flush()?;

    Ok(format!("http://localhost{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_key_is_per_user() {
        assert_eq!(credential_url("alice"), "kchess-lichess-alice");
    }

    #[test]
    fn code_challenge_is_url_safe() {
        let challenge = code_challenge("sample-verifier");
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(!challenge.contains('='));
    }
}

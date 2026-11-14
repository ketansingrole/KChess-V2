//! ChatGPT plan usage (Sign in with ChatGPT's open-source flow): the HTTP side. A person's plan
//! pays for the requests, so nothing here runs but at their asking, nothing retries a limit, and
//! an answer counts only once the stream says it completed.
//!
//! The ID token comes straight from the token endpoint over TLS, so its claims are checked and
//! its signature is not (OpenID Connect Core 3.1.3.7).

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::lichess::client::http_client_builder;
use kchess_contracts::error::{CoreError, Result};

const AUTH: &str = "https://auth.openai.com";
const API: &str = "https://api.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
/// The name OpenAI registers on the first sign-in.
const AGENT_NAME: &str = "KChess";
/// The port of the first registration; later sign-ins may use any.
pub const FIRST_PORT: u16 = 1455;
pub const CALLBACK_PATH: &str = "/auth/callback";
/// Refresh this long before a token runs out.
const REFRESH_MARGIN_MS: f64 = 60_000.0;
/// A whole answer's deadline.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(90);
/// More text than any explanation.
const ANSWER_LIMIT: usize = 16 * 1024;

/// A new host id, kept for as long as this profile lives: a UUIDv4 URI, the form OpenAI accepts.
pub fn new_host_id() -> Result<String> {
    Ok(format!("urn:uuid:{}", crate::ids::random_uuid()?))
}

/// Whether `id` has the form `new_host_id` gives.
pub fn valid_host_id(id: &str) -> bool {
    let Some(uuid) = id.strip_prefix("urn:uuid:") else {
        return false;
    };
    let lengths: Vec<usize> = uuid.split('-').map(str::len).collect();
    lengths == [8, 4, 4, 4, 12] && uuid.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
}

/// What is kept of a sign-in, sealed by the credential store.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    /// Milliseconds since the epoch.
    pub expires_at: f64,
}

/// Never the tokens themselves, so a stray `{:?}` cannot log them.
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access", &"[redacted]")
            .field("refresh", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Tokens {
    pub fn fresh(&self, now: f64) -> bool {
        self.expires_at - now > REFRESH_MARGIN_MS
    }
}

#[derive(Deserialize, Debug)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<f64>,
    scope: Option<String>,
}

/// Who signed in, from a checked ID token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub sub: String,
    pub email: Option<String>,
}

pub use kchess_contracts::types::{AiModel as Model, AiProblem as Problem};

/// A failed request: what kind, and the code that said so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub problem: Problem,
    pub detail: String,
}

impl Refusal {
    fn new(problem: Problem, detail: impl Into<String>) -> Refusal {
        Refusal {
            problem,
            detail: detail.into(),
        }
    }
}

/// `error.code` of a Responses error, as the recovery guide sorts them.
fn problem_of(code: &str) -> Problem {
    match code {
        "subscription_sharing_usage_limit_exceeded" => Problem::Limit,
        "subscription_sharing_user_not_eligible"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => Problem::Ineligible,
        "subscription_sharing_invalid_user" => Problem::Connect,
        "subscription_sharing_unsupported_capability" => Problem::Unsupported,
        _ => Problem::Unavailable,
    }
}

fn refused(status: u16, body: &str) -> Refusal {
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    // Not every error is an `error` object: some are `{"detail": "…"}`. Only the log reads this.
    let said: String = parsed["error"]["message"]
        .as_str()
        .or(parsed["detail"].as_str())
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect();
    if let Some(code) = parsed["error"]["code"].as_str() {
        return Refusal::new(problem_of(code), format!("{code}: {said}"));
    }
    let problem = match status {
        401 => Problem::Connect,
        403 => Problem::Ineligible,
        429 => Problem::Limit,
        _ => Problem::Unavailable,
    };
    Refusal::new(problem, format!("HTTP {status}: {said}"))
}

/// Whether a refresh failure means the sign-in is over, rather than a bad moment.
fn refresh_is_final(code: &str) -> bool {
    matches!(
        code,
        "invalid_grant"
            | "invalid_refresh_token"
            | "token_expired"
            | "refresh_token_expired"
            | "refresh_token_invalidated"
            | "refresh_token_reused"
    )
}

/// The claims of `id_token`, checked for this sign-in.
pub fn check_id_token(
    id_token: &str,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now_ms: f64,
) -> Result<Identity> {
    let invalid = || CoreError::new("ChatGPT sign-in could not be verified.");
    let payload = id_token.split('.').nth(1).ok_or_else(invalid)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| invalid())?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let audience = match &claims["aud"] {
        Value::Array(list) => list.iter().any(|aud| aud == client_id),
        aud => aud == client_id,
    };
    let unexpired = claims["exp"]
        .as_f64()
        .is_some_and(|exp| exp * 1000.0 > now_ms);
    if claims["iss"] != issuer || !audience || !unexpired || claims["nonce"] != nonce {
        return Err(invalid());
    }
    Ok(Identity {
        sub: claims["sub"].as_str().ok_or_else(invalid)?.to_string(),
        email: claims["email"].as_str().map(str::to_string),
    })
}

/// A finished sign-in.
#[derive(Debug, Clone, PartialEq)]
pub struct SignedIn {
    pub tokens: Tokens,
    pub identity: Identity,
}

/// One authorization attempt's secrets and addresses.
pub struct Attempt<'a> {
    pub client_id: Option<&'a str>,
    pub host_id: &'a str,
    pub redirect: &'a str,
    pub state: &'a str,
    pub nonce: &'a str,
    pub challenge: &'a str,
}

/// Reads one Responses stream to its end.
#[derive(Default)]
struct Stream {
    pending: String,
    text: String,
    done: Option<std::result::Result<(), Refusal>>,
}

impl Stream {
    /// Takes the next bytes of text; events are single `data:` lines.
    fn feed(&mut self, chunk: &str) {
        self.pending.push_str(chunk);
        while let Some(end) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=end).collect();
            if let Some(data) = line.trim_end().strip_prefix("data:") {
                self.event(data.trim());
            }
        }
    }

    fn event(&mut self, data: &str) {
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return;
        };
        match event["type"].as_str() {
            Some("response.output_text.delta") => {
                self.text
                    .push_str(event["delta"].as_str().unwrap_or_default());
            }
            Some("response.completed") => self.done = Some(Ok(())),
            Some("response.failed") => {
                let code = event["response"]["error"]["code"]
                    .as_str()
                    .or(event["error"]["code"].as_str())
                    .unwrap_or("failed");
                self.done = Some(Err(Refusal::new(problem_of(code), code)));
            }
            Some("response.incomplete") => {
                self.done = Some(Err(Refusal::new(Problem::Unavailable, "incomplete")));
            }
            _ => {}
        }
    }
}

/// The HTTP side of ChatGPT plan usage.
pub struct Ai {
    client: reqwest::Client,
    auth: String,
    api: String,
    lifetime: CancellationToken,
    /// Where the sign-in callback listens first: OpenAI's registered port, or any free one
    /// under test, so test runs side by side do not fight over it.
    callback_port: u16,
    /// Refresh tokens rotate: two refreshes at once would burn the sign-in.
    pub refreshing: AsyncMutex<()>,
}

impl Ai {
    /// `origin` replaces OpenAI's addresses under test.
    pub fn new(origin: Option<String>, lifetime: CancellationToken) -> Result<Ai> {
        let client = http_client_builder()
            .build()
            .map_err(|cause| CoreError::new(format!("Could not start HTTP: {cause}")))?;
        Ok(Ai {
            client,
            callback_port: if origin.is_some() { 0 } else { FIRST_PORT },
            auth: origin.clone().unwrap_or_else(|| AUTH.to_string()),
            api: origin.unwrap_or_else(|| API.to_string()),
            lifetime,
            refreshing: AsyncMutex::new(()),
        })
    }

    pub fn callback_port(&self) -> u16 {
        self.callback_port
    }

    pub fn issuer(&self) -> &str {
        &self.auth
    }

    pub fn lifetime(&self) -> &CancellationToken {
        &self.lifetime
    }

    /// The address the browser opens.
    pub fn authorize_url(&self, attempt: &Attempt) -> Result<String> {
        let mut url = reqwest::Url::parse(&format!("{}/api/accounts/authorize", self.auth))
            .map_err(|cause| CoreError::new(format!("Invalid sign-in address: {cause}")))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair(
                "client_id",
                attempt.client_id.unwrap_or("dynamic_agent_client"),
            );
            if attempt.client_id.is_none() {
                query.append_pair("agent_name_hint", AGENT_NAME);
            }
            query
                .append_pair("ext_agent_host_id", attempt.host_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", attempt.redirect)
                .append_pair("scope", SCOPES)
                .append_pair("resource", RESOURCE)
                .append_pair("state", attempt.state)
                .append_pair("nonce", attempt.nonce)
                .append_pair("code_challenge_method", "S256")
                .append_pair("code_challenge", attempt.challenge);
        }
        Ok(url.into())
    }

    /// A request's status and whole body, within `ANSWER_TIMEOUT` and the core's lifetime. The
    /// deadline covered only the headers: a server that sent them and then stalled the body kept
    /// the call, and the sign-in refresh lock it holds, forever, so every later AI call waited.
    async fn answered(
        &self,
        sent: impl std::future::Future<Output = reqwest::Result<reqwest::Response>>,
    ) -> std::result::Result<(u16, String), Refusal> {
        let read = async {
            let response = sent.await?;
            let status = response.status().as_u16();
            Ok::<_, reqwest::Error>((status, response.text().await?))
        };
        tokio::select! {
            answer = tokio::time::timeout(ANSWER_TIMEOUT, read) => answer
                .map_err(|_| Refusal::new(Problem::Unavailable, "timeout"))?
                .map_err(|cause| Refusal::new(Problem::Unavailable, cause.without_url().to_string())),
            () = self.lifetime.cancelled() => Err(Refusal::new(Problem::Unavailable, "closing")),
        }
    }

    async fn token(&self, form: &[(&str, &str)]) -> std::result::Result<TokenResponse, Refusal> {
        let sent = self
            .client
            .post(format!("{}/api/accounts/oauth/token", self.auth))
            .form(form)
            .send();
        let (status, body) = self.answered(sent).await?;
        if status != 200 {
            let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let code = parsed["error"]
                .as_str()
                .or(parsed["error"]["code"].as_str())
                .unwrap_or_default();
            let problem = if refresh_is_final(code) {
                Problem::Connect
            } else {
                Problem::Unavailable
            };
            return Err(Refusal::new(problem, code));
        }
        serde_json::from_str(&body)
            .map_err(|_| Refusal::new(Problem::Unavailable, "unreadable token response"))
    }

    /// Exchanges the callback's code for tokens, and proves plan use was granted.
    pub async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        redirect: &str,
        client_id: &str,
        nonce: &str,
        now_ms: f64,
    ) -> Result<SignedIn> {
        let response = self
            .token(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", redirect),
                ("client_id", client_id),
                ("resource", RESOURCE),
            ])
            .await
            .map_err(|_| CoreError::new("ChatGPT sign-in failed. Try again."))?;
        let scopes = response.scope.as_deref().unwrap_or_default();
        if !scopes.split_whitespace().any(|scope| scope == PLAN_SCOPE) {
            return Err(CoreError::new("ChatGPT plan use was not allowed."));
        }
        let id_token = response
            .id_token
            .as_deref()
            .ok_or_else(|| CoreError::new("ChatGPT sign-in could not be verified."))?;
        let identity = check_id_token(id_token, &self.auth, client_id, nonce, now_ms)?;
        let refresh = response
            .refresh_token
            .clone()
            .ok_or_else(|| CoreError::new("ChatGPT sign-in failed. Try again."))?;
        Ok(SignedIn {
            tokens: Tokens {
                expires_at: now_ms + response.expires_in.unwrap_or(3600.0) * 1000.0,
                access: response.access_token,
                refresh,
            },
            identity,
        })
    }

    /// Renews `tokens`. The caller holds `refreshing` and saves the whole result together.
    pub async fn refresh(
        &self,
        tokens: &Tokens,
        client_id: &str,
        now_ms: f64,
    ) -> std::result::Result<Tokens, Refusal> {
        let response = self
            .token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", tokens.refresh.as_str()),
                ("client_id", client_id),
                ("resource", RESOURCE),
            ])
            .await?;
        Ok(Tokens {
            expires_at: now_ms + response.expires_in.unwrap_or(3600.0) * 1000.0,
            access: response.access_token,
            refresh: response
                .refresh_token
                .ok_or_else(|| Refusal::new(Problem::Unavailable, "no refresh token"))?,
        })
    }

    /// The models this account may use.
    pub async fn models(&self, access: &str) -> std::result::Result<Vec<Model>, Refusal> {
        let sent = self
            .client
            .get(format!("{}/v1/models", self.api))
            .bearer_auth(access)
            .send();
        let (status, body) = self.answered(sent).await?;
        if status != 200 {
            return Err(refused(status, &body));
        }
        let parsed: Value = serde_json::from_str(&body)
            .map_err(|_| Refusal::new(Problem::Unavailable, "unreadable models"))?;
        let listed = parsed["models"].as_array().cloned().unwrap_or_default();
        Ok(listed
            .iter()
            .filter(|model| model["visibility"] == "list")
            .filter_map(|model| {
                let slug = model["slug"].as_str()?.to_string();
                let name = model["display_name"].as_str().unwrap_or(&slug).to_string();
                Some(Model { slug, name })
            })
            .collect())
    }

    /// One answer. `effort` is left out on `None`.
    pub async fn respond(
        &self,
        access: &str,
        model: &str,
        effort: Option<&str>,
        instructions: &str,
        input: &str,
    ) -> std::result::Result<String, Refusal> {
        let mut body = json!({
            "model": model,
            "instructions": instructions,
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": input }],
            }],
            "store": false,
            "stream": true,
        });
        if let Some(effort) = effort {
            body["reasoning"] = json!({ "effort": effort });
        }
        let answer = async {
            let mut response = self
                .client
                .post(format!("{}/v1/responses", self.api))
                .bearer_auth(access)
                .json(&body)
                .send()
                .await
                .map_err(|cause| {
                    Refusal::new(Problem::Unavailable, cause.without_url().to_string())
                })?;
            let status = response.status().as_u16();
            if status != 200 {
                let text = response.text().await.unwrap_or_default();
                return Err(refused(status, &text));
            }
            let mut stream = Stream::default();
            let mut pending = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|cause| {
                Refusal::new(Problem::Unavailable, cause.without_url().to_string())
            })? {
                pending.extend_from_slice(&chunk);
                // A chunk may end inside a character: keep what is not whole yet.
                let whole = match std::str::from_utf8(&pending) {
                    Ok(text) => text.len(),
                    Err(cause) => cause.valid_up_to(),
                };
                stream.feed(&String::from_utf8_lossy(&pending[..whole]));
                pending.drain(..whole);
                if stream.text.len() > ANSWER_LIMIT {
                    return Err(Refusal::new(Problem::Unavailable, "answer too long"));
                }
            }
            match stream.done {
                Some(Ok(())) => Ok(stream.text),
                Some(Err(refusal)) => Err(refusal),
                None => Err(Refusal::new(Problem::Unavailable, "stream ended early")),
            }
        };
        tokio::select! {
            _ = self.lifetime.cancelled() => Err(Refusal::new(Problem::Unavailable, "closing")),
            answer = tokio::time::timeout(ANSWER_TIMEOUT, answer) => match answer {
                Ok(answer) => answer,
                Err(_) => Err(Refusal::new(Problem::Unavailable, "timeout")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn id_token(claims: Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("e30.{payload}.signature")
    }

    fn claims() -> Value {
        json!({
            "iss": AUTH, "aud": "oaiapp_1", "exp": 2000, "nonce": "n",
            "sub": "user", "email": "a@example.test"
        })
    }

    /// The sign-in callback waits on OpenAI's registered port, and on any free one under test:
    /// two test binaries both holding 1455 once failed whichever started second.
    #[tokio::test]
    async fn only_a_real_sign_in_waits_on_the_registered_port() {
        let real = Ai::new(None, CancellationToken::new()).unwrap();
        assert_eq!(real.callback_port(), FIRST_PORT);
        let fixture = Ai::new(Some("http://127.0.0.1:9".into()), CancellationToken::new()).unwrap();
        assert_eq!(fixture.callback_port(), 0);
    }

    /// OpenAI refused a bare random string (`invalid_authorize_request`, `ext_agent_host_id`).
    #[test]
    fn a_host_id_is_a_uuid_uri() {
        let id = new_host_id().unwrap();
        assert!(valid_host_id(&id), "{id}");
        assert_ne!(id, new_host_id().unwrap());
        assert!(!valid_host_id("b7mAFy8G5PmJeX1wdx_DZQwk0eFOg3fR"));
        assert!(!valid_host_id("urn:uuid:1234"));
    }

    #[test]
    fn an_id_token_is_tied_to_this_sign_in() {
        let check = |claims: Value| check_id_token(&id_token(claims), AUTH, "oaiapp_1", "n", 1e6);
        assert_eq!(check(claims()).unwrap().sub, "user");
        for (key, bad) in [
            ("iss", json!("https://evil.test")),
            ("aud", json!("oaiapp_2")),
            ("exp", json!(10)),
            ("nonce", json!("other")),
            ("sub", Value::Null),
        ] {
            let mut forged = claims();
            forged[key] = bad;
            assert!(check(forged).is_err(), "{key}");
        }
        assert!(check_id_token("not a token", AUTH, "oaiapp_1", "n", 1e6).is_err());
    }

    #[test]
    fn only_a_completed_stream_counts() {
        let mut stream = Stream::default();
        stream.feed("event: x\ndata: {\"type\":\"response.output_text.delta\",\"del");
        stream.feed("ta\":\"Hi\"}\n\n");
        assert_eq!(stream.text, "Hi");
        assert_eq!(stream.done, None);
        stream.feed("data: {\"type\":\"response.completed\"}\n");
        assert_eq!(stream.done, Some(Ok(())));
    }

    #[test]
    fn a_stream_can_fail_after_it_started() {
        let mut stream = Stream::default();
        stream.feed(concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":",
            "{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n"
        ));
        let refusal = stream.done.unwrap().unwrap_err();
        assert_eq!(refusal.problem, Problem::Limit);
    }

    #[test]
    fn refusals_are_sorted_by_what_they_ask_of_the_person() {
        let of =
            |code: &str| refused(400, &json!({ "error": { "code": code } }).to_string()).problem;
        assert_eq!(
            of("subscription_sharing_user_not_eligible"),
            Problem::Ineligible
        );
        assert_eq!(of("subscription_sharing_invalid_user"), Problem::Connect);
        assert_eq!(
            of("subscription_sharing_unsupported_capability"),
            Problem::Unsupported
        );
        assert_eq!(
            of("subscription_sharing_usage_unavailable"),
            Problem::Unavailable
        );
        assert_eq!(
            refused(503, "{\"detail\":\"x\"}").problem,
            Problem::Unavailable
        );
        assert_eq!(refused(401, "").problem, Problem::Connect);
        assert_eq!(
            refused(400, "{\"detail\":\"Input must be a list\"}").detail,
            "HTTP 400: Input must be a list"
        );
        let coded = "{\"error\":{\"code\":\"bad\",\"message\":\"No such field\"}}";
        assert_eq!(refused(400, coded).detail, "bad: No such field");
    }

    /// Serves `responses` in turn, one per connection, and hands back the requests.
    async fn serve(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let served = tokio::spawn(async move {
            let mut seen = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let read = socket.read(&mut chunk).await.unwrap();
                    request.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_string)
                            })
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        if body.len() >= length {
                            break;
                        }
                    }
                }
                seen.push(String::from_utf8_lossy(&request).into_owned());
                socket.write_all(response.as_bytes()).await.unwrap();
                tokio::spawn(async move { crate::loopback::close_after_client(&mut socket).await });
            }
            seen
        });
        (origin, served)
    }

    fn http(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    /// The deadline covered only the headers: a server that sent them and stalled the body kept
    /// `models` (and the sign-in refresh lock held around it) waiting forever, even as the core
    /// closed.
    #[tokio::test]
    async fn a_body_that_never_ends_does_not_hold_the_call() {
        let stalled = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"models\":".to_string();
        let (origin, _served) = serve(vec![stalled]).await;
        let lifetime = CancellationToken::new();
        let ai = Ai::new(Some(origin), lifetime.clone()).unwrap();
        let closing = lifetime.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            closing.cancel();
        });
        let answer = tokio::time::timeout(Duration::from_secs(10), ai.models("token"))
            .await
            .expect("the call ends when the core closes");
        assert!(answer.is_err());
    }

    #[tokio::test]
    async fn a_request_sends_what_the_plan_requires() {
        let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Ok\"}\n\ndata: {\"type\":\"response.completed\"}\n\n";
        let (origin, served) = serve(vec![http("200 OK", body)]).await;
        let ai = Ai::new(Some(origin), CancellationToken::new()).unwrap();
        let answer = ai
            .respond("token", "gpt", Some("low"), "rules", "facts")
            .await
            .unwrap();
        assert_eq!(answer, "Ok");
        let request = &served.await.unwrap()[0];
        assert!(request.starts_with("POST /v1/responses"), "{request}");
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer token")
        );
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["stream"], true);
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(body.get("temperature").is_none());
        assert!(body.get("max_output_tokens").is_none());
    }

    #[tokio::test]
    async fn a_stream_cut_short_is_no_answer() {
        let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Half\"}\n\n";
        let (origin, _served) = serve(vec![http("200 OK", body)]).await;
        let ai = Ai::new(Some(origin), CancellationToken::new()).unwrap();
        let refusal = ai.respond("t", "gpt", None, "r", "f").await.unwrap_err();
        assert_eq!(refusal.problem, Problem::Unavailable);
    }

    #[tokio::test]
    async fn only_listed_models_are_offered() {
        let body = json!({ "models": [
            { "slug": "a", "display_name": "Model A", "visibility": "list" },
            { "slug": "b", "display_name": "Hidden", "visibility": "hide" },
        ]})
        .to_string();
        let (origin, _served) = serve(vec![http("200 OK", &body)]).await;
        let ai = Ai::new(Some(origin), CancellationToken::new()).unwrap();
        assert_eq!(
            ai.models("t").await.unwrap(),
            [Model {
                slug: "a".into(),
                name: "Model A".into()
            }]
        );
    }

    #[tokio::test]
    async fn a_refresh_rotates_and_a_dead_one_asks_for_sign_in() {
        let renewed = json!({ "access_token": "a2", "refresh_token": "r2", "expires_in": 3600 });
        let dead = json!({ "error": "refresh_token_reused" }).to_string();
        let (origin, _served) = serve(vec![
            http("200 OK", &renewed.to_string()),
            http("400 Bad Request", &dead),
        ])
        .await;
        let ai = Ai::new(Some(origin), CancellationToken::new()).unwrap();
        let old = Tokens {
            access: "a1".into(),
            refresh: "r1".into(),
            expires_at: 0.0,
        };
        let tokens = ai.refresh(&old, "oaiapp_1", 1000.0).await.unwrap();
        assert_eq!(tokens.refresh, "r2");
        assert_eq!(tokens.expires_at, 3_601_000.0);
        let refusal = ai.refresh(&tokens, "oaiapp_1", 2000.0).await.unwrap_err();
        assert_eq!(refusal.problem, Problem::Connect);
    }

    #[tokio::test]
    async fn a_passing_outage_keeps_the_sign_in() {
        let (origin, _served) = serve(vec![http("503 Service Unavailable", "{}")]).await;
        let ai = Ai::new(Some(origin), CancellationToken::new()).unwrap();
        let old = Tokens {
            access: "a".into(),
            refresh: "r".into(),
            expires_at: 0.0,
        };
        let refusal = ai.refresh(&old, "c", 0.0).await.unwrap_err();
        assert_eq!(refusal.problem, Problem::Unavailable);
    }

    #[test]
    fn the_first_sign_in_registers_and_later_ones_do_not() {
        let ai = Ai::new(None, CancellationToken::new()).unwrap();
        let attempt = |client_id| Attempt {
            client_id,
            host_id: "host",
            redirect: "http://127.0.0.1:1455/auth/callback",
            state: "s",
            nonce: "n",
            challenge: "c",
        };
        let first = ai.authorize_url(&attempt(None)).unwrap();
        assert!(first.contains("client_id=dynamic_agent_client"));
        assert!(first.contains("agent_name_hint=KChess"));
        assert!(first.contains("chatgpt.tokens.use.direct"));
        let later = ai.authorize_url(&attempt(Some("oaiapp_1"))).unwrap();
        assert!(later.contains("client_id=oaiapp_1"));
        assert!(!later.contains("agent_name_hint"));
    }
}

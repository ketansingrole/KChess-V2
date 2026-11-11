//! The facade's ChatGPT methods: connecting a plan, the model and effort every AI feature uses,
//! and explanations. Explanations are engine assistance, so a live game refuses them.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::net::TcpListener;

use kchess_contracts::core_api::{AiApi, Assist, EngineApi};
use kchess_domain::misc::checks;

use super::args::normal;

use crate::ai::{
    Attempt, CALLBACK_PATH, FIRST_PORT, Model, Problem, Refusal, Tokens, new_host_id, valid_host_id,
};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::Level;
use crate::ids::hex;
use crate::lichess::oauth::{Redirect, default_look, new_pkce, new_state, wait_for_redirect};
use crate::time::now_ms;
use kchess_domain::coach::{self, Brief};
use kchess_domain::records::oauth::connection_page;
use kchess_domain::tactics::Mates;

const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
const UNREADABLE: &str = "The ChatGPT connection saved in this profile could not be read.";
/// How long the engine looks for the answer to a wrong puzzle move, in milliseconds.
const REFUTATION_TIME: u64 = 500;

/// The connection document.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Connection {
    host_id: String,
    client_id: Option<String>,
    email: Option<String>,
    /// `Tokens` as JSON, sealed.
    sealed: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    /// The chosen model refused an effort; requests leave it out.
    effort_refused: bool,
}

/// What kind of explanation is wanted.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum Kind {
    Move,
    Nudge,
    Solution,
    Wrong,
}

/// What `aiExplain` is asked to explain.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    kind: Kind,
    fen: String,
    #[serde(default)]
    played: Option<String>,
    #[serde(default)]
    moves: Vec<String>,
    #[serde(default)]
    best: Vec<String>,
    #[serde(default)]
    refutation: Vec<String>,
    #[serde(default)]
    mate_before: Option<f64>,
    #[serde(default)]
    mate_after: Option<f64>,
}

fn problem(refusal: Problem) -> Value {
    json!({ "problem": refusal })
}

impl Core {
    /// The saved connection; none yet is the default. One this release cannot read (damaged,
    /// or written by another release) is an error, so nothing is saved over it.
    async fn ai_connection(&self) -> Result<Connection> {
        let stored = self
            .database
            .read_async(crate::store::ai::connection)
            .await?;
        let Some(text) = stored else {
            return Ok(Connection::default());
        };
        serde_json::from_str(&text).map_err(|cause| {
            self.host
                .log(Level::Warn, "ai", &format!("{UNREADABLE} {cause}"));
            CoreError::new(UNREADABLE)
        })
    }

    async fn save_ai_connection(&self, connection: &Connection) -> Result<()> {
        let value = serde_json::to_value(connection)
            .map_err(|_| CoreError::new("ChatGPT connection could not be saved."))?;
        self.database
            .write_async(move |ctx| crate::store::ai::save_connection(ctx, &value))
            .await
    }

    async fn seal_tokens(&self, tokens: &Tokens) -> Result<String> {
        let text = serde_json::to_string(tokens)
            .map_err(|_| CoreError::new("ChatGPT connection could not be saved."))?;
        self.capabilities.encrypt(&text).await
    }

    /// A usable access token, renewed when due. A sign-in OpenAI ended is forgotten.
    async fn ai_access(&self) -> std::result::Result<String, Refusal> {
        let unavailable = |detail: &str| Refusal {
            problem: Problem::Unavailable,
            detail: detail.to_string(),
        };
        let _renewing = self.ai.refreshing.lock().await;
        let mut connection = self
            .ai_connection()
            .await
            .map_err(|_| unavailable("read"))?;
        let (Some(sealed), Some(client_id)) =
            (connection.sealed.clone(), connection.client_id.clone())
        else {
            return Err(Refusal {
                problem: Problem::Connect,
                detail: "not connected".into(),
            });
        };
        let text = self
            .capabilities
            .decrypt(&sealed)
            .await
            .map_err(|_| unavailable("locked"))?;
        let tokens: Tokens = serde_json::from_str(&text).map_err(|_| unavailable("tokens"))?;
        let now = now_ms() as f64;
        if tokens.fresh(now) {
            return Ok(tokens.access);
        }
        match self.ai.refresh(&tokens, &client_id, now).await {
            Ok(renewed) => {
                connection.sealed = Some(
                    self.seal_tokens(&renewed)
                        .await
                        .map_err(|_| unavailable("seal"))?,
                );
                self.save_ai_connection(&connection)
                    .await
                    .map_err(|_| unavailable("save"))?;
                Ok(renewed.access)
            }
            Err(refusal) => {
                if refusal.problem == Problem::Connect {
                    connection.sealed = None;
                    if let Err(cause) = self.save_ai_connection(&connection).await {
                        self.host.log(
                            Level::Warn,
                            "ai",
                            &format!("Save failed: {}", cause.message),
                        );
                    }
                }
                Err(refusal)
            }
        }
    }

    /// Signs a ChatGPT account in through the browser.
    async fn ai_sign_in(&self) -> Result<Value> {
        let mut connection = self.ai_connection().await?;
        if !valid_host_id(&connection.host_id) {
            connection.host_id = new_host_id()?;
            self.save_ai_connection(&connection).await?;
        }
        let pkce = new_pkce()?;
        let (state, nonce) = (new_state()?, new_state()?);
        let listener = match TcpListener::bind(("127.0.0.1", self.ai.callback_port())).await {
            Ok(listener) => listener,
            Err(_) if connection.client_id.is_some() => TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|_| CoreError::new("Could not open the sign-in callback."))?,
            Err(_) => {
                return Err(CoreError::new(format!(
                    "Port {FIRST_PORT} is busy. Close what uses it and retry."
                )));
            }
        };
        let port = listener
            .local_addr()
            .map_err(|_| CoreError::new("Could not open the sign-in callback."))?
            .port();
        let redirect = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let url = self.ai.authorize_url(&Attempt {
            client_id: connection.client_id.as_deref(),
            host_id: &connection.host_id,
            redirect: &redirect,
            state: &state,
            nonce: &nonce,
            challenge: &pkce.challenge,
        })?;
        let look = default_look();
        let page = |authorized: bool| connection_page(authorized, &look, "ChatGPT");
        let opening = async {
            self.capabilities.open_external(&url).await?;
            std::future::pending::<Result<crate::lichess::oauth::Callback>>().await
        };
        let returned = tokio::select! {
            returned = wait_for_redirect(
                &listener,
                Redirect {
                    path: CALLBACK_PATH,
                    state: &state,
                    page: &page,
                    timed_out: "ChatGPT sign-in timed out.",
                    timeout: SIGN_IN_TIMEOUT,
                },
                self.ai.lifetime(),
                &|| {},
            ) => returned?,
            failed = opening => failed?,
        };
        if let Err(cause) = self.capabilities.focus().await {
            self.host.log(
                Level::Warn,
                "ai",
                &format!("Could not focus: {}", cause.message),
            );
        }
        let code = returned
            .code
            .ok_or_else(|| CoreError::new("ChatGPT sign-in was cancelled."))?;
        let client_id = match (&connection.client_id, returned.client_id) {
            (Some(known), Some(issued)) if *known != issued => {
                return Err(CoreError::new("ChatGPT returned a different registration."));
            }
            (Some(known), _) => known.clone(),
            (None, Some(issued)) if issued != "dynamic_agent_client" => issued,
            _ => return Err(CoreError::new("ChatGPT registration did not finish.")),
        };
        let signed = self
            .ai
            .exchange(
                &code,
                &pkce.verifier,
                &redirect,
                &client_id,
                &nonce,
                now_ms() as f64,
            )
            .await?;
        let sealed = self.seal_tokens(&signed.tokens).await?;
        // The sign-in took minutes: merge into the connection as saved now, so a model or effort
        // chosen meanwhile (in this window or another) is kept.
        let renewing = self.ai.refreshing.lock().await;
        let mut saved = self.ai_connection().await?;
        saved.host_id = connection.host_id;
        saved.sealed = Some(sealed);
        saved.client_id = Some(client_id);
        saved.email = signed.identity.email;
        self.save_ai_connection(&saved).await?;
        drop(renewing);
        self.ai_state().await
    }

    async fn ai_model_list(&self) -> std::result::Result<Vec<Model>, Refusal> {
        let access = self.ai_access().await?;
        self.ai.models(&access).await
    }

    async fn ai_state(&self) -> Result<Value> {
        let connection = self.ai_connection().await?;
        Ok(json!({
            "connected": connection.sealed.is_some(),
            "registered": connection.client_id.is_some(),
            "email": connection.email,
            "model": connection.model,
            "effort": connection.effort.as_deref().unwrap_or("auto"),
            "effortRefused": connection.effort_refused,
        }))
    }

    /// The answer of `brief`, checked, with one more try after a reply outside the facts.
    async fn ai_answer(
        &self,
        connection: &mut Connection,
        model: &str,
        brief: &Brief,
    ) -> std::result::Result<String, Refusal> {
        let access = self.ai_access().await?;
        let mut input = brief.input.clone();
        for _ in 0..2 {
            let effort = connection
                .effort
                .as_deref()
                .filter(|effort| *effort != "auto" && !connection.effort_refused);
            let reply = self
                .ai
                .respond(&access, model, effort, brief.instructions, &input)
                .await;
            let text = match reply {
                Err(refusal) if refusal.problem == Problem::Unsupported && effort.is_some() => {
                    let (chosen, refused) = (connection.model.clone(), connection.effort.clone());
                    connection.effort_refused = true;
                    if let Err(cause) = self.refuse_ai_effort(chosen, refused).await {
                        self.host.log(
                            Level::Warn,
                            "ai",
                            &format!("Save failed: {}", cause.message),
                        );
                    }
                    self.ai
                        .respond(&access, model, None, brief.instructions, &input)
                        .await?
                }
                reply => reply?,
            };
            if coach::check_reply(&text, &brief.allowed) {
                return Ok(text.trim().to_string());
            }
            self.host.log(
                Level::Debug,
                "ai",
                "An answer named something outside the facts.",
            );
            input = format!(
                "{}\nYour last answer named something outside these facts. Keep strictly to them.",
                brief.input
            );
        }
        Err(Refusal {
            problem: Problem::Unverified,
            detail: "outside the facts".into(),
        })
    }

    /// Records that the plan refused the effort chosen with `model`, on the connection as saved
    /// now: the copy an explanation read before it may hold tokens a refresh has since replaced.
    /// A model or effort chosen since is not marked refused.
    async fn refuse_ai_effort(&self, model: Option<String>, effort: Option<String>) -> Result<()> {
        let _renewing = self.ai.refreshing.lock().await;
        let mut connection = self.ai_connection().await?;
        if connection.model != model || connection.effort != effort {
            return Ok(());
        }
        connection.effort_refused = true;
        self.save_ai_connection(&connection).await
    }

    /// The engine's reply to `played` in `fen`, when an engine can give one.
    async fn ai_refutation(&self, fen: &str, played: &str) -> Vec<String> {
        let Some(mut after) = kchess_domain::rules::standard_from_fen(fen) else {
            return Vec::new();
        };
        let Some(chosen) = kchess_domain::rules::parse_uci(played)
            .and_then(|uci| kchess_domain::rules::legal_move(&after, uci))
        else {
            return Vec::new();
        };
        kchess_domain::rules::san_and_play(&mut after, chosen);
        let options = json!({
            "fen": kchess_domain::rules::make_fen(&after),
            "movetime": REFUTATION_TIME,
        });
        let reply = async {
            self.assistance_allowed(Assist::Engine.refusal())?;
            let options = normal(checks::best_move_options, &options)?;
            EngineApi::best_move(self, json!([]), json!("club"), Some(options)).await
        };
        match reply.await {
            Ok(Value::String(reply)) => vec![reply],
            Ok(_) => Vec::new(),
            Err(cause) => {
                self.host.log(
                    Level::Debug,
                    "ai",
                    &format!("No refutation: {}", cause.message),
                );
                Vec::new()
            }
        }
    }

    async fn ai_brief(&self, request: &Request) -> Result<Brief> {
        let invalid = || CoreError::new("That position cannot be explained.");
        let (fen, moves) = (request.fen.as_str(), &request.moves);
        match request.kind {
            Kind::Move => {
                let played = request.played.as_deref().ok_or_else(invalid)?;
                let mates = Mates {
                    before: request.mate_before.map(|mate| mate as i32),
                    after: request.mate_after.map(|mate| mate as i32),
                };
                coach::move_brief(fen, played, &request.best, &request.refutation, mates)
                    .map(|(_, brief)| brief)
            }
            Kind::Nudge => coach::nudge_brief(fen, moves),
            Kind::Solution => coach::solution_brief(fen, moves),
            Kind::Wrong => {
                let played = request.played.as_deref().ok_or_else(invalid)?;
                let refutation = self.ai_refutation(fen, played).await;
                coach::wrong_brief(fen, moves, played, &refutation)
            }
        }
        .ok_or_else(invalid)
    }

    async fn explain(&self, raw: Value) -> Result<Value> {
        let request: Request = serde_json::from_value(raw)
            .map_err(|_| CoreError::new("That position cannot be explained."))?;
        let brief = self.ai_brief(&request).await?;
        let mut connection = self.ai_connection().await?;
        if connection.sealed.is_none() {
            return Ok(problem(Problem::Connect));
        }
        let model = match connection.model.clone() {
            Some(model) => model,
            None => match self.ai_model_list().await {
                Ok(models) => match models.into_iter().next() {
                    Some(first) => first.slug,
                    None => return Ok(problem(Problem::Ineligible)),
                },
                Err(refusal) => return Ok(problem(refusal.problem)),
            },
        };
        let key = hex(Sha256::digest(format!("{model}\n{}", brief.input).as_bytes()).as_slice());
        let lookup = key.clone();
        let cached = self
            .database
            .read_async(move |ctx| crate::store::ai::explanation(ctx, &lookup))
            .await?;
        if cached.is_string() {
            return Ok(json!({ "text": cached }));
        }
        match self.ai_answer(&mut connection, &model, &brief).await {
            Ok(text) => {
                let (key, answer) = (json!(key), json!(text));
                let saved = self
                    .database
                    .write_async(move |ctx| crate::store::ai::save_explanation(ctx, key, answer))
                    .await;
                if let Err(cause) = saved {
                    self.host
                        .log(Level::Warn, "ai", &format!("Not saved: {}", cause.message));
                }
                Ok(json!({ "text": text }))
            }
            Err(refusal) => {
                self.host
                    .log(Level::Warn, "ai", &format!("Refused: {}", refusal.detail));
                Ok(problem(refusal.problem))
            }
        }
    }
}

impl AiApi for Core {
    async fn ai_status(&self) -> Result<Value> {
        self.ai_state().await
    }

    async fn ai_connect(&self) -> Result<Value> {
        self.ai_sign_in().await
    }

    async fn ai_disconnect(&self) -> Result<Value> {
        let _renewing = self.ai.refreshing.lock().await;
        let mut connection = self.ai_connection().await?;
        connection.sealed = None;
        connection.email = None;
        self.save_ai_connection(&connection).await?;
        self.ai_state().await
    }

    async fn ai_models(&self) -> Result<Value> {
        match self.ai_model_list().await {
            Ok(models) => Ok(json!({ "models": models })),
            Err(refusal) => {
                self.host
                    .log(Level::Warn, "ai", &format!("Refused: {}", refusal.detail));
                Ok(problem(refusal.problem))
            }
        }
    }

    async fn ai_set_options(&self, model: Value, effort: Value) -> Result<Value> {
        let _renewing = self.ai.refreshing.lock().await;
        let mut connection = self.ai_connection().await?;
        let model = model.as_str().map(str::to_string);
        let effort = effort.as_str().map(str::to_string);
        connection.effort_refused &= connection.model == model && connection.effort == effort;
        connection.model = model;
        connection.effort = effort;
        self.save_ai_connection(&connection).await?;
        self.ai_state().await
    }

    // Assistance: the dispatch refuses it during a live game, before and after.
    async fn ai_explain(&self, request: Value) -> Result<Value> {
        self.explain(request).await
    }
}

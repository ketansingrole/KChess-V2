//! A plan that refuses the chosen effort, after the explanation's access renewed the tokens: the
//! refusal is recorded without bringing back the refresh token the renewal used up, and only for
//! the effort it refused; once refused, the effort is no longer sent. A wrong puzzle move is
//! explained with the engine's reply to it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::fixtures::{Fixture, Recording, Reply, Request, default_answers, http_client, serve};
use crate::support;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use kchess_core::{Config, Core, TestSwitches};
use serde_json::{Value, json};
use tokio::sync::watch;

const FORK: &str = "r3k3/pp3ppp/8/1N6/8/8/PPP2PPP/4K3 w - - 0 1";

fn stream(text: &str) -> Reply {
    let delta = json!({ "type": "response.output_text.delta", "delta": text });
    let body = format!("data: {delta}\n\ndata: {{\"type\":\"response.completed\"}}\n\n");
    Reply::Text(200, "text/event-stream", body)
}

fn request(played: &str) -> Value {
    let review = json!({
        "key": "k", "source": "local", "depth": 18, "complete": true, "updatedAt": 0,
        "fen": FORK,
        "moves": [played],
        "evals": [
            { "cp": 500, "mate": null, "pv": ["b5c7", "e8d7", "c7a8"] },
            { "cp": 0, "pv": ["e8d7"] },
        ],
    });
    let review = serde_json::from_value(review).expect("a stored review");
    kchess_domain::coach::review_moment(&review, 0).unwrap()["request"].clone()
}

/// The form field `name` of an `application/x-www-form-urlencoded` body.
fn field(body: &str, name: &str) -> Option<String> {
    reqwest::Url::parse(&format!("http://form/?{body}"))
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// A profile signed in to the plan at a local fixture, whose `/v1/responses` answers come from
/// `responses`; every access renews the tokens, and `renewals` is the refresh token each renewal
/// sent, in order.
struct Plan {
    fixture: Fixture,
    core: Arc<Core>,
    renewals: Arc<Mutex<Vec<String>>>,
    /// While set, a renewal's answer waits for it to open.
    renewal_gate: Arc<Mutex<Option<watch::Receiver<bool>>>>,
    dir: tempfile::TempDir,
}

async fn signed_in_plan(responses: impl Fn(&Request) -> Reply + Send + 'static) -> Plan {
    let nonce = Arc::new(Mutex::new(String::new()));
    let origin = Arc::new(Mutex::new(String::new()));
    let renewals = Arc::new(Mutex::new(Vec::<String>::new()));
    let renewal_gate = Arc::new(Mutex::new(None::<watch::Receiver<bool>>));
    let fixture = {
        let (nonce, origin, renewals, gate) = (
            Arc::clone(&nonce),
            Arc::clone(&origin),
            Arc::clone(&renewals),
            Arc::clone(&renewal_gate),
        );
        serve(move |seen| match seen.path() {
            "/api/accounts/oauth/token" => {
                let refresh = match field(&seen.body, "refresh_token") {
                    Some(used) => {
                        let mut renewals = renewals.lock().unwrap();
                        renewals.push(used);
                        format!("refresh-{}", renewals.len() + 1)
                    }
                    None => "refresh-1".to_string(),
                };
                let claims = json!({
                    "iss": *origin.lock().unwrap(),
                    "aud": "oaiapp_1",
                    "exp": 4_000_000_000u64,
                    "nonce": *nonce.lock().unwrap(),
                    "sub": "user-1",
                    "email": "player@example.test",
                });
                let id_token = format!(
                    "e30.{}.signature",
                    URL_SAFE_NO_PAD.encode(claims.to_string())
                );
                // Expired at once: every access renews.
                let answer = Reply::Json(
                    200,
                    json!({
                        "access_token": "access-secret",
                        "refresh_token": refresh,
                        "id_token": id_token,
                        "expires_in": 0,
                        "scope": "openid chatgpt.tokens.use.direct",
                    })
                    .to_string(),
                );
                match gate.lock().unwrap().clone() {
                    Some(gated) if field(&seen.body, "refresh_token").is_some() => {
                        Reply::Gated(gated, Box::new(answer))
                    }
                    _ => answer,
                }
            }
            "/v1/responses" => responses(seen),
            _ => Reply::Json(404, "{}".into()),
        })
        .await
    };
    *origin.lock().unwrap() = fixture.base.clone();
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Recording::default());
    *host.answers.lock().unwrap() = default_answers();
    {
        let nonce = Arc::clone(&nonce);
        *host.browser.lock().unwrap() = Some(Box::new(move |url: String| {
            let url = reqwest::Url::parse(&url).unwrap();
            let query = |name: &str| {
                url.query_pairs()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.into_owned())
                    .unwrap_or_default()
            };
            *nonce.lock().unwrap() = query("nonce");
            let back = format!(
                "{}?code=c&client_id=oaiapp_1&state={}",
                query("redirect_uri"),
                query("state")
            );
            tokio::spawn(async move {
                http_client().get(back).send().await.expect("callback");
            });
        }));
    }
    let config = Config {
        system_credentials: true,
        test: Some(TestSwitches {
            openai_origin: Some(fixture.base.clone()),
            ..TestSwitches::default()
        }),
        ..Config::new(dir.path().to_path_buf())
    };
    let core =
        Arc::new(Core::open(config, Arc::clone(&host) as Arc<dyn kchess_core::Host>).unwrap());
    *host.capabilities.lock().unwrap() = Some(core.capabilities());
    Plan {
        fixture,
        core,
        renewals,
        renewal_gate,
        dir,
    }
}

/// The plan refuses any request that names an effort.
fn refusing_effort(seen: &Request, hold: &AtomicBool, gated: &watch::Receiver<bool>) -> Reply {
    let sent: Value = serde_json::from_str(&seen.body).unwrap_or_default();
    if sent["reasoning"]["effort"].is_string() {
        let refusal = Reply::Json(
            400,
            json!({ "error": {
                "code": "subscription_sharing_unsupported_capability"
            }})
            .to_string(),
        );
        if hold.load(Ordering::SeqCst) {
            return Reply::Gated(gated.clone(), Box::new(refusal));
        }
        return refusal;
    }
    stream("You missed Nc7+, forking the Ke8 and the Ra8.")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_effort_keeps_the_renewed_tokens() {
    // While set, a refusal waits for the gate to open.
    let hold = Arc::new(AtomicBool::new(false));
    let (gate, gated) = watch::channel(false);
    let Plan {
        fixture,
        core,
        renewals,
        dir: _dir,
        ..
    } = {
        let hold = Arc::clone(&hold);
        signed_in_plan(move |seen| refusing_effort(seen, &hold, &gated)).await
    };
    core.call("resumeOnline", vec![]).await.unwrap();
    let status = core.call("aiConnect", vec![]).await.unwrap();
    assert_eq!(status["connected"], true);
    core.call("aiSetOptions", vec![json!("gpt-a"), json!("low")])
        .await
        .unwrap();

    // The access renews refresh-1 into refresh-2, then the plan refuses the effort.
    core.call("aiExplain", vec![request("e1e2")]).await.unwrap();
    let status = core.call("aiStatus", vec![]).await.unwrap();
    assert_eq!(status["effortRefused"], true);
    assert_eq!(status["connected"], true);
    // Choosing the same model and effort again keeps them refused.
    let status = core
        .call("aiSetOptions", vec![json!("gpt-a"), json!("low")])
        .await
        .unwrap();
    assert_eq!(status["effortRefused"], true);
    // The next access renews refresh-2: refresh-1 was used up, and sending it again would end
    // the sign-in. The refused effort is not sent again: one request, without it.
    let asked = fixture.to("/v1/responses").len();
    core.call("aiExplain", vec![request("e1d1")]).await.unwrap();
    let sent = fixture.to("/v1/responses");
    assert_eq!(sent.len(), asked + 1);
    let body: Value = serde_json::from_str(&sent[asked].body).unwrap();
    assert!(body["reasoning"]["effort"].is_null(), "{body}");
    assert_eq!(*renewals.lock().unwrap(), ["refresh-1", "refresh-2"]);

    // A refusal that arrives after another effort was chosen leaves the new one alone.
    core.call("aiSetOptions", vec![json!("gpt-a"), json!("medium")])
        .await
        .unwrap();
    hold.store(true, Ordering::SeqCst);
    let asked = fixture.to("/v1/responses").len();
    let explaining = tokio::spawn({
        let core = Arc::clone(&core);
        async move { core.call("aiExplain", vec![request("e1f1")]).await }
    });
    fixture.requested("/v1/responses", asked + 1).await;
    core.call("aiSetOptions", vec![json!("gpt-a"), json!("high")])
        .await
        .unwrap();
    gate.send(true).unwrap();
    explaining.await.unwrap().unwrap();
    let status = core.call("aiStatus", vec![]).await.unwrap();
    assert_eq!(status["effort"], "high");
    assert_eq!(status["effortRefused"], false);
}

/// An access whose caller goes away (its window closed) while OpenAI answers the renewal still
/// saves the renewed tokens: OpenAI had used up the old refresh token, which was all the profile
/// kept, and the next access sent it again and ended the sign-in.
#[tokio::test(flavor = "multi_thread")]
async fn a_renewal_whose_caller_went_away_is_still_saved() {
    let Plan {
        fixture,
        core,
        renewals,
        renewal_gate,
        dir: _dir,
    } = signed_in_plan(|_| stream("Nc7+ forks the king and rook.")).await;
    core.call("resumeOnline", vec![]).await.unwrap();
    let status = core.call("aiConnect", vec![]).await.unwrap();
    assert_eq!(status["connected"], true);
    let (gate, gated) = watch::channel(false);
    *renewal_gate.lock().unwrap() = Some(gated);
    // Counted before the explanation starts: its renewal can reach the fixture before this line
    // would, and a count taken after it waits for one more renewal that never comes.
    let asked = fixture.to("/api/accounts/oauth/token").len();
    let explaining = tokio::spawn({
        let core = Arc::clone(&core);
        async move { core.call("aiExplain", vec![request("e1e2")]).await }
    });
    // OpenAI has the old token and renews it; the caller goes before the answer comes.
    fixture
        .requested("/api/accounts/oauth/token", asked + 1)
        .await;
    explaining.abort();
    let _ = explaining.await;
    *renewal_gate.lock().unwrap() = None;
    gate.send(true).unwrap();
    core.call("aiExplain", vec![request("e1d1")]).await.unwrap();
    assert_eq!(*renewals.lock().unwrap(), ["refresh-1", "refresh-2"]);
}

/// A wrong puzzle move is explained with the engine's reply to it: the engine is asked about the
/// position after the move, briefly, and its mate goes to the plan with the facts.
#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_move_is_explained_with_the_engines_reply() {
    let Plan {
        fixture, core, dir, ..
    } = signed_in_plan(|_| stream("That bishop move leaves your back rank open.")).await;
    let engine = support::engine_copy(dir.path(), "search");
    support::save_engine_path(&core, &engine);
    core.call("resumeOnline", vec![]).await.unwrap();
    core.call("aiConnect", vec![]).await.unwrap();
    core.call("aiSetOptions", vec![json!("gpt-a"), json!("auto")])
        .await
        .unwrap();
    // Black to move; after Bc3 White mates with Ra8 (the scripted engine's move here).
    let wrong = json!({
        "kind": "wrong",
        "fen": "6k1/5ppp/8/8/8/8/1b6/R5K1 b - - 0 1",
        "played": "b2c3",
        "moves": ["h7h6"],
    });
    let told = core.call("aiExplain", vec![wrong]).await.unwrap();
    assert_eq!(
        told["text"],
        json!("That bishop move leaves your back rank open.")
    );
    let asked = support::received(&support::copy_log(&engine));
    assert!(
        asked
            .iter()
            .any(|line| line.starts_with("position fen 6k1/5ppp/8/8/8/2b5/8/R5K1 w ")),
        "{asked:?}"
    );
    assert!(
        asked.iter().any(|line| line == "go movetime 500"),
        "{asked:?}"
    );
    let sent = fixture.to("/v1/responses");
    assert!(sent[0].body.contains("Ra8"), "{}", sent[0].body);
    core.close().await;
}

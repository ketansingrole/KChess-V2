//! A plan that refuses the chosen effort, after the explanation's access renewed the tokens: the
//! refusal is recorded without bringing back the refresh token the renewal used up, and only for
//! the effort it refused.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::fixtures::{Recording, Reply, default_answers, http_client, serve};
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

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_effort_keeps_the_renewed_tokens() {
    let nonce = Arc::new(Mutex::new(String::new()));
    let origin = Arc::new(Mutex::new(String::new()));
    // The refresh token each renewal sent, in order.
    let renewals = Arc::new(Mutex::new(Vec::<String>::new()));
    // While set, a refusal waits for the gate to open.
    let hold = Arc::new(AtomicBool::new(false));
    let (gate, gated) = watch::channel(false);
    let fixture = {
        let (nonce, origin, renewals, hold) = (
            Arc::clone(&nonce),
            Arc::clone(&origin),
            Arc::clone(&renewals),
            Arc::clone(&hold),
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
                Reply::Json(
                    200,
                    json!({
                        "access_token": "access-secret",
                        "refresh_token": refresh,
                        "id_token": id_token,
                        "expires_in": 0,
                        "scope": "openid chatgpt.tokens.use.direct",
                    })
                    .to_string(),
                )
            }
            "/v1/responses" => {
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
    // The next access renews refresh-2: refresh-1 was used up, and sending it again would end
    // the sign-in.
    core.call("aiExplain", vec![request("e1d1")]).await.unwrap();
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

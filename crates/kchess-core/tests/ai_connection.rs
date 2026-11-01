//! A ChatGPT plan from sign-in to an explanation, over a local fixture of OpenAI: the browser's
//! trip back, sealed tokens, the model and effort sent, a reply outside the facts thrown away,
//! the saved answer, a spent allowance, a live game, and disconnecting.
// Tests set process environment (`std::env::set_var`), which is unsafe in edition 2024.
#![allow(unsafe_code)]
#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fixtures::{Recording, Reply, default_answers, http_client, serve};
use kchess_core::{Config, Core};
use serde_json::{Value, json};

const FORK: &str = "r3k3/pp3ppp/8/1N6/8/8/PPP2PPP/4K3 w - - 0 1";

fn stream(text: &str) -> Reply {
    let delta = json!({ "type": "response.output_text.delta", "delta": text });
    let body = format!("data: {delta}\n\ndata: {{\"type\":\"response.completed\"}}\n\n");
    Reply::Text(200, "text/event-stream", body)
}

fn request(played: &str) -> Value {
    json!({
        "kind": "move",
        "fen": FORK,
        "played": played,
        "best": ["b5c7", "e8d7", "c7a8"],
        "refutation": ["e8d7"],
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_connects_explains_and_disconnects() {
    let nonce = Arc::new(Mutex::new(String::new()));
    let answers = Arc::new(AtomicUsize::new(0));
    let spent = Arc::new(AtomicBool::new(false));
    let origin = Arc::new(Mutex::new(String::new()));
    let seen_bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let fixture = {
        let (nonce, answers, spent, origin, bodies) = (
            Arc::clone(&nonce),
            Arc::clone(&answers),
            Arc::clone(&spent),
            Arc::clone(&origin),
            Arc::clone(&seen_bodies),
        );
        serve(move |seen| match seen.path() {
            "/api/accounts/oauth/token" => {
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
                Reply::Json(
                    200,
                    json!({
                        "access_token": "access-secret",
                        "refresh_token": "refresh-secret",
                        "id_token": id_token,
                        "expires_in": 3600,
                        "scope": "openid chatgpt.tokens.use.direct",
                    })
                    .to_string(),
                )
            }
            "/v1/models" => Reply::Json(
                200,
                json!({ "models": [
                    { "slug": "gpt-a", "display_name": "A", "visibility": "list" }
                ]})
                .to_string(),
            ),
            "/v1/responses" => {
                bodies.lock().unwrap().push(seen.body.clone());
                if spent.load(Ordering::SeqCst) {
                    return Reply::Json(
                        429,
                        json!({ "error": {
                            "code": "subscription_sharing_usage_limit_exceeded"
                        }})
                        .to_string(),
                    );
                }
                if answers.fetch_add(1, Ordering::SeqCst) == 0 {
                    stream("Bxf7+ was the winning move.")
                } else {
                    stream("You missed Nc7+, forking the Ke8 and the Ra8.")
                }
            }
            _ => Reply::Json(404, "{}".into()),
        })
        .await
    };
    *origin.lock().unwrap() = fixture.base.clone();
    // SAFETY: this test binary has one test; the fixture origin is set before opening the core.
    unsafe {
        std::env::set_var("KCHESS_STORE_DEBUG", "1");
        std::env::set_var("KCHESS_TEST_OPENAI_BASE", &fixture.base);
    }
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
            assert_eq!(query("client_id"), "dynamic_agent_client");
            assert!(query("ext_agent_host_id").starts_with("urn:uuid:"));
            assert!(query("scope").contains("chatgpt.tokens.use.direct"));
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
    let mut config = Config::new(dir.path().to_path_buf());
    config.system_credentials = true;
    let core = Core::open(config, Arc::clone(&host) as Arc<dyn kchess_core::Host>).unwrap();
    *host.capabilities.lock().unwrap() = Some(core.capabilities());

    // Nothing is connected, and a game may still be in progress.
    let status = core.call("aiStatus", vec![]).await.unwrap();
    assert_eq!(status["connected"], false);
    let blocked = core.call("aiExplain", vec![request("e1e2")]).await;
    assert!(blocked.unwrap_err().message.contains("live Lichess game"));
    core.call("resumeOnline", vec![]).await.unwrap();
    let unconnected = core.call("aiExplain", vec![request("e1e2")]).await.unwrap();
    assert_eq!(unconnected["problem"], "connect");

    let status = core.call("aiConnect", vec![]).await.unwrap();
    assert_eq!(status["connected"], true);
    assert_eq!(status["email"], "player@example.test");
    let models = core.call("aiModels", vec![]).await.unwrap();
    assert_eq!(models["models"][0]["slug"], "gpt-a");
    let status = core
        .call("aiSetOptions", vec![json!("gpt-a"), json!("low")])
        .await
        .unwrap();
    assert_eq!(status["effort"], "low");

    // The invented move is thrown away and the second, faithful answer shown.
    let told = core.call("aiExplain", vec![request("e1e2")]).await.unwrap();
    assert!(told["text"].as_str().unwrap().contains("Nc7+"), "{told}");
    assert_eq!(answers.load(Ordering::SeqCst), 2);
    let sent: Value = serde_json::from_str(&seen_bodies.lock().unwrap()[0]).unwrap();
    assert_eq!(sent["model"], "gpt-a");
    assert_eq!(sent["reasoning"]["effort"], "low");
    assert_eq!(sent["store"], false);

    // Asked again, it costs nothing.
    let again = core.call("aiExplain", vec![request("e1e2")]).await.unwrap();
    assert_eq!(again, told);
    assert_eq!(answers.load(Ordering::SeqCst), 2);

    // A spent allowance is reported, once, and keeps the sign-in.
    spent.store(true, Ordering::SeqCst);
    let before = seen_bodies.lock().unwrap().len();
    let limited = core.call("aiExplain", vec![request("e1d1")]).await.unwrap();
    assert_eq!(limited["problem"], "limit");
    assert_eq!(seen_bodies.lock().unwrap().len(), before + 1);
    let status = core.call("aiStatus", vec![]).await.unwrap();
    assert_eq!(status["connected"], true);

    // No token lies in the profile in the clear.
    let database = std::fs::read(dir.path().join("kchess.db")).unwrap();
    let text = String::from_utf8_lossy(&database);
    assert!(!text.contains("access-secret") && !text.contains("refresh-secret"));

    let status = core.call("aiDisconnect", vec![]).await.unwrap();
    assert_eq!(status["connected"], false);
    assert_eq!(status["registered"], true);
    let after = core.call("aiExplain", vec![request("e1d1")]).await.unwrap();
    assert_eq!(after["problem"], "connect");
}

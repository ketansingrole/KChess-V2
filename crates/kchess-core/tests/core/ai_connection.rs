//! A ChatGPT plan from sign-in to an explanation, over a local fixture of OpenAI: the browser's
//! trip back, sealed tokens, the model and effort sent, a reply outside the facts thrown away,
//! the saved answer, a spent allowance, a live game, and disconnecting.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::fixtures::{Recording, Reply, default_answers, http_client, serve};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use kchess_core::{Config, Core, TestSwitches};
use serde_json::{Value, json};

const FORK: &str = "r3k3/pp3ppp/8/1N6/8/8/PPP2PPP/4K3 w - - 0 1";

fn stream(text: &str) -> Reply {
    let delta = json!({ "type": "response.output_text.delta", "delta": text });
    let body = format!("data: {delta}\n\ndata: {{\"type\":\"response.completed\"}}\n\n");
    Reply::Text(200, "text/event-stream", body)
}

/// What the review page sends, so this fixture cannot drift from it: neither score here is a mate.
fn request(played: &str) -> Value {
    let review = json!({
        "fen": FORK,
        "moves": [played],
        "evals": [
            { "cp": 500, "mate": null, "pv": ["b5c7", "e8d7", "c7a8"] },
            { "cp": 0, "pv": ["e8d7"] },
        ],
    });
    kchess_domain::coach::review_moment(&review, 0).unwrap()["request"].clone()
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
    let dir = tempfile::tempdir().unwrap();
    // When set, the browser's trip back waits for this.
    let hold = Arc::new(Mutex::new(None::<tokio::sync::oneshot::Receiver<()>>));
    let host = Arc::new(Recording::default());
    *host.answers.lock().unwrap() = default_answers();
    {
        let (nonce, hold) = (Arc::clone(&nonce), Arc::clone(&hold));
        *host.browser.lock().unwrap() = Some(Box::new(move |url: String| {
            let url = reqwest::Url::parse(&url).unwrap();
            let query = |name: &str| {
                url.query_pairs()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.into_owned())
                    .unwrap_or_default()
            };
            // The first sign-in registers; a later one reuses the registration.
            assert!(
                ["dynamic_agent_client", "oaiapp_1"].contains(&query("client_id").as_str()),
                "{url}"
            );
            assert!(query("ext_agent_host_id").starts_with("urn:uuid:"));
            assert!(query("scope").contains("chatgpt.tokens.use.direct"));
            *nonce.lock().unwrap() = query("nonce");
            let back = format!(
                "{}?code=c&client_id=oaiapp_1&state={}",
                query("redirect_uri"),
                query("state")
            );
            let held = hold.lock().unwrap().take();
            tokio::spawn(async move {
                if let Some(held) = held {
                    held.await.ok();
                }
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

    // A model and effort chosen while a sign-in waits in the browser survive its return.
    let (release, held) = tokio::sync::oneshot::channel();
    *hold.lock().unwrap() = Some(held);
    let (signed, chosen) = tokio::join!(core.call("aiConnect", vec![]), async {
        crate::support::wait_until(
            std::time::Duration::from_secs(10),
            "the sign-in to wait",
            || hold.lock().unwrap().is_none().then_some(()),
        )
        .await;
        let chosen = core
            .call("aiSetOptions", vec![json!("gpt-a"), json!("high")])
            .await;
        release.send(()).unwrap();
        chosen
    });
    assert_eq!(chosen.unwrap()["effort"], "high");
    let signed = signed.unwrap();
    assert_eq!(signed["connected"], true);
    assert_eq!(signed["effort"], "high", "{signed}");
}

/// 2026-11-11: a connection document this release could not read was taken for no connection,
/// and the next choice of model wrote a fresh one over it, losing the sign-in. It is refused and
/// kept as it is.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_connection_is_kept_rather_than_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Recording::default());
    let config = Config::new(dir.path().to_path_buf());
    let core = Core::open(config, Arc::clone(&host) as Arc<dyn kchess_core::Host>).unwrap();
    let damaged = r#"{"hostId":"urn:uuid:1","sealed":42}"#;
    let database = core.database_for_tests();
    tokio::task::block_in_place(|| {
        database.write(|ctx| {
            ctx.db
                .execute(
                    "INSERT INTO documents (key, body, updatedAt) VALUES ('ai:connection', ?1, 0)",
                    [damaged],
                )
                .map_err(|cause| kchess_core::CoreError::new(cause.to_string()))
        })
    })
    .unwrap();

    let unreadable = "The ChatGPT connection saved in this profile could not be read.";
    let status = core.call("aiStatus", vec![]).await.unwrap_err();
    assert_eq!(status.message, unreadable);
    let chosen = core
        .call("aiSetOptions", vec![json!("gpt-a"), json!("low")])
        .await
        .unwrap_err();
    assert_eq!(chosen.message, unreadable);
    let kept =
        tokio::task::block_in_place(|| database.read(kchess_core::store::ai::connection)).unwrap();
    assert_eq!(kept.as_deref(), Some(damaged));
    core.close().await;
}

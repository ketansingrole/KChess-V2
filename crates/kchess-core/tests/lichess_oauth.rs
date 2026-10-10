//! Sign-in through the loopback callback (`connectLichess`, `oauthPage.ts`) over local fixtures:
//! PKCE (S256), the state check, the single-use callback, the callback deadline, a denied
//! login and a login that a logout overtook. A fake browser reads the authorize URL the core
//! opens and visits the callback the way a browser would.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fixtures::{
    MemoryStore, MemoryTokens, Recording, Reply, capabilities_for, default_answers, http_client,
    serve,
};
use kchess_core::lichess::accounts::Lichess;
use kchess_core::lichess::client::LichessClient;
use kchess_core::lichess::policy::Policy;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn query_of(url: &str) -> HashMap<String, String> {
    reqwest::Url::parse(url)
        .expect("a URL")
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// A Lichess service over the fixture origin, with the given host and stores.
fn service(base: &str, host: &Arc<Recording>, store: Arc<MemoryStore>) -> Arc<Lichess> {
    let policy = Policy::with_timeout(
        Arc::clone(host) as Arc<dyn kchess_core::host::Host>,
        http_client(),
        CancellationToken::new(),
        Duration::from_secs(5),
    );
    let client = LichessClient::with_base(base, Arc::clone(&policy), http_client());
    let capabilities = capabilities_for(host);
    Arc::new(Lichess::new(
        client,
        capabilities,
        store,
        MemoryTokens::with(&[]),
        Arc::clone(host) as Arc<dyn kchess_core::host::Host>,
        CancellationToken::new(),
    ))
}

/// The Lichess origin of a sign-in: the token and account answers.
async fn lichess_origin() -> fixtures::Fixture {
    serve(|seen| {
        if seen.path() == "/api/token" {
            Reply::Json(
                200,
                r#"{"access_token":"lip_new","token_type":"Bearer"}"#.into(),
            )
        } else if seen.path() == "/api/account" {
            Reply::Json(200, r#"{"username":"Alice"}"#.into())
        } else {
            Reply::Json(404, "{}".into())
        }
    })
    .await
}

/// Visits `url` as a browser would, and sends its status and body.
fn visit(url: String, report: mpsc::UnboundedSender<(u16, String)>) {
    tokio::spawn(async move {
        let response = http_client().get(url).send().await.expect("callback visit");
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        let _ = report.send((status, body));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_completes_through_the_loopback_callback() {
    let origin = lichess_origin().await;
    let host = Arc::new(Recording::default());
    *host.answers.lock().unwrap() = default_answers();
    let store = MemoryStore::with_accounts(&[]);
    let lichess = service(&origin.base, &host, Arc::clone(&store));

    let (report, mut visits) = mpsc::unbounded_channel();
    *host.browser.lock().unwrap() = Some(Box::new(move |url: String| {
        let query = query_of(&url);
        let callback = format!(
            "{}?code=auth_code_1&state={}",
            query["redirect_uri"], query["state"]
        );
        visit(callback, report.clone());
    }));

    let connected = lichess.connect_lichess(None, None).await.unwrap();
    assert_eq!(connected.username, "Alice");

    // The browser was sent to Lichess with the S256 challenge and every scope.
    let opened = host.opened.lock().unwrap()[0].clone();
    let query = query_of(&opened);
    assert!(opened.starts_with(&format!("{}/oauth?", origin.base)));
    assert_eq!(query["response_type"], "code");
    assert_eq!(query["client_id"], "kchess-desktop");
    assert_eq!(query["code_challenge_method"], "S256");
    assert!(query["redirect_uri"].starts_with("http://127.0.0.1:"));
    assert!(
        query["scope"]
            .split(' ')
            .any(|scope| scope == "study:write")
    );
    assert_eq!(query["scope"].split(' ').count(), 11);

    // The token exchange proves the verifier behind the challenge, and carries the code.
    let exchange = query_of(&format!("http://x/?{}", origin.to("/api/token")[0].body));
    assert_eq!(exchange["code"], "auth_code_1");
    assert_eq!(exchange["grant_type"], "authorization_code");
    assert_eq!(exchange["redirect_uri"], query["redirect_uri"]);
    let verifier = exchange["code_verifier"].clone();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    assert_eq!(challenge, query["code_challenge"]);

    // The browser was shown the success page, and the token was stored encrypted.
    let (status, page) = visits.recv().await.unwrap();
    assert_eq!(status, 200);
    assert!(page.contains("Signed in to Lichess"));
    assert!(!page.contains("<script"));
    assert_eq!(
        store.logins.lock().unwrap().clone(),
        vec![("Alice".to_string(), "enc:lip_new".to_string())]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_callback_with_the_wrong_state_or_path_does_not_end_the_login() {
    let origin = lichess_origin().await;
    let host = Arc::new(Recording::default());
    *host.answers.lock().unwrap() = default_answers();
    let store = MemoryStore::with_accounts(&[]);
    let lichess = service(&origin.base, &host, Arc::clone(&store));

    let (report, mut visits) = mpsc::unbounded_channel();
    *host.browser.lock().unwrap() = Some(Box::new(move |url: String| {
        let query = query_of(&url);
        let redirect = query["redirect_uri"].clone();
        let state = query["state"].clone();
        let report = report.clone();
        tokio::spawn(async move {
            let client = http_client();
            // A favicon request, a forged state, and then the real callback.
            let origin = redirect.trim_end_matches("/callback").to_string();
            let favicon = client
                .get(format!("{origin}/favicon.ico"))
                .send()
                .await
                .unwrap();
            let _ = report.send((favicon.status().as_u16(), String::new()));
            let forged = client
                .get(format!("{redirect}?code=forged&state=not-the-state"))
                .send()
                .await
                .unwrap();
            let _ = report.send((forged.status().as_u16(), String::new()));
            let real = client
                .get(format!("{redirect}?code=real&state={state}"))
                .send()
                .await
                .unwrap();
            let _ = report.send((real.status().as_u16(), String::new()));
        });
    }));

    let connected = lichess.connect_lichess(None, None).await.unwrap();
    assert_eq!(connected.username, "Alice");
    let statuses: Vec<u16> = {
        let mut seen = Vec::new();
        while let Ok(Some((status, _))) =
            tokio::time::timeout(Duration::from_millis(500), visits.recv()).await
        {
            seen.push(status);
        }
        seen
    };
    assert_eq!(statuses, vec![404, 404, 200]);
    // The forged code was never exchanged.
    let exchange = origin.to("/api/token");
    assert_eq!(exchange.len(), 1);
    assert!(exchange[0].body.contains("code=real"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_nobody_completes_times_out() {
    let origin = lichess_origin().await;
    let host = Arc::new(Recording::default());
    *host.answers.lock().unwrap() = default_answers();
    let store = MemoryStore::with_accounts(&[]);
    let lichess = service(&origin.base, &host, Arc::clone(&store));

    let error = lichess
        .connect_lichess_within(None, None, Duration::from_millis(150))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Lichess login timed out.");
    assert!(origin.requests().is_empty(), "no token was requested");
    assert!(store.logins.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_login_is_reported_and_saves_nothing() {
    let origin = lichess_origin().await;
    let host = Arc::new(Recording::default());
    *host.answers.lock().unwrap() = default_answers();
    let store = MemoryStore::with_accounts(&[]);
    let lichess = service(&origin.base, &host, Arc::clone(&store));

    let (report, mut visits) = mpsc::unbounded_channel();
    *host.browser.lock().unwrap() = Some(Box::new(move |url: String| {
        let query = query_of(&url);
        let callback = format!(
            "{}?error=access_denied&state={}",
            query["redirect_uri"], query["state"]
        );
        visit(callback, report.clone());
    }));

    let error = lichess.connect_lichess(None, None).await.unwrap_err();
    assert_eq!(error.message, "Lichess login was not completed.");
    let (status, page) = visits.recv().await.unwrap();
    assert_eq!(status, 400);
    assert!(page.contains("Connection cancelled"));
    assert!(origin.requests().is_empty());
    assert!(store.logins.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_that_a_logout_overtakes_saves_nothing() {
    let origin = lichess_origin().await;
    let host = Arc::new(Recording::default());
    *host.answers.lock().unwrap() = default_answers();
    let store = MemoryStore::with_accounts(&[]);
    let lichess = service(&origin.base, &host, Arc::clone(&store));

    let (report, mut visits) = mpsc::unbounded_channel();
    let logout_during = Arc::clone(&lichess);
    *host.browser.lock().unwrap() = Some(Box::new(move |url: String| {
        // The user signs out while the browser is still on Lichess.
        logout_during.invalidate_login(&[]);
        let query = query_of(&url);
        let callback = format!(
            "{}?code=late&state={}",
            query["redirect_uri"], query["state"]
        );
        visit(callback, report.clone());
    }));

    let error = lichess.connect_lichess(None, None).await.unwrap_err();
    assert_eq!(error.message, "Login was cancelled by logout.");
    assert!(store.logins.lock().unwrap().is_empty());
    let _ = visits.recv().await;
}

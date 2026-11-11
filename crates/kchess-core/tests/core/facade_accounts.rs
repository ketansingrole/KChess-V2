//! The facade's account calls end what an account had: signing out or removing an account forgets
//! the tournaments it joined (whose event streams would otherwise reopen), and each call tells
//! the other windows the accounts changed.

use crate::fixtures::{Reply, serve};
use crate::signed_in;

use kchess_contracts::types::JoinedTournament;
use kchess_core::store::library;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_out_or_removing_an_account_forgets_the_tournaments_it_joined() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    core.call("addAccount", vec![json!("Bob")]).await.unwrap();
    let now = kchess_core::time::now_ms() as f64;
    core.database_for_tests()
        .write(|c| {
            for (id, account) in [
                ("Arena001", "Alice"),
                ("Arena002", "Bob"),
                ("Arena003", "Carol"),
            ] {
                let entry: JoinedTournament = serde_json::from_value(json!({
                    "system": "arena", "id": id, "account": account, "name": id,
                    "until": now + 86_400_000.0,
                }))
                .expect("a joined tournament");
                library::remember_tournament(c.host, c.db, &entry, now)?;
            }
            Ok(())
        })
        .unwrap();
    let joined = || async {
        let mut accounts: Vec<String> = core
            .call("joinedTournaments", vec![])
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["account"].as_str().unwrap().to_string())
            .collect();
        accounts.sort();
        accounts
    };
    assert_eq!(joined().await, ["Alice", "Bob", "Carol"]);

    // Bob has no login: removing him still ends what he had.
    core.call("removeAccount", vec![json!("Bob")])
        .await
        .unwrap();
    assert_eq!(joined().await, ["Alice", "Carol"]);
    // Alice's session ends with her login.
    let data = core.call("logout", vec![json!("Alice")]).await.unwrap();
    assert!(
        data["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|account| account["connected"] == json!(false)),
        "{data}"
    );
    assert_eq!(joined().await, ["Carol"]);

    let changes: Vec<Value> = profile
        .events()
        .into_iter()
        .filter(|(name, _)| name == "profile:changed")
        .map(|(_, payload)| payload["areas"].clone())
        .collect();
    assert_eq!(changes, vec![json!(["accounts"]); 3], "{changes:?}");
    core.close().await;
}

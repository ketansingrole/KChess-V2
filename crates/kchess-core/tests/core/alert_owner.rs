//! Which window alerts the person (`alerts:owner`): the core tells every client each time the
//! owner changes as windows connect, gain focus and go, and only then.

use std::sync::Arc;

use kchess_core::connection::ClientInfo;
use kchess_core::{Config, Core, Host};
use serde_json::{Value, json};

use crate::support::Recording;

fn owners(host: &Recording) -> Vec<Value> {
    host.events
        .lock()
        .expect("events")
        .iter()
        .filter(|(name, _)| name == "alerts:owner")
        .map(|(_, payload)| payload["connection"].clone())
        .collect()
}

#[tokio::test]
async fn each_change_of_the_alerting_window_is_reported_once() {
    let dir = tempfile::tempdir().expect("data folder");
    let host = Arc::new(Recording::default());
    let core = Core::open(
        Config::new(dir.path().into()),
        Arc::clone(&host) as Arc<dyn Host>,
    )
    .expect("core");
    core.serve_connections();
    let first = core.connect(ClientInfo::app());
    let second = core.connect(ClientInfo::app());
    // Focusing the window that already alerts changes nothing, so nothing is said.
    core.focus(second, true);
    core.focus(first, true);
    core.disconnect(first);
    core.disconnect(second);
    assert_eq!(
        owners(&host),
        [
            Value::Null,
            json!(first),
            json!(second),
            json!(first),
            json!(second),
            Value::Null
        ]
    );
    assert_eq!(
        core.with_alert_owner(|payload| payload),
        json!({ "connection": null })
    );
    core.call("close", vec![]).await.expect("close");
}

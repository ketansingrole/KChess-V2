//! `crates/kchess-wasm/js/accounts.ts`: choosing which connected Lichess account plays online.

use serde_json::{Value, json};

use super::{Out, arg, list};
use crate::js;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "connectedAccountIndex" => list(args, 0, "accounts").map(|accounts| {
            let preferred = arg(args, 1).and_then(Value::as_str);
            index_json(connected_index(accounts, preferred))
        }),
        _ => return None,
    })
}

fn index_json(index: Option<usize>) -> Value {
    index.map_or(Value::Null, |i| json!(i))
}

/// `pickConnectedAccount`, as an index into `accounts`: the connected account matching
/// `preferred` (case-insensitive), else the first connected one.
fn connected_index(accounts: &[Value], preferred: Option<&str>) -> Option<usize> {
    let connected: Vec<usize> = accounts
        .iter()
        .enumerate()
        .filter(|(_, account)| !js::falsy(account.get("connected")))
        .map(|(i, _)| i)
        .collect();
    let wanted = preferred.map(str::to_lowercase).filter(|w| !w.is_empty());
    let matching = wanted.and_then(|wanted| {
        connected.iter().copied().find(|&i| {
            accounts[i]
                .get("username")
                .and_then(Value::as_str)
                .is_some_and(|name| name.to_lowercase() == wanted)
        })
    });
    matching.or_else(|| connected.first().copied())
}

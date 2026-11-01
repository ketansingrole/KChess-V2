//! The ChatGPT connection (tokens sealed by the credential store) with the model and effort every
//! AI feature uses, and the explanations already paid for.

use rusqlite::Connection;
use serde_json::{Value, json};

use super::StoreContext;
use super::documents::{read_document, write_document};
use crate::error::Result;

const CONNECTION: &str = "ai:connection";
const EXPLANATIONS: &str = "ai:explanations";
/// Explanations kept, oldest dropped first.
const MAX_EXPLANATIONS: usize = 200;

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    let first = args.first().cloned().unwrap_or(Value::Null);
    Some(match method {
        "store.ai.connection" => {
            read_document(ctx.host, db, CONNECTION).map(|found| found.unwrap_or(Value::Null))
        }
        "store.ai.saveConnection" => write_document(
            db,
            CONNECTION,
            &first,
            "The ChatGPT connection is too large.",
        )
        .map(|_| Value::Null),
        "store.ai.explanation" => explanation(ctx, db, first.as_str().unwrap_or_default()),
        "store.ai.saveExplanation" => {
            let text = args.get(1).cloned().unwrap_or(Value::Null);
            save_explanation(ctx, db, first, text).map(|()| Value::Null)
        }
        _ => return None,
    })
}

fn explanations(ctx: &StoreContext, db: &Connection) -> Result<Vec<Value>> {
    Ok(match read_document(ctx.host, db, EXPLANATIONS)? {
        Some(Value::Array(list)) => list,
        _ => Vec::new(),
    })
}

fn explanation(ctx: &StoreContext, db: &Connection, key: &str) -> Result<Value> {
    Ok(explanations(ctx, db)?
        .into_iter()
        .find(|entry| entry["key"] == key)
        .map_or(Value::Null, |entry| entry["text"].clone()))
}

fn save_explanation(ctx: &StoreContext, db: &Connection, key: Value, text: Value) -> Result<()> {
    let mut list = explanations(ctx, db)?;
    list.retain(|entry| entry["key"] != key);
    list.push(json!({ "key": key, "text": text }));
    let excess = list.len().saturating_sub(MAX_EXPLANATIONS);
    list.drain(..excess);
    write_document(
        db,
        EXPLANATIONS,
        &Value::Array(list),
        "Explanations are too large.",
    )
    .map(|_| ())
}

//! The ChatGPT connection (tokens sealed by the credential store) with the model and effort every
//! AI feature uses, and the explanations already paid for.

use serde_json::{Value, json};

use super::StoreContext;
use super::documents::{read_document, write_document};
use crate::error::Result;

const CONNECTION: &str = "ai:connection";
const EXPLANATIONS: &str = "ai:explanations";
/// Explanations kept, oldest dropped first.
const MAX_EXPLANATIONS: usize = 200;

/// The stored connection, or null.
pub fn connection(ctx: &StoreContext) -> Result<Value> {
    Ok(read_document(ctx.host, ctx.db, CONNECTION)?.unwrap_or(Value::Null))
}

pub fn save_connection(ctx: &StoreContext, connection: &Value) -> Result<()> {
    write_document(
        ctx.db,
        CONNECTION,
        connection,
        "The ChatGPT connection is too large.",
    )
    .map(|_| ())
}

fn explanations(ctx: &StoreContext) -> Result<Vec<Value>> {
    Ok(match read_document(ctx.host, ctx.db, EXPLANATIONS)? {
        Some(Value::Array(list)) => list,
        _ => Vec::new(),
    })
}

pub fn explanation(ctx: &StoreContext, key: &str) -> Result<Value> {
    Ok(explanations(ctx)?
        .into_iter()
        .find(|entry| entry["key"] == key)
        .map_or(Value::Null, |entry| entry["text"].clone()))
}

pub fn save_explanation(ctx: &StoreContext, key: Value, text: Value) -> Result<()> {
    let mut list = explanations(ctx)?;
    list.retain(|entry| entry["key"] != key);
    list.push(json!({ "key": key, "text": text }));
    let excess = list.len().saturating_sub(MAX_EXPLANATIONS);
    list.drain(..excess);
    write_document(
        ctx.db,
        EXPLANATIONS,
        &Value::Array(list),
        "Explanations are too large.",
    )
    .map(|_| ())
}

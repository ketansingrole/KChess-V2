//! `crates/kchess-wasm/js/analysisContext.ts`: the key an analysis is cached under.

use serde_json::Value;

use super::review::join;
use super::{Out, arg, text_of};

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "analysisContext" => match arg(args, 1) {
            None => Ok(Value::String(format!("{}|", text_of(args, 0)))),
            Some(Value::Array(moves)) => Ok(Value::String(format!(
                "{}|{}",
                text_of(args, 0),
                join(moves)
            ))),
            Some(_) => Err("moves must be a list".into()),
        },
        _ => return None,
    })
}

//! Rules moved from `crates/kchess-wasm/js` (see RUST_MIGRATION.md): game review helpers, ratings,
//! studies, game results, time controls, accounts, Lichess events and errors, OAuth looks and
//! analysis contexts. Reached through `api::call`.

mod accounts;
pub mod challenge;
mod context;
mod games;
mod lichess;
pub mod oauth;
mod online;
mod ratings;
mod review;
mod studies;
mod time_control;

use serde_json::Value;
use shakmaty::variant::VariantPosition;

use crate::js;
use crate::rules;

type Out = Result<Value, String>;

/// Argument `i`; JSON null stands for an argument that was not given.
fn arg(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i).filter(|v| !v.is_null())
}

/// Argument `i` as a list.
fn list<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a [Value], String> {
    arg(args, i)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{name} must be a list"))
}

/// A `{ variant, fen }` setup as the legal position it describes.
fn setup_position(value: Option<&Value>) -> Result<VariantPosition, String> {
    let setup = value
        .and_then(Value::as_object)
        .ok_or("setup must be an object")?;
    let variant = setup
        .get("variant")
        .and_then(Value::as_str)
        .ok_or("setup.variant")?;
    let fen = setup
        .get("fen")
        .and_then(Value::as_str)
        .ok_or("setup.fen")?;
    rules::setup_start(variant, fen).ok_or_else(|| "setup is not a legal position".into())
}

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    review::call(method, args)
        .or_else(|| ratings::call(method, args))
        .or_else(|| studies::call(method, args))
        .or_else(|| games::call(method, args))
        .or_else(|| time_control::call(method, args))
        .or_else(|| accounts::call(method, args))
        .or_else(|| online::call(method, args))
        .or_else(|| challenge::call(method, args))
        .or_else(|| lichess::call(method, args))
        .or_else(|| oauth::call(method, args))
        .or_else(|| context::call(method, args))
}

/// `js::to_string` of an argument, for the TypeScript template strings that take any value.
fn text_of(args: &[Value], i: usize) -> String {
    js::to_string(arg(args, i))
}

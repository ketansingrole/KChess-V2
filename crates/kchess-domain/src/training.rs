//! Rules moved from `core/src/domain` (see RUST_MIGRATION.md); reached through `api::call`.
//!
//! Training: puzzle runs (Storm, Streak, Rush), puzzle solving, endgame drills, openings, the
//! client clock, engine levels, UCI `info` lines and review coaching. Their constants are data
//! shared with TypeScript (`core/src/domain/data/`).

mod clock;
mod coach;
mod data;
mod endgames;
mod engine_levels;
mod openings;
mod puzzle;
mod rush;
mod uci_info;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::js;

pub(super) type Out = Result<Value, String>;

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    type Module = fn(&str, &[Value]) -> Option<Out>;
    let modules: [Module; 8] = [
        rush::call,
        puzzle::call,
        endgames::call,
        openings::call,
        clock::call,
        engine_levels::call,
        uci_info::call,
        coach::call,
    ];
    modules.iter().find_map(|call| call(method, args))
}

/// Argument `i` when present; `undefined` crosses the boundary as null, so null counts as absent.
pub(super) fn arg(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i).filter(|v| !v.is_null())
}

/// Argument `i` as a string.
pub(super) fn text_arg<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a str, String> {
    arg(args, i)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

/// Argument `i` as a JavaScript number (`Number(…)`): absent is NaN, and the non-finite values
/// the TypeScript wrappers send as their text (`"NaN"`, `"Infinity"`) read back the same way.
pub(super) fn number_arg(args: &[Value], i: usize) -> f64 {
    js::to_number(arg(args, i))
}

/// Argument `i` deserialized as `T`.
pub(super) fn value_arg<T: DeserializeOwned>(
    args: &[Value],
    i: usize,
    name: &str,
) -> Result<T, String> {
    let value = arg(args, i)
        .cloned()
        .ok_or_else(|| format!("{name} is missing"))?;
    serde_json::from_value(value).map_err(|e| format!("{name}: {e}"))
}

pub(super) fn to_value<T: Serialize>(value: T) -> Out {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// A JavaScript number as the text `String(n)` gives it.
pub(super) fn text_of(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n == f64::INFINITY {
        "Infinity".into()
    } else if n == f64::NEG_INFINITY {
        "-Infinity".into()
    } else {
        js::number_to_string(n)
    }
}

/// A number result. JSON has no NaN or infinity, so those go out as their text, which the
/// TypeScript wrappers turn back into numbers.
pub(super) fn num_out(n: f64) -> Value {
    if n.is_finite() {
        Value::from(n)
    } else {
        Value::String(text_of(n))
    }
}

/// `String(n).padStart(2, '0')`
pub(super) fn pad2(n: f64) -> String {
    let text = text_of(n);
    if text.len() < 2 {
        format!("0{text}")
    } else {
        text
    }
}

/// `Math.max(a, b)`: NaN wins, and the two zeros compare equal.
pub(super) fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if b > a {
        b
    } else {
        a
    }
}

/// `Math.min(a, b)`
pub(super) fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if b < a {
        b
    } else {
        a
    }
}

/// `Math.round(x)`: halves round toward positive infinity, unlike Rust's `round`.
pub(super) fn js_round(x: f64) -> f64 {
    let rounded = x.round();
    if rounded - x == -0.5 {
        rounded + 1.0
    } else {
        rounded
    }
}

/// The end index of `array.slice(0, end)` over `len` items.
pub(super) fn js_slice_end(len: usize, end: f64) -> usize {
    let end = if end.is_nan() { 0.0 } else { end.trunc() };
    let len = len as f64;
    if end < 0.0 {
        (len + end).max(0.0) as usize
    } else {
        end.min(len) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dispatches_every_training_method() {
        for method in [
            "newRush",
            "correctMove",
            "puzzleFromApi",
            "evaluateEndgame",
            "openingAt",
            "formatClock",
            "normalizeEngineLevels",
            "parseInfo",
            "explainReviewedMove",
        ] {
            assert!(
                call(method, &[]).is_some(),
                "{method} is dispatched by training"
            );
        }
        assert!(call("noSuchTrainingRule", &[]).is_none());
    }

    #[test]
    fn javascript_rounding_and_slicing() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.49999999999999994), 0.0);
        assert_eq!(js_slice_end(4, 2.7), 2);
        assert_eq!(js_slice_end(4, -1.0), 3);
        assert_eq!(js_slice_end(4, f64::NAN), 0);
        assert_eq!(js_slice_end(4, f64::INFINITY), 4);
        assert_eq!(text_of(f64::NAN), "NaN");
        assert_eq!(pad2(9.0), "09");
        assert_eq!(number_arg(&[json!("Infinity")], 0), f64::INFINITY);
    }
}

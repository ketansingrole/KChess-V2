//! Library documents, and the JavaScript-like values (`J`) arguments and documents are read as.

pub mod constants;
pub mod documents;
pub mod patterns;
pub mod unicode_tables;
pub mod value;

pub use constants::MAX_DOCUMENT;

use serde_json::Value;
use value::J;

impl From<&Value> for J {
    fn from(value: &Value) -> J {
        from_json(value)
    }
}

pub(crate) fn from_json(v: &Value) -> J {
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Number(n) => J::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => J::Str(s.clone()),
        Value::Array(items) => J::Arr(items.iter().map(from_json).collect()),
        Value::Object(fields) => J::Obj(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), from_json(v)))
                .collect(),
        ),
    }
}

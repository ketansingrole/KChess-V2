//! Game sessions as every KChess frontend drives them: each decision is a Rust transition in
//! `kchess-domain` (`games/`); these drivers only answer the reads a transition asks for, keep the
//! state it wrote and hand its effects to the frontend, in order. The GPUI app and the CLI share
//! them.

pub mod archive;
pub mod computer;
pub mod local;

use serde_json::{Map, Value};

/// A random number in [0, 1), like `Math.random()`, for the rules that draw from them.
pub fn random_unit() -> f64 {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64
}

/// A count or index from the rules, which write JavaScript numbers: `2` and `2.0` alike.
pub fn whole(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|n| *n >= 0.).map(|n| n as u64))
        .unwrap_or_default()
}

/// A rules method by name, as `rules()` calls it in the renderer.
pub fn rules(method: &str, args: &[Value]) -> Result<Value, String> {
    let text = kchess_domain::api::call(method, &Value::Array(args.to_vec()).to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("{method} answered something unreadable: {e}"))
}

/// Run a session transition: the rules answer with the finished transition or with the next
/// host read they need; each read is answered once, in the order the rules take it.
pub fn transition(
    method: &str,
    args: &[Value],
    mut read: impl FnMut(&str) -> Result<Value, String>,
) -> Result<Value, String> {
    let mut reads: Map<String, Value> = Map::new();
    loop {
        let mut call = args.to_vec();
        call.push(Value::Object(reads.clone()));
        let out = rules(method, &call)?;
        if let Some(done) = out.get("done") {
            return Ok(done.clone());
        }
        let kind = out
            .get("read")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{method} asked for nothing it could be given"))?
            .to_string();
        let value = read(&kind)?;
        match reads
            .entry(kind)
            .or_insert_with(|| Value::Array(Vec::new()))
        {
            Value::Array(values) => values.push(value),
            _ => return Err(format!("{method} reads were not a list")),
        }
    }
}

/// Assign the fields a transition wrote (`assignWritten`).
pub fn assign_written(target: &mut Value, next: &Value, written: &Value) {
    let (Some(target), Some(next)) = (target.as_object_mut(), next.as_object()) else {
        return;
    };
    for key in written
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        match next.get(key) {
            Some(value) => {
                target.insert(key.to_string(), value.clone());
            }
            None => {
                target.remove(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_written_fields_change() {
        let mut state = json!({ "a": 1, "b": 2, "c": 3 });
        assign_written(&mut state, &json!({ "a": 10, "b": 20 }), &json!(["a", "c"]));
        assert_eq!(state, json!({ "a": 10, "b": 2 }));
    }
}

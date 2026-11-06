//! Game sessions as every KChess frontend drives them: each decision is a Rust transition in
//! `kchess-domain` (`games/`); these drivers only answer the reads a transition asks for, keep the
//! state it wrote and hand its effects to the frontend, in order. The GPUI app and the CLI share
//! them.

pub mod archive;
pub mod computer;
pub mod local;

use serde::Deserialize as _;
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

/// A rules method by name (`kchess_domain::api::call`), with its JSON answer.
pub fn rules(method: &str, args: &[Value]) -> Result<Value, String> {
    // Trees and deep arguments go through JSON text. Everything else skips writing the arguments
    // out and reading the answer back (`call_value` answers as `call` does).
    if !kchess_domain::api::needs_large_stack(method, args) {
        return kchess_domain::api::call_value(method, args);
    }
    let text = kchess_domain::api::call(method, &Value::Array(args.to_vec()).to_string())?;
    // A game's tree nests once a move: past JSON's usual limit of 128 once it runs some 64 plies.
    let mut reader = serde_json::Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let answer = Value::deserialize(&mut reader)
        .map_err(|e| format!("{method} answered something unreadable: {e}"))?;
    reader
        .end()
        .map_err(|e| format!("{method} answered something unreadable: {e}"))?;
    Ok(answer)
}

/// Run a session transition: the rules answer with the finished transition or with the next
/// host read they need; each read is answered once, in the order the rules take it.
pub fn transition(
    method: &str,
    args: &[Value],
    mut read: impl FnMut(&str) -> Result<Value, String>,
) -> Result<Value, String> {
    // The arguments are copied once; each round only adds the new read to the last slot.
    let mut call = Vec::with_capacity(args.len() + 1);
    call.extend_from_slice(args);
    call.push(Value::Object(Map::new()));
    loop {
        let mut out = rules(method, &call)?;
        if let Some(done) = out.get_mut("done") {
            return Ok(done.take());
        }
        let kind = out
            .get("read")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{method} asked for nothing it could be given"))?
            .to_string();
        let value = read(&kind)?;
        let Some(Value::Object(reads)) = call.last_mut() else {
            unreachable!("the reads slot is the object pushed above");
        };
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

    /// A 100-move game could not be opened from Game history: its tree was refused at depth 128.
    #[test]
    fn a_long_game_tree_reads_back() {
        let shuffle = ["Nf3", "Nf6", "Ng1", "Ng8"];
        let moves: Vec<String> = (0..200)
            .map(|ply| {
                let number = format!("{}. ", ply / 2 + 1);
                let prefix = if ply % 2 == 0 { number.as_str() } else { "" };
                format!("{prefix}{}", shuffle[ply % 4])
            })
            .collect();
        let tree = rules("treeFromPgn", &[Value::String(moves.join(" "))]).unwrap();
        let (mut node, mut plies) = (&tree, 0);
        while let Some(next) = node["children"].get(0) {
            node = next;
            plies += 1;
        }
        assert_eq!(plies, 200);
    }

    #[test]
    fn whole_reads_integers_and_floats_alike() {
        assert_eq!(whole(&json!(2)), 2);
        assert_eq!(whole(&json!(2.0)), 2);
        assert_eq!(whole(&json!(-1)), 0);
        assert_eq!(whole(&json!("2")), 0);
        assert_eq!(whole(&Value::Null), 0);
    }

    #[test]
    fn random_unit_stays_in_range() {
        for _ in 0..1_000 {
            let n = random_unit();
            assert!((0. ..1.).contains(&n), "{n}");
        }
    }

    #[test]
    fn an_unknown_rules_method_is_an_error() {
        assert!(rules("noSuchMethod", &[]).is_err());
    }

    #[test]
    fn a_host_read_that_fails_fails_the_transition() {
        let error = transition("localInit", &[], |kind| Err(format!("no {kind}"))).unwrap_err();
        assert_eq!(error, "no now");
    }

    #[test]
    fn each_read_is_answered_in_order() {
        let mut asked = Vec::new();
        let done = transition("localInit", &[], |kind| {
            asked.push(kind.to_string());
            Ok(json!(5))
        })
        .unwrap();
        assert_eq!(asked, ["now"]);
        assert!(done.get("runtime").is_some());
    }

    #[test]
    fn written_fields_missing_from_the_next_state_are_removed() {
        let mut state = json!({ "a": 1, "b": 2 });
        assign_written(&mut state, &json!({}), &json!(["a"]));
        assert_eq!(state, json!({ "b": 2 }));
        // A target that is not an object is left alone.
        let mut scalar = json!(3);
        assign_written(&mut scalar, &json!({ "a": 1 }), &json!(["a"]));
        assert_eq!(scalar, json!(3));
    }
}

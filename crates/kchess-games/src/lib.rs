//! Game sessions as every KChess frontend drives them: each decision is a Rust transition in
//! `kchess-domain` (`games/`); these drivers only answer the reads a transition asks for, keep the
//! state it wrote and hand its effects to the frontend, in order. The GPUI app and the CLI share
//! them.

pub mod archive;
pub mod computer;
pub mod local;

use serde::de::DeserializeOwned;
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
    rules_as(method, args)
}

/// A rules method's answer read straight into `T`. A deep answer (a game tree nests once a ply,
/// some 2,000 levels at the 1024-ply limit) is called, parsed and read on the domain's large
/// stack, so no deep intermediate `Value` is walked or dropped on the caller's thread: a crafted
/// 800-ply PGN overflowed a 1 MiB thread (Windows' main thread) doing that.
pub fn rules_as<T: DeserializeOwned + Send>(method: &str, args: &[Value]) -> Result<T, String> {
    if !kchess_domain::api::needs_large_stack(method, args) {
        let value = kchess_domain::api::call_value(method, args)?;
        return serde_json::from_value(value)
            .map_err(|e| format!("{method} answered something unreadable: {e}"));
    }
    let call = Value::Array(args.to_vec()).to_string();
    kchess_domain::api::on_large_stack(|| {
        let text = kchess_domain::api::call(method, &call)?;
        // Past JSON's usual limit of 128 once a tree runs some 64 plies; `call` bounds the depth.
        let mut reader = serde_json::Deserializer::from_str(&text);
        reader.disable_recursion_limit();
        let answer = T::deserialize(&mut reader)
            .map_err(|e| format!("{method} answered something unreadable: {e}"))?;
        reader
            .end()
            .map_err(|e| format!("{method} answered something unreadable: {e}"))?;
        Ok(answer)
    })
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

    /// A deepest legal tree (1024 plies) read on a 1 MiB thread, as on Windows' main thread:
    /// reading the answer back there overflowed the stack and aborted the app on a dropped PGN.
    #[derive(serde::Deserialize)]
    struct Node {
        children: Vec<Node>,
    }

    #[test]
    fn the_deepest_tree_reads_on_a_small_stack() {
        let shuffle = ["Nf3", "Nf6", "Ng1", "Ng8"];
        let moves: Vec<String> = (0..1024)
            .map(|ply| {
                let number = format!("{}. ", ply / 2 + 1);
                let prefix = if ply % 2 == 0 { number.as_str() } else { "" };
                format!("{prefix}{}", shuffle[ply % 4])
            })
            .collect();
        let pgn = Value::String(format!("{} *", moves.join(" ")));
        let plies = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || {
                let tree: Node = rules_as("treeFromPgn", &[pgn]).unwrap();
                let (mut node, mut plies) = (&tree, 0);
                while let Some(next) = node.children.first() {
                    node = next;
                    plies += 1;
                }
                // Dropping the tree recurses once a ply too, here on the small stack.
                drop(tree);
                plies
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(plies, 1024);
    }
}

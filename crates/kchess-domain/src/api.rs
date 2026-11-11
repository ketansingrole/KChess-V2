//! One entry point for every host: `call(method, args)` with a JSON array of arguments and a
//! JSON result. Every host binds this one function.

use serde_json::Value;

use crate::error::{DomainError, Result};

/// The deepest argument nesting `call` parses. The largest analysis tree (1024 plies, see
/// `tree.rs`) nests 2051 levels. Parsing, walking and dropping it recurse, and an overflowed
/// stack aborts the host process, which no panic guard can catch.
const MAX_JSON_DEPTH: usize = 4096;
/// Deeper arguments, and methods that build a tree, are handled on a thread with `DEEP_STACK`
/// bytes of stack: through `Value`, the largest tree overflows a 2 MiB thread (tokio's and Rust's
/// default) in a debug build.
const DEEP_JSON: usize = 256;
const TREE_METHODS: &[&str] = &["treeFromPgn"];
const DEEP_STACK: usize = 32 * 1024 * 1024;

/// How deeply `json` nests arrays and objects, counting no further than one past
/// `MAX_JSON_DEPTH`. Brackets inside strings do not count; malformed input is left to the parser.
fn nesting(json: &str) -> usize {
    let (mut depth, mut deepest, mut in_string, mut escaped) = (0usize, 0usize, false, false);
    for byte in json.bytes() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
                if deepest > MAX_JSON_DEPTH {
                    break;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// Answer one call, its arguments a JSON array. Unknown methods and malformed arguments are
/// errors.
pub fn call(method: &str, args: &str) -> std::result::Result<String, String> {
    let depth = nesting(args);
    if depth > MAX_JSON_DEPTH {
        return Err("arguments nest too deeply".to_string());
    }
    if goes_deep(method, depth) {
        return call_deep(method, args);
    }
    call_inner(method, args).map_err(|e| e.to_string())
}

/// Whether `call` answers on the large stack: arguments `depth` deep past `DEEP_JSON`, and the
/// methods that build deep trees.
fn goes_deep(method: &str, depth: usize) -> bool {
    depth > DEEP_JSON || TREE_METHODS.contains(&method)
}

/// `call_inner` on a thread whose stack holds `MAX_JSON_DEPTH` levels, whatever thread asked.
fn call_deep(method: &str, args: &str) -> std::result::Result<String, String> {
    on_large_stack(|| call_inner(method, args).map_err(|e| e.to_string()))
}

/// `work` on a thread whose stack holds `MAX_JSON_DEPTH` levels, whatever thread asked. Callers
/// that read a deep answer (a game tree) do so here too: parsing, walking and dropping it recurse.
pub fn on_large_stack<T: Send>(
    work: impl FnOnce() -> std::result::Result<T, String> + Send,
) -> std::result::Result<T, String> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("kchess-deep-call".into())
            .stack_size(DEEP_STACK)
            .spawn_scoped(scope, work)
            .map_err(|error| format!("could not start a thread: {error}"))?
            .join()
            .map_err(|_| "internal error".to_string())?
    })
}

fn call_inner(method: &str, args: &str) -> Result<String> {
    // Analysis trees nest deeper than serde_json's default limit of 128 levels; `call` bounds
    // the depth and gives deep arguments a large stack instead.
    let mut reader = serde_json::Deserializer::from_str(args);
    reader.disable_recursion_limit();
    let args: Vec<Value> = serde::Deserialize::deserialize(&mut reader)?;
    // One JSON array, nothing after it.
    reader.end()?;
    Ok(serde_json::to_string(&dispatch(method, &args)?)?)
}

/// `call` for arguments a Rust caller already holds as values: the same rules and answers,
/// without writing the arguments out as JSON and reading the answer back.
pub fn call_value(method: &str, args: &[Value]) -> std::result::Result<Value, String> {
    if deeper_than(args, MAX_JSON_DEPTH) {
        return Err("arguments nest too deeply".to_string());
    }
    if needs_large_stack(method, args) {
        return on_large_stack(|| dispatch(method, args).map_err(|e| e.to_string()));
    }
    dispatch(method, args).map_err(|e| e.to_string())
}

/// Whether `call_value` answers this call on a thread of its own with a large stack: deep
/// arguments, and the methods that build deep trees.
pub fn needs_large_stack(method: &str, args: &[Value]) -> bool {
    TREE_METHODS.contains(&method) || deeper_than(args, DEEP_JSON)
}

/// Whether the values nest more than `limit` arrays and objects deep.
fn deeper_than(args: &[Value], limit: usize) -> bool {
    let mut stack: Vec<(&Value, usize)> = args.iter().map(|value| (value, 2)).collect();
    while let Some((value, depth)) = stack.pop() {
        let children: Box<dyn Iterator<Item = &Value>> = match value {
            Value::Array(items) => Box::new(items.iter()),
            Value::Object(map) => Box::new(map.values()),
            _ => continue,
        };
        if depth > limit {
            return true;
        }
        stack.extend(children.map(|child| (child, depth + 1)));
    }
    false
}

/// Dispatch one call. Unknown methods and malformed arguments are errors.
fn dispatch(method: &str, args: &[Value]) -> Result<Value> {
    let moved = [
        crate::voice::call,
        crate::training::call,
        crate::records::call,
        crate::misc::call,
        crate::games::call,
        crate::online_game::call,
        crate::trainer::call,
        crate::theme::call,
    ];
    match moved.iter().find_map(|call| call(method, args)) {
        Some(result) => Ok(result.map_err(DomainError::Message)?),
        None => Err(DomainError::Message(format!(
            "unknown rules method {method}"
        ))),
    }
}

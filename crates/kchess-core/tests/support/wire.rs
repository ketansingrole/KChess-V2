//! What the core answers, byte for byte, as recorded: a method's answer is compared with the JSON
//! kept in `tests/wire/<name>.json`, so a change of its Rust type (a contract type in place of
//! `Value` or of a mirror) is shown to leave the bytes a frontend reads as they were: field names
//! and order, `null` against absent, and integers against `1.0`.
//!
//! `KCHESS_WRITE_WIRE=1` (re)writes the recordings instead of comparing. Record before the change,
//! on the code being replaced, then change it and run the test again without the variable.

use std::path::PathBuf;

use serde::Serialize;

fn recording(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/wire")
        .join(format!("{name}.json"))
}

/// `answer` as the core sends it, the same as the recording `name`.
pub fn assert_wire(name: &str, answer: &impl Serialize) {
    let sent = serde_json::to_string(answer).expect("the answer is JSON");
    let path = recording(name);
    if std::env::var_os("KCHESS_WRITE_WIRE").is_some() {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("the folder is made");
        std::fs::write(&path, format!("{sent}\n")).expect("the recording is written");
        return;
    }
    let recorded = std::fs::read_to_string(&path)
        .unwrap_or_else(|cause| panic!("no recording {}: {cause}", path.display()));
    assert_eq!(
        sent,
        recorded.trim_end_matches('\n'),
        "{name}: the answer's bytes changed"
    );
}

/// `answer` as `assert_wire` compares it, with the values that change from run to run (times,
/// temporary paths) at `paths` (`/`-separated keys, `*` for every item of a list) replaced by
/// one of the same kind: a whole number by `0`, a fractional one by `0.0`, a string by `"…"`.
pub fn assert_wire_steady(name: &str, answer: &impl Serialize, paths: &[&str]) {
    let mut value = serde_json::to_value(answer).expect("the answer is JSON");
    for path in paths {
        let keys: Vec<&str> = path.split('/').collect();
        steady(&mut value, &keys);
    }
    assert_wire(name, &value);
}

fn steady(value: &mut serde_json::Value, keys: &[&str]) {
    use serde_json::{Value, json};
    match keys.split_first() {
        None => {
            let replaced = match &*value {
                Value::Number(n) if n.is_f64() => json!(0.0),
                Value::Number(_) => json!(0),
                Value::String(_) => json!("…"),
                _ => return,
            };
            *value = replaced;
        }
        Some((&"*", rest)) => {
            if let Value::Array(items) = value {
                for item in items {
                    steady(item, rest);
                }
            }
        }
        Some((key, rest)) => {
            if let Some(inner) = value.get_mut(*key) {
                steady(inner, rest);
            }
        }
    }
}

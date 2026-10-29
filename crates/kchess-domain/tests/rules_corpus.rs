//! Every rules method answers as it did when the TypeScript suites last ran against it.
//!
//! `fixtures/rules-corpus.jsonl.gz` holds 32k calls the TypeScript test suites made through the
//! Node module before they were removed (`tests/core`, the renderer's unit tests): up to 200 per
//! method, chosen by the hash of their arguments, with the first 16 hex digits of the SHA-256 of
//! each answer (keys sorted, fractions to 9 significant digits), or `e:` and the error. This pins the behaviour those suites checked, including
//! the Rust-only rules their golden digests covered. Regenerate it only for an intended change:
//! `KCHESS_WRITE_CORPUS=1 cargo test -p kchess-domain --test rules_corpus` rewrites each digest.

use std::io::{Read, Write};
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// An answer with sorted keys and fractions to 9 significant digits: the last bit of `exp`
/// differs between CPUs (as it did for the recorded suites), and nothing else may.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Number(n) if !n.is_i64() && !n.is_u64() => {
            let x = n.as_f64().unwrap_or_default();
            if x.fract() == 0.0 {
                json!(x)
            } else {
                json!(format!("{x:.8e}").parse::<f64>().unwrap_or(x))
            }
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), canonical(&map[k])))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

fn digest(answer: &Result<String, String>) -> String {
    match answer {
        Ok(text) => {
            let text = serde_json::from_str::<Value>(text)
                .map_or_else(|_| text.clone(), |value| canonical(&value).to_string());
            let hash = Sha256::digest(text.as_bytes());
            hash.iter()
                .take(8)
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
        Err(message) => format!("e:{message}"),
    }
}

#[test]
fn every_method_answers_as_recorded() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules-corpus.jsonl.gz");
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let write = std::env::var("KCHESS_WRITE_CORPUS").as_deref() == Ok("1");
    let mut rewritten = Vec::new();
    let mut differ = Vec::new();
    let mut cases = 0;
    for line in text.lines() {
        let mut case: Value = serde_json::from_str(line).unwrap();
        let (method, args) = (case["m"].as_str().unwrap(), case["a"].as_str().unwrap());
        let actual = digest(&kchess_domain::api::call(method, args));
        cases += 1;
        // Rust callers pass values (`call_value`): the same answer, whatever the arguments.
        if let Ok(values) = serde_json::from_str::<Vec<Value>>(args) {
            let by_value = kchess_domain::api::call_value(method, &values).map(|v| v.to_string());
            if digest(&by_value) != actual && differ.len() < 20 {
                differ.push(format!(
                    "{method} by value {}: {actual} ≠ {}",
                    &args[..args.len().min(200)],
                    digest(&by_value)
                ));
            }
        }
        if actual != case["d"] {
            if differ.len() < 20 {
                differ.push(format!(
                    "{method} {}: {} ≠ {actual}",
                    &args[..args.len().min(200)],
                    case["d"]
                ));
            }
            case["d"] = json!(actual);
        }
        rewritten.push(case.to_string());
    }
    if write {
        let mut out = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::best(),
        );
        out.write_all((rewritten.join("\n") + "\n").as_bytes())
            .unwrap();
        out.finish().unwrap();
        return;
    }
    assert!(cases > 30_000, "the corpus holds {cases} cases");
    assert!(differ.is_empty(), "answers changed:\n{}", differ.join("\n"));
}

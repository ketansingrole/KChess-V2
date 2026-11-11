//! The rules by the names the recorded suites called them, with JSON arguments and answers: the
//! corpus's own door into the typed functions. Each group answers its methods, `None` for the
//! others, in the order the recorded dispatch asked them.

mod games;
mod misc;
mod online_game;
mod records;
mod rules;
mod sessions;
mod theme;
mod trainer;
mod training;
mod voice;

use serde_json::Value;

/// One method's answer, or the message it rejected its arguments with.
pub type Out = Result<Value, String>;

/// The deepest argument nesting a call parses, as recorded.
const MAX_JSON_DEPTH: usize = 4096;

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
                    return deepest;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// Answer one call, its arguments a JSON array, as JSON text. Run it on a thread with a large
/// stack: trees nest some 2,000 levels.
pub fn call(method: &str, args: &str) -> Result<String, String> {
    if nesting(args) > MAX_JSON_DEPTH {
        return Err("arguments nest too deeply".to_string());
    }
    let mut reader = serde_json::Deserializer::from_str(args);
    reader.disable_recursion_limit();
    let args: Vec<Value> =
        serde::Deserialize::deserialize(&mut reader).map_err(|e| format!("serialization: {e}"))?;
    reader.end().map_err(|e| format!("serialization: {e}"))?;
    let answer = dispatch(method, &args)?;
    serde_json::to_string(&answer).map_err(|e| format!("serialization: {e}"))
}

fn dispatch(method: &str, args: &[Value]) -> Out {
    type Group = fn(&str, &[Value]) -> Option<Out>;
    let groups: [Group; 9] = [
        rules::call,
        voice::call,
        training::call,
        records::call,
        misc::call,
        games::call,
        online_game::call,
        trainer::call,
        theme::call,
    ];
    groups
        .iter()
        .find_map(|group| group(method, args))
        .unwrap_or_else(|| Err(format!("unknown rules method {method}")))
}

/// A variant by the Lichess key a recorded call named it with; empty is standard chess.
pub fn variant(key: &str) -> Result<kchess_domain::variant::Variant, String> {
    if key.is_empty() {
        return Ok(kchess_domain::variant::Variant::Standard);
    }
    kchess_domain::variant::Variant::from_key(key).ok_or_else(|| format!("unknown variant {key}"))
}

/// Argument `i`; JSON null stands for an argument that was not given.
/// A side as the recorded calls named it: anything but `white` was Black.
pub fn side(name: &str) -> kchess_domain::color::Color {
    kchess_domain::color::from_name(name).unwrap_or(kchess_domain::color::Color::Black)
}

pub fn arg(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i).filter(|v| !v.is_null())
}

#[test]
fn nesting_counts_open_levels_outside_strings() {
    assert_eq!(nesting("[]"), 1);
    assert_eq!(nesting("[[], [], {}]"), 2);
    assert_eq!(nesting("[[[]], [[]]]"), 3);
    assert_eq!(nesting(r#"["]"]"#), 1);
    assert_eq!(nesting(r#"["\"", [[]]]"#), 3);
    assert_eq!(nesting(r#"["\" [[[", 1]"#), 1);
    assert_eq!(nesting(r#"["\\", [[]]]"#), 3);
}

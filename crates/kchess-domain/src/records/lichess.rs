//! `core/src/domain/lichessError.ts`: which failed Lichess response becomes which error. The
//! `LichessError` class stays in TypeScript and builds the message from these fields.

use serde_json::{Map, Value, json};

use super::{Out, arg};
use crate::js;

/// The most of a response body an error shows (JavaScript's `slice(0, 250)`).
const DETAIL_UNITS: usize = 250;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "lichessError" => match arg(args, 0).and_then(|response| response.get("status")) {
            Some(status) => Ok(error_fields(
                status,
                arg(args, 1),
                arg(args, 2).and_then(Value::as_str).unwrap_or(""),
            )),
            None => Err("response.status must be a number".into()),
        },
        _ => return None,
    })
}

/// `{ status, endpoint, detail }`: an HTML page (Lichess's 404 for unknown users) never becomes
/// the detail; a JSON body does, cut to 250 characters.
fn error_fields(status: &Value, error: Option<&Value>, endpoint: &str) -> Value {
    let page = error
        .and_then(Value::as_str)
        .is_some_and(|text| text.trim_start_matches(js::is_space).starts_with('<'));
    let detail = if page {
        if status.as_f64() == Some(404.0) {
            "not found".to_string()
        } else {
            "unexpected page response".to_string()
        }
    } else {
        match error {
            None | Some(Value::Null) => String::new(),
            Some(value) => first_units(&js_json(value), DETAIL_UNITS),
        }
    };
    json!({ "status": status, "endpoint": endpoint, "detail": detail })
}

/// The first `n` UTF-16 units of `text`. A cut through a surrogate pair drops the half that
/// would be left alone, since a Rust string cannot hold it.
fn first_units(text: &str, n: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() <= n {
        return text.to_string();
    }
    let mut end = n;
    if (0xD800..=0xDBFF).contains(&units[end - 1]) {
        end -= 1;
    }
    String::from_utf16_lossy(&units[..end])
}

/// `JSON.stringify(value)`: compact, with JavaScript's number text and key order (integer-like
/// keys first, in order, then the rest as they were written).
pub fn js_json(value: &Value) -> String {
    let mut out = String::new();
    write_json(value, &mut out);
    out
}

fn write_json(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(
            &number
                .as_f64()
                .map_or_else(|| "null".to_string(), js::number_to_string),
        ),
        Value::String(text) => write_quoted(text, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, key) in property_order(map).iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_quoted(key, out);
                out.push(':');
                if let Some(entry) = map.get(key.as_str()) {
                    write_json(entry, out);
                }
            }
            out.push('}');
        }
    }
}

fn write_quoted(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Object keys in JavaScript's own order: array indices ascending, then the other keys in
/// insertion order.
fn property_order(map: &Map<String, Value>) -> Vec<&String> {
    let mut indices: Vec<(u32, &String)> = map
        .keys()
        .filter_map(|key| array_index(key).map(|i| (i, key)))
        .collect();
    indices.sort_by_key(|(i, _)| *i);
    let others = map.keys().filter(|key| array_index(key).is_none());
    indices
        .into_iter()
        .map(|(_, key)| key)
        .chain(others)
        .collect()
}

/// A canonical array index: digits without a leading zero, below 2^32 - 1.
fn array_index(key: &str) -> Option<u32> {
    let canonical = key == "0" || (!key.starts_with('0') && !key.is_empty());
    if !canonical || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u32>().ok().filter(|i| *i < u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_text_matches_javascript() {
        assert_eq!(
            js_json(&json!({ "b": 1.0, "a": [true, null] })),
            r#"{"b":1,"a":[true,null]}"#
        );
        // Integer-like keys come first, in order, as in JavaScript objects.
        let object: Value = serde_json::from_str(r#"{"b":1,"10":"x","2":null}"#).unwrap();
        assert_eq!(js_json(&object), r#"{"2":null,"10":"x","b":1}"#);
        assert_eq!(js_json(&json!("a\u{1}\"\\\n")), r#""a\u0001\"\\\n""#);
    }

    #[test]
    fn html_pages_never_become_detail() {
        let page = json!("  <html>");
        let fields = error_fields(&json!(404), Some(&page), "GET /x");
        assert_eq!(fields["detail"], json!("not found"));
        let fields = error_fields(&json!(500), Some(&page), "GET /x");
        assert_eq!(fields["detail"], json!("unexpected page response"));
    }

    #[test]
    fn detail_is_cut_to_250_units() {
        let long = json!("x".repeat(400));
        let fields = error_fields(&json!(400), Some(&long), "POST /y");
        assert_eq!(fields["detail"].as_str().unwrap().len(), 250);
    }
}

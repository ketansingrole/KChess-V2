//! `JSON.stringify`-compatible text, UTF-16 lengths and slicing, and study-name cleanup.
//! Split from the library store; the other storage modules share this codec so stored
//! documents keep matching the TypeScript bytes.

use serde_json::Value;

use kchess_domain::js;

/// `JSON.stringify(value)`: the same text for the values the core stores (keys in order, whole
/// numbers without `.0`, no spaces).
pub fn js_json(value: &Value) -> String {
    let mut out = String::new();
    write_js_json(value, &mut out);
    out
}

fn write_js_json(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_js_json(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                match serde_json::to_string(key) {
                    Ok(encoded) => out.push_str(&encoded),
                    Err(cause) => {
                        tracing::warn!("library JSON key encoding failed: {cause}");
                        out.push_str("null");
                    }
                }
                out.push(':');
                write_js_json(item, out);
            }
            out.push('}');
        }
        Value::Number(n) => match n.as_f64() {
            Some(x) if n.is_f64() && x.is_finite() && x.fract() == 0.0 && x.abs() < 9.0e15 => {
                out.push_str(&(x as i64).to_string());
            }
            _ => out.push_str(&n.to_string()),
        },
        other => match serde_json::to_string(other) {
            Ok(encoded) => out.push_str(&encoded),
            Err(cause) => {
                tracing::warn!("library JSON value encoding failed: {cause}");
                out.push_str("null");
            }
        },
    }
}

/// `s.length`: UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    js::utf16_len(s)
}

/// `s.slice(0, max)` in UTF-16 units. A cut inside a surrogate pair drops the whole character
/// (JavaScript would keep a lone surrogate, which a Rust string cannot hold).
pub fn js_slice(s: &str, max: usize) -> String {
    let mut units = 0;
    let mut end = 0;
    for (index, c) in s.char_indices() {
        let next = units + c.len_utf16();
        if next > max {
            break;
        }
        units = next;
        end = index + c.len_utf8();
    }
    s[..end].to_string()
}

/// `name.trim().slice(0, 120)`
pub fn clean_name(name: &str) -> String {
    js_slice(js::trim(name), 120)
}

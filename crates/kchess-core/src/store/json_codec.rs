//! Compact JSON text, UTF-16 lengths and slicing, and study-name cleanup. The storage modules
//! share this codec so stored documents keep the bytes earlier releases wrote.

use serde_json::Value;

use kchess_domain::js;

/// Compact JSON text for the values the core stores (keys in order, whole
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
                // Serializing a string or a scalar cannot fail; `Display` is the infallible path.
                out.push_str(&Value::from(key.as_str()).to_string());
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
        other => out.push_str(&other.to_string()),
    }
}

/// At most 120 UTF-16 units of the trimmed name.
pub fn clean_name(name: &str) -> String {
    js::slice_utf16(js::trim(name), 120).to_string()
}

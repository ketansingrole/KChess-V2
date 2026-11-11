//! JavaScript value semantics that stored documents and the rules corpus depend on (string
//! lengths in UTF-16 units, `\s`/`trim`, `Number(…)` and `String(…)` coercion,
//! `Math.round/max/min`, `JSON.stringify`). Every crate takes these from here rather than keeping
//! its own copy.

use serde_json::{Map, Value};

/// Characters matched by JavaScript's `\s` and removed by `String.prototype.trim`.
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

pub fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `/^\s*$/.test(s)`
pub fn is_blank(s: &str) -> bool {
    s.chars().all(is_space)
}

/// `s.length`: UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(0, max)` in UTF-16 units. A cut inside a surrogate pair drops the whole character
/// (JavaScript would keep a lone surrogate, which a Rust string cannot hold).
pub fn slice_utf16(s: &str, max: usize) -> &str {
    let mut units = 0;
    for (index, c) in s.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &s[..index];
        }
    }
    s
}

/// `typeof v === 'string' && v.length <= max`
pub fn text(v: Option<&Value>, max: usize) -> Option<&str> {
    match v {
        Some(Value::String(s)) if utf16_len(s) <= max => Some(s),
        _ => None,
    }
}

pub fn string(v: Option<&Value>) -> Option<&str> {
    match v {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// `Number.isFinite(v)`: a number (JSON numbers are always finite).
pub fn finite(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        _ => None,
    }
}

/// `Number.isInteger(v)`
pub fn integer(v: Option<&Value>) -> Option<f64> {
    finite(v).filter(|n| n.fract() == 0.0)
}

/// `!v`
pub fn falsy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => !b,
        Some(Value::Number(n)) => n.as_f64().is_none_or(|n| n == 0.0 || n.is_nan()),
        Some(Value::String(s)) => s.is_empty(),
        _ => false,
    }
}

/// `!!v`
pub fn truthy(v: Option<&Value>) -> bool {
    !falsy(v)
}

/// `Math.round(x)`: halves round toward positive infinity, unlike Rust's `round`, and the sign
/// of zero is kept for -0.5 ≤ x ≤ -0.
pub fn round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let floor = x.floor();
    // `x - floor` is exact, where `x + 0.5` rounds 0.49999999999999994 up to 1.
    let rounded = if x - floor >= 0.5 { floor + 1.0 } else { floor };
    if rounded == 0.0 && x.is_sign_negative() {
        -0.0
    } else {
        rounded
    }
}

/// `Math.max(a, b)`: NaN wins, and +0 is larger than −0.
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b || (a == b && a.is_sign_positive()) {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)`: NaN wins, and −0 is smaller than +0.
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// `Number(v)`
pub fn to_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => string_to_number(s),
        Some(Value::Array(items)) => match items.as_slice() {
            [] => 0.0,
            [only] => string_to_number(&to_string(Some(only))),
            _ => f64::NAN,
        },
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// `Number(s)` for a string.
pub fn string_to_number(s: &str) -> f64 {
    // Plain digits, by far the common case, need no general parse.
    if (1..=15).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) {
        return s.bytes().fold(0u64, |n, b| n * 10 + u64::from(b - b'0')) as f64;
    }
    let s = trim(s);
    if s.is_empty() {
        return 0.0;
    }
    let radix = |prefix: &[&str], radix: u32| {
        prefix
            .iter()
            .find_map(|p| s.strip_prefix(p))
            .map(|digits| u64::from_str_radix(digits, radix).map_or(f64::NAN, |n| n as f64))
    };
    if let Some(n) = radix(&["0x", "0X"], 16)
        .or_else(|| radix(&["0o", "0O"], 8))
        .or_else(|| radix(&["0b", "0B"], 2))
    {
        return n;
    }
    match s {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ if s
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b)) =>
        {
            s.parse().unwrap_or(f64::NAN)
        }
        _ => f64::NAN,
    }
}

/// `String(v)`
pub fn to_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.as_f64().map_or_else(|| n.to_string(), number_to_string),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => to_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `String(n)`, `NaN` and the infinities included.
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n.is_sign_positive() {
            "Infinity"
        } else {
            "-Infinity"
        }
        .into();
    }
    if n == 0.0 {
        return "0".into();
    }
    // Shortest round-trip digits and exponent, then JavaScript's layout rules.
    let sci = format!("{:e}", n.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k = digits.len() as i32;
    let point = exp + 1; // position of the decimal point relative to the digits
    let body = if (1..=21).contains(&point) && k <= point {
        format!("{digits}{}", "0".repeat((point - k) as usize))
    } else if (1..=21).contains(&point) {
        format!(
            "{}.{}",
            &digits[..point as usize],
            &digits[point as usize..]
        )
    } else if (-5..=0).contains(&point) {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign}{}", exp.abs())
        } else {
            format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], exp.abs())
        }
    };
    if n < 0.0 { format!("-{body}") } else { body }
}

/// A number as `JSON.stringify` keeps it: whole numbers without a fraction, NaN and the
/// infinities as null.
pub fn number_to_json(n: f64) -> Value {
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
    if n.fract() == 0.0 && n.abs() <= MAX_SAFE {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

/// `JSON.stringify(value)`: compact, with JavaScript's number text and key order (integer-like
/// keys first, in order, then the rest as they were written).
pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write_json(value, &mut out);
    out
}

fn write_json(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => match number.as_f64() {
            Some(n) => out.push_str(&number_to_string(n)),
            None => out.push_str("null"),
        },
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

    #[test]
    fn numbers_print_like_javascript() {
        for (n, s) in [
            (1.0, "1"),
            (1.5, "1.5"),
            (-0.001, "-0.001"),
            (1e21, "1e+21"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (2.5e-10, "2.5e-10"),
        ] {
            assert_eq!(number_to_string(n), s);
        }
    }

    #[test]
    fn coercion_matches_javascript() {
        use serde_json::json;
        assert_eq!(to_number(Some(&json!("2"))), 2.0);
        assert_eq!(to_number(Some(&json!(" 1 "))), 1.0);
        assert_eq!(to_number(Some(&json!(true))), 1.0);
        assert_eq!(to_number(Some(&json!([2]))), 2.0);
        assert_eq!(to_number(Some(&json!(null))), 0.0);
        assert!(to_number(Some(&json!("x"))).is_nan());
        assert!(to_number(None).is_nan());
        assert_eq!(utf16_len("a😀"), 3);
        assert!(truthy(Some(&json!(1))) && !truthy(Some(&json!(""))) && !truthy(None));
    }

    #[test]
    fn math_matches_javascript() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(0.49999999999999994), 0.0);
        assert!(round(-0.3).is_sign_negative());
        assert_eq!(round(f64::INFINITY), f64::INFINITY);
        assert!(round(f64::NAN).is_nan());
        assert!(max(f64::NAN, 1.0).is_nan() && min(1.0, f64::NAN).is_nan());
        assert!(max(-0.0, 0.0).is_sign_positive() && max(0.0, -0.0).is_sign_positive());
        assert!(min(0.0, -0.0).is_sign_negative() && min(-0.0, 0.0).is_sign_negative());
        assert_eq!((max(1.0, 2.0), min(1.0, 2.0)), (2.0, 1.0));
        assert_eq!(number_to_string(f64::NAN), "NaN");
        assert_eq!(number_to_string(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(number_to_string(-0.0), "0");
    }

    #[test]
    fn json_text_matches_javascript() {
        use serde_json::json;
        assert_eq!(
            stringify(&json!({ "b": 1.0, "a": [true, null] })),
            r#"{"b":1,"a":[true,null]}"#
        );
        // Integer-like keys come first, in order, as in JavaScript objects.
        let object: Value = serde_json::from_str(r#"{"b":1,"10":"x","2":null}"#).unwrap();
        assert_eq!(stringify(&object), r#"{"2":null,"10":"x","b":1}"#);
        assert_eq!(stringify(&json!("a\u{1}\"\\\n")), r#""a\u0001\"\\\n""#);
        assert_eq!(stringify(&json!(1e16)), "10000000000000000");
        assert_eq!(number_to_json(3.0), json!(3));
        assert_eq!(number_to_json(0.25), json!(0.25));
        assert_eq!(number_to_json(f64::NAN), Value::Null);
    }

    #[test]
    fn slices_count_utf16_units() {
        assert_eq!(slice_utf16("ab😀cd", 3), "ab");
        assert_eq!(slice_utf16("ab😀cd", 4), "ab😀");
        assert_eq!(slice_utf16("abc", 10), "abc");
        assert_eq!(slice_utf16("abc", 0), "");
    }
}

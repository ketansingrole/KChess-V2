//! JavaScript value semantics the TypeScript core relies on (string lengths in UTF-16 units,
//! `\s`/`trim`, `Number(…)` and `String(…)` coercion), so decoders accept and reject exactly
//! what the TypeScript ones do.

use serde_json::Value;

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

/// `Number.prototype.toString()` for finite numbers.
pub fn number_to_string(n: f64) -> String {
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
    }
}

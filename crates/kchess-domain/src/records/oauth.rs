//! `core/src/domain/oauthLook.ts`: the theme of the OAuth callback page. Anything malformed
//! falls back to the default look, so the page never takes a colour it cannot trust.

use serde_json::{Map, Value, json};

use super::{Out, arg};
use crate::js;

/// The colour keys of one scheme, in their order.
const COLOR_KEYS: &[&str] = &["bg", "elevated", "text", "textMuted", "primary", "border"];

/// `APPEARANCES` (`core/src/contracts/types.ts`).
const APPEARANCES: &[&str] = &["system", "light", "dark"];

/// `DEFAULT_OAUTH_LOOK`: KChess's default look, used until the app sends its own colours.
pub fn default_look() -> Value {
    json!({
        "appearance": "system",
        "light": {
            "bg": "#ffffff",
            "elevated": "#f1f5f9",
            "text": "#0f172a",
            "textMuted": "#64748b",
            "primary": "#4b6231",
            "border": "#e2e8f0",
        },
        "dark": {
            "bg": "#0f172a",
            "elevated": "#1e293b",
            "text": "#f1f5f9",
            "textMuted": "#94a3b8",
            "primary": "#91af6a",
            "border": "#334155",
        },
    })
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "oauthLook" => Ok(look(arg(args, 0))),
        "oauthLookDefault" => Ok(default_look()),
        _ => return None,
    })
}

/// `oauthLook(value)`: the requested look when every colour and the appearance are valid, else
/// null; the TypeScript side then answers with its default look.
fn look(value: Option<&Value>) -> Value {
    let Some(Value::Object(map)) = value else {
        return Value::Null;
    };
    let appearance = map.get("appearance").and_then(Value::as_str);
    let light = colors(map.get("light"));
    let dark = colors(map.get("dark"));
    match (appearance, light, dark) {
        (Some(appearance), Some(light), Some(dark)) if APPEARANCES.contains(&appearance) => {
            json!({ "appearance": appearance, "light": light, "dark": dark })
        }
        _ => Value::Null,
    }
}

/// One scheme's colours, when every key is a colour of acceptable form and length.
fn colors(value: Option<&Value>) -> Option<Value> {
    let Some(Value::Object(map)) = value else {
        return None;
    };
    let mut out = Map::new();
    for key in COLOR_KEYS {
        let color = map.get(*key)?.as_str()?;
        if js::utf16_len(color) > 240 || !css_color(color) {
            return None;
        }
        out.insert((*key).to_string(), json!(color));
    }
    Some(Value::Object(out))
}

/// `/^(?:#[0-9a-f]{3,8}|color-mix\(in srgb,[#0-9a-z(),.% -]+\))$/i`: a hex colour, or a
/// `color-mix(in srgb, …)` of such characters. No `;`, brace or `<` can leave the declaration.
fn css_color(color: &str) -> bool {
    if let Some(hex) = color.strip_prefix('#') {
        return (3..=8).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    const PREFIX: &str = "color-mix(in srgb,";
    let Some(head) = color.get(..PREFIX.len()) else {
        return false;
    };
    if !head.eq_ignore_ascii_case(PREFIX) || !color.ends_with(')') {
        return false;
    }
    let Some(middle) = color.get(PREFIX.len()..color.len() - 1) else {
        return false;
    };
    !middle.is_empty()
        && middle
            .bytes()
            .all(|b| b == b'#' || b.is_ascii_alphanumeric() || b"(),.% -".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_follow_the_declaration_grammar() {
        assert!(css_color("#abc"));
        assert!(css_color("#ABCDEF80"));
        assert!(!css_color("#ab"));
        assert!(!css_color("#fffffffff"));
        assert!(css_color("color-mix(in srgb, #fff 50%, #000)"));
        assert!(css_color("COLOR-MIX(IN SRGB, red)"));
        assert!(!css_color("color-mix(in srgb,)"));
        assert!(!css_color("color-mix(in srgb, #fff;)"));
        assert!(!css_color("red"));
        assert!(!css_color("url(x)"));
    }

    #[test]
    fn a_malformed_look_is_declined() {
        let good = default_look();
        assert_eq!(look(Some(&good)), good);
        assert_eq!(look(Some(&json!({ "appearance": "neon" }))), Value::Null);
        assert_eq!(look(None), Value::Null);
    }
}

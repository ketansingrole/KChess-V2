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
        "oauthPage" => {
            let authorized = arg(args, 0).and_then(Value::as_bool).unwrap_or(false);
            let look = arg(args, 1).cloned().unwrap_or_else(default_look);
            Ok(Value::String(callback_page(authorized, &look)))
        }
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

const APP_ICON_SVG: &str = r##"<svg viewBox="0 0 1024 1024"><defs><linearGradient id="tile" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F7DC90"/><stop offset="1" stop-color="#D8A23F"/></linearGradient><linearGradient id="rook" gradientUnits="userSpaceOnUse" x1="0" y1="204" x2="0" y2="820"><stop offset="0" stop-color="#3B2D20"/><stop offset="1" stop-color="#1E1610"/></linearGradient><filter id="shadow" x="-10%" y="-10%" width="120%" height="125%"><feDropShadow dx="0" dy="14" stdDeviation="12" flood-color="#140a00" flood-opacity="0.38"/></filter></defs><polygon points="716.10,0.00 733.82,0.07 746.29,0.28 757.33,0.64 767.51,1.14 777.10,1.78 786.22,2.56 794.97,3.48 803.40,4.55 811.56,5.75 819.46,7.10 827.14,8.59 834.62,10.22 841.90,11.99 849.00,13.90 855.93,15.94 862.69,18.13 869.29,20.46 875.74,22.93 882.04,25.53 888.19,28.28 894.20,31.16 900.06,34.17 905.79,37.33 911.37,40.62 916.82,44.05 922.13,47.61 927.31,51.31 932.35,55.14 937.26,59.11 942.04,63.21 946.67,67.45 951.18,71.82 955.55,76.33 959.79,80.96 963.89,85.74 967.86,90.65 971.69,95.69 975.39,100.87 978.95,106.18 982.38,111.63 985.67,117.21 988.83,122.94 991.84,128.80 994.72,134.81 997.47,140.96 1000.07,147.26 1002.54,153.71 1004.87,160.31 1007.06,167.07 1009.10,174.00 1011.01,181.10 1012.78,188.38 1014.41,195.86 1015.90,203.54 1017.25,211.44 1018.45,219.60 1019.52,228.03 1020.44,236.78 1021.22,245.90 1021.86,255.49 1022.36,265.67 1022.72,276.71 1022.93,289.18 1023.00,306.90 1023.00,716.10 1022.93,733.82 1022.72,746.29 1022.36,757.33 1021.86,767.51 1021.22,777.10 1020.44,786.22 1019.52,794.97 1018.45,803.40 1017.25,811.56 1015.90,819.46 1014.41,827.14 1012.78,834.62 1011.01,841.90 1009.10,849.00 1007.06,855.93 1004.87,862.69 1002.54,869.29 1000.07,875.74 997.47,882.04 994.72,888.19 991.84,894.20 988.83,900.06 985.67,905.79 982.38,911.37 978.95,916.82 975.39,922.13 971.69,927.31 967.86,932.35 963.89,937.26 959.79,942.04 955.55,946.67 951.18,951.18 946.67,955.55 942.04,959.79 937.26,963.89 932.35,967.86 927.31,971.69 922.13,975.39 916.82,978.95 911.37,982.38 905.79,985.67 900.06,988.83 894.20,991.84 888.19,994.72 882.04,997.47 875.74,1000.07 869.29,1002.54 862.69,1004.87 855.93,1007.06 849.00,1009.10 841.90,1011.01 834.62,1012.78 827.14,1014.41 819.46,1015.90 811.56,1017.25 803.40,1018.45 794.97,1019.52 786.22,1020.44 777.10,1021.22 767.51,1021.86 757.33,1022.36 746.29,1022.72 733.82,1022.93 716.10,1023.00 306.90,1023.00 289.18,1022.93 276.71,1022.72 265.67,1022.36 255.49,1021.86 245.90,1021.22 236.78,1020.44 228.03,1019.52 219.60,1018.45 211.44,1017.25 203.54,1015.90 195.86,1014.41 188.38,1012.78 181.10,1011.01 174.00,1009.10 167.07,1007.06 160.31,1004.87 153.71,1002.54 147.26,1000.07 140.96,997.47 134.81,994.72 128.80,991.84 122.94,988.83 117.21,985.67 111.63,982.38 106.18,978.95 100.87,975.39 95.69,971.69 90.65,967.86 85.74,963.89 80.96,959.79 76.33,955.55 71.82,951.18 67.45,946.67 63.21,942.04 59.11,937.26 55.14,932.35 51.31,927.31 47.61,922.13 44.05,916.82 40.62,911.37 37.33,905.79 34.17,900.06 31.16,894.20 28.28,888.19 25.53,882.04 22.93,875.74 20.46,869.29 18.13,862.69 15.94,855.93 13.90,849.00 11.99,841.90 10.22,834.62 8.59,827.14 7.10,819.46 5.75,811.56 4.55,803.40 3.48,794.97 2.56,786.22 1.78,777.10 1.14,767.51 0.64,757.33 0.28,746.29 0.07,733.82 0.00,716.10 0.00,306.90 0.07,289.18 0.28,276.71 0.64,265.67 1.14,255.49 1.78,245.90 2.56,236.78 3.48,228.03 4.55,219.60 5.75,211.44 7.10,203.54 8.59,195.86 10.22,188.38 11.99,181.10 13.90,174.00 15.94,167.07 18.13,160.31 20.46,153.71 22.93,147.26 25.53,140.96 28.28,134.81 31.16,128.80 34.17,122.94 37.33,117.21 40.62,111.63 44.05,106.18 47.61,100.87 51.31,95.69 55.14,90.65 59.11,85.74 63.21,80.96 67.45,76.33 71.82,71.82 76.33,67.45 80.96,63.21 85.74,59.11 90.65,55.14 95.69,51.31 100.87,47.61 106.18,44.05 111.63,40.62 117.21,37.33 122.94,34.17 128.80,31.16 134.81,28.28 140.96,25.53 147.26,22.93 153.71,20.46 160.31,18.13 167.07,15.94 174.00,13.90 181.10,11.99 188.38,10.22 195.86,8.59 203.54,7.10 211.44,5.75 219.60,4.55 228.03,3.48 236.78,2.56 245.90,1.78 255.49,1.14 265.67,0.64 276.71,0.28 289.18,0.07 306.90,0.00" fill="url(#tile)"/><g fill="url(#rook)" filter="url(#shadow)"><rect x="254.4" y="758.4" width="515.2" height="61.6" rx="20.2"/><rect x="288.0" y="696.8" width="448.0" height="69.4" rx="17.9"/><polygon points="366.4,366.4 657.6,366.4 702.4,708.0 321.6,708.0"/><rect x="299.2" y="316.0" width="425.6" height="58.2" rx="13.4"/><rect x="310.4" y="260.0" width="403.2" height="58.2" rx="0.0"/><rect x="310.4" y="204.0" width="100.8" height="114.2" rx="7.8"/><rect x="461.6" y="204.0" width="100.8" height="114.2" rx="7.8"/><rect x="612.8" y="204.0" width="100.8" height="114.2" rx="7.8"/></g></svg>"##;

fn colours(look: &Value) -> String {
    let field = |key: &str| look[key].as_str().unwrap_or("").to_string();
    format!(
        "--bg:{};--card:{};--text:{};--muted:{};--primary:{};--border:{};",
        field("bg"),
        field("elevated"),
        field("text"),
        field("textMuted"),
        field("primary"),
        field("border"),
    )
}

/// `oauthPage(authorized, look)`: a self-contained callback page. It loads nothing external and
/// carries no credentials or callback values.
pub fn callback_page(authorized: bool, look: &Value) -> String {
    let title = if authorized {
        "Signed in to Lichess"
    } else {
        "Connection cancelled"
    };
    let message = if authorized {
        "KChess is finishing the connection and has been brought back to the front."
    } else {
        "Your Lichess account was not connected. Return to KChess to try again when you’re ready."
    };
    let hint = if authorized {
        "You can close this browser tab and continue in KChess."
    } else {
        "You can close this browser tab."
    };
    let appearance = look["appearance"].as_str().unwrap_or("system");
    let theme = if appearance == "system" {
        format!(
            ":root{{color-scheme:light;{}}}@media(prefers-color-scheme:dark){{:root{{color-scheme:dark;{}}}}}",
            colours(&look["light"]),
            colours(&look["dark"]),
        )
    } else {
        format!(
            ":root{{color-scheme:{appearance};{}}}",
            colours(&look[appearance]),
        )
    };
    let icon = APP_ICON_SVG.replacen("<svg ", "<svg aria-hidden=\"true\" ", 1);
    let mark = if authorized { "✓" } else { "↩" };
    PAGE_TEMPLATE
        .replace("__THEME__", &theme)
        .replace("__TITLE__", title)
        .replace("__ICON__", &icon)
        .replace("__MARK__", mark)
        .replace("__MESSAGE__", message)
        .replace("__HINT__", hint)
}

const PAGE_TEMPLATE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="referrer" content="no-referrer"><title>__TITLE__ · KChess</title>
<style>
__THEME__
*{box-sizing:border-box}html{font-family:Inter,ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;-webkit-font-smoothing:antialiased}body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px;background:radial-gradient(ellipse at top,color-mix(in srgb,var(--primary) 14%,transparent) 0,transparent 60%),var(--bg);color:var(--text)}main{width:100%;max-width:440px;padding:36px;border:1px solid var(--border);border-radius:16px;background:var(--card)}.brand{display:flex;align-items:center;gap:8px;font-size:15px;font-weight:600;color:var(--muted)}.brand svg{width:24px;height:24px;flex:none}.mark{display:grid;place-items:center;width:56px;height:56px;margin:28px 0 20px;border-radius:50%;background:color-mix(in srgb,var(--primary) 16%,transparent);color:var(--primary);font-size:26px;font-weight:700}h1{font-size:24px;line-height:1.25;margin:0 0 12px;letter-spacing:-.02em}p{font-size:15px;line-height:1.6;color:color-mix(in srgb,var(--text) 80%,var(--muted));margin:0 0 20px}.hint{border-top:1px solid var(--border);padding-top:20px;margin:0;font-size:13px;color:var(--muted)}@media(max-width:480px){main{padding:24px}h1{font-size:22px}}
</style></head><body><main><div class="brand">__ICON__KChess</div><div class="mark" aria-hidden="true">__MARK__</div><h1>__TITLE__</h1><p>__MESSAGE__</p><p class="hint">__HINT__</p></main></body></html>"##;

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

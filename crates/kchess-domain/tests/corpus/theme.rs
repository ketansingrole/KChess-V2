use serde_json::Value;

use kchess_domain::theme;

use super::Out;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "presetThemes" => Ok(theme::presets().clone()),
        "defaultThemeColors" => Ok(Value::Object(theme::default_colors(
            args.first().and_then(Value::as_bool).unwrap_or(false),
        ))),
        "themeColors" => {
            let palette = args.first().unwrap_or(&Value::Null);
            let dark = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            theme::colors(palette, dark)
                .map(Value::Object)
                .ok_or_else(|| {
                    "themeColors needs a palette with bg, text and primary colors".to_string()
                })
        }
        _ => return None,
    })
}

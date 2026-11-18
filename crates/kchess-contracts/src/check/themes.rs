//! A custom theme file, checked as the core reads it from the profile's `themes` folder.

use serde::Deserialize;

use kchess_domain::js;
use kchess_domain::misc::patterns;
use kchess_domain::misc::value::J;

use super::*;
use crate::types::{AppTheme, ThemePalette};

const HEX: &str = "Colors must be hex, like #1a2b3c.";
const NAME: &str = "A theme needs a \"name\".";

/// The file as read: every field checked for its own message.
#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentTheme {
    id: String,
    name: String,
    light: Option<SentPalette>,
    dark: Option<SentPalette>,
}

#[derive(Deserialize)]
#[serde(expecting = "an object", rename_all = "camelCase")]
struct SentPalette {
    bg: String,
    text: String,
    primary: String,
    muted: Option<String>,
    elevated: Option<String>,
    accented: Option<String>,
    border: Option<String>,
    text_muted: Option<String>,
    success: Option<String>,
    warning: Option<String>,
    error: Option<String>,
    info: Option<String>,
}

impl SentPalette {
    /// The palette, every color hex.
    fn checked(self) -> Result<ThemePalette, Refusal> {
        let optional = [
            &self.muted,
            &self.elevated,
            &self.accented,
            &self.border,
            &self.text_muted,
            &self.success,
            &self.warning,
            &self.error,
            &self.info,
        ];
        [&self.bg, &self.text, &self.primary]
            .into_iter()
            .chain(optional.into_iter().flatten())
            .try_for_each(|color| matches(color, patterns::hex_color, Some(HEX), "", ""))?;
        Ok(ThemePalette {
            bg: self.bg,
            text: self.text,
            primary: self.primary,
            muted: self.muted,
            elevated: self.elevated,
            accented: self.accented,
            border: self.border,
            text_muted: self.text_muted,
            success: self.success,
            warning: self.warning,
            error: self.error,
            info: self.info,
        })
    }
}

/// A theme file: an id, a name (trimmed) and at least one palette of hex colors.
impl Validate for AppTheme {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentTheme = decode(value, |keys| match keys {
            [] => Some("A theme file must be a JSON object."),
            ["id"] => Some("A theme needs an \"id\"."),
            ["name"] => Some(NAME),
            ["light" | "dark", _] => Some(HEX),
            _ => None,
        })?;
        let id = Some("The \"id\" must be 1–32 characters: a–z, 0–9 and dashes.");
        matches(&sent.id, patterns::theme_id, id, "", "")?;
        let name = js::trim(&sent.name).to_owned();
        min_chars(&name, 1, Some(NAME), "")?;
        max_chars(&name, 40, Some("The \"name\" is too long."), "")?;
        let light = sent.light.map(SentPalette::checked).transpose()?;
        let dark = sent.dark.map(SentPalette::checked).transpose()?;
        let palette = Some("Add a \"dark\" or \"light\" palette.");
        ensure(light.is_some() || dark.is_some(), palette, "", String::new)?;
        Ok(AppTheme {
            id: sent.id,
            name,
            light,
            dark,
            custom: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_theme_has_a_name_and_one_palette_or_both() {
        let palette = json!({ "bg": "#000000", "text": "#ffffff", "primary": "#1a2b3c" });
        for side in ["light", "dark"] {
            let mut theme = json!({ "id": "night", "name": " Night " });
            theme[side] = palette.clone();
            let theme = check::<AppTheme>(J::from(&theme)).expect("one palette is enough");
            assert_eq!(theme.name, "Night");
        }
        assert_eq!(
            checked::<AppTheme>(json!({ "id": "night", "name": "Night" })),
            Err("Add a \"dark\" or \"light\" palette.".into())
        );
        assert_eq!(
            checked::<AppTheme>(json!({ "id": "night", "name": 1, "dark": palette })),
            Err(NAME.into())
        );
    }
}

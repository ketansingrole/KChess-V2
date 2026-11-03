//! App themes, shared by every frontend: the preset palettes (`data/themes.json`) and the colors
//! a palette resolves to. The presets come from OpenChamber's built-in themes (MIT,
//! <https://github.com/openchamber/openchamber>), reduced to the colors KChess uses, with alpha
//! colors flattened over the theme's own background; names and palettes belong to their
//! original authors (see `THIRD_PARTY_NOTICES.md`).
//!
//! `colors` is what the GPUI app paints with, so a theme looks the same in every frontend. Colors a palette leaves out are mixed from
//! `bg`, `text` and `primary` as CSS `color-mix(in srgb, a p%, b)` would mix them.

use serde_json::{Map, Value};
use std::sync::LazyLock;

const THEMES_JSON: &str = include_str!("../data/themes.json");

static PRESETS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(THEMES_JSON).unwrap_or_else(|_| Value::Array(Vec::new()))
});

/// The preset themes, in the order the picker lists them.
pub fn presets() -> &'static Value {
    &PRESETS
}

/// An sRGB color, each channel 0..=255.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f64, pub f64, pub f64);

impl Rgb {
    /// `#rgb` or `#rrggbb`.
    pub fn parse(text: &str) -> Option<Rgb> {
        let hex = text.trim().strip_prefix('#')?;
        let digit = |i: usize, width: usize| -> Option<f64> {
            let part = hex.get(i..i + width)?;
            let value = u8::from_str_radix(part, 16).ok()?;
            Some(f64::from(if width == 1 { value * 17 } else { value }))
        };
        match hex.len() {
            3 => Some(Rgb(digit(0, 1)?, digit(1, 1)?, digit(2, 1)?)),
            6 => Some(Rgb(digit(0, 2)?, digit(2, 2)?, digit(4, 2)?)),
            _ => None,
        }
    }

    /// `color-mix(in srgb, self share%, other)`.
    pub fn mix(self, share: f64, other: Rgb) -> Rgb {
        let p = share / 100.0;
        let blend = |a: f64, b: f64| a * p + b * (1.0 - p);
        Rgb(
            blend(self.0, other.0),
            blend(self.1, other.1),
            blend(self.2, other.2),
        )
    }

    /// WCAG relative luminance, of the color as `hex` writes it.
    fn luminance(self) -> f64 {
        let linear = |value: f64| {
            let value = value.round().clamp(0.0, 255.0) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.0) + 0.7152 * linear(self.1) + 0.0722 * linear(self.2)
    }

    /// WCAG contrast ratio with `other`, 1 to 21.
    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// This color, or the least mix of it toward `toward` that reaches `ratio` against `against`
    /// (`toward` itself when nothing short of it does).
    pub fn at_least(self, ratio: f64, toward: Rgb, against: Rgb) -> Rgb {
        (0..=100)
            .map(|share| toward.mix(f64::from(share), self))
            .find(|mixed| mixed.contrast(against) >= ratio)
            .unwrap_or(toward)
    }

    /// `#rrggbb`, each channel rounded to the nearest step.
    pub fn hex(self) -> String {
        let channel = |value: f64| value.round().clamp(0.0, 255.0) as u8;
        format!(
            "#{:02x}{:02x}{:02x}",
            channel(self.0),
            channel(self.1),
            channel(self.2)
        )
    }
}

/// The least contrast with the background muted text (`--ui-text-muted`) has.
pub const MUTED_CONTRAST: f64 = 7.0;
/// The least contrast with the background dimmed text (`--ui-text-dimmed`) has: WCAG AA.
pub const DIMMED_CONTRAST: f64 = 4.5;

/// The color names `colors` resolves, as the CSS variables they set.
pub const NAMES: &[&str] = &[
    "--ui-bg",
    "--ui-bg-muted",
    "--ui-bg-elevated",
    "--ui-bg-accented",
    "--ui-bg-inverted",
    "--ui-border",
    "--ui-border-muted",
    "--ui-border-accented",
    "--ui-border-inverted",
    "--ui-text",
    "--ui-text-highlighted",
    "--ui-text-toned",
    "--ui-text-muted",
    "--ui-text-dimmed",
    "--ui-text-inverted",
    "--ui-primary",
    "--ui-success",
    "--ui-warning",
    "--ui-error",
    "--ui-info",
    "--ui-win",
    "--ui-loss",
];

/// Every color a palette resolves to, by CSS variable name; `None` when `bg`, `text` or
/// `primary` is missing or a color is not `#rgb`/`#rrggbb`.
pub fn colors(palette: &Value, dark: bool) -> Option<Map<String, Value>> {
    let given = |name: &str| palette.get(name).and_then(Value::as_str);
    let color = |text: &str| Rgb::parse(text).map(Rgb::hex);
    let bg = Rgb::parse(given("bg")?)?;
    let text = Rgb::parse(given("text")?)?;
    // A given color is kept (written as `#rrggbb`); a missing one is mixed.
    let or_mix = |name: &str, derived: Rgb| -> Option<String> {
        match given(name) {
            Some(value) => color(value),
            None => Some(derived.hex()),
        }
    };
    let or_default = |name: &str, light: &str, night: &str| -> Option<String> {
        color(given(name).unwrap_or(if dark { night } else { light }))
    };
    let muted = or_mix("muted", bg.mix(95.0, text))?;
    let elevated = or_mix("elevated", bg.mix(91.0, text))?;
    let accented = or_mix("accented", bg.mix(84.0, text))?;
    let border = or_mix("border", bg.mix(82.0, text))?;
    // Secondary text stays legible whatever a palette gives: thin grey labels under these
    // ratios read as blurred on a 1x display.
    let text_muted_rgb =
        Rgb::parse(&or_mix("textMuted", text.mix(62.0, bg))?)?.at_least(MUTED_CONTRAST, text, bg);
    let text_muted = text_muted_rgb.hex();
    let border_rgb = Rgb::parse(&border)?;
    let success = or_default("success", "#246b37", "#62b57a")?;
    let error = or_default("error", "#b4232c", "#e07979")?;
    let values = [
        bg.hex(),
        muted,
        elevated,
        accented,
        text.hex(),
        border.clone(),
        border_rgb.mix(55.0, bg).hex(),
        border_rgb.mix(65.0, text).hex(),
        text.hex(),
        text.hex(),
        text.hex(),
        text.mix(80.0, text_muted_rgb).hex(),
        text_muted.clone(),
        text_muted_rgb
            .mix(60.0, bg)
            .at_least(DIMMED_CONTRAST, text, bg)
            .hex(),
        bg.hex(),
        color(given("primary")?)?,
        success.clone(),
        or_default("warning", "#b8862b", "#e0b25c")?,
        error.clone(),
        or_default("info", "#2f7fd0", "#6cb6ff")?,
        success,
        error,
    ];
    Some(
        NAMES
            .iter()
            .zip(values)
            .map(|(name, value)| ((*name).to_string(), Value::String(value)))
            .collect(),
    )
}

/// The colors KChess ships with (the default theme, which has no palette): a graphite ramp warmed
/// a touch toward the board's wood, and the moss accent, as `#rrggbb`. The GPUI app paints with
/// these.
pub fn default_colors(dark: bool) -> Map<String, Value> {
    let values: [&str; 22] = if dark {
        [
            "#171615", "#1c1b1a", "#201f1d", "#2c2a28", "#f5f4f1", "#33312e", "#282725", "#45433f",
            "#f5f4f1", "#ecebe8", "#f5f4f1", "#c4c1bb", "#b0ada6", "#918e88", "#171615", "#91af6a",
            "#62b57a", "#e0b25c", "#e07979", "#6cb6ff", "#62b57a", "#e07979",
        ]
    } else {
        [
            "#fcfcfb", "#f7f6f4", "#f4f3f0", "#e9e7e3", "#1c1b19", "#e4e2dd", "#ecebe7", "#d3d0ca",
            "#1c1b19", "#1c1b19", "#141312", "#48453f", "#514e47", "#6a665d", "#ffffff", "#4b6231",
            "#246b37", "#805500", "#b4232c", "#1d5b91", "#246b37", "#d05f5f",
        ]
    };
    NAMES
        .iter()
        .zip(values)
        .map(|(name, value)| ((*name).to_string(), Value::String(value.to_string())))
        .collect()
}

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "presetThemes" => Ok(presets().clone()),
        "defaultThemeColors" => Ok(Value::Object(default_colors(
            args.first().and_then(Value::as_bool).unwrap_or(false),
        ))),
        "themeColors" => {
            let palette = args.first().unwrap_or(&Value::Null);
            let dark = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            colors(palette, dark).map(Value::Object).ok_or_else(|| {
                "themeColors needs a palette with bg, text and primary colors".to_string()
            })
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_preset_resolves_in_both_modes() {
        let presets = presets().as_array().unwrap();
        assert_eq!(presets.len(), 23);
        for theme in presets {
            for (mode, dark) in [("light", false), ("dark", true)] {
                if let Some(palette) = theme.get(mode) {
                    let resolved = colors(palette, dark).unwrap();
                    assert_eq!(resolved.len(), NAMES.len(), "{}", theme["id"]);
                }
            }
        }
    }

    #[test]
    fn missing_colors_are_mixed_as_css_color_mix_would() {
        let resolved = colors(
            &json!({ "bg": "#ffffff", "text": "#000000", "primary": "#4b6231" }),
            false,
        )
        .unwrap();
        // 95% white over black: 255 * 0.95 = 242.25.
        assert_eq!(resolved["--ui-bg-muted"], "#f2f2f2");
        assert_eq!(resolved["--ui-bg-elevated"], "#e8e8e8");
        assert_eq!(resolved["--ui-border"], "#d1d1d1");
        // 62% black over white (#616161) is lifted to the muted contrast.
        assert_eq!(resolved["--ui-text-muted"], "#595959");
        assert_eq!(resolved["--ui-primary"], "#4b6231");
        assert_eq!(resolved["--ui-success"], "#246b37");
        assert_eq!(resolved["--ui-win"], resolved["--ui-success"]);
    }

    /// Mono's grey labels (#808080 on black, 5.3:1) read as blurred on a 1x display. Secondary
    /// text now has at least `MUTED_CONTRAST`, and dimmed text WCAG AA, in every theme.
    #[test]
    fn secondary_text_is_legible_in_every_theme() {
        let mono = colors(
            &json!({ "bg": "#000000", "text": "#e5e5e5", "textMuted": "#808080", "primary": "#fff" }),
            true,
        )
        .unwrap();
        assert_ne!(mono["--ui-text-muted"], "#808080");
        let presets: Vec<Value> = serde_json::from_value(presets().clone()).unwrap();
        let mut themes: Vec<Map<String, Value>> = vec![default_colors(false), default_colors(true)];
        for preset in &presets {
            for (mode, dark) in [("light", false), ("dark", true)] {
                themes.extend(colors(&preset[mode], dark));
            }
        }
        themes.push(mono);
        for theme in &themes {
            let color = |name: &str| Rgb::parse(theme[name].as_str().unwrap()).unwrap();
            let bg = color("--ui-bg");
            // A palette whose own text is fainter than a floor gets its text there instead.
            let text = color("--ui-text").contrast(bg);
            assert!(
                color("--ui-text-muted").contrast(bg) >= MUTED_CONTRAST.min(text),
                "{theme:?}"
            );
            assert!(
                color("--ui-text-dimmed").contrast(bg) >= DIMMED_CONTRAST.min(text),
                "{theme:?}"
            );
        }
        // A palette that already meets them keeps its colors.
        let calm = colors(
            &json!({ "bg": "#000000", "text": "#ffffff", "textMuted": "#bbbbbb", "primary": "#fff" }),
            true,
        )
        .unwrap();
        assert_eq!(calm["--ui-text-muted"], "#bbbbbb");
    }

    #[test]
    fn given_colors_are_kept_and_short_forms_widened() {
        let resolved = colors(
            &json!({ "bg": "#fff", "text": "#000", "primary": "#abc", "border": "#123456", "error": "#f00" }),
            true,
        )
        .unwrap();
        assert_eq!(resolved["--ui-bg"], "#ffffff");
        assert_eq!(resolved["--ui-primary"], "#aabbcc");
        assert_eq!(resolved["--ui-border"], "#123456");
        assert_eq!(resolved["--ui-loss"], "#ff0000");
        assert_eq!(resolved["--ui-warning"], "#e0b25c");
    }

    #[test]
    fn the_default_look_names_every_color() {
        for dark in [false, true] {
            let colors = default_colors(dark);
            assert_eq!(colors.len(), NAMES.len());
            assert!(
                colors
                    .values()
                    .all(|value| Rgb::parse(value.as_str().unwrap()).is_some())
            );
        }
        assert_eq!(default_colors(false)["--ui-primary"], "#4b6231");
        assert_eq!(default_colors(true)["--ui-bg"], "#171615");
    }

    #[test]
    fn a_palette_without_its_base_colors_is_refused() {
        assert!(colors(&json!({ "bg": "#fff", "text": "#000" }), false).is_none());
        assert!(
            colors(
                &json!({ "bg": "white", "text": "#000", "primary": "#000" }),
                false
            )
            .is_none()
        );
        assert!(call("themeColors", &[json!({})]).unwrap().is_err());
    }
}

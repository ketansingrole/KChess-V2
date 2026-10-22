//! The app's colors: KChess's default look or the theme chosen in Settings, resolved by
//! `kchess_domain::theme` exactly as the Electron renderer resolves them, then given to GPUI Kit's
//! theme so every component paints with them. Follows the system appearance when Settings says so.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig};
use gpui_kit::{App, Window, WindowAppearance};
use kchess_contracts::types::{AppTheme, Appearance, ThemePalette};
use kchess_domain::theme;
use serde_json::{Map, Value, json};

use crate::settings::SettingsHandle;

/// The default theme's id: no palette, KChess's own colors.
const DEFAULT_THEME: &str = "kchess";

/// GPUI Kit's theme keys, and the KChess color each one takes (`--ui-*`, `theme::NAMES`).
/// Keys left out are derived by GPUI Kit from these.
const KEYS: &[(&str, &str)] = &[
    ("background", "--ui-bg"),
    ("foreground", "--ui-text"),
    ("border", "--ui-border"),
    ("input.border", "--ui-border-accented"),
    ("muted.background", "--ui-bg-elevated"),
    ("muted.foreground", "--ui-text-muted"),
    ("accent.background", "--ui-bg-elevated"),
    ("accent.foreground", "--ui-text-highlighted"),
    ("primary.background", "--ui-primary"),
    ("primary.foreground", "--ui-text-inverted"),
    ("secondary.background", "--ui-bg-elevated"),
    ("secondary.foreground", "--ui-text"),
    ("secondary.hover.background", "--ui-bg-accented"),
    ("danger.background", "--ui-error"),
    ("danger.foreground", "--ui-text-inverted"),
    ("success.background", "--ui-success"),
    ("success.foreground", "--ui-text-inverted"),
    ("warning.background", "--ui-warning"),
    ("warning.foreground", "--ui-text-inverted"),
    ("info.background", "--ui-info"),
    ("info.foreground", "--ui-text-inverted"),
    ("ring", "--ui-primary"),
    ("link", "--ui-primary"),
    ("caret", "--ui-primary"),
    ("popover.background", "--ui-bg"),
    ("popover.foreground", "--ui-text"),
    ("sidebar.background", "--ui-bg"),
    ("sidebar.foreground", "--ui-text"),
    ("sidebar.border", "--ui-border"),
    ("sidebar.accent.background", "--ui-bg-elevated"),
    ("sidebar.accent.foreground", "--ui-text-highlighted"),
    ("sidebar.primary.background", "--ui-primary"),
    ("sidebar.primary.foreground", "--ui-text-inverted"),
    ("title_bar.background", "--ui-bg"),
    ("title_bar.border", "--ui-border"),
    ("list.hover.background", "--ui-bg-elevated"),
    ("list.active.background", "--ui-bg-elevated"),
    ("table.hover.background", "--ui-bg-elevated"),
    ("tab_bar.background", "--ui-bg-elevated"),
    ("tab.active.background", "--ui-bg"),
    ("skeleton.background", "--ui-bg-accented"),
    ("switch.background", "--ui-bg-accented"),
    ("progress.bar.background", "--ui-primary"),
    ("slider.background", "--ui-primary"),
    ("base.red", "--ui-error"),
    ("base.green", "--ui-success"),
    ("base.blue", "--ui-info"),
    ("base.yellow", "--ui-warning"),
];

/// Whether the window should be dark: the setting, or the system's appearance.
pub fn is_dark(appearance: Appearance, window: &Window) -> bool {
    match appearance {
        Appearance::Light => false,
        Appearance::Dark => true,
        Appearance::System => matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    }
}

/// The palette a theme has for a mode; a theme with one variant serves both.
fn palette(theme: &AppTheme, dark: bool) -> Option<&ThemePalette> {
    if dark {
        theme.dark.as_ref().or(theme.light.as_ref())
    } else {
        theme.light.as_ref().or(theme.dark.as_ref())
    }
}

/// Every KChess color for theme `id` in a mode; the default look when the theme is unknown.
pub fn colors(id: &str, custom: &[AppTheme], dark: bool) -> Map<String, Value> {
    if id != DEFAULT_THEME {
        let presets: Vec<AppTheme> =
            serde_json::from_value(theme::presets().clone()).unwrap_or_default();
        // A custom theme replaces a preset with the same id, as `allThemes` does.
        let chosen = custom
            .iter()
            .rev()
            .chain(presets.iter())
            .find(|theme| theme.id == id);
        if let Some(resolved) = chosen
            .and_then(|theme| palette(theme, dark))
            .and_then(|palette| theme::colors(&json!(palette), dark))
        {
            return resolved;
        }
    }
    theme::default_colors(dark)
}

/// Paint the app with the chosen theme for the window's appearance.
pub fn apply(window: &mut Window, cx: &mut App) {
    let handle = cx.global::<SettingsHandle>().0.clone();
    let (appearance, light, dark_theme, custom) = {
        let state = handle.read(cx);
        let frontend = &state.settings.frontend;
        (
            frontend.appearance,
            frontend.light_theme.clone(),
            frontend.dark_theme.clone(),
            state.custom_themes.clone(),
        )
    };
    let dark = is_dark(appearance, window);
    let chosen = if dark { &dark_theme } else { &light };
    let resolved = colors(chosen, &custom, dark);
    let mut keys = Map::new();
    for (key, name) in KEYS {
        if let Some(value) = resolved.get(*name) {
            keys.insert((*key).to_string(), value.clone());
        }
    }
    // Nuxt UI's controls: 6px corners (`rounded-md` over a 0.25rem radius), 8px for dialogs.
    let config: Option<ThemeConfig> = serde_json::from_value(json!({
        "name": "KChess",
        "mode": if dark { "dark" } else { "light" },
        "radius": 6,
        "radius.lg": 8,
        "shadow": false,
        "colors": keys,
    }))
    .ok();
    if let Some(config) = config {
        Theme::global_mut(cx).apply_config(&Rc::new(config));
        window.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_look_is_used_for_an_unknown_theme() {
        assert_eq!(colors("gone", &[], false), theme::default_colors(false));
        assert_eq!(
            colors(DEFAULT_THEME, &[], true),
            theme::default_colors(true)
        );
    }

    #[test]
    fn a_custom_theme_replaces_the_preset_with_its_id() {
        let custom: AppTheme = serde_json::from_value(json!({
            "id": "nord", "name": "Mine", "light": { "bg": "#ffffff", "text": "#000000", "primary": "#123456" }
        }))
        .unwrap();
        let resolved = colors("nord", &[custom], true);
        // The only variant serves the dark mode too.
        assert_eq!(resolved["--ui-primary"], "#123456");
        let preset = colors("nord", &[], true);
        assert_ne!(preset["--ui-primary"], "#123456");
    }

    #[test]
    fn every_key_names_a_kchess_color() {
        for (_, name) in KEYS {
            assert!(theme::NAMES.contains(name), "{name}");
        }
    }
}

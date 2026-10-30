//! The app's colors: KChess's default look or the theme chosen in Settings, resolved by
//! `kchess_domain::theme` exactly as the Electron app resolved them, then given to GPUI Kit's
//! theme so every component paints with them. Follows the system appearance when Settings says so.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig};
use gpui_kit::{App, Global, Hsla, Window, WindowAppearance};
use kchess_contracts::types::{AppTheme, Appearance, ThemePalette};
use kchess_domain::theme;
use serde_json::{Map, Value, json};

use crate::settings::SettingsHandle;

/// Corner radii: one scale for every rounded box, so neighbouring surfaces agree. A box nested in
/// another takes a step down the scale (about the outer radius less the gap between them), so
/// their corners stay concentric.
pub mod radii {
    use gpui_kit::{Pixels, px};

    /// Badges, swatches, small chips such as a move in the tree.
    pub const XS: Pixels = px(4.);
    /// Controls: buttons, tabs in a segmented row, tags, tooltips (GPUI Kit's `radius`).
    pub const SM: Pixels = px(6.);
    /// Inputs, list rows, segmented rows, menus and dialogs (GPUI Kit's `radius.lg`).
    pub const MD: Pixels = px(8.);
    /// Boxes inside a card: lists, banners, icon tiles.
    pub const LG: Pixels = px(10.);
    /// Cards, side panels, page tiles and the board.
    pub const XL: Pixels = px(14.);
}

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
    // Navigation: muted items, the current one in the primary color.
    ("sidebar.foreground", "--ui-text-muted"),
    ("sidebar.border", "--ui-border"),
    ("sidebar.accent.background", "--ui-bg-elevated"),
    ("sidebar.accent.foreground", "--ui-primary"),
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

/// KChess's own colors, by name, for what GPUI Kit's theme has no slot for
/// (accented backgrounds, toned and dimmed text, wins and losses): `cx.global::<Palette>()`.
#[derive(Clone, Copy)]
// The whole palette, as the stylesheet defines it; each view reads what it paints with.
#[allow(dead_code)]
pub struct Palette {
    pub dark: bool,
    pub bg: Hsla,
    pub muted: Hsla,
    pub elevated: Hsla,
    pub accented: Hsla,
    pub border: Hsla,
    pub border_accented: Hsla,
    pub text: Hsla,
    pub highlighted: Hsla,
    pub toned: Hsla,
    pub text_muted: Hsla,
    pub dimmed: Hsla,
    pub inverted: Hsla,
    pub primary: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub error: Hsla,
    pub info: Hsla,
    pub win: Hsla,
    pub loss: Hsla,
}

impl Global for Palette {}

impl Palette {
    pub(crate) fn of(colors: &Map<String, Value>, dark: bool) -> Palette {
        let c = |name: &str| {
            colors
                .get(name)
                .and_then(Value::as_str)
                .map(crate::board::looks::hex)
                .unwrap_or_default()
        };
        Palette {
            dark,
            bg: c("--ui-bg"),
            muted: c("--ui-bg-muted"),
            elevated: c("--ui-bg-elevated"),
            accented: c("--ui-bg-accented"),
            border: c("--ui-border"),
            border_accented: c("--ui-border-accented"),
            text: c("--ui-text"),
            highlighted: c("--ui-text-highlighted"),
            toned: c("--ui-text-toned"),
            text_muted: c("--ui-text-muted"),
            dimmed: c("--ui-text-dimmed"),
            inverted: c("--ui-text-inverted"),
            primary: c("--ui-primary"),
            success: c("--ui-success"),
            warning: c("--ui-warning"),
            error: c("--ui-error"),
            info: c("--ui-info"),
            win: c("--ui-win"),
            loss: c("--ui-loss"),
        }
    }
}

/// A review judgment's colour (`--judgment-*`, Lichess's): inaccuracy, mistake or blunder.
pub fn judgment_color(judgment: &str, dark: bool) -> Hsla {
    let hex = match (judgment, dark) {
        ("inaccuracy", false) => 0x3d9ad6,
        ("inaccuracy", true) => 0x56b4e9,
        ("mistake", false) => 0xd48a00,
        ("mistake", true) => 0xe69f00,
        (_, false) => 0xd14848,
        (_, true) => 0xdf5353,
    };
    gpui_kit::rgb(hex).into()
}

/// `color-mix(in srgb, color share%, transparent)`.
pub fn tint(color: Hsla, share: f32) -> Hsla {
    color.opacity(share / 100.)
}

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
    cx.set_global(Palette::of(&resolved, dark));
    let mut keys = Map::new();
    for (key, name) in KEYS {
        if let Some(value) = resolved.get(*name) {
            keys.insert((*key).to_string(), value.clone());
        }
    }
    match kit_config(dark, keys) {
        Ok(config) => {
            Theme::global_mut(cx).apply_config(&Rc::new(config));
            window.refresh();
        }
        Err(error) => crate::log::write(
            kchess_core::Level::Error,
            "theme",
            &format!("GPUI Kit refused the theme: {error}"),
        ),
    }
}

/// GPUI Kit's theme: the colors, controls rounded to `radii::SM` and dialogs to `radii::MD`
/// (whole pixels, as the kit reads them).
fn kit_config(dark: bool, colors: Map<String, Value>) -> serde_json::Result<ThemeConfig> {
    serde_json::from_value(json!({
        "name": "KChess",
        "mode": if dark { "dark" } else { "light" },
        "radius": f32::from(radii::SM) as usize,
        "radius.lg": f32::from(radii::MD) as usize,
        "shadow": false,
        "colors": colors,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A theme GPUI Kit cannot read leaves its components in the kit's own colors and corners.
    #[test]
    fn the_kit_reads_the_theme() {
        for dark in [false, true] {
            let config = kit_config(dark, Map::new()).unwrap();
            assert_eq!(config.radius, Some(6));
            assert_eq!(config.radius_lg, Some(8));
        }
    }

    /// Every rounded box takes its corners from `radii`; only Settings' theme preview, a scale
    /// model of a page, rounds its own.
    #[test]
    fn corners_come_from_the_radius_scale() {
        fn sources(folder: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    sources(&path, found);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    found.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&root, &mut files);
        let mut stray = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let preview = text
                .find("fn theme_preview(")
                .map(|start| (start, start + text[start..].find("\n}").unwrap_or(0)));
            for (at, _) in text.match_indices(".rounded") {
                let call = &text[at..];
                let literal = call.find('(').is_some_and(|open| {
                    call[open + 1..].starts_with("px(") && !call[..open].contains('\n')
                });
                let in_preview = preview.is_some_and(|(start, end)| (start..end).contains(&at));
                if literal && !in_preview {
                    let line = text[..at].lines().count();
                    stray.push(format!(
                        "{}:{line}",
                        file.strip_prefix(&root).unwrap().display()
                    ));
                }
            }
        }
        assert!(stray.is_empty(), "corners outside `radii`: {stray:?}");
    }

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

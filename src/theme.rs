use gpui::{Context, Rgba, WindowAppearance, rgb, rgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppearanceMode {
    System,
    Light,
    Dark,
}

impl AppearanceMode {
    pub fn as_storage_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_storage_str(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("system") {
            Some(Self::System)
        } else if value.eq_ignore_ascii_case("light") {
            Some(Self::Light)
        } else if value.eq_ignore_ascii_case("dark") {
            Some(Self::Dark)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectiveAppearance {
    Light,
    Dark,
}

impl EffectiveAppearance {
    pub fn from_window_appearance(window_appearance: WindowAppearance) -> Self {
        match window_appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub app_bg: Rgba,
    pub top_bar_bg: Rgba,
    pub top_bar_border: Rgba,
    pub sidebar_bg: Rgba,
    pub sidebar_border: Rgba,
    pub panel_bg: Rgba,
    pub surface: Rgba,
    pub surface_alt: Rgba,
    pub surface_hover: Rgba,
    pub surface_selected: Rgba,
    pub border: Rgba,
    pub border_muted: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub accent_bg: Rgba,
    pub accent_bg_hover: Rgba,
    pub accent_border: Rgba,
    pub accent_text: Rgba,
    pub input_bg: Rgba,
    pub input_border: Rgba,
    pub input_focus_border: Rgba,
    pub danger_bg: Rgba,
    pub danger_bg_hover: Rgba,
    pub danger_border: Rgba,
    pub danger_text: Rgba,
    pub status_info_bg: Rgba,
    pub status_info_text: Rgba,
    pub status_error_bg: Rgba,
    pub status_error_text: Rgba,
    pub status_success_bg: Rgba,
    pub status_success_text: Rgba,
    pub selection_bg: Rgba,
    pub overlay_bg: Rgba,
}

impl ThemePalette {
    pub fn for_effective_appearance(effective: EffectiveAppearance) -> Self {
        match effective {
            EffectiveAppearance::Light => Self::light(),
            EffectiveAppearance::Dark => Self::dark(),
        }
    }

    fn light() -> Self {
        Self {
            app_bg: rgba(0xf4f5f7e8),
            top_bar_bg: rgba(0xf7f8faf7),
            top_bar_border: rgba(0xd6d9deff),
            sidebar_bg: rgba(0xf2f3f4d9),
            sidebar_border: rgba(0xd9dce2ff),
            panel_bg: rgba(0xffffffdb),
            surface: rgba(0xffffffed),
            surface_alt: rgba(0xf8fafcff),
            surface_hover: rgba(0xf1f5f9ff),
            surface_selected: rgba(0xe7edf7ff),
            border: rgba(0xdbe1e8ff),
            border_muted: rgba(0xe2e8f088),
            text_primary: rgb(0x0f172a),
            text_secondary: rgb(0x334155),
            text_muted: rgb(0x64748b),
            accent: rgb(0x3f7fe5),
            accent_bg: rgba(0xe8f0feff),
            accent_bg_hover: rgba(0xdbeafeff),
            accent_border: rgba(0x3f7fe544),
            accent_text: rgb(0x1d4ed8),
            input_bg: rgba(0xffffffff),
            input_border: rgba(0xcbd5e1ff),
            input_focus_border: rgb(0x3f7fe5),
            danger_bg: rgba(0xfef2f2ff),
            danger_bg_hover: rgba(0xfee2e2ff),
            danger_border: rgba(0xf8717188),
            danger_text: rgb(0xb91c1c),
            status_info_bg: rgba(0xe0f2feaa),
            status_info_text: rgb(0x0369a1),
            status_error_bg: rgba(0xfee2e2aa),
            status_error_text: rgb(0xb91c1c),
            status_success_bg: rgba(0xdcfce7ff),
            status_success_text: rgb(0x166534),
            selection_bg: rgba(0x3f7fe533),
            overlay_bg: rgba(0xffffffff),
        }
    }

    fn dark() -> Self {
        Self {
            app_bg: rgba(0x11151ce8),
            top_bar_bg: rgba(0x141a23f7),
            top_bar_border: rgba(0x2b3545ff),
            sidebar_bg: rgba(0x171f2bd9),
            sidebar_border: rgba(0x2f3b4fff),
            panel_bg: rgba(0x111923db),
            surface: rgba(0x1a2432ed),
            surface_alt: rgba(0x1f2b3cff),
            surface_hover: rgba(0x26354aff),
            surface_selected: rgba(0x274363ff),
            border: rgba(0x33445cff),
            border_muted: rgba(0x33445c88),
            text_primary: rgb(0xe2e8f0),
            text_secondary: rgb(0xcbd5e1),
            text_muted: rgb(0x94a3b8),
            accent: rgb(0x5ea1ff),
            accent_bg: rgba(0x1f3555ff),
            accent_bg_hover: rgba(0x24426bff),
            accent_border: rgba(0x5ea1ff66),
            accent_text: rgb(0x9ec8ff),
            input_bg: rgba(0x182231ff),
            input_border: rgba(0x3a4d68ff),
            input_focus_border: rgb(0x5ea1ff),
            danger_bg: rgba(0x412126ff),
            danger_bg_hover: rgba(0x50282fff),
            danger_border: rgba(0xf8717199),
            danger_text: rgb(0xfca5a5),
            status_info_bg: rgba(0x0d3a4fcc),
            status_info_text: rgb(0x7dd3fc),
            status_error_bg: rgba(0x4a1e23cc),
            status_error_text: rgb(0xfca5a5),
            status_success_bg: rgba(0x1f3d2ecc),
            status_success_text: rgb(0x86efac),
            selection_bg: rgba(0x5ea1ff55),
            overlay_bg: rgba(0x17212fff),
        }
    }
}

pub fn resolve_effective_appearance(
    mode: AppearanceMode,
    window_appearance: WindowAppearance,
) -> EffectiveAppearance {
    match mode {
        AppearanceMode::System => EffectiveAppearance::from_window_appearance(window_appearance),
        AppearanceMode::Light => EffectiveAppearance::Light,
        AppearanceMode::Dark => EffectiveAppearance::Dark,
    }
}

pub struct ThemeState {
    mode: AppearanceMode,
    effective_appearance: EffectiveAppearance,
    palette: ThemePalette,
}

impl ThemeState {
    pub fn new(mode: AppearanceMode, window_appearance: WindowAppearance) -> Self {
        let effective_appearance = resolve_effective_appearance(mode, window_appearance);
        let palette = ThemePalette::for_effective_appearance(effective_appearance);

        Self {
            mode,
            effective_appearance,
            palette,
        }
    }

    pub fn mode(&self) -> AppearanceMode {
        self.mode
    }

    pub fn palette(&self) -> ThemePalette {
        self.palette
    }

    pub fn set_mode(
        &mut self,
        mode: AppearanceMode,
        window_appearance: WindowAppearance,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.set_mode_without_notify(mode, window_appearance) {
            cx.notify();
            true
        } else {
            false
        }
    }

    pub fn update_for_window_appearance(
        &mut self,
        window_appearance: WindowAppearance,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.update_for_window_appearance_without_notify(window_appearance) {
            cx.notify();
            true
        } else {
            false
        }
    }

    pub fn set_mode_without_notify(
        &mut self,
        mode: AppearanceMode,
        window_appearance: WindowAppearance,
    ) -> bool {
        self.mode = mode;
        self.recompute(window_appearance)
    }

    pub fn update_for_window_appearance_without_notify(
        &mut self,
        window_appearance: WindowAppearance,
    ) -> bool {
        if self.mode != AppearanceMode::System {
            return false;
        }

        self.recompute(window_appearance)
    }

    fn recompute(&mut self, window_appearance: WindowAppearance) -> bool {
        let effective = resolve_effective_appearance(self.mode, window_appearance);
        let palette = ThemePalette::for_effective_appearance(effective);

        let changed = self.effective_appearance != effective;
        self.effective_appearance = effective;
        self.palette = palette;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_effective_appearance_honors_system_mode() {
        assert_eq!(
            resolve_effective_appearance(AppearanceMode::System, WindowAppearance::Light),
            EffectiveAppearance::Light
        );
        assert_eq!(
            resolve_effective_appearance(AppearanceMode::System, WindowAppearance::VibrantLight),
            EffectiveAppearance::Light
        );
        assert_eq!(
            resolve_effective_appearance(AppearanceMode::System, WindowAppearance::Dark),
            EffectiveAppearance::Dark
        );
        assert_eq!(
            resolve_effective_appearance(AppearanceMode::System, WindowAppearance::VibrantDark),
            EffectiveAppearance::Dark
        );
    }

    #[test]
    fn resolve_effective_appearance_honors_explicit_mode() {
        assert_eq!(
            resolve_effective_appearance(AppearanceMode::Light, WindowAppearance::Dark),
            EffectiveAppearance::Light
        );
        assert_eq!(
            resolve_effective_appearance(AppearanceMode::Dark, WindowAppearance::Light),
            EffectiveAppearance::Dark
        );
    }

    #[test]
    fn system_appearance_changes_only_affect_system_mode() {
        let mut system_state = ThemeState::new(AppearanceMode::System, WindowAppearance::Light);
        assert!(system_state.update_for_window_appearance_without_notify(WindowAppearance::Dark));
        assert_eq!(system_state.effective_appearance, EffectiveAppearance::Dark);

        let mut light_state = ThemeState::new(AppearanceMode::Light, WindowAppearance::Light);
        assert!(!light_state.update_for_window_appearance_without_notify(WindowAppearance::Dark));
        assert_eq!(light_state.effective_appearance, EffectiveAppearance::Light);

        let mut dark_state = ThemeState::new(AppearanceMode::Dark, WindowAppearance::Dark);
        assert!(!dark_state.update_for_window_appearance_without_notify(WindowAppearance::Light));
        assert_eq!(dark_state.effective_appearance, EffectiveAppearance::Dark);
    }
}

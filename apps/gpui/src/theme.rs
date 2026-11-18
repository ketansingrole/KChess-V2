//! The app's colors (`kchess_ui::theme`), painted with the themes Settings chose.

use gpui_kit::{App, Window};
pub use kchess_ui::theme::*;

use crate::settings::SettingsHandle;

/// Paint the app with the chosen theme for the window's appearance.
pub fn apply(window: &mut Window, cx: &mut App) {
    let handle = cx.global::<SettingsHandle>().0.clone();
    let choice = {
        let state = handle.read(cx);
        let frontend = &state.settings.frontend;
        ThemeChoice {
            appearance: frontend.appearance,
            light: frontend.light_theme.clone(),
            dark: frontend.dark_theme.clone(),
            custom: state.custom_themes.clone(),
        }
    };
    kchess_ui::theme::apply(window, cx, &choice);
}

//! Text fields with more than GPUI Kit's own: a secret's field with a full-size reveal button.

use gpui_kit::component::input::{Input, InputState};

use super::*;

/// A field for a secret (an entry code), masked until its eye button shows it. GPUI Kit's own
/// `mask_toggle` draws that button 12 px square, too small a target, so this draws a small kit
/// button in the field instead. `state` starts masked (`InputState::masked(true)`).
pub fn secret_input(state: &Entity<InputState>, label: &'static str, cx: &App) -> Input {
    let masked = state.read(cx).presentation().is_masked();
    let field = state.clone();
    Input::new(state).aria_label(label).suffix(
        Button::new("reveal")
            .ghost()
            .small()
            .icon(icon(if masked { "eye" } else { "eye-off" }))
            .accessibility_label(format!(
                "{} {}",
                if masked { "Show" } else { "Hide" },
                label.to_lowercase()
            ))
            .tab_stop(false)
            .on_click(move |_, window, cx| {
                field.update(cx, |field, cx| field.toggle_masked(window, cx))
            }),
    )
}

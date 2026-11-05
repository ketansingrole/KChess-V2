//! The keyboard shortcuts, listed in a dialog (View → Keyboard Shortcuts, ⌘/ or Quick Pick): the
//! app's, and the keys the boards take while no text field has them.

use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use crate::theme::Palette;

/// Cmd on macOS, Ctrl elsewhere, as the menus bind `secondary-`.
const MOD: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl"
};

/// Each group's heading and its rows: what a key does, and the keys (space-separated chips).
pub(crate) fn groups() -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let (back, forward, run_command, redo) = if cfg!(target_os = "macos") {
        ("⌘ [", "⌘ ]", "⇧ ⌘ P", "⇧ ⌘ Z")
    } else {
        ("Alt ←", "Alt →", "Ctrl ⇧ P", "Ctrl Y")
    };
    vec![
        (
            "App",
            vec![
                ("Search", format!("{MOD} K")),
                ("Run a command", run_command.to_string()),
                ("Open a copied game, FEN or link", format!("{MOD} V")),
                ("Settings", format!("{MOD} ,")),
                ("Back", back.to_string()),
                ("Forward", forward.to_string()),
                ("Back to the list", "Esc".to_string()),
                ("Zoom in / out", format!("{MOD} + −")),
                ("Actual size", format!("{MOD} 0")),
                ("Keyboard shortcuts", format!("{MOD} /")),
            ],
        ),
        (
            "Boards",
            vec![
                ("Previous / next move", "← →".to_string()),
                ("First / last move", "Home End".to_string()),
                ("Flip board", "F".to_string()),
                ("Zen mode", "Z".to_string()),
            ],
        ),
        (
            "Analysis board",
            vec![
                ("Undo an edit", format!("{MOD} Z")),
                ("Redo", redo.to_string()),
                ("Engine on or off", "L".to_string()),
                ("Show threat", "X".to_string()),
                ("Play the best move", "Space".to_string()),
                ("Comment", "C".to_string()),
            ],
        ),
        (
            "Lichess games",
            vec![(
                "Confirm / cancel a correspondence move",
                "Enter Esc".to_string(),
            )],
        ),
        (
            "Puzzles",
            vec![("Next puzzle, once one is over", "Enter".to_string())],
        ),
        (
            "Chess clock",
            vec![
                ("Press the clock", "Space".to_string()),
                ("Pause", "P".to_string()),
            ],
        ),
    ]
}

/// Open the list over the window.
pub fn open(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let palette = *cx.global::<Palette>();
        // A short window scrolls the list rather than cut it off.
        let room = (window.viewport_size().height - px(180.)).max(px(160.));
        dialog
            .title("Keyboard shortcuts")
            .w(px(420.))
            .child(list(&palette, room))
    });
}

fn list(palette: &Palette, height: Pixels) -> impl IntoElement {
    v_flex()
        .id("shortcuts")
        .test_support()
        .max_h(height)
        .overflow_y_scroll()
        .gap(px(12.))
        .children(groups().into_iter().map(|(heading, rows)| {
            v_flex()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_muted)
                        .child(heading),
                )
                .children(rows.into_iter().map(|(action, keys)| {
                    h_flex()
                        .justify_between()
                        .gap(px(12.))
                        .text_size(px(13.))
                        .child(action)
                        .child(crate::palette::kbd(&keys, palette))
                }))
        }))
}

#[cfg(test)]
mod tests {
    use super::groups;

    /// Every shortcut names what it does and its keys, once.
    #[test]
    fn every_shortcut_has_its_keys() {
        let mut seen = std::collections::HashSet::new();
        for (heading, rows) in groups() {
            assert!(!rows.is_empty(), "{heading}");
            for (action, keys) in rows {
                assert!(!keys.trim().is_empty(), "{action}");
                assert!(seen.insert((heading, action)), "{heading}: {action} twice");
            }
        }
    }
}

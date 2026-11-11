//! The keyboard shortcuts, listed in a dialog (View → Keyboard Shortcuts, ⌘/ or Quick Pick): the
//! app's, and the keys the boards take while no text field has them.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::theme::Palette;

/// Each group's heading and its rows: what a key does, and the keys (space-separated chips). The
/// app's keys read from their bindings; the boards' own keys are their pages'.
pub(crate) fn groups(cx: &App) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    use crate::menu::{self, *};
    use gpui_kit::component::input::{Copy, Paste, Redo, Undo};
    let keys = |action: &dyn Action| menu::keys(action, cx).unwrap_or_default();
    vec![
        (
            "App",
            vec![
                ("Search", keys(&Search)),
                ("Search this page", keys(&Find)),
                ("New game, study or analysis", keys(&New)),
                ("Run a command", keys(&Commands)),
                ("Open a copied game, FEN or link", keys(&Paste)),
                ("Copy the board's game or position", keys(&Copy)),
                ("Settings", keys(&OpenSettings)),
                ("Back", keys(&GoBack)),
                ("Forward", keys(&GoForward)),
                ("Show / hide the sidebar", keys(&ToggleSidebar)),
                ("Back to the list", "Esc".to_string()),
                (
                    "Zoom in / out",
                    format!(
                        "{} {}",
                        keys(&ZoomIn),
                        menu::key_alone(&ZoomOut, cx).unwrap_or_default()
                    ),
                ),
                ("Actual size", keys(&ActualSize)),
                (
                    "Keyboard shortcuts",
                    menu::all_keys(&ShowShortcuts, cx).join(" "),
                ),
            ],
        ),
        (
            "Boards",
            vec![
                ("Previous / next move", "← →".to_string()),
                ("First / last move", "Home End".to_string()),
                ("Flip board", "F".to_string()),
                (
                    "After Tab to the board: move, pick up or put down, let go",
                    "← ↑ → ↓ Enter Esc".to_string(),
                ),
                (
                    "Take back, against the computer or over the board",
                    keys(&Undo),
                ),
                ("Zen mode", keys(&ToggleZen)),
                (
                    "Undo an edit, on the analysis board or in the editor",
                    keys(&Undo),
                ),
                ("Redo", keys(&Redo)),
            ],
        ),
        (
            "Analysis board",
            vec![
                ("Into / out of a variation", "⇧ → ⇧ ←".to_string()),
                ("Engine on or off", "L".to_string()),
                ("Show threat", "X".to_string()),
                ("Play the best move", "Space".to_string()),
                ("Comment", "C".to_string()),
                ("Explorer", "E".to_string()),
                ("Delete from the move shown", "Delete".to_string()),
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
            vec![
                ("Next puzzle, once one is over", "Enter".to_string()),
                (
                    "Storm, Streak or Rush again, once a run is over",
                    "Enter".to_string(),
                ),
                ("Next due mistake, once one is solved", "Enter".to_string()),
                ("Next lesson step, once one is done", "Enter".to_string()),
            ],
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
    crate::widgets::open_dialog(window, cx, move |dialog, window, cx| {
        let palette = *cx.global::<Palette>();
        // A short window scrolls the list rather than cut it off.
        let room = (window.viewport_size().height - px(180.)).max(px(160.));
        dialog
            .title("Keyboard shortcuts")
            .w(px(420.))
            .child(list(&palette, room, cx))
    });
}

fn list(palette: &Palette, height: Pixels, cx: &App) -> impl IntoElement {
    v_flex()
        .id("shortcuts")
        .test_support()
        .max_h(height)
        .overflow_y_scroll()
        .gap(px(12.))
        .children(groups(cx).into_iter().map(|(heading, rows)| {
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

    /// Every shortcut names what it does and its keys, once, and the app's keys read as their
    /// bindings do.
    #[gpui_kit::test]
    fn every_shortcut_has_its_keys(cx: &mut gpui_kit::TestAppContext) {
        let groups = cx.update(|cx| {
            crate::menu::init(cx);
            groups(cx)
        });
        let app = &groups[0].1;
        let row = |name: &str| {
            app.iter()
                .find(|(action, _)| *action == name)
                .map(|(_, keys)| keys.as_str())
        };
        if cfg!(target_os = "macos") {
            assert_eq!(row("Run a command"), Some("⇧ ⌘ P"));
            assert_eq!(row("Zoom in / out"), Some("⌘ + −"));
            assert_eq!(row("Keyboard shortcuts"), Some("⌘ / ?"));
            assert_eq!(row("Show / hide the sidebar"), Some("⌃ ⌘ S"));
            assert_eq!(row("Back"), Some("⌘ ["));
        } else {
            assert_eq!(row("Run a command"), Some("Ctrl ⇧ P"));
            assert_eq!(row("Zoom in / out"), Some("Ctrl + −"));
            assert_eq!(row("Back"), Some("Alt ←"));
        }
        let mut seen = std::collections::HashSet::new();
        for (heading, rows) in groups {
            assert!(!rows.is_empty(), "{heading}");
            for (action, keys) in rows {
                assert!(!keys.trim().is_empty(), "{action}");
                assert!(seen.insert((heading, action)), "{heading}: {action} twice");
            }
        }
    }
}

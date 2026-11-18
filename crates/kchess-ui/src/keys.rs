//! The keys bound to an action, as the app shows them: "⌘ K" on macOS, "Ctrl K" elsewhere.

use gpui_kit::{Action, App, KeyBinding, Modifiers};

/// The keys bound to `action`, as the shortcuts list, Quick Pick and tooltips show them: "⌘ K" on
/// macOS, "Ctrl K" elsewhere. The first binding made for it outside any context wins (`init`
/// makes the one to show first), else its first.
pub fn keys(action: &dyn Action, cx: &App) -> Option<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let first = keymap.bindings_for_action(action).next().map(binding_label);
    keymap
        .bindings_for_action(action)
        .find(|binding| binding.predicate().is_none())
        .map(binding_label)
        .or(first)
}

/// Every binding of `action`, in the order `init` makes them.
pub fn all_keys(action: &dyn Action, cx: &App) -> Vec<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap
        .bindings_for_action(action)
        .map(binding_label)
        .collect()
}

/// The key of `action`'s binding alone, without its modifiers (the − of ⌘ −).
pub fn key_alone(action: &dyn Action, cx: &App) -> Option<String> {
    keys(action, cx)?.rsplit(' ').next().map(str::to_string)
}

fn binding_label(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|keystroke| keystroke_label(keystroke.modifiers(), keystroke.key()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One keystroke as its chips read: the modifiers in the platform's order, then the key.
fn keystroke_label(modifiers: &Modifiers, key: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let mut parts: Vec<String> = Vec::new();
    if modifiers.control {
        parts.push(if mac { "⌃" } else { "Ctrl" }.into());
    }
    if modifiers.alt {
        parts.push(if mac { "⌥" } else { "Alt" }.into());
    }
    if modifiers.shift {
        parts.push("⇧".into());
    }
    if modifiers.platform {
        parts.push(if mac { "⌘" } else { "Win" }.into());
    }
    parts.push(match key {
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "-" => "−".into(),
        "escape" => "Esc".into(),
        "enter" => "Enter".into(),
        "space" => "Space".into(),
        "delete" => "Delete".into(),
        key => key.to_uppercase(),
    });
    parts.join(" ")
}

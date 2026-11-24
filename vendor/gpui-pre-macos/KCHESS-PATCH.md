# gpui-pre-macos 0.3.14, patched for KChess

This is the published `gpui-pre-macos` 0.3.14 crate (Zed's `gpui_macos`, Apache-2.0, see `LICENSE-APACHE`), used
through `[patch.crates-io]` in the workspace `Cargo.toml`. Everything is unchanged except for the
lines marked `KChess patch`.

## What the patch does: notifications answered with typed text

GPUI's system notifications carry buttons, and a response names the button pressed, but nothing
can be typed into one. KChess lets a correspondence game's "your move" notification take a move
(`apps/gpui/src/notifications/move_reply.rs`). GPUI's types are left as they are, so that `gpui-pre`, the
Linux backend and GPUI Kit stay unpatched; the patch keeps to a convention instead:

- A `SystemNotificationAction` whose `id` starts with `input:` is a text box with its button.
  Its `label` is the button's title, then after a tab the box's placeholder
  (`"Play\tYour move, such as Nf3"`). At most one such action is shown per notification.
- When it is answered, the response's `action_id` is that id, a newline, then what was typed
  (`"input:move\ne4"`).

Here that is a `UNTextInputNotificationAction` in place of a plain action, and the typed text read from the `UNTextInputNotificationResponse` it answers with. A backend without the patch (Linux) shows the button alone, whose response
is its id without text.

## Traffic lights centred by their own height

`set_traffic_light_position` places the buttons' top, so a caller centring them on its title bar
has to guess their height: KChess guessed 10pt, and macOS 27's buttons are about 13pt, which sat
them 2pt low. `set_traffic_light_centre` (declared in `gpui-pre`, `Window::set_traffic_light_centre`)
takes the line their middle goes on instead. `move_traffic_light` then puts their top at that line
less half the live buttons' height, on a whole device pixel (`centred_button_top`), on every
layout pass, so it holds on any macOS and at any interface size. The app calls it from
`apps/gpui/src/interface_zoom.rs`.

`rg "KChess patch" vendor/gpui-pre-macos` lists the changed lines. When GPUI is updated, take the new
release's crate and apply them again, unless it has learned typed replies of its own.

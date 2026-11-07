# gpui-pre-windows 0.3.11, patched for KChess

This is the published `gpui-pre-windows` 0.3.11 crate (Zed's `gpui_windows`, Apache-2.0, see `LICENSE-APACHE`), used
through `[patch.crates-io]` in the workspace `Cargo.toml`. Everything is unchanged except for the
lines marked `KChess patch`.

## What the patch does: notifications answered with typed text

GPUI's system notifications carry buttons, and a response names the button pressed, but nothing
can be typed into one. KChess lets a correspondence game's "your move" notification take a move
(`apps/gpui/src/move_reply.rs`). GPUI's types are left as they are, so that `gpui-pre`, the
Linux backend and GPUI Kit stay unpatched; the patch keeps to a convention instead:

- A `SystemNotificationAction` whose `id` starts with `input:` is a text box with its button.
  Its `label` is the button's title, then after a tab the box's placeholder
  (`"Play\tYour move, such as Nf3"`). At most one such action is shown per notification.
- When it is answered, the response's `action_id` is that id, a newline, then what was typed
  (`"input:move\ne4"`).

Here that is a toast's text box: an `<input>` before the buttons, its button pointing at it with `hint-inputId`, and the typed text read from `ToastActivatedEventArgs::UserInput` (which needs the `Foundation_Collections` feature, added to `Cargo.toml`). A backend without the patch (Linux) shows the button alone, whose response
is its id without text.

`rg "KChess patch" vendor/gpui-pre-windows` lists the changed lines. When GPUI is updated, take the new
release's crate and apply them again, unless it has learned typed replies of its own.

# gpui-pre 0.3.9, patched for KChess

This is the published `gpui-pre` 0.3.9 crate (Zed's GPUI, Apache-2.0, see `LICENSE-APACHE`),
used through `[patch.crates-io]` in the workspace `Cargo.toml`. Its `examples/` and `tests/`
are left out, and so are their targets in `Cargo.toml`. Everything else is unchanged except for
the lines marked `KChess patch`, which add a **window zoom** for the GPUI app's Settings →
Interface size: the whole window scales like a browser's page zoom. GPUI Kit sizes its widgets
in pixels, so scaling `rem` alone could not do this.

## What the patch does

- `Window::zoom` / `Window::set_zoom` (`src/window.rs`). The zoom multiplies the display's scale
  factor, so everything is drawn `zoom` times larger and stays sharp. The logical viewport is
  `1 / zoom` the window's size.
- Input from the platform (`on_input` in `src/window.rs`) is converted by `PlatformInput::scaled`
  (`src/interactive.rs`). This covers pointer, touch, gesture and file-drop positions, and pixel
  scroll distances, from window points into zoomed pixels. Events that GPUI dispatches again
  internally are already converted.
- Positions handed back to the platform are converted the other way: IME caret and candidate
  bounds (`PlatformInputHandler::bounds_for_range`, `invalidate_character_coordinates`) and the
  point lookups (`character_index_for_point`) in `src/platform.rs`.
- `bounds_changed` keeps the zoom when the window is resized or moves to another display.

The app sets the zoom (`apps/gpui/src/interface_zoom.rs`) and moves the macOS traffic lights to
stay centred on its top bar.

## Upgrading GPUI

Copy the new release's `src`, `build.rs`, `resources`, `README.md`, `LICENSE-APACHE` and
`Cargo.toml` (dropping its example, test and bench targets) over this folder. Then re-apply the
`KChess patch` changes; `rg "KChess patch" vendor/gpui-pre` lists them. Check them against the
new release, since input handling may have moved.

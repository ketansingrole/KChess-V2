# gpui-pre 0.3.9, patched for KChess

This is the published `gpui-pre` 0.3.9 crate (Zed's GPUI, Apache-2.0, see `LICENSE-APACHE`),
used through `[patch.crates-io]` in the workspace `Cargo.toml`. Its `examples/` and `tests/`
are left out, and so are their targets in `Cargo.toml`. Everything else is unchanged except for
the lines marked `KChess patch`, which add a **window zoom** for the GPUI app's Settings →
Interface size: the whole window scales like a browser's page zoom (GPUI Kit sizes its widgets
in pixels, so scaling `rem` alone could not do this), and make **reusing cached views** cheaper
(see below).

## What the zoom patch does

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

A second, test-only patch makes the test platform's `show_window_menu` and `start_window_move`
(`src/platform/test/window.rs`) do nothing instead of panicking, so the app's end-to-end tests
(`apps/gpui/src/e2e`) can click controls drawn over its title bar.

The app sets the zoom (`apps/gpui/src/interface_zoom.rs`) and moves the macOS traffic lights to
stay centred on its top bar.

## Reusing cached views

The app draws its shell, pages and their parts as cached views (`AnyView::cached`,
`apps/gpui/src/part.rs`), so a frame redraws only what changed and replays the rest. Four
patches make that replay cheaper; none changes what is drawn.

- **Cached views inside a redrawn cached view are reused** (`prepaint_view` in `src/view.rs`).
  Upstream set `window.refreshing` while a cached view drew again, so every cached view inside it
  drew again too: a page redrawn for its clock redrew its board and panel. Now only a window
  refresh redraws every view.
- **Only from the last frame** (`ViewElementState::drawn_in`, `Window::frames_drawn`). A cached
  view keeps the ranges of the frame it was last drawn or reused in. When a view around it was
  reused whole, those ranges point into an older frame, so it draws again instead (as upstream
  always did). Reusing stale ranges panicked in the text layout cache.
- **Replayed primitives keep their order** (`Scene::replay` in `src/scene.rs`,
  `BoundsTree::insert_span` in `src/bounds_tree.rs`). A reused view's primitives move up together
  above whatever the frame drew under their bounds so far, and their bounds go into the tree once,
  at the highest of their orders. Upstream inserted each primitive into the tree again. Inside a
  layer, each is still inserted.
- **Tab order is sorted when Tab asks for it** (`TabStopMap` in `src/tab_stop.rs`). Upstream kept
  a sorted tree updated on every insert, and a reused view replays all its tab stops each frame.

The app's end-to-end tests draw as the window does (cached views reused), and
`e2e::screens` checks that every page drawn again from its cached frame is pixel for pixel the
picture drawn afresh. `e2e::shell::tab_and_shift_tab_walk_the_same_controls` checks the tab order.

## Upgrading GPUI

Copy the new release's `src`, `build.rs`, `resources`, `README.md`, `LICENSE-APACHE` and
`Cargo.toml` (dropping its example, test and bench targets) over this folder. Then re-apply the
`KChess patch` changes; `rg "KChess patch" vendor/gpui-pre` lists them. Check them against the
new release, since input handling and view caching may have moved.

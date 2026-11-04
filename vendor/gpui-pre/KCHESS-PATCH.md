# gpui-pre 0.3.11, patched for KChess

This is the published `gpui-pre` 0.3.11 crate (Zed's GPUI, Apache-2.0, see `LICENSE-APACHE`),
used through `[patch.crates-io]` in the workspace `Cargo.toml`. Its `examples/` and `tests/`
are left out, and so are their targets in `Cargo.toml`. Everything else is unchanged except for
the lines marked `KChess patch`, which add a **window zoom** for the GPUI app's Settings →
Interface size: the whole window scales like a browser's page zoom (GPUI Kit sizes its widgets
in pixels, so scaling `rem` alone could not do this), make **reusing cached views** cheaper, and
draw **icons with the drawing made for their size** (see below).

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

A third, test-only patch adds `Window::window_control_at` (`src/window.rs`), which answers as the
platform's title-bar hit test does, so the end-to-end tests can check that controls drawn over the
title bar take their own clicks on Windows.

A fourth patch makes a reused view keep its **title-bar areas** (`Window::reuse_paint`, with
`PaintIndex::window_control_hitboxes_index`). Upstream replayed everything else a view paints but
not these, so a frame that reused the top bar left Windows with no drag area or window buttons
until the bar next drew: the bar could not move the window and the buttons did nothing.

The app sets the zoom (`apps/gpui/src/interface_zoom.rs`) and moves the macOS traffic lights to
stay centred on its top bar.

## Reusing cached views

The app draws its shell, pages and their parts as cached views (`AnyView::cached`,
`apps/gpui/src/part.rs`), so a frame redraws only what changed and replays the rest. Four
patches make that replay cheaper, and one keeps shaped text; none changes what is drawn.

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

- **Shaped text is kept for a while** (`LineLayoutCacheState::retained` in
  `src/text_system/line_layout.rs`). Upstream kept a line's shaping for one frame after its
  last use, so a page opened again shaped all its text with CoreText again. Lines no frame has
  used for two frames are kept, up to 16,384 of them, and dropped when new fonts are added.
  `apps/gpui/src/gpui_patches.rs` checks it.

## Icons drawn for their size

An icon set draws each icon on a pixel grid; scaled to another size its lines fall between
pixels and blur on a 1x display (Lucide's 2-unit strokes on a 24-unit grid were 1.17 px wide at
14 px). The app's icons (`apps/gpui/assets/icons`, `cargo xtask icons`) come as `name.svg` plus
the drawings made for each size, `name@16.svg`, `name@20.svg` and so on.

- `SvgRenderer::render_alpha_mask` (`src/svg_renderer.rs`) draws `icons/name.svg` with the
  drawing `icon_drawing` picks for the icon's size in device pixels: one at its own size or
  scaled a whole number of times, at most 8/7 of the box (a 16 px drawing keeps a margin, so it
  fits a 14 px box), preferring the least scale that nearly fills it. The tile is that drawing's
  size, and `paint_svg` centres it in the icon's bounds as before. With no drawing small enough,
  `name.svg` is scaled to the box as upstream does.
- `icon_drawing_for` treats a size the asset source cannot load as absent: GPUI Kit's asset
  source answers an error for a file it does not have, which once hid every icon without a
  12 px drawing.

`apps/gpui/src/gpui_patches.rs` checks the choice and the lookup.

The app's end-to-end tests draw as the window does (cached views reused), and
`e2e::screens` checks that every page drawn again from its cached frame is pixel for pixel the
picture drawn afresh. `e2e::shell::tab_and_shift_tab_walk_the_same_controls` checks the tab order.

## Upgrading GPUI

Copy the new release's `src`, `build.rs`, `resources`, `README.md`, `LICENSE-APACHE` and
`Cargo.toml` (dropping its example, test and bench targets) over this folder. Then re-apply the
`KChess patch` changes; `rg "KChess patch" vendor/gpui-pre` lists them. Check them against the
new release, since input handling and view caching may have moved.

A three-way merge does most of it: the published the previous release as the base, this folder as one side and
the new release as the other (0.3.9 → 0.3.11 conflicted only in `src/window.rs` and
`src/text_system/line_layout.rs`). Keep the gpui-kit pin in `apps/gpui/Cargo.toml` on the
release whose `gpui-pre` this folder holds.

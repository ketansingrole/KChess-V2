# gpui-pre-macos 0.3.9, patched for KChess

This is the published `gpui-pre-macos` 0.3.9 crate (Zed's `gpui_macos`, Apache-2.0, see
`LICENSE-APACHE`), used through `[patch.crates-io]` in the workspace `Cargo.toml`. Everything is
unchanged except for the lines marked `KChess patch` and, in `Cargo.toml`, allowing the
`deprecated` lint (the published crate is built with capped lints, a path dependency is not).

## What the patch does: frames on demand

Upstream drives every window from a `CVDisplayLink` that runs while the window is visible. It
wakes the main thread on every refresh of the display (120 times a second on a ProMotion
display) whether or not anything changed, and each start of the link spawns a thread.

GPUI already supports platforms that stop asking for frames while a window is idle: the window
calls `PlatformWindow::frame_waker` whenever it becomes dirty, after a frame that left it dirty,
and for next-frame callbacks. The patch implements it for macOS (`FrameDemand` in
`src/window.rs`):

- A change on its own (a clock tick, an engine line, a model's update) is drawn as soon as the
  main thread is free, through the window's frame source (`WindowFrameSource::request_frame` in
  `src/display_link.rs`), without starting the link.
- Input, and frames asked for less than 12 ms apart (an animation driven by
  `request_animation_frame`), start the link, which then paces frames to the display.
- The link stops 25 ms after the last frame asked for, or 1 s after the last input (as long as
  GPUI keeps presenting after high-rate input, so the display is not underclocked).
- A link that fails to start is tried again on the next demand.

Occluded windows still draw nothing, as upstream. The app's frame benchmark
(`apps/gpui/src/bench.rs`) reports the time from a change to its frame (`latency_ms_p50`): it is
shorter than upstream's, which waited for the link's next refresh.

## Upgrading GPUI

Copy the new release's `src`, `LICENSE-APACHE` and `Cargo.toml` over this folder, keep the
`[lints.rust.deprecated]` entry, and re-apply the `KChess patch` changes;
`rg "KChess patch" vendor/gpui-pre-macos` lists them.

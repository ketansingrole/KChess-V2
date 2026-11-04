# gpui-pre-apple 0.3.11, patched for KChess

This is the published `gpui-pre-apple` 0.3.11 crate (Zed's `gpui_apple`, Apache-2.0, see
`LICENSE-APACHE`), used through `[patch.crates-io]` in the workspace `Cargo.toml`. Everything is
unchanged except for the lines marked `KChess patch`.

## What the patch does: large glyphs are drawn whole

A glyph is drawn into a bitmap whose size comes from `raster_bounds` (`src/text_system.rs`).
Upstream took it from font-kit, which scales the glyph's outline at the font's own small size.
The system font has optical sizes, so at a large size CoreText draws a different outline, and
the bitmap cut it off: the clock's 120 px time lost half of its colon and the edges of its
digits. The patch also measures the glyph at the size it is drawn and uses the bounds that cover
both. `test_large_system_font_glyphs_are_not_clipped` keeps it.

`rg "KChess patch" vendor/gpui-pre-apple` lists the changed lines.

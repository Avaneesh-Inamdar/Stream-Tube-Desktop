# Patches to `gpui-pre-wgpu`

Copy of [gpui-pre-wgpu 0.3.12](https://crates.io/crates/gpui-pre-wgpu)
(Zed's `gpui_wgpu` crate, Apache-2.0) with one fix in `src/cosmic_text_system.rs`:
right-to-left lines (Arabic, Hebrew, ...) need visual left-to-right glyph ordering.

- `CosmicTextSystem::layout_line_no_separators`: `cosmic-text` lays out RTL
  paragraphs (`line.rtl == true`) from `start_x = line_width` leftwards, so
  `layout.glyphs` is ordered right-to-left with `glyph.x` decreasing down to `0`.
  GPUI's `LineLayout` wrapper and `paint_line` expect glyphs in visual
  left-to-right order (`position.x` non-decreasing from `0` to `layout.width`).
  Without this, wrapped RTL lines compute negative widths and paint subsequent
  lines to the left of `origin.x`, clipping Arabic and Hebrew titles in card views.
  Iterating `layout.glyphs.iter().rev()` when `line.rtl` is `true` emits the
  glyphs in visual left-to-right order.

# epaint 0.36.2, patched for Glass

This is epaint 0.36.2 from crates.io (MIT OR Apache-2.0, © the egui authors,
https://github.com/emilk/egui) with two changes: its font atlas, and where
glyphs of fallback fonts sit on a line. `Cargo.toml` in the app uses it via
`[patch.crates-io]`; everything else is unchanged.

Why: epaint makes the font atlas as wide as the GPU's largest texture (16384 on
a Mac) and doubles its height as glyphs arrive. A row is as tall as its tallest
glyph, so a 16384-wide row that holds one big title is mostly empty. Measured
on Glass's pages at 2x (a Retina screen): 8.4 MB at 16384 wide, 4.2 MB at 2048,
and the same again for the copy on the GPU. egui starts the atlas over at 80% of
a square, so the width also bounds it: 16.8 MB at 2048, where 16384 would let it
grow past 1 GB with enough glyphs (many scripts in lyrics, say).

egui has no setting for this. `RawInput::max_texture_side` sets it, but eframe
doesn't pass immediate viewports (the mini player) through `raw_input_hook`,
and fonts are rebuilt whenever that value differs between passes.

Changes (marked `Glass` in the source):

- `src/text/fonts.rs` — the atlas is at most 2048 wide.
- `src/text/text_layout.rs`, `text_layout_types.rs`, `font.rs` — a glyph from
  a fallback font is centered on the main font by the height of their glyph
  boxes, without the line gap. Upstream counts the line gap, so Japanese in
  Hiragino (line gap half an em) sat a quarter em above the baseline of the
  Latin text around it (SF Mono: 0.967 em; Hiragino: 0.719 em upstream, 0.969
  em now). Fonts without a line gap are placed as before.

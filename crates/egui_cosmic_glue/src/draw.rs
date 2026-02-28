 //! Functions for drawing cosmic‑text layout runs and buffers.

 use crate::atlas::TextureAtlas;
use cosmic_text::{FontSystem, SwashCache, LayoutRun, Buffer};
 use egui::{Painter, Rect};

 /// Draw a single [`cosmic_text::LayoutRun`] onto the painter.  Each glyph
 /// is rasterized via the provided [`TextureAtlas`], then drawn with
 /// [`GlyphImage::paint`].
/// Draw a single [`cosmic_text::LayoutRun`] onto the painter.  Each glyph is
/// rasterized via the provided [`TextureAtlas`], then drawn with
/// [`GlyphImage::paint`].  The `offset` parameter specifies the upper‑left
/// corner of the text area in *physical* pixels (i.e. logical points
/// multiplied by `pixels_per_point`).  Passing a non‑zero offset will
/// translate the glyphs into the desired UI region.  You can obtain
/// `offset` from the clip rectangle by computing `(clip_rect.min.x *
/// pixels_per_point, clip_rect.min.y * pixels_per_point)`.
pub fn draw_run(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    atlas: &mut TextureAtlas,
    run: &LayoutRun<'_>,
    painter: &mut Painter,
    offset: (f32, f32),
) {
    for glyph in run.glyphs.iter() {
        // Convert to a physical glyph.  Apply the offset to place the glyph
        // within the UI region.  See [`LayoutGlyph::physical`] for details.
        let physical = glyph.physical(offset, 1.0);
        if let Some(image) = atlas.alloc(physical.cache_key, font_system, swash_cache) {
            image.paint(glyph, physical, run, painter);
        }
    }
}

 /// Draw all layout runs from a borrow of a [`cosmic_text::Buffer`].  Runs
 /// outside of the clip rectangle are skipped.  The clip rectangle is
 /// specified in logical points, matching `egui`'s coordinate system.
 pub fn draw_buffer(
     font_system: &mut FontSystem,
     swash_cache: &mut SwashCache,
     atlas: &mut TextureAtlas,
    buffer: &Buffer,
     painter: &mut Painter,
     clip_rect: Rect,
 ) {
     let pixels_per_point = painter.ctx().pixels_per_point();
     let clip_min_y = clip_rect.min.y * pixels_per_point;
     let clip_max_y = clip_rect.max.y * pixels_per_point;
    // Precompute the physical offset from the clip rectangle.  `offset` is
    // expressed in physical pixels (i.e. logical points multiplied by
    // `pixels_per_point`).  This offset will be applied to every glyph in
    // the visible runs.
    let offset = (
        clip_rect.min.x * pixels_per_point,
        clip_rect.min.y * pixels_per_point,
    );
    // Iterate over all runs, skipping those outside the vertical clip bounds.
    for run in buffer.layout_runs() {
        let line_top = run.line_top;
        let line_bottom = run.line_top + run.line_height;
        let visible = line_bottom > clip_min_y && line_top <= clip_max_y;
        if !visible {
            continue;
        }
        draw_run(font_system, swash_cache, atlas, &run, painter, offset);
    }
 }

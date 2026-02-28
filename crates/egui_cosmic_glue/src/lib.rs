 //! An integration layer between [`cosmic_text`](https://crates.io/crates/cosmic-text)
 //! and [`egui`](https://crates.io/crates/egui) suitable for projects built
 //! with [`eframe`](https://crates.io/crates/eframe).  This crate provides a
 //! [`TextureAtlas`] for caching glyph rasters and a [`CosmicRenderer`] that
 //! can draw a [`cosmic_text::Buffer`] onto an [`egui::Painter`].  It is
 //! loosely based on the design of the unofficial [`egui_cosmic_text`]
 //! crate, but updated for modern versions of the dependencies.  The code
 //! intentionally avoids implementing an entire text editor.  Instead, it
 //! focuses on efficient rendering of laid‑out text.  See the
 //! [`CosmicRenderer::draw_buffer`] method for an example of how to use
 //! this module.

 mod atlas;
 mod draw;

 pub use atlas::{TextureAtlas, GlyphImage};
 pub use draw::{draw_run, draw_buffer};

 use cosmic_text::{FontSystem, SwashCache};
 use egui::{Color32, Context, Painter, Rect};

 /// Convenience structure that owns a [`FontSystem`], [`SwashCache`] and
 /// [`TextureAtlas`].  You can reuse a single `CosmicRenderer` across many
 /// buffers.  Each frame you should call [`TextureAtlas::trim`] to allow
 /// unused glyphs to be reclaimed.
 pub struct CosmicRenderer {
     /// The font system used for font discovery and shaping.  One per
     /// application is generally sufficient.
     pub font_system: FontSystem,
     /// Cache for rasterizing glyphs with swash.  One per application.
     pub swash_cache: SwashCache,
     /// Dynamic texture atlas holding rasterized glyphs.  It grows as
     /// needed up to the maximum texture size permitted by `egui`.
     pub atlas: TextureAtlas,
 }

 impl CosmicRenderer {
     /// Create a new renderer.  You must provide an [`egui::Context`]
     /// (usually obtained from your `egui` callback) and a default text
     /// color.  The default color is used when rendering glyphs with
     /// [`SwashContent::Mask`], i.e. monochrome glyphs.  Colored glyphs
     /// (emoji or bitmap fonts) bypass this tint and are drawn in their
     /// original colour.
     pub fn new(ctx: &Context, default_color: Color32) -> Self {
         let font_system = FontSystem::new();
         let swash_cache = SwashCache::new();
         let atlas = TextureAtlas::new(ctx.clone(), default_color);
         Self { font_system, swash_cache, atlas }
     }

     /// Render a [`cosmic_text::Buffer`] onto an [`egui::Painter`].  The
     /// buffer will be shaped and laid out before drawing.  A clip
     /// rectangle in logical points can be supplied if you wish to avoid
     /// drawing off‑screen glyphs.  Pass `egui::Rect::EVERYTHING` to draw
     /// all glyphs.  The buffer is borrowed with the renderer's font
     /// system for the duration of this call.
    pub fn draw_buffer(
        &mut self,
        buffer: &mut cosmic_text::Buffer,
        painter: &mut Painter,
        clip_rect: Rect,
    ) {
        // First borrow the buffer with the font system to shape text.  Once the
        // buffer is shaped we drop the borrow to release the mutable borrow on
        // `font_system`.  This allows us to borrow `font_system` again when
        // drawing.
        {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.shape_until_scroll(true);
        }
        // Now draw the buffer.  We iterate over the layout runs directly from
        // the buffer since shaping has already been performed.  Using the
        // buffer directly avoids holding a borrow of the font system while
        // rasterizing glyphs.
        draw_buffer(
            &mut self.font_system,
            &mut self.swash_cache,
            &mut self.atlas,
            buffer,
            painter,
            clip_rect,
        );
    }
 }

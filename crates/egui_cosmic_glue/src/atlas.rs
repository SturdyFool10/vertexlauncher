 //! Texture atlas for storing rasterized glyphs.
 //!
 //! The [`TextureAtlas`] maintains a dynamic GPU texture into which
 //! individual glyph bitmaps are packed on demand.  The glyphs are
 //! rasterized via [`cosmic_text::SwashCache`] and cached in an LRU
 //! cache so that frequently used glyphs can be reused across frames.
 //! When the atlas becomes full, it will first evict unused glyphs and
 //! eventually grow until it reaches `max_texture_side` as reported
 //! by the enclosing [`egui::Context`].

 use cosmic_text::{CacheKey, FontSystem, PhysicalGlyph, Placement, SwashCache, SwashContent, SwashImage};
 use egui::{pos2, vec2, Color32, ColorImage, Context, NumExt, Painter, Rect, TextureHandle, TextureId, TextureOptions, Vec2};
 use etagere::{size2, Allocation, BucketedAtlasAllocator, Size};
 use imgref::{Img, ImgRefMut};
 use lru::LruCache;
 use std::collections::HashSet;
use std::hash::BuildHasher;

 /// Internal state for a cached glyph.  This includes the atlas allocation
 /// rectangle, the placement information returned by swash and whether the
 /// glyph is colorable (monochrome mask) or must be drawn without tint
 /// (colour or subpixel mask).
 #[derive(Clone)]
 struct GlyphState {
     allocation: Allocation,
     placement: Placement,
     colorable: bool,
 }

 /// Write a [`SwashImage`] into a subregion of the atlas.  The
 /// `default_color` parameter is used to tint monochrome glyph masks.  The
 /// destination `sub_image` is an [`imgref::ImgRefMut`] over a slice of
 /// [`Color32`] representing the atlas pixels.
 fn write_glyph_image(image: SwashImage, default_color: Color32, mut sub_image: ImgRefMut<Color32>) {
     debug_assert!(sub_image.width() == image.placement.width as usize && sub_image.height() == image.placement.height as usize);
     match image.content {
         SwashContent::Mask => {
             // 8‑bit alpha mask.  Tint with the default color.
             for (a, slot) in image.data.into_iter().zip(sub_image.pixels_mut()) {
                 *slot = Color32::from_rgba_unmultiplied(default_color.r(), default_color.g(), default_color.b(), a);
             }
         }
         SwashContent::Color => {
             // 32‑bit RGBA bitmap.  Use the supplied colours without tint.
             for (pixel, slot) in image.data.chunks_exact(4).zip(sub_image.pixels_mut()) {
                 // Safety: slice has length 4
                 let [r, g, b, a] = <[u8; 4]>::try_from(pixel).unwrap();
                 *slot = Color32::from_rgba_premultiplied(r, g, b, a);
             }
         }
         SwashContent::SubpixelMask => {
             // Subpixel mask is a 32‑bit RGBA mask with per channel coverage.
             // We treat it like a full colour bitmap.  This will likely
             // result in accurate rendering on most displays.  See
             // https://github.com/pop-os/cosmic-text for details.
             for (pixel, slot) in image.data.chunks_exact(4).zip(sub_image.pixels_mut()) {
                 let [r, g, b, a] = <[u8; 4]>::try_from(pixel).unwrap();
                 *slot = Color32::from_rgba_premultiplied(r, g, b, a);
             }
         }
     }
 }

 /// A drawable glyph image.  It holds the texture ID and UV coordinates
 /// pointing into the atlas along with metrics used to compute its
 /// screen position.  To draw the glyph call [`GlyphImage::paint`].
 pub struct GlyphImage {
     atlas_texture_id: TextureId,
     uv_rect: Rect,
     default_color: Color32,
     colorable: bool,
     top: i32,
     left: i32,
     width: f32,
     height: f32,
 }

 impl GlyphImage {
     fn new(atlas_texture: &TextureHandle, rect: etagere::Rectangle, placement: Placement, default_color: Color32, colorable: bool) -> Self {
         let atlas_texture_id = atlas_texture.id();
         let [atlas_width, atlas_height] = [atlas_texture.size()[0] as f32, atlas_texture.size()[1] as f32];
         let glyph_width = placement.width as f32;
         let glyph_height = placement.height as f32;
         let uv_rect = Rect::from_min_size(
             pos2(rect.min.x as f32 / atlas_width, rect.min.y as f32 / atlas_height),
             vec2(glyph_width / atlas_width, glyph_height / atlas_height),
         );
         Self { atlas_texture_id, uv_rect, default_color, colorable, top: placement.top, left: placement.left, width: glyph_width, height: glyph_height }
     }

     /// Paint the glyph at the given position relative to the run.  The
     /// `layout_glyph` provides the optional colour override; `physical_glyph`
     /// supplies the integer offset to add to the line.  The `run` contains
     /// the y‑position of the baseline in physical pixels.  The painter
     /// converts physical pixels into logical points internally.
     pub fn paint(self, layout_glyph: &cosmic_text::LayoutGlyph, physical_glyph: PhysicalGlyph, run: &cosmic_text::LayoutRun<'_>, painter: &mut Painter) {
         // Compute the position of the glyph.  `physical_glyph.x` and `y` are
         // integer offsets relative to the run.  Add the swash placement
         // offsets for proper positioning.  The run's line_y is in physical
         // pixels too (f32), convert to i32 to match.
         let x = physical_glyph.x + self.left;
         let y = run.line_y as i32 + physical_glyph.y - self.top;
         // Determine the final tint colour.  For monochrome masks we allow
         // overriding the colour via the LayoutGlyph's optional colour.  For
         // coloured glyphs the tint must be white to avoid washing out the
         // colours.
         let color_override = layout_glyph.color_opt.map(|c| Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), c.a()));
         let tint = if self.colorable { color_override.unwrap_or(self.default_color) } else { Color32::WHITE };
         let pixels_per_point = painter.ctx().pixels_per_point();
         painter.image(
             self.atlas_texture_id,
             Rect::from_min_size(pos2(x as f32, y as f32), vec2(self.width, self.height)) / pixels_per_point,
             self.uv_rect,
             tint,
         );
     }
 }

 /// A dynamic atlas used to pack rasterised glyphs into an `egui` texture.
 /// Use [`TextureAtlas::alloc`] to allocate space for a glyph and retrieve
 /// a [`GlyphImage`] that can be painted.  Call [`TextureAtlas::trim`]
 /// each frame to clear out which glyphs are in use.  This allows the
 /// atlas to evict unused glyphs on the next allocation pass.
 pub struct TextureAtlas<S: BuildHasher + Default = std::collections::hash_map::RandomState> {
     packer: BucketedAtlasAllocator,
     cache: LruCache<CacheKey, Option<GlyphState>, S>,
     in_use: HashSet<CacheKey, S>,
     atlas_side: usize,
     max_texture_side: usize,
     texture: TextureHandle,
     ctx: Context,
     default_color: Color32,
 }

 impl<S: BuildHasher + Default> TextureAtlas<S> {
     const ATLAS_TEXTURE_NAME: &'static str = "my_cosmic_text atlas";
     /// Create a new atlas with a reasonable initial size.  The atlas will
     /// automatically grow as more glyphs are rasterised.  All glyphs are
     /// tinted using `default_color` when their swash image is an 8‑bit
     /// mask.
     pub fn new(ctx: Context, default_color: Color32) -> Self {
         let atlas_side = 256_usize;
         let packer = BucketedAtlasAllocator::new(Size::splat(atlas_side as i32));
        // Create an initial transparent image for the atlas.  Use `filled` to
        // generate a `ColorImage` of the correct size instead of calling
        // `ColorImage::new` with a single color.  The `new` constructor
        // expects a vector of pixels, while `filled` expands the color into
        // a full vector and also populates the `source_size` field.
        let initial_image = ColorImage::filled(
            [atlas_side, atlas_side],
            Color32::TRANSPARENT,
        );
        let texture = ctx.load_texture(
            Self::ATLAS_TEXTURE_NAME,
            initial_image,
            TextureOptions::NEAREST,
        );
         let max_texture_side = ctx.input(|i| i.max_texture_side);
         Self {
             packer,
             cache: LruCache::unbounded_with_hasher(S::default()),
             in_use: HashSet::with_hasher(S::default()),
             atlas_side,
             max_texture_side,
             texture,
             ctx,
             default_color,
         }
     }

     /// Grow the atlas to accommodate more glyphs.  Existing glyphs are
     /// recopied into the new texture.  This is called automatically by
     /// [`alloc`] when the atlas is full.
     fn grow(&mut self, font_system: &mut FontSystem, swash_cache: &mut SwashCache) {
         assert!(self.atlas_side < self.max_texture_side);
         let new_side_size = (self.atlas_side * 2).at_most(self.max_texture_side);
         self.atlas_side = new_side_size;
         self.packer.grow(Size::splat(new_side_size as i32));
         // Create a new image filled with transparency.
         let mut new_image = Img::new(vec![Color32::TRANSPARENT; new_side_size * new_side_size], new_side_size, new_side_size);
         // Recopy all cached glyphs.
         for (&cache_key, state_opt) in self.cache.iter() {
             if let Some(state) = state_opt {
                 // Rasterize again to get the image; we intentionally avoid caching the actual image data to save memory.
                 if let Some(image) = swash_cache.get_image_uncached(font_system, cache_key) {
                     let rect = state.allocation.rectangle;
                     let region = new_image.sub_image_mut(rect.min.x as usize, rect.min.y as usize, image.placement.width as usize, image.placement.height as usize);
                     write_glyph_image(image, self.default_color, region);
                 }
             }
         }
         // Replace the atlas texture.
        // Construct a new `ColorImage` from the buffer.  Use the `ColorImage::new`
        // constructor to ensure `source_size` is set correctly.  Without
        // specifying `source_size` the struct literal would fail to compile.
        let buf = new_image.into_buf();
        let new_color_image = ColorImage::new([new_side_size, new_side_size], buf);
        self.texture = self.ctx.load_texture(
            Self::ATLAS_TEXTURE_NAME,
            new_color_image,
            TextureOptions::NEAREST,
        );
     }

     /// Try to allocate a rectangle of the given width and height.  If the
     /// atlas is full it will evict unused glyphs and possibly grow.
     fn alloc_packer(&mut self, width: u32, height: u32) -> Option<Allocation> {
         let size = size2(width as i32, height as i32);
         loop {
             if let Some(alloc) = self.packer.allocate(size) {
                 return Some(alloc);
             }
             // Evict the least recently used glyph not used this frame.
             let unused = loop {
                 let (key, _) = self.cache.peek_lru()?;
                 if self.in_use.contains(key) {
                     // This glyph is in use this frame; we cannot evict it, so the atlas must grow.
                     return None;
                 }
                 let (_, value) = self.cache.pop_lru()?;
                 match value {
                     Some(state) => break state,
                     None => continue,
                 }
             };
             self.packer.deallocate(unused.allocation.id);
         }
     }

     /// Promote a glyph to mark it as recently used.
     fn promote(&mut self, cache_key: CacheKey) {
         self.cache.promote(&cache_key);
         self.in_use.insert(cache_key);
     }

     /// Insert a glyph state into the LRU cache and mark it as in use.
     fn put(&mut self, cache_key: CacheKey, value: Option<GlyphState>) {
         self.cache.put(cache_key, value);
         self.in_use.insert(cache_key);
     }

     /// Allocate a glyph from the atlas.  If the glyph is already cached
     /// this will simply mark it as used and return the existing entry.  If
     /// it is not cached it will be rasterized, packed into the atlas and
     /// cached.  Returns `None` if the glyph has zero size (e.g. a space
     /// character).
     pub fn alloc(&mut self, cache_key: CacheKey, font_system: &mut FontSystem, swash_cache: &mut SwashCache) -> Option<GlyphImage> {
         // Check if glyph is in cache.  None means the glyph had zero size and should be skipped.
         let glyph_state = match self.cache.get(&cache_key) {
             None => {
                 // Not cached; rasterize using swash.
                 let image = swash_cache.get_image_uncached(font_system, cache_key)?;
                 if image.placement.width == 0 || image.placement.height == 0 {
                     self.put(cache_key, None);
                     return None;
                 }
                 // Try to allocate space in the atlas, evicting or growing if necessary.
                 loop {
                     match self.alloc_packer(image.placement.width, image.placement.height) {
                         Some(alloc) => {
                             let state = GlyphState { allocation: alloc, placement: image.placement, colorable: matches!(image.content, SwashContent::Mask) };
                             self.put(cache_key, Some(state.clone()));
                             // Write the glyph into a temporary buffer then upload via set_partial.
                             let width = image.placement.width as usize;
                             let height = image.placement.height as usize;
                             let mut pixels = vec![Color32::TRANSPARENT; width * height];
                             write_glyph_image(image, self.default_color, Img::new(&mut pixels, width, height));
                             // Upload the glyph into the atlas.  Build a `ColorImage`
                             // via `ColorImage::new` to set the `source_size` field.
                             let img = ColorImage::new([width, height], pixels);
                             self.texture.set_partial(
                                 alloc.rectangle.min.to_array().map(|c| c as usize),
                                 img,
                                 TextureOptions::NEAREST,
                             );
                             break Some(state);
                         }
                         None => {
                             // Could not allocate; need to grow the atlas.
                             self.grow(font_system, swash_cache);
                         }
                     }
                 }
             }
             Some(state_opt) => {
                 let state = state_opt.clone();
                 self.promote(cache_key);
                 state
             }
         }?;
         // Return a GlyphImage referencing the cached entry.
         Some(GlyphImage::new(
             &self.texture,
             glyph_state.allocation.rectangle,
             glyph_state.placement,
             self.default_color,
             glyph_state.colorable,
         ))
     }

     /// Get the texture ID for the atlas.  Can be used to draw the entire
     /// atlas for debugging.
     pub fn atlas_texture(&self) -> TextureId { self.texture.id() }

     /// Get the atlas size in logical points as a [`Vec2`].
     pub fn atlas_texture_size(&self) -> Vec2 { self.texture.size_vec2() }

     /// Update the maximum texture side length.  Should be called when the
     /// `egui` context updates its input (e.g. when the window is moved
     /// between monitors with different capabilities).
     pub fn update_max_texture_side(&mut self) { self.max_texture_side = self.ctx.input(|i| i.max_texture_side); }

     /// Clear the set of glyphs in use this frame.  Call this at the
     /// beginning of each frame before drawing any text.  Glyphs that are
     /// not re‑added via [`alloc`] will be considered for eviction when
     /// space is needed.
     pub fn trim(&mut self) { self.in_use.clear(); }
 }

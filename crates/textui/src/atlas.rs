use super::*;

#[path = "atlas/dirty_atlas_rect.rs"]
mod dirty_atlas_rect;
#[path = "atlas/field_line_segment.rs"]
mod field_line_segment;
#[path = "atlas/flattened_outline.rs"]
mod flattened_outline;
#[path = "atlas/glyph_atlas_texture.rs"]
mod glyph_atlas_texture;
#[path = "atlas/glyph_atlas_worker_message.rs"]
mod glyph_atlas_worker_message;
#[path = "atlas/glyph_error_score.rs"]
mod glyph_error_score;
#[path = "atlas/native_glyph_atlas_texture.rs"]
mod native_glyph_atlas_texture;

use self::dirty_atlas_rect::DirtyAtlasRect;
use self::field_line_segment::FieldLineSegment;
use self::flattened_outline::FlattenedOutline;
use self::glyph_atlas_texture::GlyphAtlasTexture;
use self::glyph_atlas_worker_message::GlyphAtlasWorkerMessage;
use self::glyph_error_score::GlyphErrorScore;
use self::native_glyph_atlas_texture::NativeGlyphAtlasTexture;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct GlyphRasterKey {
    cache_key: CacheKey,
    display_scale_bits: u32,
    raster_flags: u8,
    content_mode: GlyphContentMode,
    field_range_bits: u32,
    variation_settings: Arc<[TextVariationSetting]>,
}

impl GlyphRasterKey {
    const STEM_DARKENING: u8 = 1 << 0;

    #[inline]
    pub(super) fn new(
        cache_key: CacheKey,
        display_scale: f32,
        stem_darkening: bool,
        content_mode: GlyphContentMode,
        field_range_px: f32,
        variation_settings: Arc<[TextVariationSetting]>,
    ) -> Self {
        Self {
            cache_key,
            display_scale_bits: display_scale.to_bits(),
            raster_flags: if stem_darkening {
                Self::STEM_DARKENING
            } else {
                0
            },
            content_mode,
            field_range_bits: field_range_px.to_bits(),
            variation_settings,
        }
    }

    #[inline]
    pub(super) fn display_scale(&self) -> f32 {
        f32::from_bits(self.display_scale_bits)
    }

    #[inline]
    pub(super) fn stem_darkening(&self) -> bool {
        self.raster_flags & Self::STEM_DARKENING != 0
    }

    #[inline]
    pub(super) fn content_mode(&self) -> GlyphContentMode {
        self.content_mode
    }

    #[inline]
    pub(super) fn field_range_px(&self) -> f32 {
        f32::from_bits(self.field_range_bits)
    }

    pub(super) fn for_content_mode(
        &self,
        content_mode: GlyphContentMode,
        field_range_px: f32,
    ) -> Self {
        let mut key = self.clone();
        key.content_mode = content_mode;
        key.field_range_bits = field_range_px.to_bits();
        if content_mode != GlyphContentMode::AlphaMask {
            key.cache_key.x_bin = SubpixelBin::Zero;
            key.cache_key.y_bin = SubpixelBin::Zero;
        }
        key
    }
}

#[inline]
pub(super) fn glyph_logical_font_size_points(glyph: &GlyphRasterKey) -> f32 {
    let ppem = f32::from_bits(glyph.cache_key.font_size_bits);
    let display_scale = glyph.display_scale().max(1.0);
    (ppem / display_scale).max(1.0)
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum GlyphContentMode {
    AlphaMask,
    Sdf,
    Msdf,
}

pub(super) struct GlyphAtlas {
    entries: ThreadSafeLru<GlyphRasterKey, GlyphAtlasEntry>,
    pages: Vec<GlyphAtlasPage>,
    page_side_px: usize,
    padding_px: usize,
    sampling: TextAtlasSampling,
    rasterization: TextRasterizationConfig,
    /// Kept to coordinate atlas recreation when the broader text rendering
    /// pipeline switches modes. Atlas pages themselves stay FP16.
    linear_pipeline: bool,
    pub(super) wgpu_render_state: Option<EguiWgpuRenderState>,
    pub(super) pending: FxHashSet<GlyphRasterKey>,
    ready: VecDeque<GlyphAtlasWorkerResponse>,
    generation: u64,
    tx: Option<mpsc::Sender<GlyphAtlasWorkerMessage>>,
    rx: Option<mpsc::Receiver<GlyphAtlasWorkerResponse>>,
}

pub(super) struct GlyphAtlasPage {
    allocator: AtlasAllocator,
    content_mode: GlyphContentMode,
    texture: GlyphAtlasTexture,
    backing: ColorImage,
    cached_page_data: Mutex<Option<TextAtlasPageData>>,
    dirty_rect: Option<DirtyAtlasRect>,
    live_glyphs: usize,
}

#[derive(Clone, Debug)]
pub(super) struct GlyphAtlasEntry {
    page_index: usize,
    allocation_id: AllocId,
    atlas_min_px: [usize; 2],
    size_px: [usize; 2],
    placement_left_px: i32,
    placement_top_px: i32,
    is_color: bool,
    content_mode: GlyphContentMode,
    last_used_frame: u64,
    approx_bytes: usize,
}

#[derive(Clone)]
pub(super) struct ResolvedGlyphAtlasEntry {
    pub(super) page_index: usize,
    pub(super) uv: Rect,
    pub(super) size_px: [usize; 2],
    pub(super) placement_left_px: i32,
    pub(super) placement_top_px: i32,
    pub(super) is_color: bool,
    pub(super) content_mode: GlyphContentMode,
}

#[derive(Clone)]
pub(super) struct PreparedAtlasGlyph {
    pub(super) upload_image: ColorImage,
    pub(super) size_px: [usize; 2],
    pub(super) placement_left_px: i32,
    pub(super) placement_top_px: i32,
    pub(super) is_color: bool,
    pub(super) content_mode: GlyphContentMode,
    pub(super) approx_bytes: usize,
}

#[derive(Clone, Debug)]
pub(super) struct PaintTextQuad {
    pub(super) page_index: usize,
    pub(super) positions: [Pos2; 4],
    pub(super) uvs: [Pos2; 4],
    pub(super) tint: Color32,
    pub(super) content_mode: GlyphContentMode,
}

pub(super) fn hash_text_fundamentals<H: Hasher>(fundamentals: &TextFundamentals, state: &mut H) {
    fundamentals.kerning.hash(state);
    fundamentals.stem_darkening.hash(state);
    fundamentals.standard_ligatures.hash(state);
    fundamentals.contextual_alternates.hash(state);
    fundamentals.discretionary_ligatures.hash(state);
    fundamentals.historical_ligatures.hash(state);
    fundamentals.case_sensitive_forms.hash(state);
    fundamentals.slashed_zero.hash(state);
    fundamentals.tabular_numbers.hash(state);
    fundamentals.smart_quotes.hash(state);
    fundamentals.letter_spacing_points.to_bits().hash(state);
    fundamentals.word_spacing_points.to_bits().hash(state);
    fundamentals.letter_spacing_floor.to_bits().hash(state);
    fundamentals.feature_settings.len().hash(state);
    for feature in &fundamentals.feature_settings {
        feature.hash(state);
    }
    fundamentals.variation_settings.len().hash(state);
    for variation in &fundamentals.variation_settings {
        variation.hash(state);
    }
}

pub(super) fn shared_variation_settings(
    fundamentals: &TextFundamentals,
) -> Arc<[TextVariationSetting]> {
    Arc::from(fundamentals.variation_settings.clone().into_boxed_slice())
}

fn glyph_cluster_text<'a>(run_text: &'a str, glyph: &LayoutGlyph) -> &'a str {
    run_text.get(glyph.start..glyph.end).unwrap_or_default()
}

/// Replaces typographic characters with their plain-text equivalents so that
/// clipboard content is maximally compatible with other applications.
///
/// Currently converts:
/// - `…` (U+2026 HORIZONTAL ELLIPSIS) → `...`
fn spacing_after_glyph_points(
    run_text: &str,
    glyph: &LayoutGlyph,
    glyph_index: usize,
    glyph_count: usize,
    fundamentals: &TextFundamentals,
) -> f32 {
    if glyph_index + 1 >= glyph_count {
        return 0.0;
    }
    let mut spacing = fundamentals
        .letter_spacing_points
        .max(fundamentals.letter_spacing_floor);
    let cluster = glyph_cluster_text(run_text, glyph);
    if !cluster.is_empty() && cluster.chars().all(char::is_whitespace) {
        spacing += fundamentals.word_spacing_points.max(0.0);
    }
    spacing
}

fn spacing_after_glyph_pixels(
    run_text: &str,
    glyph: &LayoutGlyph,
    glyph_index: usize,
    glyph_count: usize,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> f32 {
    spacing_after_glyph_points(run_text, glyph, glyph_index, glyph_count, fundamentals) * scale
}

pub(super) fn collect_glyph_spacing_prefixes_px(
    run_text: &str,
    glyphs: &[LayoutGlyph],
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Vec<f32> {
    let mut prefixes = Vec::with_capacity(glyphs.len());
    let mut extra_px = 0.0;
    for (glyph_index, glyph) in glyphs.iter().enumerate() {
        prefixes.push(extra_px);
        extra_px += spacing_after_glyph_pixels(
            run_text,
            glyph,
            glyph_index,
            glyphs.len(),
            fundamentals,
            scale,
        );
    }
    prefixes
}

pub(super) fn adjusted_glyph_x_px(glyph: &LayoutGlyph, prefix_px: f32) -> f32 {
    glyph.x + prefix_px
}

pub(super) fn adjusted_glyph_right_px(glyph: &LayoutGlyph, prefix_px: f32) -> f32 {
    adjusted_glyph_x_px(glyph, prefix_px) + glyph.w
}

fn run_cursor_from_glyph_right(run: &LayoutRun<'_>, glyph: &LayoutGlyph) -> Cursor {
    if run.rtl {
        Cursor::new_with_affinity(run.line_i, glyph.start, Affinity::After)
    } else {
        Cursor::new_with_affinity(run.line_i, glyph.end, Affinity::Before)
    }
}

fn run_cursor_stops(
    run: &LayoutRun<'_>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Vec<(Cursor, f32)> {
    cursor_stops_for_glyphs(run.line_i, run.text, run.glyphs, fundamentals, scale)
}

pub(super) fn cursor_stops_for_glyphs(
    line_i: usize,
    text: &str,
    glyphs: &[LayoutGlyph],
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Vec<(Cursor, f32)> {
    let mut stops = Vec::new();
    if glyphs.is_empty() {
        stops.push((Cursor::new(line_i, 0), 0.0));
        return stops;
    }

    let prefixes = collect_glyph_spacing_prefixes_px(text, glyphs, fundamentals, scale);
    for (glyph_index, glyph) in glyphs.iter().enumerate() {
        let cluster = glyph_cluster_text(text, glyph);
        let graphemes = cluster.grapheme_indices(true).collect::<Vec<_>>();
        let total = graphemes.len().max(1);
        let glyph_x = adjusted_glyph_x_px(glyph, prefixes[glyph_index]);

        for step in 0..=total {
            let cursor_index = graphemes
                .get(step)
                .map_or(glyph.end, |(offset, _)| glyph.start + *offset);
            let offset = glyph.w * step as f32 / total as f32;
            let x = if glyph.level.is_rtl() {
                glyph_x + glyph.w - offset
            } else {
                glyph_x + offset
            };
            stops.push((Cursor::new(line_i, cursor_index), x));
        }
    }

    stops.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.index.cmp(&b.0.index))
    });
    stops.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() <= 0.25);
    stops
}

pub(super) fn hit_buffer_with_fundamentals(
    buffer: &Buffer,
    x: f32,
    y: f32,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<Cursor> {
    let mut new_cursor_opt = None;

    let mut runs = buffer.layout_runs().peekable();
    let mut first_run = true;
    while let Some(run) = runs.next() {
        let line_top = run.line_top;
        let line_height = run.line_height;

        if first_run && y < line_top {
            first_run = false;
            new_cursor_opt = Some(Cursor::new(run.line_i, 0));
        } else if y >= line_top && y < line_top + line_height {
            let stops = run_cursor_stops(&run, fundamentals, scale);
            if let Some((first_cursor, first_x)) = stops.first().copied() {
                if x <= first_x {
                    return Some(first_cursor);
                }
            }

            for window in stops.windows(2) {
                let (left_cursor, left_x) = window[0];
                let (right_cursor, right_x) = window[1];
                let mid_x = (left_x + right_x) * 0.5;
                if x <= mid_x {
                    return Some(left_cursor);
                }
                if x <= right_x {
                    return Some(right_cursor);
                }
            }

            if let Some((last_cursor, _)) = stops.last().copied() {
                return Some(last_cursor);
            }

            return Some(Cursor::new(run.line_i, 0));
        } else if runs.peek().is_none() && y > run.line_y {
            if let Some(glyph) = run.glyphs.last() {
                new_cursor_opt = Some(run_cursor_from_glyph_right(&run, glyph));
            } else {
                new_cursor_opt = Some(Cursor::new(run.line_i, 0));
            }
        }
    }

    new_cursor_opt
}

pub(super) fn collect_prepared_glyphs_from_buffer(
    buffer: &Buffer,
    scale: f32,
    default_color: Color32,
    fundamentals: &TextFundamentals,
) -> (Vec<PreparedGlyph>, f32) {
    let mut glyphs = Vec::new();
    let mut max_line_extra_points: f32 = 0.0;
    let variation_settings = shared_variation_settings(fundamentals);

    for run in buffer.layout_runs() {
        let baseline_y_px = run.line_y as i32;
        let mut line_extra_points = 0.0;

        for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            glyphs.push(PreparedGlyph {
                cache_key: GlyphRasterKey::new(
                    physical.cache_key,
                    scale,
                    fundamentals.stem_darkening,
                    GlyphContentMode::AlphaMask,
                    0.0,
                    Arc::clone(&variation_settings),
                ),
                offset_points: egui::vec2(
                    physical.x as f32 / scale + line_extra_points,
                    (baseline_y_px + physical.y) as f32 / scale,
                ),
                color: glyph.color_opt.map_or(default_color, cosmic_to_egui_color),
            });
            line_extra_points += spacing_after_glyph_points(
                run.text,
                glyph,
                glyph_index,
                run.glyphs.len(),
                fundamentals,
            );
        }

        max_line_extra_points = max_line_extra_points.max(line_extra_points);
    }

    (glyphs, max_line_extra_points)
}

pub(super) struct GlyphAtlasWorkerResponse {
    generation: u64,
    cache_key: GlyphRasterKey,
    glyph: Option<PreparedAtlasGlyph>,
}

/// High-level text rendering engine built on cosmic-text + Swash.

impl GlyphAtlas {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel::<GlyphAtlasWorkerMessage>();
        let (result_tx, result_rx) = mpsc::channel::<GlyphAtlasWorkerResponse>();
        let _ =
            tokio_runtime::spawn_blocking_detached(move || glyph_atlas_worker_loop(rx, result_tx));
        Self {
            entries: ThreadSafeLru::new(GLYPH_ATLAS_MAX_BYTES),
            pages: Vec::new(),
            page_side_px: GLYPH_ATLAS_PAGE_TARGET_PX,
            padding_px: GLYPH_ATLAS_PADDING_PX.max(0) as usize,
            sampling: TextAtlasSampling::Linear,
            rasterization: TextRasterizationConfig::default(),
            linear_pipeline: false,
            wgpu_render_state: None,
            pending: FxHashSet::default(),
            ready: VecDeque::new(),
            generation: 0,
            tx: Some(tx),
            rx: Some(result_rx),
        }
    }

    pub(super) fn set_render_state(&mut self, render_state: Option<&EguiWgpuRenderState>) {
        let render_state_changed = match (&self.wgpu_render_state, render_state) {
            (Some(current), Some(next)) => {
                !Arc::ptr_eq(&current.renderer, &next.renderer)
                    || current.target_format != next.target_format
            }
            (None, None) => false,
            _ => true,
        };
        if render_state_changed {
            self.generation = self.generation.saturating_add(1);
            self.pending.clear();
            self.ready.clear();
            let _ = self.entries.write(|state| state.clear());
            self.free_all_pages();
        }
        self.wgpu_render_state = render_state.cloned();
    }

    pub(super) fn set_page_side(&mut self, page_side_px: usize) {
        self.page_side_px = page_side_px.max(1);
    }

    pub(super) fn set_sampling(&mut self, sampling: TextAtlasSampling) {
        self.sampling = sampling;
    }

    pub(super) fn set_padding(&mut self, padding_px: usize) {
        self.padding_px = padding_px;
    }

    pub(super) fn set_rasterization(&mut self, rasterization: TextRasterizationConfig) {
        self.rasterization = rasterization;
    }

    /// Enables or disables the linear pipeline.
    ///
    /// When changed, all atlas pages are cleared so the atlas contents are
    /// rebuilt against the current text pipeline assumptions.
    pub(super) fn set_linear_pipeline(&mut self, linear_pipeline: bool) {
        if self.linear_pipeline == linear_pipeline {
            return;
        }
        self.linear_pipeline = linear_pipeline;
        self.generation = self.generation.saturating_add(1);
        self.pending.clear();
        self.ready.clear();
        let _ = self.entries.write(|state| state.clear());
        self.free_all_pages();
    }

    pub(super) fn register_font(&self, bytes: Vec<u8>) {
        if let Some(tx) = self.tx.as_ref() {
            let _ = tx.send(GlyphAtlasWorkerMessage::RegisterFont(bytes));
        }
    }

    pub(super) fn clear(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.pending.clear();
        self.ready.clear();
        let _ = self.entries.write(|state| state.clear());
        self.free_all_pages();
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn poll_ready(&mut self, ctx: &Context, current_frame: u64) {
        let Some(rx) = self.rx.as_ref() else {
            return;
        };
        let mut worker_disconnected = false;
        for _ in 0..GLYPH_ATLAS_FETCH_MAX_PER_FRAME {
            match rx.try_recv() {
                Ok(response) => self.ready.push_back(response),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    worker_disconnected = true;
                    break;
                }
            }
        }

        let mut uploaded_glyphs = 0usize;
        let mut uploaded_bytes = 0usize;
        while uploaded_glyphs < GLYPH_ATLAS_UPLOAD_MAX_GLYPHS_PER_FRAME
            && uploaded_bytes < GLYPH_ATLAS_UPLOAD_MAX_BYTES_PER_FRAME
        {
            let Some(response) = self.ready.pop_front() else {
                break;
            };
            if response.generation != self.generation {
                continue;
            }
            self.pending.remove(&response.cache_key);
            if self
                .entries
                .read(|state| state.contains_key(&response.cache_key))
            {
                continue;
            }
            if let Some(glyph) = response.glyph {
                uploaded_glyphs = uploaded_glyphs.saturating_add(1);
                uploaded_bytes = uploaded_bytes.saturating_add(glyph.approx_bytes);
                self.insert_prepared_glyph(ctx, response.cache_key, glyph, current_frame, false);
            }
        }

        self.flush_dirty_pages();

        if worker_disconnected {
            self.tx = None;
            self.rx = None;
            self.pending.clear();
            self.ready.clear();
        }
    }

    pub(super) fn trim_stale(&mut self, current_frame: u64) {
        let stale_before = current_frame.saturating_sub(GLYPH_ATLAS_STALE_FRAMES);
        let evicted = self
            .entries
            .write(|state| state.retain(|_, entry| entry.value.last_used_frame >= stale_before));
        for (_, entry) in evicted {
            self.deallocate_entry(entry);
        }
    }

    fn free_all_pages(&mut self) {
        if let Some(render_state) = self.wgpu_render_state.as_ref() {
            let mut renderer = render_state.renderer.write();
            for page in self.pages.drain(..) {
                if let GlyphAtlasTexture::Wgpu(texture) = page.texture {
                    renderer.free_texture(&texture.id);
                }
            }
        } else {
            self.pages.clear();
        }
    }

    pub(super) fn resolve_or_queue(
        &mut self,
        ctx: &Context,
        font_system: &mut FontSystem,
        scale_context: &mut ScaleContext,
        cache_key: GlyphRasterKey,
        current_frame: u64,
    ) -> Option<ResolvedGlyphAtlasEntry> {
        if let Some(entry) = self.entries.write(|state| {
            let entry = state.touch(&cache_key)?;
            entry.value.last_used_frame = current_frame;
            Some(entry.value.clone())
        }) {
            return Some(self.resolve_entry(&entry));
        }

        if !self.pending.contains(&cache_key) {
            let queued = self.tx.as_ref().is_some_and(|tx| {
                tx.send(GlyphAtlasWorkerMessage::Rasterize {
                    generation: self.generation,
                    cache_key: cache_key.clone(),
                    rasterization: self.rasterization,
                    padding_px: self.padding_px,
                })
                .is_ok()
            });
            if queued {
                self.pending.insert(cache_key);
                ctx.request_repaint();
                return None;
            }
        }

        let glyph = rasterize_atlas_glyph(
            font_system,
            scale_context,
            &cache_key,
            self.rasterization,
            self.padding_px,
        )?;
        self.insert_prepared_glyph(ctx, cache_key, glyph, current_frame, true)
    }

    pub(super) fn resolve_sync(
        &mut self,
        ctx: &Context,
        font_system: &mut FontSystem,
        scale_context: &mut ScaleContext,
        cache_key: GlyphRasterKey,
        current_frame: u64,
    ) -> Option<ResolvedGlyphAtlasEntry> {
        if let Some(entry) = self.entries.write(|state| {
            let entry = state.touch(&cache_key)?;
            entry.value.last_used_frame = current_frame;
            Some(entry.value.clone())
        }) {
            return Some(self.resolve_entry(&entry));
        }

        let glyph = rasterize_atlas_glyph(
            font_system,
            scale_context,
            &cache_key,
            self.rasterization,
            self.padding_px,
        )?;
        self.insert_prepared_glyph(ctx, cache_key, glyph, current_frame, true)
    }

    fn insert_prepared_glyph(
        &mut self,
        ctx: &Context,
        cache_key: GlyphRasterKey,
        glyph: PreparedAtlasGlyph,
        current_frame: u64,
        flush_immediately: bool,
    ) -> Option<ResolvedGlyphAtlasEntry> {
        let allocation_size = size2(
            glyph.upload_image.size[0] as i32,
            glyph.upload_image.size[1] as i32,
        );
        if allocation_size.width > self.page_side_px as i32
            || allocation_size.height > self.page_side_px as i32
        {
            return None;
        }

        let (page_index, allocation) = loop {
            if let Some(found) = self.try_allocate(allocation_size, glyph.content_mode) {
                break found;
            }
            if self.try_add_page(ctx, glyph.content_mode) {
                continue;
            }
            if !self.evict_one_lru() {
                return None;
            }
        };

        self.write_glyph(page_index, allocation, &glyph.upload_image);

        let entry = GlyphAtlasEntry {
            page_index,
            allocation_id: allocation.id,
            atlas_min_px: [
                (allocation.rectangle.min.x + self.padding_px as i32) as usize,
                (allocation.rectangle.min.y + self.padding_px as i32) as usize,
            ],
            size_px: glyph.size_px,
            placement_left_px: glyph.placement_left_px,
            placement_top_px: glyph.placement_top_px,
            is_color: glyph.is_color,
            content_mode: glyph.content_mode,
            last_used_frame: current_frame,
            approx_bytes: glyph.approx_bytes,
        };
        let resolved = self.resolve_entry(&entry);
        let approx_bytes = entry.approx_bytes;
        self.entries.write(|state| {
            state.insert_without_eviction(cache_key, entry, approx_bytes);
        });
        if flush_immediately {
            self.flush_page_upload(page_index);
        }
        Some(resolved)
    }

    fn try_allocate(
        &mut self,
        size: etagere::Size,
        content_mode: GlyphContentMode,
    ) -> Option<(usize, Allocation)> {
        for (page_index, page) in self.pages.iter_mut().enumerate() {
            if page.content_mode != content_mode {
                continue;
            }
            if let Some(allocation) = page.allocator.allocate(size) {
                return Some((page_index, allocation));
            }
        }
        None
    }

    fn try_add_page(&mut self, ctx: &Context, content_mode: GlyphContentMode) -> bool {
        let side = self.page_side_px;
        let side_i = side as i32;

        // Reuse any page that has been fully evicted — reset its allocator in place.
        // The GPU texture is kept as-is; stale pixels at unreachable UVs are harmless.
        for page in &mut self.pages {
            if page.live_glyphs == 0 && page.content_mode == content_mode {
                page.allocator = AtlasAllocator::new(size2(side_i, side_i));
                return true;
            }
        }

        // No reusable page; allocate a fresh GPU texture.
        let texture = self.allocate_page_texture(ctx, side);
        self.pages.push(GlyphAtlasPage {
            allocator: AtlasAllocator::new(size2(side_i, side_i)),
            content_mode,
            texture,
            backing: ColorImage::filled([side, side], Color32::TRANSPARENT),
            cached_page_data: Mutex::new(None),
            dirty_rect: None,
            live_glyphs: 0,
        });
        true
    }

    fn allocate_page_texture(&mut self, ctx: &Context, side: usize) -> GlyphAtlasTexture {
        if let Some(render_state) = self.wgpu_render_state.as_ref() {
            let atlas_format = wgpu::TextureFormat::Rgba16Float;
            let texture = render_state
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("textui_glyph_atlas"),
                    size: wgpu::Extent3d {
                        width: side as u32,
                        height: side as u32,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: atlas_format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[atlas_format],
                });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let id = render_state.renderer.write().register_native_texture(
                &render_state.device,
                &view,
                wgpu_filter_mode_for_sampling(self.sampling),
            );
            GlyphAtlasTexture::Wgpu(NativeGlyphAtlasTexture { id, texture })
        } else {
            GlyphAtlasTexture::Egui(ctx.load_texture(
                format!("textui_glyph_atlas_{}", self.pages.len()),
                ColorImage::filled([side, side], Color32::TRANSPARENT),
                texture_options_for_sampling(self.sampling),
            ))
        }
    }

    fn evict_one_lru(&mut self) -> bool {
        let removed = self.entries.write(|state| state.pop_lru());
        if let Some((_, entry)) = removed {
            self.deallocate_entry(entry);
            true
        } else {
            false
        }
    }

    fn deallocate_entry(&mut self, entry: GlyphAtlasEntry) {
        let Some(page) = self.pages.get_mut(entry.page_index) else {
            return;
        };
        page.allocator.deallocate(entry.allocation_id);
        page.live_glyphs = page.live_glyphs.saturating_sub(1);
        // Empty pages are reclaimed by try_add_page on the next allocation demand;
        // we do not remove them here so that existing page_index values stay valid.
    }

    fn resolve_entry(&self, entry: &GlyphAtlasEntry) -> ResolvedGlyphAtlasEntry {
        let side = self.page_side_px as f32;
        let uv = Rect::from_min_max(
            Pos2::new(
                entry.atlas_min_px[0] as f32 / side,
                entry.atlas_min_px[1] as f32 / side,
            ),
            Pos2::new(
                (entry.atlas_min_px[0] + entry.size_px[0]) as f32 / side,
                (entry.atlas_min_px[1] + entry.size_px[1]) as f32 / side,
            ),
        );

        ResolvedGlyphAtlasEntry {
            page_index: entry.page_index,
            uv,
            size_px: entry.size_px,
            placement_left_px: entry.placement_left_px,
            placement_top_px: entry.placement_top_px,
            is_color: entry.is_color,
            content_mode: entry.content_mode,
        }
    }

    pub(super) fn page_snapshot(&self, page_index: usize) -> Option<TextAtlasPageSnapshot> {
        let page = self.pages.get(page_index)?;
        // Safety: Color32 is repr(C) struct([u8; 4]) — identical layout to 4 contiguous u8 bytes.
        let rgba8 = unsafe {
            std::slice::from_raw_parts(
                page.backing.pixels.as_ptr() as *const u8,
                page.backing.pixels.len() * 4,
            )
        }
        .to_vec();
        Some(TextAtlasPageSnapshot {
            page_index,
            size_px: page.backing.size,
            rgba8,
        })
    }

    pub(super) fn page_data(&self, page_index: usize) -> Option<TextAtlasPageData> {
        let page = self.pages.get(page_index)?;
        if let Ok(cached) = page.cached_page_data.lock()
            && let Some(data) = cached.as_ref()
        {
            return Some(data.clone());
        }

        let data = color_image_to_page_data(page_index, &page.backing);
        if let Ok(mut cached) = page.cached_page_data.lock() {
            *cached = Some(data.clone());
        }
        Some(data)
    }

    pub(super) fn texture_id_for_page(&self, page_index: usize) -> Option<TextureId> {
        self.pages.get(page_index).map(|page| page.texture.id())
    }

    pub(super) fn native_texture_for_page(&self, page_index: usize) -> Option<wgpu::Texture> {
        let page = self.pages.get(page_index)?;
        match &page.texture {
            GlyphAtlasTexture::Wgpu(texture) => Some(texture.texture.clone()),
            GlyphAtlasTexture::Egui(_) => None,
        }
    }

    fn write_glyph(&mut self, page_index: usize, allocation: Allocation, glyph: &ColorImage) {
        if glyph.size[0] == 0 || glyph.size[1] == 0 {
            return;
        }
        let Some(page) = self.pages.get_mut(page_index) else {
            return;
        };

        let pos = [
            allocation.rectangle.min.x.max(0) as usize,
            allocation.rectangle.min.y.max(0) as usize,
        ];
        blit_color_image(&mut page.backing, glyph, pos[0], pos[1]);
        if let Ok(mut cached) = page.cached_page_data.lock() {
            *cached = None;
        }
        let dirty = DirtyAtlasRect::new(pos, glyph.size);
        page.dirty_rect = Some(
            page.dirty_rect
                .map_or(dirty, |existing| existing.union(dirty)),
        );
        page.live_glyphs = page.live_glyphs.saturating_add(1);
    }

    fn flush_dirty_pages(&mut self) {
        for page_index in 0..self.pages.len() {
            self.flush_page_upload(page_index);
        }
    }

    fn flush_page_upload(&mut self, page_index: usize) {
        let Some(page) = self.pages.get_mut(page_index) else {
            return;
        };
        let Some(dirty_rect) = page.dirty_rect.take() else {
            return;
        };
        let image = color_image_sub_image(&page.backing, dirty_rect);
        match &mut page.texture {
            GlyphAtlasTexture::Egui(texture) => {
                texture.set_partial(
                    dirty_rect.min,
                    egui::ImageData::Color(image.into()),
                    texture_options_for_sampling(self.sampling),
                );
            }
            GlyphAtlasTexture::Wgpu(texture) => {
                if let Some(render_state) = self.wgpu_render_state.as_ref() {
                    write_color_image_to_wgpu_texture(
                        &render_state.queue,
                        &texture.texture,
                        dirty_rect.min,
                        &image,
                    );
                }
            }
        }
    }
}

impl Drop for GlyphAtlas {
    fn drop(&mut self) {
        self.pending.clear();
        self.ready.clear();
        self.free_all_pages();
    }
}

fn glyph_atlas_worker_loop(
    rx: mpsc::Receiver<GlyphAtlasWorkerMessage>,
    tx: mpsc::Sender<GlyphAtlasWorkerResponse>,
) {
    let mut font_system = FontSystem::new();
    configure_text_font_defaults(&mut font_system);
    let mut scale_context = ScaleContext::new();

    while let Ok(message) = rx.recv() {
        match message {
            GlyphAtlasWorkerMessage::RegisterFont(bytes) => {
                font_system.db_mut().load_font_data(bytes);
            }
            GlyphAtlasWorkerMessage::Rasterize {
                generation,
                cache_key,
                rasterization,
                padding_px,
            } => {
                let glyph = rasterize_atlas_glyph(
                    &mut font_system,
                    &mut scale_context,
                    &cache_key,
                    rasterization,
                    padding_px,
                );
                let _ = tx.send(GlyphAtlasWorkerResponse {
                    generation,
                    cache_key,
                    glyph,
                });
            }
        }
    }
}

pub(super) fn rasterize_atlas_glyph(
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    cache_key: &GlyphRasterKey,
    rasterization: TextRasterizationConfig,
    padding_px: usize,
) -> Option<PreparedAtlasGlyph> {
    if cache_key.content_mode() != GlyphContentMode::AlphaMask
        && !cache_key
            .cache_key
            .flags
            .contains(cosmic_text::CacheKeyFlags::PIXEL_FONT)
    {
        if let Some(glyph) = rasterize_field_glyph(
            font_system,
            scale_context,
            cache_key,
            rasterization,
            padding_px,
        ) {
            return Some(glyph);
        }
    }

    let image = rasterize_best_alpha_glyph(font_system, scale_context, cache_key, rasterization)?;
    let glyph_width = image.placement.width as usize;
    let glyph_height = image.placement.height as usize;
    if glyph_width == 0 || glyph_height == 0 {
        return None;
    }

    let glyph_image = swash_image_to_color_image(&image)?;
    let upload_image = build_atlas_upload_image(&glyph_image, padding_px);
    Some(PreparedAtlasGlyph {
        approx_bytes: color_image_byte_size(&upload_image),
        upload_image,
        size_px: [glyph_width, glyph_height],
        placement_left_px: image.placement.left,
        placement_top_px: image.placement.top,
        is_color: matches!(image.content, SwashContent::Color),
        content_mode: GlyphContentMode::AlphaMask,
    })
}

fn rasterize_best_alpha_glyph(
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    raster_key: &GlyphRasterKey,
    rasterization: TextRasterizationConfig,
) -> Option<SwashImage> {
    let primary = render_swash_image(font_system, scale_context, raster_key, rasterization)?;
    if !matches!(primary.content, SwashContent::Mask) {
        return Some(primary);
    }

    let mut reference_rasterization = rasterization;
    reference_rasterization.hinting = TextHintingMode::Disabled;
    let Some(outline_commands) = render_swash_outline_commands(
        font_system,
        scale_context,
        raster_key,
        reference_rasterization,
    ) else {
        return Some(primary);
    };
    let outline = flatten_outline_commands_for_field(&outline_commands, GlyphContentMode::Sdf);
    if outline.segments.is_empty()
        || !outline.min[0].is_finite()
        || !outline.min[1].is_finite()
        || !outline.max[0].is_finite()
        || !outline.max[1].is_finite()
    {
        return Some(primary);
    }

    let mut best_image = primary.clone();
    let mut best_score = alpha_glyph_error_against_outline(&outline, &primary);

    let mut variants = [rasterization; 3];
    variants[0].stem_darkening = TextStemDarkeningMode::Disabled;
    variants[1].hinting = TextHintingMode::Disabled;
    variants[2].stem_darkening = TextStemDarkeningMode::Disabled;
    variants[2].hinting = TextHintingMode::Disabled;

    for variant in variants {
        if variant == rasterization {
            continue;
        }
        let Some(candidate) = render_swash_image(font_system, scale_context, raster_key, variant)
        else {
            continue;
        };
        if !matches!(candidate.content, SwashContent::Mask) {
            continue;
        }
        let score = alpha_glyph_error_against_outline(&outline, &candidate);
        if score.total_error < best_score.total_error {
            best_image = candidate;
            best_score = score;
        }
    }

    Some(best_image)
}

fn render_swash_image(
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    raster_key: &GlyphRasterKey,
    rasterization: TextRasterizationConfig,
) -> Option<SwashImage> {
    let cache_key = raster_key.cache_key;
    let display_scale = raster_key.display_scale().max(1.0);
    let ppem = f32::from_bits(cache_key.font_size_bits);
    let logical_font_size = (ppem / display_scale).max(1.0);

    let font = font_system.get_font(cache_key.font_id, cache_key.font_weight)?;
    let swash_font = font.as_swash();

    let mut settings_by_tag = std::collections::BTreeMap::<[u8; 4], f32>::new();
    if let Some(variation) = swash_font
        .variations()
        .find_by_tag(SwashTag::from_be_bytes(*b"wght"))
    {
        settings_by_tag.insert(
            *b"wght",
            f32::from(cache_key.font_weight.0).clamp(variation.min_value(), variation.max_value()),
        );
    }
    if let Some(variation) = swash_font
        .variations()
        .find_by_tag(SwashTag::from_be_bytes(*b"opsz"))
    {
        if rasterization.optical_sizing != TextOpticalSizingMode::Disabled {
            settings_by_tag.insert(
                *b"opsz",
                opsz_for_font_size(logical_font_size)
                    .clamp(variation.min_value(), variation.max_value()),
            );
        }
    }
    for variation in raster_key.variation_settings.iter().copied() {
        let tag = SwashTag::from_be_bytes(variation.tag);
        if let Some(axis) = swash_font.variations().find_by_tag(tag) {
            settings_by_tag.insert(
                variation.tag,
                variation.value().clamp(axis.min_value(), axis.max_value()),
            );
        }
    }
    let settings = settings_by_tag
        .into_iter()
        .map(|(tag, value)| SwashSetting {
            tag: SwashTag::from_be_bytes(tag),
            value,
        })
        .collect::<Vec<_>>();

    let mut scaler = scale_context
        .builder(swash_font)
        .size(ppem)
        .hint(resolved_hinting_enabled(display_scale, rasterization));
    if !settings.is_empty() {
        scaler = scaler.variations(settings.into_iter());
    }
    let mut scaler = scaler.build();

    let offset = if cache_key
        .flags
        .contains(cosmic_text::CacheKeyFlags::PIXEL_FONT)
    {
        SwashVector::new(
            cache_key.x_bin.as_float().round() + 1.0,
            cache_key.y_bin.as_float().round(),
        )
    } else {
        SwashVector::new(cache_key.x_bin.as_float(), cache_key.y_bin.as_float())
    };

    let mut render = Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ]);
    render
        .format(SwashFormat::Alpha)
        .offset(offset)
        .embolden(resolved_stem_darkening_strength(
            ppem,
            raster_key.stem_darkening(),
            rasterization,
        ))
        .transform(
            if cache_key
                .flags
                .contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC)
            {
                Some(SwashTransform::skew(
                    SwashAngle::from_degrees(14.0),
                    SwashAngle::from_degrees(0.0),
                ))
            } else {
                None
            },
        );

    render.render(&mut scaler, cache_key.glyph_id)
}

pub(super) fn render_swash_outline_commands(
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    raster_key: &GlyphRasterKey,
    rasterization: TextRasterizationConfig,
) -> Option<Box<[swash::zeno::Command]>> {
    use swash::zeno::PathData as _;

    let cache_key = raster_key.cache_key;
    let display_scale = raster_key.display_scale().max(1.0);
    let ppem = f32::from_bits(cache_key.font_size_bits);
    let logical_font_size = (ppem / display_scale).max(1.0);

    let font = font_system.get_font(cache_key.font_id, cache_key.font_weight)?;
    let swash_font = font.as_swash();

    let mut settings_by_tag = std::collections::BTreeMap::<[u8; 4], f32>::new();
    if let Some(variation) = swash_font
        .variations()
        .find_by_tag(SwashTag::from_be_bytes(*b"wght"))
    {
        settings_by_tag.insert(
            *b"wght",
            f32::from(cache_key.font_weight.0).clamp(variation.min_value(), variation.max_value()),
        );
    }
    if let Some(variation) = swash_font
        .variations()
        .find_by_tag(SwashTag::from_be_bytes(*b"opsz"))
    {
        if rasterization.optical_sizing != TextOpticalSizingMode::Disabled {
            settings_by_tag.insert(
                *b"opsz",
                opsz_for_font_size(logical_font_size)
                    .clamp(variation.min_value(), variation.max_value()),
            );
        }
    }
    for variation in raster_key.variation_settings.iter().copied() {
        let tag = SwashTag::from_be_bytes(variation.tag);
        if let Some(axis) = swash_font.variations().find_by_tag(tag) {
            settings_by_tag.insert(
                variation.tag,
                variation.value().clamp(axis.min_value(), axis.max_value()),
            );
        }
    }
    let settings = settings_by_tag
        .into_iter()
        .map(|(tag, value)| SwashSetting {
            tag: SwashTag::from_be_bytes(tag),
            value,
        })
        .collect::<Vec<_>>();

    let mut scaler = scale_context
        .builder(swash_font)
        .size(ppem)
        .hint(resolved_hinting_enabled(display_scale, rasterization));
    if !settings.is_empty() {
        scaler = scaler.variations(settings.into_iter());
    }
    let mut scaler = scaler.build();
    let mut outline = scaler
        .scale_outline(cache_key.glyph_id)
        .or_else(|| scaler.scale_color_outline(cache_key.glyph_id))?;
    if cache_key
        .flags
        .contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC)
    {
        outline.transform(&SwashTransform::skew(
            SwashAngle::from_degrees(14.0),
            SwashAngle::from_degrees(0.0),
        ));
    }
    Some(outline.path().commands().collect())
}

/// Returns true when two or more contours wind in the same direction and their bounding boxes
/// overlap. This indicates overlapping filled regions that our even-odd `point_inside_outline`
/// will mis-classify as "outside", making SDF/MSDF distances incorrect.
///
/// Counter-wound nested contours (holes, like the inside of "O") wind opposite to their parent,
/// so they are correctly excluded by the same-winding filter.
fn outline_has_same_winding_overlap(contours: &[Vec<[f32; 2]>]) -> bool {
    if contours.len() < 2 {
        return false;
    }

    // Compute signed area (shoelace) and bounding box per contour.
    // Positive area → CCW in the swash coordinate space; negative → CW.
    let mut winding = Vec::with_capacity(contours.len());
    let mut bboxes: Vec<([f32; 2], [f32; 2])> = Vec::with_capacity(contours.len());
    for contour in contours {
        let mut min = [f32::INFINITY, f32::INFINITY];
        let mut max = [f32::NEG_INFINITY, f32::NEG_INFINITY];
        let mut area = 0.0f32;
        let n = contour.len();
        for i in 0..n {
            let a = contour[i];
            let b = contour[(i + 1) % n];
            area += (b[0] - a[0]) * (b[1] + a[1]);
            min[0] = min[0].min(a[0]);
            min[1] = min[1].min(a[1]);
            max[0] = max[0].max(a[0]);
            max[1] = max[1].max(a[1]);
        }
        winding.push(area.signum());
        bboxes.push((min, max));
    }

    for i in 0..contours.len() {
        for j in (i + 1)..contours.len() {
            if winding[i] != winding[j] {
                continue;
            }
            let (a_min, a_max) = bboxes[i];
            let (b_min, b_max) = bboxes[j];
            if a_min[0] < b_max[0]
                && a_max[0] > b_min[0]
                && a_min[1] < b_max[1]
                && a_max[1] > b_min[1]
            {
                return true;
            }
        }
    }
    false
}

fn rasterize_field_glyph(
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    cache_key: &GlyphRasterKey,
    rasterization: TextRasterizationConfig,
    padding_px: usize,
) -> Option<PreparedAtlasGlyph> {
    let commands =
        render_swash_outline_commands(font_system, scale_context, cache_key, rasterization)?;
    let outline = flatten_outline_commands_for_field(&commands, cache_key.content_mode());
    if outline.segments.is_empty()
        || !outline.min[0].is_finite()
        || !outline.min[1].is_finite()
        || !outline.max[0].is_finite()
        || !outline.max[1].is_finite()
    {
        return None;
    }

    // Glyphs with overlapping same-winding contours (common in CJK, e.g. "经") fail SDF/MSDF
    // rendering because point_inside_outline uses the even-odd rule. Under even-odd, regions
    // covered by two same-winding contours flip back to "outside", producing wrong signed
    // distances and a glyph that renders too dark. field_glyph_matches_reference would catch
    // this, but only after the expensive O(width * height * segments) pixel loop and a full
    // reference rasterization. Detect the failure condition up-front from the contour geometry.
    if outline_has_same_winding_overlap(&outline.contours) {
        return None;
    }

    let field_range_px = cache_key.field_range_px().max(1.0);
    let left = (outline.min[0] - field_range_px).floor() as i32;
    let bottom = (outline.min[1] - field_range_px).floor() as i32;
    let right = (outline.max[0] + field_range_px).ceil() as i32;
    let top = (outline.max[1] + field_range_px).ceil() as i32;
    let glyph_width = (right - left).max(1) as usize;
    let glyph_height = (top - bottom).max(1) as usize;

    let mut glyph_image = ColorImage::filled([glyph_width, glyph_height], Color32::TRANSPARENT);
    for y in 0..glyph_height {
        for x in 0..glyph_width {
            let sample = [left as f32 + x as f32 + 0.5, top as f32 - y as f32 - 0.5];
            let inside = point_inside_outline(sample, &outline.contours);
            let rgba = match cache_key.content_mode() {
                GlyphContentMode::Sdf => {
                    encode_sdf_sample(sample, inside, &outline.segments, field_range_px)
                }
                GlyphContentMode::Msdf => {
                    encode_msdf_sample(sample, inside, &outline.segments, field_range_px)
                }
                GlyphContentMode::AlphaMask => unreachable!(),
            };
            glyph_image.pixels[y * glyph_width + x] =
                Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
        }
    }

    if !field_glyph_matches_reference(
        font_system,
        scale_context,
        cache_key,
        rasterization,
        &glyph_image,
        left,
        top,
        field_range_px,
    ) {
        return None;
    }

    let upload_image = build_atlas_upload_image(&glyph_image, padding_px);
    Some(PreparedAtlasGlyph {
        approx_bytes: color_image_byte_size(&upload_image),
        upload_image,
        size_px: [glyph_width, glyph_height],
        placement_left_px: left,
        placement_top_px: top,
        is_color: false,
        content_mode: cache_key.content_mode(),
    })
}

fn field_glyph_matches_reference(
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    cache_key: &GlyphRasterKey,
    rasterization: TextRasterizationConfig,
    field_image: &ColorImage,
    field_left: i32,
    field_top: i32,
    field_range_px: f32,
) -> bool {
    let Some(reference) = render_swash_image(font_system, scale_context, cache_key, rasterization)
    else {
        return false;
    };
    let Some(reference_image) = swash_image_to_color_image(&reference) else {
        return false;
    };

    let reference_left = reference.placement.left;
    let reference_top = reference.placement.top;
    let field_bottom = field_top - field_image.size[1] as i32;
    let reference_bottom = reference_top - reference_image.size[1] as i32;
    let compare_left = field_left.min(reference_left);
    let compare_right = (field_left + field_image.size[0] as i32)
        .max(reference_left + reference_image.size[0] as i32);
    let compare_bottom = field_bottom.min(reference_bottom);
    let compare_top = field_top.max(reference_top);
    let total_pixels =
        ((compare_right - compare_left).max(0) * (compare_top - compare_bottom).max(0)) as usize;
    if total_pixels == 0 {
        return false;
    }

    let mut total_error = 0.0;
    let mut max_error = 0.0_f32;
    let mut large_error_pixels = 0usize;
    for y in compare_bottom..compare_top {
        for x in compare_left..compare_right {
            let reference_alpha =
                sample_color_image_alpha(reference_left, reference_top, &reference_image, x, y);
            let field_alpha = sample_field_image_alpha(
                cache_key.content_mode(),
                field_left,
                field_top,
                field_image,
                x,
                y,
                field_range_px,
            );
            let error = (field_alpha - reference_alpha).abs();
            total_error += error;
            max_error = max_error.max(error);
            if error > FIELD_GLYPH_MAX_ALPHA_ERROR_LIMIT {
                large_error_pixels += 1;
            }
        }
    }

    let mean_error = total_error / total_pixels as f32;
    let large_error_ratio = large_error_pixels as f32 / total_pixels as f32;
    mean_error <= FIELD_GLYPH_MEAN_ALPHA_ERROR_LIMIT
        && max_error <= FIELD_GLYPH_MAX_ALPHA_ERROR_LIMIT
        && large_error_ratio <= FIELD_GLYPH_LARGE_ERROR_PIXEL_RATIO_LIMIT
}

fn alpha_glyph_error_against_outline(
    outline: &FlattenedOutline,
    image: &SwashImage,
) -> GlyphErrorScore {
    let Some(color_image) = swash_image_to_color_image(image) else {
        return GlyphErrorScore {
            total_error: f32::INFINITY,
        };
    };

    let image_left = image.placement.left;
    let image_top = image.placement.top;
    let outline_left = outline.min[0].floor() as i32;
    let outline_right = outline.max[0].ceil() as i32;
    let outline_bottom = outline.min[1].floor() as i32;
    let outline_top = outline.max[1].ceil() as i32;
    let compare_left = image_left.min(outline_left);
    let compare_right = (image_left + color_image.size[0] as i32).max(outline_right);
    let image_bottom = image_top - color_image.size[1] as i32;
    let compare_bottom = image_bottom.min(outline_bottom);
    let compare_top = image_top.max(outline_top);

    if compare_left >= compare_right || compare_bottom >= compare_top {
        return GlyphErrorScore {
            total_error: f32::INFINITY,
        };
    }

    let mut total_error = 0.0_f32;
    for y in compare_bottom..compare_top {
        for x in compare_left..compare_right {
            let actual_alpha = sample_color_image_alpha(image_left, image_top, &color_image, x, y);
            let reference_alpha = outline_pixel_coverage(outline, x, y);
            total_error += (actual_alpha - reference_alpha).abs();
        }
    }

    GlyphErrorScore { total_error }
}

fn flatten_outline_commands_for_field(
    commands: &[swash::zeno::Command],
    content_mode: GlyphContentMode,
) -> FlattenedOutline {
    let mut outline = FlattenedOutline::new();
    let mut current = [0.0, 0.0];
    let mut contour_start = [0.0, 0.0];
    let mut contour_points = Vec::<[f32; 2]>::new();
    let mut contour_segments = Vec::<([f32; 2], [f32; 2])>::new();

    let flush_contour = |outline: &mut FlattenedOutline,
                         contour_points: &mut Vec<[f32; 2]>,
                         contour_segments: &mut Vec<([f32; 2], [f32; 2])>| {
        if contour_segments.is_empty() {
            contour_points.clear();
            return;
        }
        if contour_points.len() >= 3 {
            outline.contours.push(contour_points.clone());
        }
        let colors = [1_u8, 2_u8, 4_u8];
        for (index, (a, b)) in contour_segments.iter().copied().enumerate() {
            outline.include_point(a);
            outline.include_point(b);
            let color_mask = match content_mode {
                GlyphContentMode::AlphaMask | GlyphContentMode::Sdf => 0b111,
                GlyphContentMode::Msdf => colors[index % colors.len()],
            };
            outline.segments.push(FieldLineSegment { a, b, color_mask });
        }
        contour_points.clear();
        contour_segments.clear();
    };

    for command in commands {
        match *command {
            swash::zeno::Command::MoveTo(point) => {
                flush_contour(&mut outline, &mut contour_points, &mut contour_segments);
                current = [point.x, point.y];
                contour_start = current;
                contour_points.push(current);
            }
            swash::zeno::Command::LineTo(point) => {
                let next = [point.x, point.y];
                contour_segments.push((current, next));
                contour_points.push(next);
                current = next;
            }
            swash::zeno::Command::QuadTo(control, point) => {
                let next = [point.x, point.y];
                let control = [control.x, control.y];
                let steps = curve_steps(current, control, control, next);
                let mut prev = current;
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let p = eval_quad(current, control, next, t);
                    contour_segments.push((prev, p));
                    contour_points.push(p);
                    prev = p;
                }
                current = next;
            }
            swash::zeno::Command::CurveTo(control_a, control_b, point) => {
                let next = [point.x, point.y];
                let control_a = [control_a.x, control_a.y];
                let control_b = [control_b.x, control_b.y];
                let steps = curve_steps(current, control_a, control_b, next);
                let mut prev = current;
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let p = eval_cubic(current, control_a, control_b, next, t);
                    contour_segments.push((prev, p));
                    contour_points.push(p);
                    prev = p;
                }
                current = next;
            }
            swash::zeno::Command::Close => {
                if current != contour_start {
                    contour_segments.push((current, contour_start));
                    contour_points.push(contour_start);
                    current = contour_start;
                }
                flush_contour(&mut outline, &mut contour_points, &mut contour_segments);
            }
        }
    }
    flush_contour(&mut outline, &mut contour_points, &mut contour_segments);
    outline
}

fn curve_steps(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> usize {
    let len = point_distance(p0, p1) + point_distance(p1, p2) + point_distance(p2, p3);
    ((len / 6.0).ceil() as usize).clamp(4, 24)
}

fn eval_quad(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], t: f32) -> [f32; 2] {
    let mt = 1.0 - t;
    [
        mt * mt * p0[0] + 2.0 * mt * t * p1[0] + t * t * p2[0],
        mt * mt * p0[1] + 2.0 * mt * t * p1[1] + t * t * p2[1],
    ]
}

fn eval_cubic(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], t: f32) -> [f32; 2] {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let t2 = t * t;
    [
        mt2 * mt * p0[0] + 3.0 * mt2 * t * p1[0] + 3.0 * mt * t2 * p2[0] + t2 * t * p3[0],
        mt2 * mt * p0[1] + 3.0 * mt2 * t * p1[1] + 3.0 * mt * t2 * p2[1] + t2 * t * p3[1],
    ]
}

fn point_distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn point_inside_outline(point: [f32; 2], contours: &[Vec<[f32; 2]>]) -> bool {
    let mut inside = false;
    for contour in contours {
        if contour.len() < 3 {
            continue;
        }
        let mut j = contour.len() - 1;
        for i in 0..contour.len() {
            let a = contour[i];
            let b = contour[j];
            let intersects = ((a[1] > point[1]) != (b[1] > point[1]))
                && (point[0] < (b[0] - a[0]) * (point[1] - a[1]) / ((b[1] - a[1]) + 1e-6) + a[0]);
            if intersects {
                inside = !inside;
            }
            j = i;
        }
    }
    inside
}

fn encode_sdf_sample(
    point: [f32; 2],
    inside: bool,
    segments: &[FieldLineSegment],
    field_range_px: f32,
) -> [u8; 4] {
    let signed = signed_distance_to_segments(point, inside, segments.iter().copied());
    let encoded = encode_signed_distance(signed, field_range_px);
    [encoded, encoded, encoded, 255]
}

fn encode_msdf_sample(
    point: [f32; 2],
    inside: bool,
    segments: &[FieldLineSegment],
    field_range_px: f32,
) -> [u8; 4] {
    let sign = if inside { 1.0 } else { -1.0 };
    let mut channel_distances = [field_range_px; 3];
    for segment in segments {
        let distance = distance_to_segment(point, segment.a, segment.b);
        if segment.color_mask & 1 != 0 {
            channel_distances[0] = channel_distances[0].min(distance);
        }
        if segment.color_mask & 2 != 0 {
            channel_distances[1] = channel_distances[1].min(distance);
        }
        if segment.color_mask & 4 != 0 {
            channel_distances[2] = channel_distances[2].min(distance);
        }
    }
    [
        encode_signed_distance(sign * channel_distances[0], field_range_px),
        encode_signed_distance(sign * channel_distances[1], field_range_px),
        encode_signed_distance(sign * channel_distances[2], field_range_px),
        255,
    ]
}

fn signed_distance_to_segments(
    point: [f32; 2],
    inside: bool,
    segments: impl Iterator<Item = FieldLineSegment>,
) -> f32 {
    let mut min_distance = f32::INFINITY;
    for segment in segments {
        min_distance = min_distance.min(distance_to_segment(point, segment.a, segment.b));
    }
    if inside { min_distance } else { -min_distance }
}

fn encode_signed_distance(distance: f32, field_range_px: f32) -> u8 {
    let normalized = (0.5 + 0.5 * (distance / field_range_px).clamp(-1.0, 1.0)).clamp(0.0, 1.0);
    (normalized * 255.0).round() as u8
}

fn decode_signed_distance(encoded: u8, field_range_px: f32) -> f32 {
    (((encoded as f32) / 255.0) - 0.5) * 2.0 * field_range_px
}

fn decode_field_alpha_from_pixel(
    content_mode: GlyphContentMode,
    pixel: Color32,
    field_range_px: f32,
) -> f32 {
    let signed_distance = match content_mode {
        GlyphContentMode::AlphaMask => pixel.a() as f32 / 255.0,
        GlyphContentMode::Sdf => decode_signed_distance(pixel.r(), field_range_px),
        GlyphContentMode::Msdf => {
            let red = decode_signed_distance(pixel.r(), field_range_px);
            let green = decode_signed_distance(pixel.g(), field_range_px);
            let blue = decode_signed_distance(pixel.b(), field_range_px);
            median3(red, green, blue)
        }
    };
    smoothstep(-0.5, 0.5, signed_distance)
}

fn sample_color_image_alpha(left: i32, top: i32, image: &ColorImage, x: i32, y: i32) -> f32 {
    let Some(pixel) = color_image_pixel_at_world_position(left, top, image, x, y) else {
        return 0.0;
    };
    pixel.a() as f32 / 255.0
}

fn sample_field_image_alpha(
    content_mode: GlyphContentMode,
    left: i32,
    top: i32,
    image: &ColorImage,
    x: i32,
    y: i32,
    field_range_px: f32,
) -> f32 {
    let Some(pixel) = color_image_pixel_at_world_position(left, top, image, x, y) else {
        return 0.0;
    };
    decode_field_alpha_from_pixel(content_mode, pixel, field_range_px)
}

fn color_image_pixel_at_world_position(
    left: i32,
    top: i32,
    image: &ColorImage,
    x: i32,
    y: i32,
) -> Option<Color32> {
    let width = image.size[0] as i32;
    let height = image.size[1] as i32;
    let ix = x - left;
    let iy = top - 1 - y;
    if ix < 0 || iy < 0 || ix >= width || iy >= height {
        return None;
    }
    image
        .pixels
        .get(iy as usize * image.size[0] + ix as usize)
        .copied()
}

fn median3(a: f32, b: f32, c: f32) -> f32 {
    a.max(b).min(a.min(b).max(c))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn outline_pixel_coverage(outline: &FlattenedOutline, x: i32, y: i32) -> f32 {
    let samples = OUTLINE_REFERENCE_SUPERSAMPLES_PER_AXIS as f32;
    let mut covered = 0usize;
    let total = OUTLINE_REFERENCE_SUPERSAMPLES_PER_AXIS * OUTLINE_REFERENCE_SUPERSAMPLES_PER_AXIS;
    for sy in 0..OUTLINE_REFERENCE_SUPERSAMPLES_PER_AXIS {
        for sx in 0..OUTLINE_REFERENCE_SUPERSAMPLES_PER_AXIS {
            let sample = [
                x as f32 + (sx as f32 + 0.5) / samples,
                y as f32 + (sy as f32 + 0.5) / samples,
            ];
            if point_inside_outline(sample, &outline.contours) {
                covered += 1;
            }
        }
    }
    covered as f32 / total as f32
}

fn distance_to_segment(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [point[0] - a[0], point[1] - a[1]];
    let denom = ab[0] * ab[0] + ab[1] * ab[1];
    if denom <= 1e-6 {
        return point_distance(point, a);
    }
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / denom).clamp(0.0, 1.0);
    let closest = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    point_distance(point, closest)
}

fn swash_image_to_color_image(image: &cosmic_text::SwashImage) -> Option<ColorImage> {
    let width = image.placement.width as usize;
    let height = image.placement.height as usize;
    if width == 0 || height == 0 {
        return None;
    }

    let pixels = match image.content {
        SwashContent::Mask => image
            .data
            .iter()
            .map(|alpha| Color32::from_white_alpha(*alpha))
            .collect::<Vec<_>>(),
        SwashContent::Color | SwashContent::SubpixelMask => image
            .data
            .chunks_exact(4)
            .map(|rgba| Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]))
            .collect::<Vec<_>>(),
    };

    Some(ColorImage::new([width, height], pixels))
}

fn build_atlas_upload_image(glyph: &ColorImage, padding: usize) -> ColorImage {
    let mut upload = ColorImage::filled(
        [
            glyph.size[0].saturating_add(padding * 2),
            glyph.size[1].saturating_add(padding * 2),
        ],
        Color32::TRANSPARENT,
    );
    blit_color_image(&mut upload, glyph, padding, padding);
    upload
}

pub(super) fn blit_color_image(
    dest: &mut ColorImage,
    src: &ColorImage,
    dest_x: usize,
    dest_y: usize,
) {
    let dest_width = dest.size[0];
    let copy_width = src.size[0].min(dest_width.saturating_sub(dest_x));
    if copy_width == 0 {
        return;
    }
    for y in 0..src.size[1] {
        let target_y = dest_y + y;
        if target_y >= dest.size[1] {
            break;
        }
        let src_start = y * src.size[0];
        let dest_start = target_y * dest_width + dest_x;
        dest.pixels[dest_start..dest_start + copy_width]
            .copy_from_slice(&src.pixels[src_start..src_start + copy_width]);
    }
}

fn color_image_sub_image(src: &ColorImage, rect: DirtyAtlasRect) -> ColorImage {
    let size = rect.size();
    let mut image = ColorImage::filled(size, Color32::TRANSPARENT);
    let src_width = src.size[0];
    let dst_width = size[0];
    for y in 0..size[1] {
        let src_y = rect.min[1] + y;
        let src_start = src_y * src_width + rect.min[0];
        let dst_start = y * dst_width;
        image.pixels[dst_start..dst_start + dst_width]
            .copy_from_slice(&src.pixels[src_start..src_start + dst_width]);
    }
    image
}

fn write_color_image_to_wgpu_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pos: [usize; 2],
    image: &ColorImage,
) {
    if image.size[0] == 0 || image.size[1] == 0 {
        return;
    }
    let bytes = color_image_to_rgba16f_bytes(image);
    let size = wgpu::Extent3d {
        width: image.size[0] as u32,
        height: image.size[1] as u32,
        depth_or_array_layers: 1,
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: pos[0] as u32,
                y: pos[1] as u32,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8 * image.size[0] as u32),
            rows_per_image: Some(image.size[1] as u32),
        },
        size,
    );
}

fn color_image_to_rgba16f_bytes(image: &ColorImage) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(image.pixels.len().saturating_mul(8));
    for pixel in &image.pixels {
        for channel in pixel.to_array() {
            let half = half::f16::from_f32(f32::from(channel) / 255.0);
            bytes.extend_from_slice(&half.to_bits().to_le_bytes());
        }
    }
    bytes
}

use std::{
    cmp::Ordering,
    collections::{BTreeSet, VecDeque},
    hash::{Hash, Hasher},
    mem,
    sync::{Arc, Mutex, mpsc},
};

const TEXT_WGPU_INSTANCED_SHADER: &str = include_str!("shaders/text_instanced.wgsl");

use bytemuck::{Pod, Zeroable};
use cosmic_text::{
    Action, Affinity, Attrs, AttrsOwned, BorrowedWithFontSystem, Buffer, CacheKey, Color, Cursor,
    Edit, Editor, Family, FontFeatures, FontSystem, LayoutGlyph, LayoutRun, Metrics, Motion,
    Selection, Shaping, Style as FontStyle, SubpixelBin, SwashContent, SwashImage, Weight, Wrap,
    fontdb,
};
use egui::{
    self, Color32, ColorImage, Context, CornerRadius, Id, Key, Pos2, Rect, Response, Sense,
    TextureHandle, TextureId, TextureOptions, Ui, Vec2,
};
use egui_wgpu::RenderState as EguiWgpuRenderState;
use etagere::{AllocId, Allocation, AtlasAllocator, size2};
use launcher_runtime as tokio_runtime;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use shared_lru::ThreadSafeLru;
use skrifa::raw::{FontRef as SkrifaFontRef, TableProvider as _};
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{
    Angle as SwashAngle, Format as SwashFormat, Transform as SwashTransform, Vector as SwashVector,
};
use swash::{Setting as SwashSetting, Tag as SwashTag};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle as SyntectFontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use tracing::warn;
use unicode_segmentation::UnicodeSegmentation;
use wgpu::util::DeviceExt as _;

mod advanced_text;
#[path = "text_ui_async_raster.rs"]
mod async_raster;
mod atlas;
mod button_options;
mod clipboard;
mod code_block_options;
mod conversions;
mod cursor_layout;
mod editor;
mod font_features;
mod geometry;
mod gpu;
mod input_options;
#[path = "text_ui_input_runtime.rs"]
mod input_runtime;
#[path = "text_ui_input_widget.rs"]
mod input_widget;
mod label_options;
#[path = "text_ui_label_scene.rs"]
mod label_scene;
mod markdown_options;
mod markdown_parser;
mod path_layout;
#[path = "text_ui_path_text.rs"]
mod path_text;
mod prepared_layout;
#[path = "text_ui_scene_builder.rs"]
mod scene_builder;
mod text_helpers;
#[path = "text_ui.rs"]
mod text_ui;
mod tooltip_options;

pub use self::text_ui::TextUi;
use crate::async_raster::{AsyncRasterState, AsyncRasterWorkerMessage, new_async_raster_state};
pub(crate) use crate::atlas::{
    GlyphAtlas, GlyphContentMode, GlyphRasterKey, PaintTextQuad, PreparedAtlasGlyph,
    adjusted_glyph_right_px, adjusted_glyph_x_px, collect_glyph_spacing_prefixes_px,
    collect_prepared_glyphs_from_buffer, cursor_stops_for_glyphs, glyph_logical_font_size_points,
    hash_text_fundamentals, hit_buffer_with_fundamentals, rasterize_atlas_glyph,
    render_swash_outline_commands, shared_variation_settings,
};
pub(crate) use crate::conversions::{
    core_label_options, cosmic_to_egui_color, egui_key_from_text, egui_modifiers_from_text,
    glyph_content_mode_from_rasterization, multiply_color32, texture_options_for_sampling,
    to_cosmic_color, to_cosmic_text_color, wgpu_filter_mode_for_sampling,
};
use crate::cursor_layout::{editor_cursor_x_in_run, editor_sel_rect};
use crate::editor::{
    EditorScrollMetrics, InputState, UndoEntry, UndoOpKind, clamp_borrowed_buffer_scroll,
    clamp_cursor_to_editor, clamp_selection_to_editor, click_editor_to_pointer,
    double_click_editor_to_pointer, drag_editor_selection_to_pointer, editor_horizontal_scroll,
    editor_to_string, extend_selection_to_pointer, handle_editor_key_event,
    handle_read_only_editor_key_event, is_navigation_event, measure_borrowed_buffer_scroll_metrics,
    measure_buffer_pixels, pending_modify_op, push_undo, scroll_editor_to_buffer_end, select_all,
    triple_click_editor_to_pointer, viewer_scrollbar_track_rects, viewer_visible_text_rect,
};
pub(crate) use crate::font_features::{
    build_font_features, compose_font_features, configure_text_font_defaults, opsz_for_font_size,
    parse_feature_tag_list, resolved_hinting_enabled, resolved_stem_darkening_strength,
};
use crate::geometry::{
    egui_point_from_text, egui_rect_from_text, egui_vec_from_text, snap_rect_to_pixel_grid,
    snap_width_to_bin,
};
use crate::gpu::{
    CpuSceneAtlasPage, ResolvedTextGraphicsConfig, ResolvedTextRendererBackend, TextWgpuInstance,
    TextWgpuPreparedScene, TextWgpuSceneBatchSource, TextWgpuSceneCallback,
    allocate_cpu_scene_page_slot, blit_to_page, color_image_to_page_data, cpu_page_to_page_data,
    default_gpu_scene_page_side, gpu_scene_approx_bytes, gpu_scene_page_batches_approx_bytes,
    map_scene_quads_to_rect, paint_text_quads_fallback, quad_positions_from_min_size,
    rect_from_points, rotated_quad_positions, uv_quad_points,
};
use crate::input_runtime::apply_gamepad_scroll_if_focused;
use crate::path_layout::{
    build_path_layout_from_prepared_layout, export_prepared_layout_as_shapes,
};
pub(crate) use crate::prepared_layout::{
    PreparedGlyph, PreparedTextCacheEntry, PreparedTextLayout,
};
use crate::{
    button_options::ButtonOptions, clipboard::copy_sanitized, code_block_options::CodeBlockOptions,
    input_options::InputOptions, label_options::LabelOptions, markdown_options::MarkdownOptions,
    markdown_parser::parse_markdown_blocks, tooltip_options::TooltipOptions,
};

pub use advanced_text::DEFAULT_ELLIPSIS;
pub use advanced_text::{
    RichTextSpan, RichTextStyle, TextAtlasPageData, TextAtlasPageSnapshot, TextAtlasQuad,
    TextAtlasSampling, TextColor, TextFeatureSetting, TextFrameInfo, TextFrameOutput,
    TextFundamentals, TextGlyphRasterMode, TextGpuPowerPreference, TextGpuQuad, TextGpuScene,
    TextGpuSceneDrawOptions, TextGpuScenePageBatch, TextGraphicsApi, TextGraphicsConfig,
    TextHintingMode, TextInputEvent, TextKerning, TextKey, TextLabelOptions, TextMarkdownBlock,
    TextMarkdownHeadingLevel, TextModifiers, TextOpticalSizingMode, TextPath, TextPathError,
    TextPathGlyph, TextPathLayout, TextPathOptions, TextPoint, TextPointerButton,
    TextRasterizationConfig, TextRect, TextRenderScene, TextRendererBackend, TextRenderingPolicy,
    TextStemDarkeningMode, TextVariationSetting, TextVector, VectorGlyphShape, VectorPathCommand,
    VectorTextShape,
};
pub use clipboard::{apply_smart_quotes, sanitize_for_clipboard};
pub use conversions::{
    wgpu_backends_for_text_graphics_api, wgpu_power_preference_for_text_gpu_preference,
};
#[doc(hidden)]
pub use input_options::InputOptions as EguiInputOptions;

/// Default OpenType feature tags applied when no explicit feature string is
/// provided to [`TextUi::apply_open_type_features`].
pub const DEFAULT_OPEN_TYPE_FEATURE_TAGS: &str = "kern, liga, calt, onum, pnum";
const PREPARED_TEXT_CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;
const ASYNC_RASTER_CACHE_MAX_BYTES: usize = 24 * 1024 * 1024;
const GPU_SCENE_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const GPU_SCENE_PAGE_BATCH_CACHE_MAX_BYTES: usize = 24 * 1024 * 1024;
const GPU_SCENE_DRAW_BATCH_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const GPU_SCENE_GLYPH_CACHE_MAX_BYTES: usize = 24 * 1024 * 1024;
const GLYPH_ATLAS_MAX_BYTES: usize = 64 * 1024 * 1024;
const GLYPH_ATLAS_STALE_FRAMES: u64 = 900;
const GLYPH_ATLAS_PAGE_TARGET_PX: usize = 1024;
const GLYPH_ATLAS_PADDING_PX: i32 = 1;
const GLYPH_ATLAS_FETCH_MAX_PER_FRAME: usize = 128;
const GLYPH_ATLAS_UPLOAD_MAX_GLYPHS_PER_FRAME: usize = 64;
const GLYPH_ATLAS_UPLOAD_MAX_BYTES_PER_FRAME: usize = 512 * 1024;
const AUTO_MSDF_MIN_LOGICAL_FONT_SIZE_PT: f32 = 28.0;
const FIELD_GLYPH_MEAN_ALPHA_ERROR_LIMIT: f32 = 0.045;
const FIELD_GLYPH_MAX_ALPHA_ERROR_LIMIT: f32 = 0.25;
const FIELD_GLYPH_LARGE_ERROR_PIXEL_RATIO_LIMIT: f32 = 0.02;
const OUTLINE_REFERENCE_SUPERSAMPLES_PER_AXIS: usize = 4;
const TEXT_WGPU_PASS_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const TEXT_WGPU_PASS_MSAA_SAMPLES: u32 = 4;
const INPUT_STATE_STALE_FRAMES: u64 = 900;
const UNDO_STACK_MAX: usize = 200;

// Width-bin size in device pixels.  Labels whose available width differs by
// less than this will share the same cached texture, preventing mass cache
// busts from sub-pixel layout jitter (scrollbars, fractional DPI, etc.).
const WIDTH_BIN_PX: f32 = 16.0;

/// Snap a point-space width to the nearest WIDTH_BIN_PX device-pixel boundary.
#[inline]
fn color_image_byte_size(image: &ColorImage) -> usize {
    color_image_byte_size_from_size(image.size)
}

fn color_image_byte_size_from_size(size: [usize; 2]) -> usize {
    size[0]
        .saturating_mul(size[1])
        .saturating_mul(mem::size_of::<Color32>())
}

#[inline]
fn new_fingerprint_hasher() -> FxHasher {
    FxHasher::default()
}

type SpanStyle = RichTextStyle;
type RichSpan = RichTextSpan;

impl TextUi {
    /// Advances the engine-side frame state without requiring an [`egui::Context`].
    ///
    /// Consumers that use the context-free scene/export APIs can drive frame maintenance
    /// through this method and inject any relevant input/render state separately.
    pub fn begin_frame_info(&mut self, frame_info: TextFrameInfo) {
        self.current_frame = frame_info.frame_number;
        let current_frame = self.current_frame;
        let max_texture_side_px = frame_info.max_texture_side_px.max(1);
        let graphics_config = self.resolved_graphics_config(max_texture_side_px);
        self.frame_events.clear();
        self.glyph_atlas
            .set_page_side(graphics_config.atlas_page_target_px);
        self.glyph_atlas
            .set_sampling(graphics_config.atlas_sampling);
        self.glyph_atlas
            .set_padding(graphics_config.atlas_padding_px);
        self.glyph_atlas
            .set_rasterization(graphics_config.rasterization);
        self.glyph_atlas
            .set_linear_pipeline(self.graphics_config.linear_pipeline);
        if self.max_texture_side_px != max_texture_side_px {
            self.max_texture_side_px = max_texture_side_px;
            self.invalidate_text_caches(false);
        }
        self.prepared_texts.write(|state| {
            state.retain(|_, entry| {
                current_frame.saturating_sub(entry.value.last_used_frame)
                    <= INPUT_STATE_STALE_FRAMES
            });
        });
        self.markdown_cache.retain(|_, (_, last_used_frame, _)| {
            current_frame.saturating_sub(*last_used_frame) <= INPUT_STATE_STALE_FRAMES
        });
        self.input_states.retain(|_, state| {
            current_frame.saturating_sub(state.last_used_frame) <= INPUT_STATE_STALE_FRAMES
        });
        self.glyph_atlas.trim_stale(current_frame);
        self.enforce_prepared_text_cache_budget();
        self.enforce_async_raster_cache_budget();
        self.enforce_gpu_scene_cache_budget();
        self.poll_async_raster_results();
    }

    /// Replaces the per-frame input event buffer used by interactive widgets.
    pub fn set_frame_input_events(&mut self, frame_events: Vec<TextInputEvent>) {
        self.frame_events = frame_events;
    }

    /// Clears any per-frame input events previously set by [`Self::set_frame_input_events`].
    pub fn clear_frame_input_events(&mut self) {
        self.frame_events.clear();
    }

    #[doc(hidden)]
    /// Updates the optional native WGPU render state used by the atlas renderer.
    ///
    /// When using native atlas pages, set this before [`Self::begin_frame_info`] so any
    /// frame-start invalidation can release old textures through the current renderer.
    pub fn egui_set_render_state(&mut self, render_state: Option<&EguiWgpuRenderState>) {
        self.glyph_atlas.set_render_state(render_state);
    }

    #[doc(hidden)]
    /// Flushes pending atlas work that still needs an [`egui::Context`] for texture uploads.
    pub fn egui_flush_frame(&mut self, ctx: &Context) -> TextFrameOutput {
        self.glyph_atlas.poll_ready(ctx, self.current_frame);
        let needs_repaint = !self.glyph_atlas.pending.is_empty();
        if needs_repaint {
            ctx.request_repaint();
        }
        TextFrameOutput { needs_repaint }
    }

    pub fn set_graphics_config(&mut self, graphics_config: TextGraphicsConfig) {
        if self.graphics_config != graphics_config {
            self.graphics_config = graphics_config;
            self.invalidate_text_caches(false);
        }
    }

    pub fn graphics_config(&self) -> TextGraphicsConfig {
        self.graphics_config
    }

    pub fn set_gpu_instancing_enabled(&mut self, enabled: bool) {
        let mut graphics_config = self.graphics_config;
        graphics_config.renderer_backend = if enabled {
            TextRendererBackend::WgpuInstanced
        } else {
            TextRendererBackend::EguiMesh
        };
        self.set_graphics_config(graphics_config);
    }

    pub fn gpu_instancing_enabled(&self) -> bool {
        !matches!(
            self.resolved_graphics_config(self.max_texture_side_px.max(1))
                .renderer_backend,
            ResolvedTextRendererBackend::EguiMesh
        )
    }

    fn resolved_graphics_config(&self, max_texture_side_px: usize) -> ResolvedTextGraphicsConfig {
        let renderer_backend = match self.graphics_config.renderer_backend {
            TextRendererBackend::Auto => ResolvedTextRendererBackend::WgpuInstanced,
            TextRendererBackend::EguiMesh => ResolvedTextRendererBackend::EguiMesh,
            TextRendererBackend::WgpuInstanced => ResolvedTextRendererBackend::WgpuInstanced,
        };
        ResolvedTextGraphicsConfig {
            renderer_backend,
            atlas_sampling: self.graphics_config.atlas_sampling,
            atlas_page_target_px: self
                .graphics_config
                .atlas_page_target_px
                .max(256)
                .min(max_texture_side_px.max(1)),
            atlas_padding_px: self.graphics_config.atlas_padding_px,
            rasterization: self.graphics_config.rasterization,
            output_is_hdr: self.graphics_config.output_is_hdr,
        }
    }

    fn resolved_glyph_content_mode(
        &self,
        graphics_config: ResolvedTextGraphicsConfig,
        glyph: &GlyphRasterKey,
    ) -> GlyphContentMode {
        if graphics_config.renderer_backend != ResolvedTextRendererBackend::WgpuInstanced {
            return GlyphContentMode::AlphaMask;
        }
        if self.glyph_atlas.wgpu_render_state.is_none() {
            return GlyphContentMode::AlphaMask;
        }

        match graphics_config.rasterization.glyph_raster_mode {
            TextGlyphRasterMode::Auto => {
                let logical_font_size = glyph_logical_font_size_points(glyph);
                if logical_font_size >= AUTO_MSDF_MIN_LOGICAL_FONT_SIZE_PT {
                    GlyphContentMode::Msdf
                } else {
                    GlyphContentMode::Sdf
                }
            }
            mode => glyph_content_mode_from_rasterization(mode),
        }
    }

    /// Registers additional font bytes for rendering.
    ///
    /// This clears cached textures/input states so new faces are picked up.
    pub fn register_font_data(&mut self, bytes: Vec<u8>) {
        if let Some(tx) = self.async_raster.tx.as_ref() {
            let _ = tx.send(AsyncRasterWorkerMessage::RegisterFont(bytes.clone()));
        }
        self.glyph_atlas.register_font(bytes.clone());
        self.font_system.db_mut().load_font_data(bytes);
        self.invalidate_text_caches(true);
    }

    /// Renders an asynchronously rasterized label.
    #[allow(dead_code)]
    fn label_async(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        self.label_impl(ui, id_source, text, options, Sense::hover(), true)
    }

    /// Renders an asynchronously rasterized syntax-highlighted code block.
    #[allow(dead_code)]
    fn code_block_async(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response {
        let scale = ui.ctx().pixels_per_point();
        let width_points_opt = if options.wrap {
            Some(snap_width_to_bin(
                (ui.available_width() - options.padding.x * 2.0).max(1.0),
                scale,
            ))
        } else {
            None
        };

        let spans =
            self.highlight_code_spans_impl(code, options.language.as_deref(), options.text_color);
        let label_options = LabelOptions {
            font_size: options.font_size,
            line_height: options.line_height,
            color: options.text_color,
            wrap: options.wrap,
            monospace: true,
            weight: 400,
            italic: false,
            padding: egui::Vec2::ZERO,
            fundamentals: options.fundamentals.clone(),
            ..LabelOptions::default()
        };

        let mut hasher = new_fingerprint_hasher();
        "code_async".hash(&mut hasher);
        code.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.text_color.hash(&mut hasher);
        options.background_color.hash(&mut hasher);
        options.language.hash(&mut hasher);
        scale.to_bits().hash(&mut hasher);
        width_points_opt
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();
        let _texture_id = ui.make_persistent_id(id_source).with("textui_code");

        let layout = self.get_or_queue_async_rich_layout(
            fingerprint,
            spans,
            &label_options,
            width_points_opt,
            scale,
        );

        if let Some(layout) = layout {
            let scene = self.build_text_scene_from_layout(ui.ctx(), &layout, scale);
            let scene_size = egui_vec_from_text(scene.size_points);
            let desired_size = scene_size + options.padding * 2.0;
            let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

            let bg_shape = egui::Shape::rect_filled(
                rect,
                CornerRadius::same(options.corner_radius),
                options.background_color,
            );
            ui.painter().add(bg_shape);
            if options.stroke.width > 0.0 {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(options.corner_radius),
                    options.stroke,
                    egui::StrokeKind::Inside,
                );
            }

            let image_rect = Rect::from_min_size(rect.min + options.padding, scene_size);
            let painter = ui.painter().with_clip_rect(ui.clip_rect());
            self.paint_scene_in_rect(&painter, image_rect, &scene);
            return response;
        }

        let fallback_height = (options.line_height * 2.0 + options.padding.y * 2.0).max(32.0);
        let desired_size = egui::vec2(
            width_points_opt.unwrap_or_else(|| ui.available_width().max(1.0)),
            fallback_height,
        );
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(options.corner_radius),
            options.background_color,
        );
        ui.ctx().request_repaint();
        response
    }

    /// Applies font family/size/weight preferences for subsequent text renders.
    pub fn apply_typography(&mut self, family_candidates: &[&str], size_points: f32, weight: i32) {
        let family = self.resolve_family_candidate(family_candidates);
        let size_scale = (size_points / 18.0).clamp(0.50, 3.00);
        let clamped_weight = weight.clamp(100, 900);

        if self.ui_font_family == family
            && (self.ui_font_size_scale - size_scale).abs() <= f32::EPSILON
            && self.ui_font_weight == clamped_weight
        {
            return;
        }

        self.ui_font_family = family;
        self.ui_font_size_scale = size_scale;
        self.ui_font_weight = clamped_weight;
        self.invalidate_text_caches(false);
    }

    /// Enables/disables OpenType features and updates active tag selection.
    pub fn apply_open_type_features(
        &mut self,
        enabled: bool,
        feature_tags_csv: &str,
        family_candidates: &[&str],
    ) {
        let normalized_csv = feature_tags_csv.trim().to_owned();
        let parsed_tags = parse_feature_tag_list(&normalized_csv);
        let active_tags = if enabled {
            if parsed_tags.is_empty() {
                let default_tags = parse_feature_tag_list(DEFAULT_OPEN_TYPE_FEATURE_TAGS);
                if default_tags.is_empty() {
                    self.collect_available_feature_tags_for_family(family_candidates)
                } else {
                    default_tags
                }
            } else {
                parsed_tags
            }
        } else {
            Vec::new()
        };
        let active_features = if enabled && !active_tags.is_empty() {
            Some(build_font_features(&active_tags))
        } else {
            None
        };

        if self.open_type_features_enabled == enabled
            && self.open_type_features_to_enable == normalized_csv
            && self.open_type_feature_tags == active_tags
            && self.open_type_features == active_features
        {
            return;
        }

        self.open_type_features_enabled = enabled;
        self.open_type_features_to_enable = normalized_csv;
        self.open_type_feature_tags = active_tags;
        self.open_type_features = active_features;
        self.invalidate_text_caches(false);
    }

    fn resolve_family_candidate(&self, family_candidates: &[&str]) -> Option<String> {
        for candidate in family_candidates {
            if self.font_system.db().faces().any(|face| {
                face.families
                    .iter()
                    .any(|(family, _)| family.eq_ignore_ascii_case(candidate))
            }) {
                return Some((*candidate).to_owned());
            }
        }
        None
    }

    fn resolve_face_id_for_family(
        &self,
        family_candidates: &[&str],
    ) -> Option<cosmic_text::fontdb::ID> {
        for candidate in family_candidates {
            if let Some(face) = self.font_system.db().faces().find(|face| {
                face.families
                    .iter()
                    .any(|(family, _)| family.eq_ignore_ascii_case(candidate))
            }) {
                return Some(face.id);
            }
        }
        None
    }

    fn collect_available_feature_tags_for_family(
        &self,
        family_candidates: &[&str],
    ) -> Vec<[u8; 4]> {
        let Some(face_id) = self.resolve_face_id_for_family(family_candidates) else {
            return Vec::new();
        };

        let mut tags = BTreeSet::new();
        let _ = self
            .font_system
            .db()
            .with_face_data(face_id, |font_data, face_index| {
                let Ok(face) = SkrifaFontRef::from_index(font_data, face_index) else {
                    return Some(());
                };

                if let Ok(gsub) = face.gsub() {
                    if let Ok(feature_list) = gsub.feature_list() {
                        for record in feature_list.feature_records().iter() {
                            tags.insert(record.feature_tag().into_bytes());
                        }
                    }
                }

                if let Ok(gpos) = face.gpos() {
                    if let Ok(feature_list) = gpos.feature_list() {
                        for record in feature_list.feature_records().iter() {
                            tags.insert(record.feature_tag().into_bytes());
                        }
                    }
                }

                Some(())
            });

        tags.into_iter().collect()
    }

    /// Produces syntax-highlighted rich spans for code rendering.
    pub fn highlight_code_spans(
        &self,
        code: &str,
        language: Option<&str>,
        fallback_color: TextColor,
    ) -> Vec<RichTextSpan> {
        self.highlight_code_spans_impl(code, language, fallback_color.into())
    }

    pub fn parse_markdown_blocks_cached(
        &mut self,
        id_source: impl Hash,
        markdown: &str,
        fingerprint: u64,
    ) -> Arc<[TextMarkdownBlock]> {
        let cache_id = Id::new(id_source).with("textui_markdown_blocks");
        if let Some((fp, last_used, cached)) = self.markdown_cache.get_mut(&cache_id) {
            *last_used = self.current_frame;
            if *fp == fingerprint {
                return Arc::clone(cached);
            }
            let blocks = Arc::<[TextMarkdownBlock]>::from(parse_markdown_blocks(markdown));
            *fp = fingerprint;
            *cached = Arc::clone(&blocks);
            return blocks;
        }

        let blocks = Arc::<[TextMarkdownBlock]>::from(parse_markdown_blocks(markdown));
        self.markdown_cache.insert(
            cache_id,
            (fingerprint, self.current_frame, Arc::clone(&blocks)),
        );
        blocks
    }

    pub fn atlas_page_snapshot(&self, page_index: usize) -> Option<TextAtlasPageSnapshot> {
        self.glyph_atlas.page_snapshot(page_index)
    }

    pub fn atlas_page_data(&self, page_index: usize) -> Option<TextAtlasPageData> {
        self.glyph_atlas.page_data(page_index)
    }

    pub fn atlas_page_snapshots_for_scene(
        &self,
        scene: &TextRenderScene,
    ) -> Vec<TextAtlasPageSnapshot> {
        scene
            .atlas_page_indices()
            .into_iter()
            .filter_map(|page_index| self.atlas_page_snapshot(page_index))
            .collect()
    }

    pub fn atlas_page_data_for_scene(&self, scene: &TextRenderScene) -> Vec<TextAtlasPageData> {
        scene
            .atlas_page_indices()
            .into_iter()
            .filter_map(|page_index| self.atlas_page_data(page_index))
            .collect()
    }

    pub fn gpu_scene_for_scene(&self, scene: &TextRenderScene) -> TextGpuScene {
        scene.to_gpu_scene(self.atlas_page_data_for_scene(scene))
    }

    pub fn gpu_scene_page_batches(&self, scene: &TextGpuScene) -> Arc<[TextGpuScenePageBatch]> {
        let fingerprint = if scene.fingerprint != 0 {
            scene.fingerprint
        } else {
            let mut hasher = new_fingerprint_hasher();
            scene.bounds_min[0].to_bits().hash(&mut hasher);
            scene.bounds_min[1].to_bits().hash(&mut hasher);
            scene.bounds_max[0].to_bits().hash(&mut hasher);
            scene.bounds_max[1].to_bits().hash(&mut hasher);
            scene.size_points[0].to_bits().hash(&mut hasher);
            scene.size_points[1].to_bits().hash(&mut hasher);
            for quad in &scene.quads {
                quad.atlas_page_index.hash(&mut hasher);
                for point in quad.positions {
                    point[0].to_bits().hash(&mut hasher);
                    point[1].to_bits().hash(&mut hasher);
                }
                for point in quad.uvs {
                    point[0].to_bits().hash(&mut hasher);
                    point[1].to_bits().hash(&mut hasher);
                }
                quad.tint_rgba.hash(&mut hasher);
            }
            hasher.finish()
        };

        if let Some(batches) = self.gpu_scene_page_batch_cache.write(|state| {
            state
                .touch(&fingerprint)
                .map(|entry| Arc::clone(&entry.value))
        }) {
            return batches;
        }

        let mut grouped = FxHashMap::<usize, Vec<TextGpuQuad>>::default();
        for quad in &scene.quads {
            grouped
                .entry(quad.atlas_page_index)
                .or_default()
                .push(quad.clone());
        }
        let mut page_indices = grouped.keys().copied().collect::<Vec<_>>();
        page_indices.sort_unstable();
        let batches_vec = page_indices
            .into_iter()
            .map(|page_index| TextGpuScenePageBatch {
                page_index,
                quads: Arc::from(
                    grouped
                        .remove(&page_index)
                        .unwrap_or_default()
                        .into_boxed_slice(),
                ),
            })
            .collect::<Vec<_>>();
        let approx_bytes = gpu_scene_page_batches_approx_bytes(&batches_vec);
        let batches = Arc::from(batches_vec.into_boxed_slice());
        self.gpu_scene_page_batch_cache.write(|state| {
            let _ = state.insert(fingerprint, Arc::clone(&batches), approx_bytes);
        });
        batches
    }

    pub fn prepare_gpu_scene_draw_batches(
        &self,
        scene: &TextGpuScene,
        options: TextGpuSceneDrawOptions,
    ) -> Arc<[TextGpuScenePageBatch]> {
        let scene_fingerprint = if scene.fingerprint != 0 {
            scene.fingerprint
        } else {
            let mut hasher = new_fingerprint_hasher();
            scene.bounds_min[0].to_bits().hash(&mut hasher);
            scene.bounds_min[1].to_bits().hash(&mut hasher);
            scene.bounds_max[0].to_bits().hash(&mut hasher);
            scene.bounds_max[1].to_bits().hash(&mut hasher);
            scene.size_points[0].to_bits().hash(&mut hasher);
            scene.size_points[1].to_bits().hash(&mut hasher);
            hasher.finish()
        };
        let mut hasher = new_fingerprint_hasher();
        scene_fingerprint.hash(&mut hasher);
        options.offset.x.to_bits().hash(&mut hasher);
        options.offset.y.to_bits().hash(&mut hasher);
        options.scale.x.to_bits().hash(&mut hasher);
        options.scale.y.to_bits().hash(&mut hasher);
        options.tint.to_array().hash(&mut hasher);
        let draw_fingerprint = hasher.finish();

        if let Some(batches) = self.gpu_scene_draw_batch_cache.write(|state| {
            state
                .touch(&draw_fingerprint)
                .map(|entry| Arc::clone(&entry.value))
        }) {
            return batches;
        }

        let source_batches = self.gpu_scene_page_batches(scene);
        let transformed_batches = source_batches
            .iter()
            .map(|batch| {
                let quads = batch
                    .quads
                    .iter()
                    .map(|quad| {
                        let positions = quad.positions.map(|point| {
                            [
                                options.offset.x + point[0] * options.scale.x,
                                options.offset.y + point[1] * options.scale.y,
                            ]
                        });
                        let tint = multiply_color32(
                            Color32::from_rgba_premultiplied(
                                quad.tint_rgba[0],
                                quad.tint_rgba[1],
                                quad.tint_rgba[2],
                                quad.tint_rgba[3],
                            ),
                            options.tint.into(),
                        );
                        TextGpuQuad {
                            atlas_page_index: quad.atlas_page_index,
                            positions,
                            uvs: quad.uvs,
                            tint_rgba: tint.to_array(),
                        }
                    })
                    .collect::<Vec<_>>();
                TextGpuScenePageBatch {
                    page_index: batch.page_index,
                    quads: Arc::from(quads.into_boxed_slice()),
                }
            })
            .collect::<Vec<_>>();
        let approx_bytes = gpu_scene_page_batches_approx_bytes(&transformed_batches);
        let batches = Arc::from(transformed_batches.into_boxed_slice());
        self.gpu_scene_draw_batch_cache.write(|state| {
            let _ = state.insert(draw_fingerprint, Arc::clone(&batches), approx_bytes);
        });
        batches
    }

    fn paint_scene_in_rect(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        scene: &TextRenderScene,
    ) {
        self.paint_scene_in_rect_tinted(painter, rect, scene, Color32::WHITE);
    }

    fn paint_scene_in_rect_tinted(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        scene: &TextRenderScene,
        tint: Color32,
    ) {
        let rect = snap_rect_to_pixel_grid(rect, painter.pixels_per_point());
        let quads = map_scene_quads_to_rect(
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            egui_vec_from_text(scene.size_points),
            &scene.quads,
            tint,
        );
        self.paint_text_quads(painter, rect, &quads);
    }

    #[allow(dead_code)]
    fn label_impl(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        sense: Sense,
        async_mode: bool,
    ) -> Response {
        // Apply smart-quote transformation for display when enabled.
        let display_text;
        let text = if options.fundamentals.smart_quotes {
            display_text = apply_smart_quotes(text);
            display_text.as_str()
        } else {
            text
        };

        let scale = ui.ctx().pixels_per_point();
        // Snap available_width to bin boundaries so sub-pixel jitter
        // (scrollbars appearing, fractional DPI) does not bust the cache for
        // every label on screen simultaneously.
        let width_points_opt = if options.wrap {
            Some(snap_width_to_bin(ui.available_width().max(1.0), scale))
        } else {
            None
        };

        let mut hasher = new_fingerprint_hasher();
        "label".hash(&mut hasher);
        text.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
        options.weight.hash(&mut hasher);
        options.italic.hash(&mut hasher);
        options.color.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        width_points_opt
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();
        let cache_id = ui.make_persistent_id(id_source).with("textui_label");
        let scene = if async_mode {
            match self.get_or_queue_async_plain_layout(
                fingerprint,
                text.to_owned(),
                options,
                width_points_opt,
                scale,
            ) {
                Some(layout) => self.build_text_scene_from_layout(ui.ctx(), &layout, scale),
                None => {
                    let fallback_height = (options.line_height + options.padding.y * 2.0).max(20.0);
                    let fallback_width =
                        width_points_opt.unwrap_or_else(|| ui.available_width().max(1.0));
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(fallback_width, fallback_height), sense);
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(4),
                        ui.visuals().faint_bg_color,
                    );
                    ui.ctx().request_repaint();
                    return response;
                }
            }
        } else {
            let layout =
                self.get_or_prepare_label_layout(cache_id, text, options, width_points_opt, scale);
            self.build_text_scene_from_layout(ui.ctx(), &layout, scale)
        };
        let scene_size = egui_vec_from_text(scene.size_points);
        if scene_size == Vec2::ZERO {
            let fallback_height = (options.line_height + options.padding.y * 2.0).max(20.0);
            let fallback_width = width_points_opt.unwrap_or_else(|| ui.available_width().max(1.0));
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(fallback_width, fallback_height), sense);
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), ui.visuals().faint_bg_color);
            ui.ctx().request_repaint();
            return response;
        }

        let desired_size = scene_size + options.padding * 2.0;
        let (rect, response) = ui.allocate_exact_size(desired_size, sense);
        let image_rect = Rect::from_min_size(rect.min + options.padding, scene_size);
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        self.paint_scene_in_rect(&painter, image_rect, &scene);

        response
    }

    /// Renders a button with text styles from [`ButtonOptions`].
    #[allow(dead_code)]
    fn button(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &ButtonOptions,
    ) -> Response {
        self.button_impl(ui, id_source, text, false, options)
    }

    /// Renders a selectable button variant.
    #[allow(dead_code)]
    fn selectable_button(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        selected: bool,
        options: &ButtonOptions,
    ) -> Response {
        self.button_impl(ui, id_source, text, selected, options)
    }

    #[allow(dead_code)]
    fn button_impl(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        selected: bool,
        options: &ButtonOptions,
    ) -> Response {
        let mut label_style = LabelOptions::default();
        label_style.font_size = options.font_size;
        label_style.line_height = options.line_height;
        label_style.color = options.text_color;
        label_style.wrap = false;

        let scale = ui.ctx().pixels_per_point();
        let text_cache_id = ui.make_persistent_id(id_source).with("button_text");
        let text_layout =
            self.get_or_prepare_label_layout(text_cache_id, text, &label_style, None, scale);
        let text_scene = self.build_text_scene_from_layout(ui.ctx(), &text_layout, scale);
        let text_size = egui_vec_from_text(text_scene.size_points);

        let desired_size = egui::vec2(
            (text_size.x + options.padding.x * 2.0).max(options.min_size.x),
            (text_size.y + options.padding.y * 2.0).max(options.min_size.y),
        );

        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
        let has_focus = response.has_focus();

        let fill = if response.is_pointer_button_down_on() {
            options.fill_active
        } else if response.hovered() {
            options.fill_hovered
        } else if selected || has_focus {
            options.fill_selected
        } else {
            options.fill
        };
        let stroke = if has_focus {
            ui.visuals().selection.stroke
        } else {
            options.stroke
        };

        ui.painter()
            .rect_filled(rect, CornerRadius::same(options.corner_radius), fill);
        if stroke.width > 0.0 {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(options.corner_radius),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        if has_focus {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(options.corner_radius.saturating_add(2)),
                egui::Stroke::new(
                    (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                    ui.visuals().selection.stroke.color,
                ),
                egui::StrokeKind::Outside,
            );
        }

        let text_rect = Rect::from_center_size(rect.center(), text_size);
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        self.paint_scene_in_rect(&painter, text_rect, &text_scene);
        apply_gamepad_scroll_if_focused(ui, &response);

        response
    }

    /// Shows a tooltip while the provided response is hovered.
    #[allow(dead_code)]
    fn tooltip_for_response(
        &mut self,
        ui: &Ui,
        id_source: impl Hash,
        response: &Response,
        text: &str,
        options: &TooltipOptions,
    ) {
        if !response.hovered() {
            return;
        }

        let pointer = response.hover_pos().unwrap_or(response.rect.right_bottom());
        let scale = ui.ctx().pixels_per_point();
        let width_points_opt = Some(snap_width_to_bin(
            320.0_f32.min(ui.ctx().input(|i| i.content_rect().width() * 0.35)),
            scale,
        ));

        // ── Cache the tooltip texture; rasterize only when content changes ───
        let tooltip_tex_id = ui.make_persistent_id(&id_source).with("tooltip_text");
        let _tooltip_fingerprint = {
            let mut hasher = new_fingerprint_hasher();
            "textui_tooltip".hash(&mut hasher);
            text.hash(&mut hasher);
            options.text.font_size.to_bits().hash(&mut hasher);
            options.text.line_height.to_bits().hash(&mut hasher);
            options.text.color.hash(&mut hasher);
            scale.to_bits().hash(&mut hasher);
            width_points_opt
                .map(f32::to_bits)
                .unwrap_or(0)
                .hash(&mut hasher);
            self.hash_typography(&mut hasher);
            hasher.finish()
        };

        let scene = self.prepare_label_scene(
            ui.ctx(),
            tooltip_tex_id,
            text,
            &options.text,
            width_points_opt,
        );
        let raster_size = egui_vec_from_text(scene.size_points);

        let size = raster_size + options.padding * 2.0;
        let mut rect = Rect::from_min_size(pointer + options.offset, size);
        let min_y = ui.clip_rect().top();
        if rect.min.y < min_y {
            let delta = min_y - rect.min.y;
            rect = rect.translate(egui::vec2(0.0, delta));
        }

        // Keep the tooltip background and its rasterized text on the physical pixel grid.
        // Without this, tiny cursor-position changes can move the textured glyphs onto
        // fractional coordinates, which makes the same cached tooltip look fuzzy.
        rect = snap_rect_to_pixel_grid(rect, scale);

        let layer_id = egui::LayerId::new(
            egui::Order::Tooltip,
            ui.make_persistent_id(&id_source).with("tooltip_layer"),
        );
        let painter = ui.ctx().layer_painter(layer_id);
        painter.rect_filled(
            rect,
            CornerRadius::same(options.corner_radius),
            options.background,
        );
        if options.stroke.width > 0.0 {
            painter.rect_stroke(
                rect,
                CornerRadius::same(options.corner_radius),
                options.stroke,
                egui::StrokeKind::Inside,
            );
        }

        let text_rect = snap_rect_to_pixel_grid(
            Rect::from_min_size(rect.min + options.padding, raster_size),
            scale,
        );
        self.paint_scene_in_rect(&painter, text_rect, &scene);
    }

    /// Renders a syntax-highlighted code block synchronously.
    #[allow(dead_code)]
    fn code_block(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response {
        let scale = ui.ctx().pixels_per_point();
        let width_points_opt = if options.wrap {
            Some(snap_width_to_bin(
                (ui.available_width() - options.padding.x * 2.0).max(1.0),
                scale,
            ))
        } else {
            None
        };

        let mut hasher = new_fingerprint_hasher();
        "code".hash(&mut hasher);
        code.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.text_color.hash(&mut hasher);
        options.background_color.hash(&mut hasher);
        options.language.hash(&mut hasher);
        scale.to_bits().hash(&mut hasher);
        width_points_opt
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let _fingerprint = hasher.finish();
        let scene_id = ui.make_persistent_id(id_source).with("textui_code");

        let spans =
            self.highlight_code_spans_impl(code, options.language.as_deref(), options.text_color);
        let scene = self.prepare_rich_text_scene(
            ui.ctx(),
            scene_id,
            &spans,
            &LabelOptions {
                font_size: options.font_size,
                line_height: options.line_height,
                color: options.text_color,
                wrap: options.wrap,
                monospace: true,
                weight: 400,
                italic: false,
                padding: egui::Vec2::ZERO,
                fundamentals: options.fundamentals.clone(),
                ..LabelOptions::default()
            },
            width_points_opt,
        );

        let scene_size = egui_vec_from_text(scene.size_points);
        let desired_size = scene_size + options.padding * 2.0;
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

        let bg_shape = egui::Shape::rect_filled(
            rect,
            CornerRadius::same(options.corner_radius),
            options.background_color,
        );
        ui.painter().add(bg_shape);
        if options.stroke.width > 0.0 {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(options.corner_radius),
                options.stroke,
                egui::StrokeKind::Inside,
            );
        }

        let image_rect = Rect::from_min_size(rect.min + options.padding, scene_size);
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        self.paint_scene_in_rect(&painter, image_rect, &scene);

        response
    }

    /// Renders simple markdown (headings, paragraphs, fenced code).
    #[allow(dead_code)]
    fn markdown(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        markdown: &str,
        options: &MarkdownOptions,
    ) {
        // ── Markdown block cache ──────────────────────────────────────────────
        // parse_markdown_blocks is a full pulldown-cmark parse.  Cache the
        // result by (content + options) fingerprint to avoid re-parsing every
        // frame when nothing changed.
        let cache_id = ui.make_persistent_id(&id_source).with("md_cache");
        let md_fingerprint = {
            let mut hasher = new_fingerprint_hasher();
            "markdown_blocks".hash(&mut hasher);
            markdown.hash(&mut hasher);
            options.heading_scale.to_bits().hash(&mut hasher);
            options.paragraph_spacing.to_bits().hash(&mut hasher);
            options.body.font_size.to_bits().hash(&mut hasher);
            options.body.line_height.to_bits().hash(&mut hasher);
            options.body.color.hash(&mut hasher);
            options.code.font_size.to_bits().hash(&mut hasher);
            hasher.finish()
        };

        let blocks = if let Some((fp, last_used, cached)) = self.markdown_cache.get_mut(&cache_id) {
            *last_used = self.current_frame;
            if *fp == md_fingerprint {
                Arc::clone(cached)
            } else {
                let b = Arc::<[TextMarkdownBlock]>::from(parse_markdown_blocks(markdown));
                *fp = md_fingerprint;
                *cached = Arc::clone(&b);
                b
            }
        } else {
            let b = Arc::<[TextMarkdownBlock]>::from(parse_markdown_blocks(markdown));
            self.markdown_cache.insert(
                cache_id,
                (md_fingerprint, self.current_frame, Arc::clone(&b)),
            );
            b
        };

        ui.push_id(id_source, |ui| {
            for (index, block) in blocks.iter().enumerate() {
                match block {
                    TextMarkdownBlock::Heading { level, text } => {
                        let factor = match level {
                            TextMarkdownHeadingLevel::H1 => options.heading_scale + 0.26,
                            TextMarkdownHeadingLevel::H2 => options.heading_scale + 0.12,
                            TextMarkdownHeadingLevel::H3 => options.heading_scale,
                            TextMarkdownHeadingLevel::H4 => options.heading_scale - 0.08,
                            TextMarkdownHeadingLevel::H5 => options.heading_scale - 0.12,
                            TextMarkdownHeadingLevel::H6 => options.heading_scale - 0.16,
                        }
                        .max(1.0);
                        let heading_style = LabelOptions {
                            font_size: options.body.font_size * factor,
                            line_height: options.body.line_height * factor,
                            color: options.body.color,
                            wrap: true,
                            monospace: false,
                            weight: 700,
                            italic: false,
                            padding: egui::Vec2::ZERO,
                            fundamentals: options.body.fundamentals.clone(),
                            ..LabelOptions::default()
                        };
                        let _ = self.label(ui, ("md_h", index), text.as_str(), &heading_style);
                    }
                    TextMarkdownBlock::Paragraph(text) => {
                        let _ = self.label(ui, ("md_p", index), text.as_str(), &options.body);
                    }
                    TextMarkdownBlock::Code { language, text } => {
                        let mut code_options = options.code.clone();
                        code_options.language = language.clone();
                        let _ =
                            self.code_block(ui, ("md_code", index), text.as_str(), &code_options);
                    }
                }

                if index + 1 < blocks.len() {
                    ui.add_space(options.paragraph_spacing);
                }
            }
        });
    }

    fn highlight_code_spans_impl(
        &self,
        code: &str,
        language: Option<&str>,
        fallback_color: Color32,
    ) -> Vec<RichSpan> {
        if language
            .map(|lang| {
                let normalized = lang.trim();
                normalized.eq_ignore_ascii_case("text")
                    || normalized.eq_ignore_ascii_case("txt")
                    || normalized.eq_ignore_ascii_case("plain")
                    || normalized.eq_ignore_ascii_case("plaintext")
            })
            .unwrap_or(false)
        {
            return vec![RichSpan {
                text: code.to_owned(),
                style: SpanStyle {
                    color: fallback_color.into(),
                    monospace: true,
                    italic: false,
                    weight: 400,
                },
            }];
        }

        let syntax = language
            .and_then(|lang| self.syntax_set.find_syntax_by_token(lang))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, &self.code_theme);
        let mut spans = Vec::new();

        for line in LinesWithEndings::from(code) {
            match highlighter.highlight_line(line, &self.syntax_set) {
                Ok(ranges) => {
                    for (style, segment) in ranges {
                        spans.push(RichSpan {
                            text: segment.to_owned(),
                            style: SpanStyle {
                                color: Color32::from_rgba_premultiplied(
                                    style.foreground.r,
                                    style.foreground.g,
                                    style.foreground.b,
                                    style.foreground.a,
                                )
                                .into(),
                                monospace: true,
                                italic: style.font_style.contains(SyntectFontStyle::ITALIC),
                                weight: if style.font_style.contains(SyntectFontStyle::BOLD) {
                                    700
                                } else {
                                    400
                                },
                            },
                        });
                    }
                }
                Err(_) => {
                    spans.push(RichSpan {
                        text: line.to_owned(),
                        style: SpanStyle {
                            color: fallback_color.into(),
                            monospace: true,
                            italic: false,
                            weight: 400,
                        },
                    });
                }
            }
        }

        spans
    }

    fn paint_text_quads(&mut self, painter: &egui::Painter, bounds: Rect, quads: &[PaintTextQuad]) {
        if quads.is_empty() {
            return;
        }

        if let Some(callback) = self.build_text_wgpu_scene_callback(quads) {
            let callback_rect = bounds.intersect(painter.clip_rect());
            if callback_rect.is_positive() {
                painter.add(egui_wgpu::Callback::new_paint_callback(
                    callback_rect,
                    callback,
                ));
                return;
            }
        }

        paint_text_quads_fallback(&self.glyph_atlas, painter, quads);
    }

    fn invalidate_text_caches(&mut self, clear_input_states: bool) {
        let _ = self.prepared_texts.write(|state| state.clear());
        let _ = self.async_raster.cache.write(|state| state.clear());
        self.async_raster.pending.clear();
        self.glyph_atlas.clear();
        self.markdown_cache.clear();
        let _ = self.gpu_scene_cache.write(|state| state.clear());
        let _ = self.gpu_scene_page_batch_cache.write(|state| state.clear());
        let _ = self.gpu_scene_draw_batch_cache.write(|state| state.clear());
        let _ = self.gpu_scene_glyph_cache.write(|state| state.clear());
        if clear_input_states {
            self.input_states.clear();
        }
    }

    fn enforce_prepared_text_cache_budget(&mut self) {
        self.prepared_texts.write(|state| {
            let _ = state.evict_to_budget();
        });
    }

    fn enforce_gpu_scene_cache_budget(&mut self) {
        self.gpu_scene_cache.write(|state| {
            let _ = state.evict_to_budget();
        });
        self.gpu_scene_glyph_cache.write(|state| {
            let _ = state.evict_to_budget();
        });
    }

    fn hash_typography<H: Hasher>(&self, state: &mut H) {
        self.ui_font_family.hash(state);
        self.ui_font_size_scale.to_bits().hash(state);
        self.ui_font_weight.hash(state);
        self.open_type_features_enabled.hash(state);
        self.open_type_features_to_enable.hash(state);
        self.max_texture_side_px.hash(state);
    }

    fn effective_font_size(&self, size_points: f32) -> f32 {
        (size_points * self.ui_font_size_scale).max(1.0)
    }

    fn effective_line_height(&self, line_height_points: f32) -> f32 {
        (line_height_points * self.ui_font_size_scale).max(1.0)
    }

    fn effective_weight(&self, base_weight: u16) -> u16 {
        let delta = self.ui_font_weight - 400;
        (i32::from(base_weight) + delta).clamp(100, 900) as u16
    }

    fn build_text_attrs_owned(
        &self,
        style: &SpanStyle,
        font_size_points: f32,
        line_height_points: f32,
        fundamentals: &TextFundamentals,
    ) -> AttrsOwned {
        let mut attrs = Attrs::new()
            .color(to_cosmic_text_color(style.color))
            .weight(Weight(self.effective_weight(style.weight)))
            .metrics(Metrics::new(
                self.effective_font_size(font_size_points),
                self.effective_line_height(line_height_points),
            ));

        if style.monospace {
            attrs = attrs.family(Family::Monospace);
        } else if let Some(family) = self.ui_font_family.as_deref() {
            attrs = attrs.family(Family::Name(family));
        }

        if style.italic {
            attrs = attrs.style(FontStyle::Italic);
        }
        if let Some(features) = compose_font_features(&self.open_type_feature_tags, fundamentals) {
            attrs = attrs.font_features(features);
        }

        AttrsOwned::new(&attrs)
    }
}

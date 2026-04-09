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
mod async_raster;
mod atlas;
mod button_options;
mod clipboard;
mod code_block_options;
mod editor;
mod gpu;
mod input_options;
mod input_runtime;
mod label_options;
mod markdown_options;
mod markdown_parser;
mod path_layout;
mod text_helpers;
mod tooltip_options;

use crate::async_raster::{AsyncRasterState, AsyncRasterWorkerMessage, new_async_raster_state};
pub(crate) use crate::atlas::{
    GlyphAtlas, GlyphContentMode, GlyphRasterKey, PaintTextQuad, PreparedAtlasGlyph,
    adjusted_glyph_right_px, adjusted_glyph_x_px, blit_color_image,
    collect_glyph_spacing_prefixes_px, collect_prepared_glyphs_from_buffer,
    cursor_stops_for_glyphs, glyph_logical_font_size_points, hash_text_fundamentals,
    hit_buffer_with_fundamentals, rasterize_atlas_glyph, render_swash_outline_commands,
    shared_variation_settings,
};
use crate::editor::{
    EditorScrollMetrics, InputState, UndoEntry, UndoOpKind, clamp_borrowed_buffer_scroll,
    clamp_cursor_to_editor, clamp_selection_to_editor, click_editor_to_pointer,
    double_click_editor_to_pointer, drag_editor_selection_to_pointer, editor_horizontal_scroll,
    editor_to_string, extend_selection_to_pointer, handle_editor_key_event,
    handle_read_only_editor_key_event, is_navigation_event, measure_borrowed_buffer_scroll_metrics,
    measure_buffer_pixels, pending_modify_op, push_undo, scroll_editor_to_buffer_end, select_all,
    triple_click_editor_to_pointer, viewer_scrollbar_track_rects, viewer_visible_text_rect,
};
use crate::gpu::{
    CpuSceneAtlasPage, ResolvedTextGraphicsConfig, ResolvedTextRendererBackend, TextWgpuInstance,
    TextWgpuPreparedScene, TextWgpuSceneBatchSource, TextWgpuSceneCallback,
    allocate_cpu_scene_page_slot, color_image_to_page_data, default_gpu_scene_page_side,
    gpu_scene_approx_bytes, map_scene_quads_to_rect, paint_text_quads_fallback,
    quad_positions_from_min_size, rect_from_points, rotated_quad_positions, uv_quad_points,
};
use crate::input_runtime::apply_gamepad_scroll_if_focused;
use crate::path_layout::{
    build_path_layout_from_prepared_layout, export_prepared_layout_as_shapes,
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
    TextGraphicsApi, TextGraphicsConfig, TextHintingMode, TextInputEvent, TextKerning, TextKey,
    TextLabelOptions, TextMarkdownBlock, TextMarkdownHeadingLevel, TextModifiers,
    TextOpticalSizingMode, TextPath, TextPathError, TextPathGlyph, TextPathLayout, TextPathOptions,
    TextPoint, TextPointerButton, TextRasterizationConfig, TextRect, TextRenderScene,
    TextRendererBackend, TextRenderingPolicy, TextStemDarkeningMode, TextVariationSetting,
    TextVector, VectorGlyphShape, VectorPathCommand, VectorTextShape,
};
pub use clipboard::{apply_smart_quotes, sanitize_for_clipboard};
#[doc(hidden)]
pub use input_options::InputOptions as EguiInputOptions;

/// Default OpenType feature tags applied when no explicit feature string is
/// provided to [`TextUi::apply_open_type_features`].
pub const DEFAULT_OPEN_TYPE_FEATURE_TAGS: &str = "kern, liga, calt, onum, pnum";
const PREPARED_TEXT_CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;
const ASYNC_RASTER_CACHE_MAX_BYTES: usize = 24 * 1024 * 1024;
const GPU_SCENE_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
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
const WIDTH_BIN_PX: f32 = 4.0;

/// Snap a point-space width to the nearest WIDTH_BIN_PX device-pixel boundary.
#[inline]
fn snap_width_to_bin(width_points: f32, scale: f32) -> f32 {
    let w_px = (width_points * scale).round();
    let snapped_px = (w_px / WIDTH_BIN_PX).floor() * WIDTH_BIN_PX;
    (snapped_px / scale).max(1.0)
}

/// Snap a paint rect to the physical device-pixel grid so already-antialiased
/// glyph textures are not blurred again by fractional placement.
#[inline]
fn snap_rect_to_pixel_grid(rect: Rect, pixels_per_point: f32) -> Rect {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return rect;
    }

    let snap = |value: f32| (value * pixels_per_point).round() / pixels_per_point;

    Rect::from_min_max(
        Pos2::new(snap(rect.min.x), snap(rect.min.y)),
        Pos2::new(snap(rect.max.x), snap(rect.max.y)),
    )
}

#[inline]
fn texture_options_for_sampling(sampling: TextAtlasSampling) -> TextureOptions {
    match sampling {
        TextAtlasSampling::Linear => TextureOptions::LINEAR,
        TextAtlasSampling::Nearest => TextureOptions::NEAREST,
    }
}

#[inline]
fn glyph_content_mode_from_rasterization(mode: TextGlyphRasterMode) -> GlyphContentMode {
    match mode {
        TextGlyphRasterMode::Auto => GlyphContentMode::AlphaMask,
        TextGlyphRasterMode::AlphaMask => GlyphContentMode::AlphaMask,
        TextGlyphRasterMode::Sdf => GlyphContentMode::Sdf,
        TextGlyphRasterMode::Msdf => GlyphContentMode::Msdf,
    }
}

fn egui_key_from_text(key: TextKey) -> Key {
    match key {
        TextKey::A => Key::A,
        TextKey::B => Key::B,
        TextKey::Backspace => Key::Backspace,
        TextKey::Delete => Key::Delete,
        TextKey::Down => Key::ArrowDown,
        TextKey::E => Key::E,
        TextKey::End => Key::End,
        TextKey::Enter => Key::Enter,
        TextKey::Escape => Key::Escape,
        TextKey::F => Key::F,
        TextKey::H => Key::H,
        TextKey::Home => Key::Home,
        TextKey::K => Key::K,
        TextKey::Left => Key::ArrowLeft,
        TextKey::N => Key::N,
        TextKey::P => Key::P,
        TextKey::PageDown => Key::PageDown,
        TextKey::PageUp => Key::PageUp,
        TextKey::Right => Key::ArrowRight,
        TextKey::Tab => Key::Tab,
        TextKey::U => Key::U,
        TextKey::Up => Key::ArrowUp,
        TextKey::W => Key::W,
        TextKey::Y => Key::Y,
        TextKey::Z => Key::Z,
    }
}

fn egui_modifiers_from_text(modifiers: TextModifiers) -> egui::Modifiers {
    egui::Modifiers {
        alt: modifiers.alt,
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        mac_cmd: modifiers.mac_cmd,
        command: modifiers.command,
    }
}

fn core_label_options(options: &TextLabelOptions) -> LabelOptions {
    LabelOptions {
        font_size: options.font_size,
        line_height: options.line_height,
        color: options.color.into(),
        wrap: options.wrap,
        monospace: options.monospace,
        weight: options.weight,
        italic: options.italic,
        padding: options.padding.into(),
        fundamentals: options.fundamentals.clone(),
        ellipsis: options.ellipsis.clone(),
    }
}

#[inline]
fn wgpu_filter_mode_for_sampling(sampling: TextAtlasSampling) -> wgpu::FilterMode {
    match sampling {
        TextAtlasSampling::Linear => wgpu::FilterMode::Linear,
        TextAtlasSampling::Nearest => wgpu::FilterMode::Nearest,
    }
}

pub fn wgpu_backends_for_text_graphics_api(api: TextGraphicsApi) -> wgpu::Backends {
    match api {
        TextGraphicsApi::Auto => wgpu::Backends::PRIMARY,
        TextGraphicsApi::Vulkan => wgpu::Backends::VULKAN,
        TextGraphicsApi::Metal => wgpu::Backends::METAL,
        TextGraphicsApi::Dx12 => wgpu::Backends::DX12,
        TextGraphicsApi::Gl => wgpu::Backends::GL,
    }
}

pub fn wgpu_power_preference_for_text_gpu_preference(
    preference: TextGpuPowerPreference,
) -> wgpu::PowerPreference {
    match preference {
        TextGpuPowerPreference::Auto => wgpu::PowerPreference::default(),
        TextGpuPowerPreference::LowPower => wgpu::PowerPreference::LowPower,
        TextGpuPowerPreference::HighPerformance => wgpu::PowerPreference::HighPerformance,
    }
}

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

#[inline]
fn should_hint(display_scale: f32) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = display_scale;
        false
    }

    #[cfg(target_os = "windows")]
    {
        display_scale < 1.5
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        display_scale < 1.5
    }
}

#[inline]
fn resolved_hinting_enabled(display_scale: f32, rasterization: TextRasterizationConfig) -> bool {
    match rasterization.hinting {
        TextHintingMode::Enabled => true,
        TextHintingMode::Disabled => false,
        TextHintingMode::Auto => should_hint(display_scale),
    }
}

#[inline]
fn resolved_stem_darkening_strength(
    ppem: f32,
    style_enabled: bool,
    rasterization: TextRasterizationConfig,
) -> f32 {
    let enabled = match rasterization.stem_darkening {
        TextStemDarkeningMode::Enabled => true,
        TextStemDarkeningMode::Disabled => false,
        TextStemDarkeningMode::Auto => style_enabled,
    };
    if !enabled {
        return 0.0;
    }

    let min_ppem = rasterization.stem_darkening_min_ppem.max(0.0);
    let max_ppem = rasterization.stem_darkening_max_ppem.max(min_ppem);
    let max_strength = rasterization.stem_darkening_max_strength.max(0.0);
    if ppem >= max_ppem {
        0.0
    } else if ppem <= min_ppem {
        max_strength
    } else {
        max_strength * (1.0 - (ppem - min_ppem) / (max_ppem - min_ppem))
    }
}

#[inline]
fn opsz_for_font_size(font_size_pt: f32) -> f32 {
    font_size_pt.clamp(8.0, 144.0)
}

fn font_family_available(db: &fontdb::Database, family: &str) -> bool {
    db.faces().any(|face| {
        face.families
            .iter()
            .any(|family_name| family_name.0.eq_ignore_ascii_case(family))
    })
}

fn choose_available_family<'a>(db: &fontdb::Database, families: &'a [&'a str]) -> Option<&'a str> {
    families
        .iter()
        .copied()
        .find(|family| font_family_available(db, family))
}

fn configure_text_font_defaults(font_system: &mut FontSystem) {
    let db = font_system.db_mut();

    #[cfg(target_os = "macos")]
    {
        if let Some(family) =
            choose_available_family(db, &["SF Pro Text", ".SF NS", "Helvetica Neue"])
        {
            db.set_sans_serif_family(family);
        }
        if let Some(family) = choose_available_family(db, &["SF Mono", "Menlo", "Monaco"]) {
            db.set_monospace_family(family);
        }
        if let Some(family) = choose_available_family(db, &["Times New Roman", "Times"]) {
            db.set_serif_family(family);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(family) =
            choose_available_family(db, &["Segoe UI Variable", "Segoe UI", "Arial"])
        {
            db.set_sans_serif_family(family);
        }
        if let Some(family) =
            choose_available_family(db, &["Cascadia Mono", "Consolas", "Courier New"])
        {
            db.set_monospace_family(family);
        }
        if let Some(family) = choose_available_family(db, &["Times New Roman", "Georgia"]) {
            db.set_serif_family(family);
        }
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        if let Some(family) = choose_available_family(
            db,
            &["Inter", "Noto Sans", "Cantarell", "Ubuntu", "DejaVu Sans"],
        ) {
            db.set_sans_serif_family(family);
        }
        if let Some(family) = choose_available_family(
            db,
            &["Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono"],
        ) {
            db.set_monospace_family(family);
        }
        if let Some(family) =
            choose_available_family(db, &["Noto Serif", "DejaVu Serif", "Liberation Serif"])
        {
            db.set_serif_family(family);
        }
    }
}

type SpanStyle = RichTextStyle;
type RichSpan = RichTextSpan;

#[derive(Clone, Debug)]
struct PreparedTextLayout {
    glyphs: Arc<[PreparedGlyph]>,
    size_points: Vec2,
    approx_bytes: usize,
}

#[derive(Clone, Debug)]
struct PreparedGlyph {
    cache_key: GlyphRasterKey,
    offset_points: Vec2,
    color: Color32,
}

struct PreparedTextCacheEntry {
    fingerprint: u64,
    layout: Arc<PreparedTextLayout>,
    last_used_frame: u64,
}

pub struct TextUi {
    font_system: FontSystem,
    scale_context: ScaleContext,
    syntax_set: SyntaxSet,
    code_theme: Theme,
    prepared_texts: ThreadSafeLru<Id, PreparedTextCacheEntry>,
    glyph_atlas: GlyphAtlas,
    input_states: FxHashMap<Id, InputState>,
    ui_font_family: Option<String>,
    ui_font_size_scale: f32,
    ui_font_weight: i32,
    open_type_features_enabled: bool,
    open_type_features_to_enable: String,
    open_type_feature_tags: Vec<[u8; 4]>,
    open_type_features: Option<FontFeatures>,
    async_raster: AsyncRasterState,
    graphics_config: TextGraphicsConfig,
    current_frame: u64,
    max_texture_side_px: usize,
    frame_events: Vec<TextInputEvent>,
    /// Cache for parsed markdown blocks: Id → (fingerprint, last_used_frame, blocks).
    /// Prevents re-parsing unchanged markdown every frame.
    markdown_cache: FxHashMap<Id, (u64, u64, Arc<[TextMarkdownBlock]>)>,
    /// Cache for built GPU scenes: fingerprint → scene.
    /// Avoids re-rasterizing glyphs via Swash every frame for unchanged text.
    gpu_scene_cache: ThreadSafeLru<u64, Arc<TextGpuScene>>,
    /// Cache for CPU-side glyph bitmaps used while assembling retained GPU scenes.
    /// This keeps repeated scene rebuilds from paying Swash raster cost for glyphs we just saw.
    gpu_scene_glyph_cache: ThreadSafeLru<GlyphRasterKey, Arc<PreparedAtlasGlyph>>,
}

impl Default for TextUi {
    fn default() -> Self {
        Self::new()
    }
}

impl TextUi {
    /// Creates a new text renderer and background async raster worker.
    pub fn new() -> Self {
        Self::new_with_graphics_config(TextGraphicsConfig::default())
    }

    /// Creates a new text renderer with an explicit graphics configuration.
    pub fn new_with_graphics_config(graphics_config: TextGraphicsConfig) -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let code_theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .or_else(|| theme_set.themes.values().next())
            .cloned()
            .unwrap_or_else(|| {
                warn!(
                    target: "vertexlauncher/textui",
                    "syntect theme set was unexpectedly empty; using default code theme"
                );
                Theme::default()
            });

        let glyph_atlas = GlyphAtlas::new();
        let mut font_system = FontSystem::new();
        configure_text_font_defaults(&mut font_system);
        Self {
            font_system,
            scale_context: ScaleContext::new(),
            syntax_set,
            code_theme,
            prepared_texts: ThreadSafeLru::new(PREPARED_TEXT_CACHE_MAX_BYTES),
            glyph_atlas,
            input_states: FxHashMap::default(),
            ui_font_family: None,
            ui_font_size_scale: 1.0,
            ui_font_weight: 400,
            open_type_features_enabled: false,
            open_type_features_to_enable: String::new(),
            open_type_feature_tags: Vec::new(),
            open_type_features: None,
            async_raster: new_async_raster_state(),
            graphics_config,
            current_frame: 0,
            max_texture_side_px: usize::MAX,
            frame_events: Vec::new(),
            markdown_cache: FxHashMap::default(),
            gpu_scene_cache: ThreadSafeLru::new(GPU_SCENE_CACHE_MAX_BYTES),
            gpu_scene_glyph_cache: ThreadSafeLru::new(GPU_SCENE_GLYPH_CACHE_MAX_BYTES),
        }
    }

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

    /// Renders a plain label synchronously.
    #[allow(dead_code)]
    fn label(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        self.label_impl(ui, id_source, text, options, Sense::hover(), false)
    }

    /// Renders a clickable label synchronously.
    #[allow(dead_code)]
    fn clickable_label(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        self.label_impl(ui, id_source, text, options, Sense::click(), false)
    }

    /// Measures rendered size of text for the provided style options.
    #[allow(dead_code)]
    fn measure_text_size(&mut self, ui: &Ui, text: &str, options: &LabelOptions) -> Vec2 {
        self.measure_text_size_at_scale(
            ui.ctx().pixels_per_point(),
            text,
            &TextLabelOptions {
                font_size: options.font_size,
                line_height: options.line_height,
                color: options.color.into(),
                wrap: options.wrap,
                monospace: options.monospace,
                weight: options.weight,
                italic: options.italic,
                padding: options.padding.into(),
                fundamentals: options.fundamentals.clone(),
                ellipsis: options.ellipsis.clone(),
            },
        )
        .into()
    }

    /// Measures rendered size of text at the provided output scale.
    pub fn measure_text_size_at_scale(
        &mut self,
        scale: f32,
        text: &str,
        options: &TextLabelOptions,
    ) -> TextVector {
        let options = core_label_options(options);
        let metrics = Metrics::new(
            (self.effective_font_size(options.font_size) * scale).max(1.0),
            (self.effective_line_height(options.line_height) * scale).max(1.0),
        );
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs_owned = self.build_text_attrs_owned(
            &SpanStyle {
                color: options.color.into(),
                monospace: options.monospace,
                italic: options.italic,
                weight: options.weight,
            },
            options.font_size,
            options.line_height,
            &options.fundamentals,
        );

        {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_wrap(if options.wrap {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            let attrs = attrs_owned.as_attrs();
            borrowed.set_text(text, &attrs, Shaping::Advanced, None);
            borrowed.shape_until_scroll(true);
        }

        let (width_px, height_px) = measure_buffer_pixels(&buffer);
        TextVector::new(width_px as f32 / scale, height_px as f32 / scale)
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

    fn get_or_prepare_label_layout(
        &mut self,
        cache_id: Id,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Arc<PreparedTextLayout> {
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));

        let mut hasher = new_fingerprint_hasher();
        "prepare_label".hash(&mut hasher);
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
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        self.get_cached_prepared_layout(cache_id, fingerprint)
            .unwrap_or_else(|| {
                let layout =
                    Arc::new(self.prepare_plain_text_layout(text, options, binned_width, scale));
                self.cache_prepared_layout(cache_id, fingerprint, Arc::clone(&layout));
                layout
            })
    }

    fn get_or_prepare_rich_layout(
        &mut self,
        cache_id: Id,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Arc<PreparedTextLayout> {
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));

        let mut hasher = new_fingerprint_hasher();
        "prepare_rich".hash(&mut hasher);
        for span in spans {
            span.text.hash(&mut hasher);
            span.style.color.hash(&mut hasher);
            span.style.monospace.hash(&mut hasher);
            span.style.italic.hash(&mut hasher);
            span.style.weight.hash(&mut hasher);
        }
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        self.get_cached_prepared_layout(cache_id, fingerprint)
            .unwrap_or_else(|| {
                let layout =
                    Arc::new(self.prepare_rich_text_layout(spans, options, binned_width, scale));
                self.cache_prepared_layout(cache_id, fingerprint, Arc::clone(&layout));
                layout
            })
    }

    fn prepare_label_scene(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextRenderScene {
        let scale = ctx.pixels_per_point();
        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_prepare_label_scene"),
            text,
            options,
            width_points_opt,
            scale,
        );
        self.build_text_scene_from_layout(ctx, &layout, scale)
    }

    #[allow(dead_code)]
    fn prepare_rich_text_scene(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextRenderScene {
        let scale = ctx.pixels_per_point();
        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_prepare_rich_scene"),
            spans,
            options,
            width_points_opt,
            scale,
        );
        self.build_text_scene_from_layout(ctx, &layout, scale)
    }

    pub fn prepare_label_gpu_scene_at_scale(
        &mut self,
        id_source: impl Hash,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Arc<TextGpuScene> {
        let options = core_label_options(options);
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));
        let mut hasher = new_fingerprint_hasher();
        "label_gpu_scene".hash(&mut hasher);
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
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(scene) = self
            .gpu_scene_cache
            .write(|state| state.touch(&fingerprint).map(|e| Arc::clone(&e.value)))
        {
            return scene;
        }

        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_prepare_label_gpu_scene"),
            text,
            &options,
            width_points_opt,
            scale,
        );
        let mut scene = Arc::new(self.build_text_gpu_scene_from_layout(&layout, scale));
        if let Some(s) = Arc::get_mut(&mut scene) {
            s.fingerprint = fingerprint;
        }
        let approx_bytes = gpu_scene_approx_bytes(&scene);
        self.gpu_scene_cache.write(|state| {
            let _ = state.insert(fingerprint, Arc::clone(&scene), approx_bytes);
        });
        scene
    }

    pub fn prepare_rich_text_gpu_scene_at_scale(
        &mut self,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Arc<TextGpuScene> {
        let options = core_label_options(options);
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));
        let mut hasher = new_fingerprint_hasher();
        "rich_gpu_scene".hash(&mut hasher);
        for span in spans {
            span.text.hash(&mut hasher);
            span.style.color.hash(&mut hasher);
            span.style.monospace.hash(&mut hasher);
            span.style.italic.hash(&mut hasher);
            span.style.weight.hash(&mut hasher);
        }
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(scene) = self
            .gpu_scene_cache
            .write(|state| state.touch(&fingerprint).map(|e| Arc::clone(&e.value)))
        {
            return scene;
        }

        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_prepare_rich_gpu_scene"),
            spans,
            &options,
            width_points_opt,
            scale,
        );
        let mut scene = Arc::new(self.build_text_gpu_scene_from_layout(&layout, scale));
        if let Some(s) = Arc::get_mut(&mut scene) {
            s.fingerprint = fingerprint;
        }
        let approx_bytes = gpu_scene_approx_bytes(&scene);
        self.gpu_scene_cache.write(|state| {
            let _ = state.insert(fingerprint, Arc::clone(&scene), approx_bytes);
        });
        scene
    }

    pub fn prepare_label_gpu_scene_async_at_scale(
        &mut self,
        _id_source: impl Hash,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<Arc<TextGpuScene>> {
        let options = core_label_options(options);
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));
        let mut hasher = new_fingerprint_hasher();
        "label_async_gpu_scene".hash(&mut hasher);
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
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(scene) = self
            .gpu_scene_cache
            .write(|state| state.touch(&fingerprint).map(|e| Arc::clone(&e.value)))
        {
            return Some(scene);
        }

        let layout_opt = self.get_or_queue_async_plain_layout(
            fingerprint,
            text.to_owned(),
            &options,
            binned_width,
            scale,
        );
        layout_opt.map(|layout| {
            let scene = Arc::new(self.build_text_gpu_scene_from_layout(&layout, scale));
            let approx_bytes = gpu_scene_approx_bytes(&scene);
            self.gpu_scene_cache.write(|state| {
                let _ = state.insert(fingerprint, Arc::clone(&scene), approx_bytes);
            });
            scene
        })
    }

    pub fn prepare_rich_text_gpu_scene_async_at_scale(
        &mut self,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<Arc<TextGpuScene>> {
        let options = core_label_options(options);
        let _cache_id = Id::new(id_source).with("textui_prepare_rich_gpu_scene_async");
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));
        let mut hasher = new_fingerprint_hasher();
        "prepare_rich_gpu_async".hash(&mut hasher);
        for span in spans {
            span.text.hash(&mut hasher);
            span.style.color.hash(&mut hasher);
            span.style.monospace.hash(&mut hasher);
            span.style.italic.hash(&mut hasher);
            span.style.weight.hash(&mut hasher);
        }
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(scene) = self
            .gpu_scene_cache
            .write(|state| state.touch(&fingerprint).map(|e| Arc::clone(&e.value)))
        {
            return Some(scene);
        }

        let layout_opt = self.get_or_queue_async_rich_layout(
            fingerprint,
            spans.to_vec(),
            &options,
            binned_width,
            scale,
        );
        layout_opt.map(|layout| {
            let scene = Arc::new(self.build_text_gpu_scene_from_layout(&layout, scale));
            let approx_bytes = gpu_scene_approx_bytes(&scene);
            self.gpu_scene_cache.write(|state| {
                let _ = state.insert(fingerprint, Arc::clone(&scene), approx_bytes);
            });
            scene
        })
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

    pub fn prepare_label_path_layout(
        &mut self,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let options = core_label_options(options);
        let layout = self.prepare_plain_text_layout(text, &options, width_points_opt, 1.0);
        build_path_layout_from_prepared_layout(
            &layout,
            options.font_size,
            options.line_height,
            path,
            path_options,
        )
    }

    pub fn prepare_rich_text_path_layout(
        &mut self,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let options = core_label_options(options);
        let layout = self.prepare_rich_text_layout(spans, &options, width_points_opt, 1.0);
        build_path_layout_from_prepared_layout(
            &layout,
            options.font_size,
            options.line_height,
            path,
            path_options,
        )
    }

    pub fn export_label_as_shapes(
        &mut self,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
    ) -> VectorTextShape {
        let options = core_label_options(options);
        let layout = self.prepare_plain_text_layout(text, &options, width_points_opt, 1.0);
        export_prepared_layout_as_shapes(
            &layout,
            &mut self.font_system,
            &mut self.scale_context,
            options.line_height,
            self.graphics_config.rasterization,
        )
    }

    pub fn export_rich_text_as_shapes(
        &mut self,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
    ) -> VectorTextShape {
        let options = core_label_options(options);
        let layout = self.prepare_rich_text_layout(spans, &options, width_points_opt, 1.0);
        export_prepared_layout_as_shapes(
            &layout,
            &mut self.font_system,
            &mut self.scale_context,
            options.line_height,
            self.graphics_config.rasterization,
        )
    }

    #[allow(dead_code)]
    fn paint_label_on_path(
        &mut self,
        painter: &egui::Painter,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let scale = painter.pixels_per_point();
        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_path_label"),
            text,
            options,
            width_points_opt,
            scale,
        );
        self.paint_prepared_layout_on_path(
            painter,
            &layout,
            options.font_size,
            options.line_height,
            path,
            path_options,
        )
    }

    #[allow(dead_code)]
    fn paint_rich_text_on_path(
        &mut self,
        painter: &egui::Painter,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let scale = painter.pixels_per_point();
        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_path_rich"),
            spans,
            options,
            width_points_opt,
            scale,
        );
        self.paint_prepared_layout_on_path(
            painter,
            &layout,
            options.font_size,
            options.line_height,
            path,
            path_options,
        )
    }

    #[allow(dead_code)]
    fn paint_prepared_layout_on_path(
        &mut self,
        painter: &egui::Painter,
        layout: &PreparedTextLayout,
        fallback_advance_points: f32,
        line_height_points: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let path_layout = build_path_layout_from_prepared_layout(
            layout,
            fallback_advance_points,
            line_height_points,
            path,
            path_options,
        )?;
        let scale = painter.pixels_per_point();
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        let field_range_px = graphics_config.rasterization.field_range_px.max(1.0);
        let mut quads = Vec::with_capacity(layout.glyphs.len());

        for (glyph, path_glyph) in layout.glyphs.iter().zip(path_layout.glyphs.iter()) {
            let content_mode = self.resolved_glyph_content_mode(graphics_config, &glyph.cache_key);
            let raster_key = glyph
                .cache_key
                .for_content_mode(content_mode, field_range_px);
            let Some(atlas_entry) = self.glyph_atlas.resolve_or_queue(
                painter.ctx(),
                &mut self.font_system,
                &mut self.scale_context,
                raster_key,
                self.current_frame,
            ) else {
                continue;
            };

            let size_points = egui::vec2(
                atlas_entry.size_px[0] as f32 / scale,
                atlas_entry.size_px[1] as f32 / scale,
            );
            let origin_offset = egui::vec2(
                atlas_entry.placement_left_px as f32 / scale,
                -(atlas_entry.placement_top_px as f32) / scale,
            );
            let tint = if atlas_entry.is_color {
                Color32::WHITE
            } else {
                glyph.color
            };

            quads.push(PaintTextQuad {
                page_index: atlas_entry.page_index,
                positions: rotated_quad_positions(
                    egui_point_from_text(path_glyph.anchor),
                    origin_offset,
                    size_points,
                    path_glyph.rotation_radians,
                ),
                uvs: uv_quad_points(atlas_entry.uv),
                tint,
                content_mode: atlas_entry.content_mode,
                field_range_px: atlas_entry.field_range_px,
            });
        }

        self.paint_text_quads(painter, egui_rect_from_text(path_layout.bounds), &quads);

        Ok(path_layout)
    }

    #[allow(dead_code)]
    fn prepare_label_path_scene(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextRenderScene, TextPathError> {
        let scale = ctx.pixels_per_point();
        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_prepare_path_label_scene"),
            text,
            options,
            width_points_opt,
            scale,
        );
        self.build_text_scene_on_path(
            ctx,
            &layout,
            options.font_size,
            options.line_height,
            scale,
            path,
            path_options,
        )
    }

    pub fn prepare_label_path_gpu_scene_at_scale(
        &mut self,
        id_source: impl Hash,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextGpuScene, TextPathError> {
        let options = core_label_options(options);
        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_prepare_path_label_gpu_scene"),
            text,
            &options,
            width_points_opt,
            scale,
        );
        self.build_text_gpu_scene_on_path(
            &layout,
            options.font_size,
            options.line_height,
            scale,
            path,
            path_options,
        )
    }

    pub fn prepare_rich_text_path_gpu_scene_at_scale(
        &mut self,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextGpuScene, TextPathError> {
        let options = core_label_options(options);
        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_prepare_path_rich_gpu_scene"),
            spans,
            &options,
            width_points_opt,
            scale,
        );
        self.build_text_gpu_scene_on_path(
            &layout,
            options.font_size,
            options.line_height,
            scale,
            path,
            path_options,
        )
    }

    pub fn prepare_rich_text_path_scene(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextRenderScene, TextPathError> {
        let scale = ctx.pixels_per_point();
        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_prepare_path_rich_scene"),
            spans,
            options,
            width_points_opt,
            scale,
        );
        self.build_text_scene_on_path(
            ctx,
            &layout,
            options.font_size,
            options.line_height,
            scale,
            path,
            path_options,
        )
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

    /// Renders a single-line editable text field.
    #[doc(hidden)]
    pub fn egui_singleline_input(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        self.input_widget(ui, id_source, text, options, false)
    }

    /// Renders a multi-line editable text field.
    #[doc(hidden)]
    pub fn egui_multiline_input(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        self.input_widget(ui, id_source, text, options, true)
    }

    /// Renders a read-only, selectable multi-line rich-text viewer.
    ///
    /// This keeps the same font pipeline as the rest of `TextUi`, supports drag selection and
    /// copy/select-all shortcuts, and rasterizes the visible viewport into texture tiles so large
    /// views do not depend on a single oversized GPU texture.
    #[doc(hidden)]
    pub fn egui_multiline_rich_viewer(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &InputOptions,
        stick_to_bottom: bool,
        wrap: bool,
    ) -> Response {
        let id = ui.make_persistent_id(id_source).with("textui_rich_viewer");
        let width = options
            .desired_width
            .unwrap_or_else(|| ui.available_width())
            .max(options.min_width);
        let min_height = options.line_height + (options.padding.y * 2.0);
        let height = (options.line_height * options.desired_rows.max(1) as f32
            + options.padding.y * 2.0)
            .max(min_height);

        let desired_size = egui::vec2(width, height);
        let rect = ui.allocate_space(desired_size).1;
        let mut response = ui.interact(rect, id, Sense::click_and_drag());

        let has_focus = response.has_focus();
        if has_focus {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        tab: true,
                        escape: false,
                    },
                );
            });
        }
        let scale = ui.ctx().pixels_per_point();
        let content_rect = rect.shrink2(options.padding);
        let content_width_px = (content_rect.width() * scale).max(1.0);
        let content_height_px = (content_rect.height() * scale).max(1.0);
        let text = spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>();
        let attrs_fingerprint = self.rich_viewer_attrs_fingerprint(spans, options, scale, wrap);

        let mut state = self
            .input_states
            .remove(&id)
            .unwrap_or_else(|| Self::new_input_state(&mut self.font_system, &text, true));

        let needs_text_sync =
            state.last_text != text || state.attrs_fingerprint != attrs_fingerprint;
        if needs_text_sync {
            state.scroll_metrics = self.replace_editor_rich_text(
                &mut state.editor,
                spans,
                options,
                content_width_px,
                content_height_px,
                scale,
                wrap,
            );
            state.last_text = text.clone();
            state.attrs_fingerprint = attrs_fingerprint;
            state.preferred_cursor_x_px = None;
            if stick_to_bottom && !has_focus && !response.hovered() {
                scroll_editor_to_buffer_end(&mut self.font_system, &mut state.editor);
            }
        } else {
            state.scroll_metrics = self.configure_viewer(
                &mut state.editor,
                options,
                content_width_px,
                content_height_px,
                scale,
                wrap,
            );
        }

        let pointer_pos = response.interact_pointer_pos();
        let scrollbar_tracks = viewer_scrollbar_track_rects(
            ui.style().spacing.scroll,
            response.hovered(),
            response.is_pointer_button_down_on(),
            content_rect,
            state.scroll_metrics,
        );
        let pointer_over_scrollbar = pointer_pos.is_some_and(|pos| scrollbar_tracks.contains(pos));
        let pointer_over_text = pointer_pos.is_some_and(|pos| {
            viewer_visible_text_rect(content_rect, state.scroll_metrics)
                .is_some_and(|text_rect| text_rect.contains(pos))
        }) && !pointer_over_scrollbar;
        let pointer_pressed_on_widget =
            ui.ctx().input(|i| i.pointer.primary_pressed()) && response.is_pointer_button_down_on();

        if (response.clicked() || pointer_pressed_on_widget) && !pointer_over_scrollbar {
            response.request_focus();
        }

        if pointer_over_text {
            ui.output_mut(|o| {
                o.cursor_icon = egui::CursorIcon::Text;
                o.mutable_text_under_cursor = true;
            });
        }

        let pointer_interacted = !pointer_over_scrollbar
            && (pointer_pressed_on_widget
                || response.clicked()
                || response.double_clicked()
                || response.triple_clicked()
                || response.drag_started()
                || response.dragged());

        let mut state_changed = if has_focus || response.hovered() || pointer_interacted {
            self.handle_viewer_events(
                ui,
                &response,
                &mut state.editor,
                content_rect,
                scale,
                &mut state.preferred_cursor_x_px,
                &options.fundamentals,
                has_focus,
                pointer_over_scrollbar,
                &mut state.scroll_metrics,
            )
        } else {
            false
        };

        let frame_fill = if has_focus {
            options
                .background_color_focused
                .or(options.background_color_hovered)
                .unwrap_or(options.background_color)
        } else if response.hovered() {
            options
                .background_color_hovered
                .unwrap_or(options.background_color)
        } else {
            options.background_color
        };
        let frame_stroke = if has_focus {
            options
                .stroke_focused
                .or(options.stroke_hovered)
                .unwrap_or(options.stroke)
        } else if response.hovered() {
            options.stroke_hovered.unwrap_or(options.stroke)
        } else {
            options.stroke
        };
        let corner_radius = CornerRadius::same(options.corner_radius);

        ui.painter().rect_filled(rect, corner_radius, frame_fill);
        ui.painter()
            .rect_stroke(rect, corner_radius, frame_stroke, egui::StrokeKind::Inside);

        // --- GPU mesh rendering: atlas glyphs + Shape::Rect for selection ---
        {
            let painter = ui.painter().with_clip_rect(ui.clip_rect());
            self.paint_editor_gpu(
                &painter,
                content_rect,
                &state.editor,
                options,
                scale,
                false,
                true,
            );
        }

        state_changed |= self.sync_viewer_scrollbars(
            ui,
            id,
            &mut state.editor,
            content_rect,
            scale,
            &options.fundamentals,
            &mut state.scroll_metrics,
        );

        self.input_states.insert(id, state);
        if state_changed {
            response.mark_changed();
        }
        apply_gamepad_scroll_if_focused(ui, &response);

        response
    }

    fn input_widget(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &mut String,
        options: &InputOptions,
        multiline: bool,
    ) -> Response {
        let id = ui.make_persistent_id(id_source).with("textui_input");
        let width = options
            .desired_width
            .unwrap_or_else(|| ui.available_width())
            .max(options.min_width);

        let min_height = options.line_height + (options.padding.y * 2.0);
        let height = if multiline {
            (options.line_height * options.desired_rows.max(2) as f32 + options.padding.y * 2.0)
                .max(min_height)
        } else {
            min_height
        };

        let desired_size = egui::vec2(width, height);
        let rect = ui.allocate_space(desired_size).1;
        let mut response = ui.interact(rect, id, Sense::click_and_drag());

        if response.hovered() {
            ui.output_mut(|o| {
                o.cursor_icon = egui::CursorIcon::Text;
                o.mutable_text_under_cursor = true;
            });
        }

        if response.clicked() {
            response.request_focus();
        }

        let has_focus = response.has_focus();
        if has_focus {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        tab: multiline,
                        escape: false,
                    },
                );
            });
        }
        let scale = ui.ctx().pixels_per_point();
        let content_rect = rect.shrink2(options.padding);
        let content_width_px = (content_rect.width() * scale).max(1.0);
        let content_height_px = (content_rect.height() * scale).max(1.0);
        let attrs_fingerprint = self.input_attrs_fingerprint(options, scale);

        let mut state = self
            .input_states
            .remove(&id)
            .unwrap_or_else(|| Self::new_input_state(&mut self.font_system, text, multiline));

        if state.multiline != multiline {
            state = Self::new_input_state(&mut self.font_system, text, multiline);
        }

        let needs_text_sync = !has_focus && state.last_text != *text;
        let needs_attrs_sync = state.attrs_fingerprint != attrs_fingerprint;
        if needs_text_sync || needs_attrs_sync {
            state.scroll_metrics = self.replace_editor_text(
                &mut state.editor,
                text,
                options,
                multiline,
                content_width_px,
                content_height_px,
                scale,
            );
            state.last_text.clone_from(text);
            state.attrs_fingerprint = attrs_fingerprint;
            state.preferred_cursor_x_px = None;
        }

        state.scroll_metrics = self.configure_editor(
            &mut state.editor,
            options,
            multiline,
            content_width_px,
            content_height_px,
            scale,
        );

        let pointer_interacted = response.clicked()
            || response.double_clicked()
            || response.triple_clicked()
            || response.dragged();

        let mut changed = false;
        if has_focus || pointer_interacted {
            // --- undo / redo (Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z) ---
            let (undo_pressed, redo_pressed) = if has_focus {
                ui.input(|i| {
                    let undo = i.key_pressed(Key::Z) && i.modifiers.command && !i.modifiers.shift;
                    let redo = (i.key_pressed(Key::Y) && i.modifiers.command)
                        || (i.key_pressed(Key::Z) && i.modifiers.command && i.modifiers.shift);
                    (undo, redo)
                })
            } else {
                (false, false)
            };

            if undo_pressed {
                if let Some(UndoEntry {
                    text: undo_text,
                    cursor: undo_cursor,
                    selection: undo_sel,
                }) = state.undo_stack.pop()
                {
                    let snap = UndoEntry {
                        text: editor_to_string(&state.editor),
                        cursor: state.editor.cursor(),
                        selection: state.editor.selection(),
                    };
                    state.redo_stack.push(snap);
                    state.scroll_metrics = self.replace_editor_text(
                        &mut state.editor,
                        &undo_text,
                        options,
                        multiline,
                        content_width_px,
                        content_height_px,
                        scale,
                    );
                    state
                        .editor
                        .set_cursor(clamp_cursor_to_editor(&state.editor, undo_cursor));
                    state
                        .editor
                        .set_selection(clamp_selection_to_editor(&state.editor, undo_sel));
                    state.last_text = undo_text;
                    state.last_undo_op = UndoOpKind::None;
                    state.preferred_cursor_x_px = None;
                    changed = true;
                }
            } else if redo_pressed {
                if let Some(UndoEntry {
                    text: redo_text,
                    cursor: redo_cursor,
                    selection: redo_sel,
                }) = state.redo_stack.pop()
                {
                    let snap = UndoEntry {
                        text: editor_to_string(&state.editor),
                        cursor: state.editor.cursor(),
                        selection: state.editor.selection(),
                    };
                    push_undo(&mut state.undo_stack, snap);
                    state.scroll_metrics = self.replace_editor_text(
                        &mut state.editor,
                        &redo_text,
                        options,
                        multiline,
                        content_width_px,
                        content_height_px,
                        scale,
                    );
                    state
                        .editor
                        .set_cursor(clamp_cursor_to_editor(&state.editor, redo_cursor));
                    state
                        .editor
                        .set_selection(clamp_selection_to_editor(&state.editor, redo_sel));
                    state.last_text = redo_text;
                    state.last_undo_op = UndoOpKind::None;
                    state.preferred_cursor_x_px = None;
                    changed = true;
                }
            } else {
                // --- snapshot for upcoming modification (undo grouping) ---
                if has_focus {
                    let pending_op = pending_modify_op(&self.frame_events);
                    if pending_op != UndoOpKind::None {
                        // Push a new snapshot when the operation type changes or for
                        // atomic ops (Paste/Cut always get their own undo entry).
                        let should_push = matches!(pending_op, UndoOpKind::Paste | UndoOpKind::Cut)
                            || state.last_undo_op != pending_op;
                        if should_push {
                            push_undo(
                                &mut state.undo_stack,
                                UndoEntry {
                                    text: editor_to_string(&state.editor),
                                    cursor: state.editor.cursor(),
                                    selection: state.editor.selection(),
                                },
                            );
                            state.redo_stack.clear();
                        }
                        state.last_undo_op = pending_op;
                    } else if self.frame_events.iter().any(is_navigation_event) {
                        // Navigation breaks the current insert/delete run so the
                        // next edit starts a fresh undo group.
                        state.last_undo_op = UndoOpKind::None;
                    }
                }

                changed |= self.handle_input_events(
                    ui,
                    &response,
                    &mut state.editor,
                    multiline,
                    content_rect,
                    scale,
                    &mut state.preferred_cursor_x_px,
                    &options.fundamentals,
                    has_focus,
                    &mut state.scroll_metrics,
                );
            }

            if !multiline && ui.input(|i| i.key_pressed(Key::Enter)) {
                response.surrender_focus();
            }
        }

        // --- context menu (right-click) ---
        let mut ctx_cut = false;
        let mut ctx_copy = false;
        let mut ctx_paste = false;
        let mut ctx_select_all = false;
        response.context_menu(|menu| {
            let has_selection = state.editor.selection() != Selection::None;
            if menu
                .add_enabled(has_selection, egui::Button::new("Cut"))
                .clicked()
            {
                ctx_cut = true;
                menu.close();
            }
            if menu
                .add_enabled(has_selection, egui::Button::new("Copy"))
                .clicked()
            {
                ctx_copy = true;
                menu.close();
            }
            if menu.button("Paste").clicked() {
                ctx_paste = true;
                menu.close();
            }
            menu.separator();
            if menu.button("Select All").clicked() {
                ctx_select_all = true;
                menu.close();
            }
        });
        if ctx_cut {
            if let Some(sel) = state.editor.copy_selection() {
                push_undo(
                    &mut state.undo_stack,
                    UndoEntry {
                        text: editor_to_string(&state.editor),
                        cursor: state.editor.cursor(),
                        selection: state.editor.selection(),
                    },
                );
                state.redo_stack.clear();
                state.last_undo_op = UndoOpKind::None;
                copy_sanitized(ui.ctx(), sel);
                state.editor.delete_selection();
                state.preferred_cursor_x_px = None;
                changed = true;
            }
        }
        if ctx_copy {
            if let Some(sel) = state.editor.copy_selection() {
                copy_sanitized(ui.ctx(), sel);
            }
        }
        if ctx_paste {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                if let Ok(paste_text) = cb.get_text() {
                    let paste_text = if multiline {
                        paste_text
                    } else {
                        paste_text.replace(['\n', '\r'], " ")
                    };
                    if !paste_text.is_empty() {
                        push_undo(
                            &mut state.undo_stack,
                            UndoEntry {
                                text: editor_to_string(&state.editor),
                                cursor: state.editor.cursor(),
                                selection: state.editor.selection(),
                            },
                        );
                        state.redo_stack.clear();
                        state.last_undo_op = UndoOpKind::None;
                        state.editor.insert_string(&paste_text, None);
                        state.preferred_cursor_x_px = None;
                        changed = true;
                    }
                }
            }
        }
        if ctx_select_all {
            state.preferred_cursor_x_px = None;
            changed |= select_all(&mut state.editor);
        }

        let latest_text = editor_to_string(&state.editor);
        if latest_text != *text {
            *text = latest_text.clone();
            state.last_text = latest_text;
            state.preferred_cursor_x_px = None;
            changed = true;
        }

        if changed {
            response.mark_changed();
        }

        state.last_used_frame = self.current_frame;

        let frame_fill = if has_focus {
            options
                .background_color_focused
                .or(options.background_color_hovered)
                .unwrap_or(options.background_color)
        } else if response.hovered() {
            options
                .background_color_hovered
                .unwrap_or(options.background_color)
        } else {
            options.background_color
        };
        let frame_stroke = if has_focus {
            options
                .stroke_focused
                .or(options.stroke_hovered)
                .unwrap_or(options.stroke)
        } else if response.hovered() {
            options.stroke_hovered.unwrap_or(options.stroke)
        } else {
            options.stroke
        };
        let corner_radius = CornerRadius::same(options.corner_radius);

        ui.painter().rect_filled(rect, corner_radius, frame_fill);
        ui.painter()
            .rect_stroke(rect, corner_radius, frame_stroke, egui::StrokeKind::Inside);

        // --- GPU mesh rendering: atlas glyphs + Shape::Rect for cursor/selection ---
        {
            let painter = ui.painter().with_clip_rect(ui.clip_rect());
            self.paint_editor_gpu(
                &painter,
                content_rect,
                &state.editor,
                options,
                scale,
                has_focus,
                false,
            );
        }
        self.input_states.insert(id, state);
        if !has_focus
            && text.is_empty()
            && let Some(placeholder_text) = options
                .placeholder_text
                .as_deref()
                .filter(|placeholder| !placeholder.is_empty())
        {
            let placeholder_style = LabelOptions {
                font_size: options.font_size,
                line_height: options.line_height,
                color: options
                    .placeholder_color
                    .unwrap_or_else(|| options.text_color.gamma_multiply(0.5)),
                wrap: multiline,
                monospace: options.monospace,
                fundamentals: options.fundamentals.clone(),
                ..LabelOptions::default()
            };
            let placeholder_scene = self.prepare_label_scene(
                ui.ctx(),
                id.with("placeholder"),
                placeholder_text,
                &placeholder_style,
                multiline.then_some(content_rect.width()),
            );
            let placeholder_size = egui_vec_from_text(placeholder_scene.size_points);
            let y_offset = if multiline {
                0.0
            } else {
                ((content_rect.height() - placeholder_size.y) * 0.5).max(0.0)
            };
            let placeholder_rect = Rect::from_min_size(
                Pos2::new(content_rect.min.x, content_rect.min.y + y_offset),
                placeholder_size.min(content_rect.size()),
            );
            let painter = ui.painter().with_clip_rect(ui.clip_rect());
            self.paint_scene_in_rect(&painter, placeholder_rect, &placeholder_scene);
        }

        apply_gamepad_scroll_if_focused(ui, &response);

        response
    }

    fn new_input_state(font_system: &mut FontSystem, text: &str, multiline: bool) -> InputState {
        let mut buffer = Buffer::new(font_system, Metrics::new(16.0, 22.0));
        {
            let mut borrowed = buffer.borrow_with(font_system);
            borrowed.set_wrap(if multiline {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            borrowed.set_text(text, &Attrs::new(), Shaping::Advanced, None);
            borrowed.shape_until_scroll(true);
        }

        InputState {
            editor: Editor::new(buffer),
            last_text: text.to_owned(),
            attrs_fingerprint: 0,
            multiline,
            preferred_cursor_x_px: None,
            scroll_metrics: EditorScrollMetrics::default(),
            last_used_frame: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_undo_op: UndoOpKind::None,
        }
    }

    fn replace_editor_text(
        &mut self,
        editor: &mut Editor<'static>,
        text: &str,
        options: &InputOptions,
        multiline: bool,
        width_px: f32,
        height_px: f32,
        scale: f32,
    ) -> EditorScrollMetrics {
        let attrs_owned = self.input_attrs_owned(options, scale);
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
        let previous_cursor = editor.cursor();
        let previous_selection = editor.selection();
        let previous_scroll = editor.with_buffer(|buffer| buffer.scroll());
        let mut scroll_metrics = EditorScrollMetrics::default();
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_metrics_and_size(
                Metrics::new(effective_font_size, effective_line_height),
                Some(width_px),
                Some(height_px),
            );
            borrowed.set_wrap(if multiline {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            let attrs = attrs_owned.as_attrs();
            borrowed.set_text(text, &attrs, Shaping::Advanced, None);
            borrowed.set_scroll(previous_scroll);
            borrowed.shape_until_scroll(true);
            scroll_metrics =
                clamp_borrowed_buffer_scroll(&mut borrowed, &options.fundamentals, scale);
        });
        editor.set_cursor(clamp_cursor_to_editor(editor, previous_cursor));
        editor.set_selection(clamp_selection_to_editor(editor, previous_selection));
        scroll_metrics
    }

    fn configure_editor(
        &mut self,
        editor: &mut Editor<'static>,
        options: &InputOptions,
        multiline: bool,
        width_px: f32,
        height_px: f32,
        scale: f32,
    ) -> EditorScrollMetrics {
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
        let mut scroll_metrics = EditorScrollMetrics::default();
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_metrics_and_size(
                Metrics::new(effective_font_size, effective_line_height),
                Some(width_px),
                Some(height_px),
            );
            borrowed.set_wrap(if multiline {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            borrowed.shape_until_scroll(true);
            scroll_metrics =
                clamp_borrowed_buffer_scroll(&mut borrowed, &options.fundamentals, scale);
        });
        scroll_metrics
    }

    fn replace_editor_rich_text(
        &mut self,
        editor: &mut Editor<'static>,
        spans: &[RichTextSpan],
        options: &InputOptions,
        width_px: f32,
        height_px: f32,
        scale: f32,
        wrap: bool,
    ) -> EditorScrollMetrics {
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
        let previous_cursor = editor.cursor();
        let previous_selection = editor.selection();
        let previous_scroll = editor.with_buffer(|buffer| buffer.scroll());
        let default_attrs = self.input_attrs_owned(options, scale);
        let span_attrs_owned = spans
            .iter()
            .map(|span| self.input_span_attrs_owned(&span.style, options, scale))
            .collect::<Vec<_>>();
        let mut scroll_metrics = EditorScrollMetrics::default();

        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_metrics_and_size(
                Metrics::new(effective_font_size, effective_line_height),
                Some(width_px),
                Some(height_px),
            );
            borrowed.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
            let rich_text = spans
                .iter()
                .zip(span_attrs_owned.iter())
                .map(|(span, attrs)| (span.text.as_str(), attrs.as_attrs()))
                .collect::<Vec<_>>();
            borrowed.set_rich_text(
                rich_text,
                &default_attrs.as_attrs(),
                Shaping::Advanced,
                None,
            );
            borrowed.set_scroll(previous_scroll);
            borrowed.shape_until_scroll(true);
            scroll_metrics =
                clamp_borrowed_buffer_scroll(&mut borrowed, &options.fundamentals, scale);
        });
        editor.set_cursor(clamp_cursor_to_editor(editor, previous_cursor));
        editor.set_selection(clamp_selection_to_editor(editor, previous_selection));
        scroll_metrics
    }

    fn configure_viewer(
        &mut self,
        editor: &mut Editor<'static>,
        options: &InputOptions,
        width_px: f32,
        height_px: f32,
        scale: f32,
        wrap: bool,
    ) -> EditorScrollMetrics {
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
        let mut scroll_metrics = EditorScrollMetrics::default();
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_metrics_and_size(
                Metrics::new(effective_font_size, effective_line_height),
                Some(width_px),
                Some(height_px),
            );
            borrowed.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
            borrowed.shape_until_scroll(true);
            scroll_metrics =
                clamp_borrowed_buffer_scroll(&mut borrowed, &options.fundamentals, scale);
        });
        scroll_metrics
    }

    fn handle_viewer_events(
        &mut self,
        ui: &Ui,
        response: &Response,
        editor: &mut Editor<'static>,
        content_rect: Rect,
        scale: f32,
        preferred_cursor_x_px: &mut Option<f32>,
        fundamentals: &TextFundamentals,
        process_keyboard: bool,
        pointer_over_scrollbar: bool,
        scroll_metrics: &mut EditorScrollMetrics,
    ) -> bool {
        let mut changed = false;
        let (modifiers, primary_pressed, smooth_scroll_delta) = ui.ctx().input(|i| {
            (
                i.modifiers,
                i.pointer.primary_pressed(),
                i.smooth_scroll_delta,
            )
        });
        let pointer_pressed_on_widget = primary_pressed && response.is_pointer_button_down_on();
        let horizontal_scroll = editor_horizontal_scroll(editor);

        if !pointer_over_scrollbar && let Some(pointer_pos) = response.interact_pointer_pos() {
            let x =
                (((pointer_pos.x - content_rect.min.x) * scale) + horizontal_scroll).round() as i32;
            let y = ((pointer_pos.y - content_rect.min.y) * scale).round() as i32;

            if response.triple_clicked() {
                changed |= triple_click_editor_to_pointer(
                    editor,
                    x,
                    y,
                    preferred_cursor_x_px,
                    fundamentals,
                    scale,
                );
            } else if response.double_clicked() {
                changed |= double_click_editor_to_pointer(
                    editor,
                    x,
                    y,
                    preferred_cursor_x_px,
                    fundamentals,
                    scale,
                );
            } else if pointer_pressed_on_widget {
                if modifiers.shift {
                    changed |= extend_selection_to_pointer(
                        editor,
                        x,
                        y,
                        preferred_cursor_x_px,
                        fundamentals,
                        scale,
                    );
                } else {
                    changed |= click_editor_to_pointer(
                        editor,
                        x,
                        y,
                        preferred_cursor_x_px,
                        fundamentals,
                        scale,
                    );
                }
            } else if response.clicked() {
                if modifiers.shift {
                    changed |= extend_selection_to_pointer(
                        editor,
                        x,
                        y,
                        preferred_cursor_x_px,
                        fundamentals,
                        scale,
                    );
                } else {
                    changed |= click_editor_to_pointer(
                        editor,
                        x,
                        y,
                        preferred_cursor_x_px,
                        fundamentals,
                        scale,
                    );
                }
            }

            if response.dragged() {
                changed |= drag_editor_selection_to_pointer(
                    editor,
                    x,
                    y,
                    preferred_cursor_x_px,
                    fundamentals,
                    scale,
                );
            }
        }

        if response.hovered() {
            let vertical_scroll_delta = smooth_scroll_delta.y;
            let horizontal_scroll_delta = if smooth_scroll_delta.x.abs() > f32::EPSILON {
                smooth_scroll_delta.x
            } else if modifiers.shift && smooth_scroll_delta.y.abs() > f32::EPSILON {
                smooth_scroll_delta.y
            } else {
                0.0
            };
            let horizontal_uses_vertical_wheel = modifiers.shift
                && smooth_scroll_delta.x.abs() <= f32::EPSILON
                && horizontal_scroll_delta.abs() > f32::EPSILON;

            if !horizontal_uses_vertical_wheel && vertical_scroll_delta.abs() > f32::EPSILON {
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::Scroll {
                        pixels: -vertical_scroll_delta * scale,
                    });
                changed = true;
            }
            if horizontal_scroll_delta.abs() > f32::EPSILON {
                self.adjust_editor_horizontal_scroll(
                    editor,
                    -horizontal_scroll_delta * scale,
                    scroll_metrics.max_horizontal_scroll_px,
                );
                changed = true;
            }
        }

        if process_keyboard {
            for event in &self.frame_events {
                match event {
                    TextInputEvent::Copy | TextInputEvent::Cut => {
                        if let Some(selection) = editor.copy_selection() {
                            copy_sanitized(ui.ctx(), selection);
                        }
                    }
                    TextInputEvent::Key {
                        key,
                        pressed,
                        modifiers,
                    } if *pressed => {
                        changed |= handle_read_only_editor_key_event(
                            &mut self.font_system,
                            editor,
                            egui_key_from_text(*key),
                            egui_modifiers_from_text(*modifiers),
                            preferred_cursor_x_px,
                            fundamentals,
                            scale,
                        );
                    }
                    _ => {}
                }
            }
        }

        if changed {
            editor
                .borrow_with(&mut self.font_system)
                .shape_as_needed(false);
            *scroll_metrics = self.measure_editor_scroll_metrics(editor, fundamentals, scale);
        }

        changed
    }

    fn adjust_editor_horizontal_scroll(
        &mut self,
        editor: &mut Editor<'static>,
        delta_px: f32,
        max_horizontal_scroll_px: f32,
    ) {
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            let mut scroll = borrowed.scroll();
            scroll.horizontal = (scroll.horizontal + delta_px).clamp(0.0, max_horizontal_scroll_px);
            borrowed.set_scroll(scroll);
            borrowed.shape_until_scroll(true);
        });
    }

    fn adjust_editor_vertical_scroll(&mut self, editor: &mut Editor<'static>, delta_px: f32) {
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            let mut scroll = borrowed.scroll();
            scroll.vertical += delta_px;
            borrowed.set_scroll(scroll);
            borrowed.shape_until_scroll(true);
        });
    }

    fn measure_editor_scroll_metrics(
        &mut self,
        editor: &mut Editor<'static>,
        fundamentals: &TextFundamentals,
        scale: f32,
    ) -> EditorScrollMetrics {
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            measure_borrowed_buffer_scroll_metrics(&mut borrowed, fundamentals, scale)
        })
    }

    fn sync_viewer_scrollbars(
        &mut self,
        ui: &mut Ui,
        id: Id,
        editor: &mut Editor<'static>,
        content_rect: Rect,
        scale: f32,
        fundamentals: &TextFundamentals,
        scroll_metrics: &mut EditorScrollMetrics,
    ) -> bool {
        let has_horizontal_scroll = scroll_metrics.max_horizontal_scroll_px > f32::EPSILON;
        let has_vertical_scroll = scroll_metrics.max_vertical_scroll_px > f32::EPSILON;
        if !has_horizontal_scroll && !has_vertical_scroll {
            return false;
        }

        let content_width_points =
            content_rect.width() + (scroll_metrics.max_horizontal_scroll_px / scale.max(1.0));
        let content_height_points =
            content_rect.height() + (scroll_metrics.max_vertical_scroll_px / scale.max(1.0));
        let current_horizontal_scroll_points = scroll_metrics.current_horizontal_scroll_px / scale;
        let current_vertical_scroll_points = scroll_metrics.current_vertical_scroll_px / scale;
        let scroll_output = ui
            .scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                egui::ScrollArea::both()
                    .id_salt(id.with("egui_scrollbars"))
                    .max_width(content_rect.width())
                    .max_height(content_rect.height())
                    .scroll_source(egui::containers::scroll_area::ScrollSource::SCROLL_BAR)
                    .scroll_bar_visibility(
                        egui::containers::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .scroll_offset(egui::vec2(
                        current_horizontal_scroll_points,
                        current_vertical_scroll_points,
                    ))
                    .show_viewport(ui, |ui, _viewport| {
                        ui.allocate_space(egui::vec2(
                            content_width_points.max(content_rect.width()),
                            content_height_points.max(content_rect.height()),
                        ));
                    })
            })
            .inner;
        let next_horizontal_scroll_px = (scroll_output.state.offset.x * scale)
            .clamp(0.0, scroll_metrics.max_horizontal_scroll_px);
        let next_vertical_scroll_px = (scroll_output.state.offset.y * scale)
            .clamp(0.0, scroll_metrics.max_vertical_scroll_px);
        let horizontal_delta_px =
            next_horizontal_scroll_px - scroll_metrics.current_horizontal_scroll_px;
        let vertical_delta_px = next_vertical_scroll_px - scroll_metrics.current_vertical_scroll_px;

        let horizontal_changed = horizontal_delta_px.abs() > 0.25;
        let vertical_changed = vertical_delta_px.abs() > 0.25;
        if !horizontal_changed && !vertical_changed {
            return false;
        }

        if horizontal_changed {
            self.adjust_editor_horizontal_scroll(
                editor,
                horizontal_delta_px,
                scroll_metrics.max_horizontal_scroll_px,
            );
        }
        if vertical_changed {
            self.adjust_editor_vertical_scroll(editor, vertical_delta_px);
        }
        *scroll_metrics = self.measure_editor_scroll_metrics(editor, fundamentals, scale);
        ui.ctx().request_repaint();
        true
    }

    /// Paint an editor directly via GPU meshes (glyph atlas + `Shape::Rect` for cursor/selection).
    /// This replaces the CPU pixel-blit path (`rasterize_editor` / `rasterize_editor_tiled`).
    ///
    /// Glyph positions come from `buffer.layout_runs()`, which already accounts for vertical
    /// scroll.  The horizontal scroll is subtracted manually, matching the old CPU path.
    fn paint_editor_gpu(
        &mut self,
        painter: &egui::Painter,
        content_rect: Rect,
        editor: &Editor<'static>,
        options: &InputOptions,
        scale: f32,
        has_focus: bool,
        show_selection_without_focus: bool,
    ) {
        let horizontal_scroll_px = editor.with_buffer(|b| b.scroll().horizontal.max(0.0));
        let selection_visible =
            has_focus || (show_selection_without_focus && editor.selection() != Selection::None);
        let selection_bounds = if selection_visible {
            editor.selection_bounds()
        } else {
            None
        };

        let origin = content_rect.min;
        let painter = painter.with_clip_rect(content_rect);

        struct GlyphCmd {
            cache_key: GlyphRasterKey,
            /// Buffer-space x in device pixels (horizontal scroll already subtracted).
            x_px: f32,
            /// Buffer-space y in device pixels (`line_y + physical.y`; vertical scroll
            /// is already baked into `line_y` by `layout_runs()`).
            y_px: f32,
            color: Color32,
        }

        let mut sel_rects: Vec<Rect> = Vec::new();
        let mut cursor_rect: Option<Rect> = None;
        let mut glyph_cmds: Vec<GlyphCmd> = Vec::new();
        let variation_settings = shared_variation_settings(&options.fundamentals);

        editor.with_buffer(|buffer| {
            let buf_width = buffer.size().0.unwrap_or(0.0);

            for run in buffer.layout_runs() {
                let line_i = run.line_i;
                let line_top = run.line_top; // already scroll-adjusted by LayoutRunIter
                let line_y = run.line_y;
                let line_height = run.line_height;
                let prefixes = collect_glyph_spacing_prefixes_px(
                    run.text,
                    run.glyphs,
                    &options.fundamentals,
                    scale,
                );

                // --- Selection highlights ---
                if let Some((start, end)) = selection_bounds {
                    if line_i >= start.line && line_i <= end.line {
                        let mut range_opt: Option<(i32, i32)> = None;

                        for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                            let cluster = &run.text[glyph.start..glyph.end];
                            let total = cluster.grapheme_indices(true).count().max(1);
                            let mut c_x = adjusted_glyph_x_px(glyph, prefixes[glyph_index]);
                            let c_w = glyph.w / total as f32;

                            for (i, c) in cluster.grapheme_indices(true) {
                                let c_start = glyph.start + i;
                                let c_end = glyph.start + i + c.len();
                                if (start.line != line_i || c_end > start.index)
                                    && (end.line != line_i || c_start < end.index)
                                {
                                    range_opt = match range_opt.take() {
                                        Some((mn, mx)) => {
                                            Some((mn.min(c_x as i32), mx.max((c_x + c_w) as i32)))
                                        }
                                        None => Some((c_x as i32, (c_x + c_w) as i32)),
                                    };
                                } else if let Some((mn, mx)) = range_opt.take() {
                                    sel_rects.push(editor_sel_rect(
                                        mn,
                                        mx,
                                        line_top,
                                        line_height,
                                        horizontal_scroll_px,
                                        origin,
                                        scale,
                                    ));
                                }
                                c_x += c_w;
                            }
                        }

                        if run.glyphs.is_empty() && end.line > line_i {
                            // Highlight entire empty internal lines.
                            range_opt = Some((0, buf_width as i32));
                        }

                        if let Some((mut mn, mut mx)) = range_opt.take() {
                            if end.line > line_i {
                                if run.rtl {
                                    mn = 0;
                                } else {
                                    mx = buf_width as i32;
                                }
                            }
                            sel_rects.push(editor_sel_rect(
                                mn,
                                mx,
                                line_top,
                                line_height,
                                horizontal_scroll_px,
                                origin,
                                scale,
                            ));
                        }
                    }
                }

                // --- Cursor ---
                if has_focus {
                    if let Some(cx) =
                        editor_cursor_x_in_run(&editor.cursor(), &run, &options.fundamentals, scale)
                    {
                        let x_pts = (cx as f32 - horizontal_scroll_px) / scale + origin.x;
                        let y_pts = line_top / scale + origin.y;
                        let h_pts = line_height / scale;
                        // 1 physical pixel wide, full line height
                        cursor_rect = Some(Rect::from_min_size(
                            Pos2::new(x_pts, y_pts),
                            Vec2::new((1.0_f32 / scale).max(0.5), h_pts),
                        ));
                    }
                }

                // --- Glyph draw commands ---
                for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                    let physical = glyph.physical((0.0, 0.0), 1.0);
                    let color = if selection_visible {
                        if let Some((start, end)) = selection_bounds {
                            if line_i >= start.line
                                && line_i <= end.line
                                && (start.line != line_i || glyph.end > start.index)
                                && (end.line != line_i || glyph.start < end.index)
                            {
                                options.selected_text_color
                            } else {
                                glyph
                                    .color_opt
                                    .map_or(options.text_color, cosmic_to_egui_color)
                            }
                        } else {
                            glyph
                                .color_opt
                                .map_or(options.text_color, cosmic_to_egui_color)
                        }
                    } else {
                        glyph
                            .color_opt
                            .map_or(options.text_color, cosmic_to_egui_color)
                    };

                    glyph_cmds.push(GlyphCmd {
                        cache_key: GlyphRasterKey::new(
                            physical.cache_key,
                            scale,
                            options.fundamentals.stem_darkening,
                            GlyphContentMode::AlphaMask,
                            0.0,
                            Arc::clone(&variation_settings),
                        ),
                        x_px: physical.x as f32 + prefixes[glyph_index] - horizontal_scroll_px,
                        y_px: line_y + physical.y as f32,
                        color,
                    });
                }
            }
        });

        // --- Paint selection rects (under glyphs) ---
        for sel in sel_rects {
            painter.add(egui::Shape::rect_filled(
                sel,
                CornerRadius::ZERO,
                options.selection_color,
            ));
        }

        // --- Resolve glyphs through the atlas and build text quads ---
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        let field_range_px = graphics_config.rasterization.field_range_px.max(1.0);
        let mut quads = Vec::with_capacity(glyph_cmds.len());
        for cmd in glyph_cmds {
            let content_mode = self.resolved_glyph_content_mode(graphics_config, &cmd.cache_key);
            let raster_key = cmd.cache_key.for_content_mode(content_mode, field_range_px);
            let Some(atlas_entry) = self.glyph_atlas.resolve_or_queue(
                painter.ctx(),
                &mut self.font_system,
                &mut self.scale_context,
                raster_key,
                self.current_frame,
            ) else {
                continue;
            };

            let glyph_rect = Rect::from_min_size(
                Pos2::new(
                    (cmd.x_px + atlas_entry.placement_left_px as f32) / scale + origin.x,
                    (cmd.y_px - atlas_entry.placement_top_px as f32) / scale + origin.y,
                ),
                Vec2::new(
                    atlas_entry.size_px[0] as f32 / scale,
                    atlas_entry.size_px[1] as f32 / scale,
                ),
            );

            let tint = if atlas_entry.is_color {
                Color32::WHITE
            } else {
                cmd.color
            };

            quads.push(PaintTextQuad {
                page_index: atlas_entry.page_index,
                positions: quad_positions_from_min_size(glyph_rect.min, glyph_rect.size()),
                uvs: uv_quad_points(atlas_entry.uv),
                tint,
                content_mode: atlas_entry.content_mode,
                field_range_px: atlas_entry.field_range_px,
            });
        }

        self.paint_text_quads(&painter, content_rect, &quads);

        // --- Cursor on top of glyphs ---
        if let Some(cursor_rect) = cursor_rect {
            painter.add(egui::Shape::rect_filled(
                cursor_rect,
                CornerRadius::ZERO,
                options.cursor_color,
            ));
        }
    }

    fn input_span_attrs_owned(
        &self,
        style: &RichTextStyle,
        options: &InputOptions,
        scale: f32,
    ) -> AttrsOwned {
        let mut attrs = Attrs::new()
            .color(to_cosmic_text_color(style.color))
            .weight(Weight(self.effective_weight(style.weight)))
            .metrics(Metrics::new(
                (self.effective_font_size(options.font_size) * scale).max(1.0),
                (self.effective_line_height(options.line_height) * scale).max(1.0),
            ));

        if style.monospace {
            attrs = attrs.family(Family::Monospace);
        } else if let Some(family) = self.ui_font_family.as_deref() {
            attrs = attrs.family(Family::Name(family));
        }
        if style.italic {
            attrs = attrs.style(FontStyle::Italic);
        }
        if let Some(features) =
            compose_font_features(&self.open_type_feature_tags, &options.fundamentals)
        {
            attrs = attrs.font_features(features);
        }

        AttrsOwned::new(&attrs)
    }

    fn rich_viewer_attrs_fingerprint(
        &self,
        spans: &[RichTextSpan],
        options: &InputOptions,
        scale: f32,
        wrap: bool,
    ) -> u64 {
        let mut hasher = new_fingerprint_hasher();
        "rich_viewer_attrs".hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        scale.to_bits().hash(&mut hasher);
        wrap.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        self.ui_font_family.hash(&mut hasher);
        self.ui_font_size_scale.to_bits().hash(&mut hasher);
        self.ui_font_weight.hash(&mut hasher);
        self.open_type_features_enabled.hash(&mut hasher);
        self.open_type_features_to_enable.hash(&mut hasher);
        for span in spans {
            span.text.hash(&mut hasher);
            span.style.color.hash(&mut hasher);
            span.style.monospace.hash(&mut hasher);
            span.style.italic.hash(&mut hasher);
            span.style.weight.hash(&mut hasher);
        }
        hasher.finish()
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

    fn get_cached_prepared_layout(
        &mut self,
        id: Id,
        fingerprint: u64,
    ) -> Option<Arc<PreparedTextLayout>> {
        let current_frame = self.current_frame;
        self.prepared_texts.write(|state| {
            let entry = state.touch(&id)?;
            if entry.value.fingerprint != fingerprint {
                return None;
            }
            entry.value.last_used_frame = current_frame;
            Some(Arc::clone(&entry.value.layout))
        })
    }

    fn cache_prepared_layout(&mut self, id: Id, fingerprint: u64, layout: Arc<PreparedTextLayout>) {
        let approx_bytes = layout.approx_bytes;
        let current_frame = self.current_frame;
        self.prepared_texts.write(|state| {
            let _ = state.insert(
                id,
                PreparedTextCacheEntry {
                    fingerprint,
                    layout,
                    last_used_frame: current_frame,
                },
                approx_bytes,
            );
        });
    }

    fn prepare_plain_text_layout(
        &mut self,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> PreparedTextLayout {
        let spans = vec![RichSpan {
            text: text.to_owned(),
            style: SpanStyle {
                color: options.color.into(),
                monospace: options.monospace,
                italic: options.italic,
                weight: options.weight,
            },
        }];
        self.prepare_rich_text_layout(&spans, options, width_points_opt, scale)
    }

    fn prepare_rich_text_layout(
        &mut self,
        spans: &[RichSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> PreparedTextLayout {
        let metrics = Metrics::new(
            (self.effective_font_size(options.font_size) * scale).max(1.0),
            (self.effective_line_height(options.line_height) * scale).max(1.0),
        );

        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let default_attrs_owned = self.build_text_attrs_owned(
            &SpanStyle {
                color: options.color.into(),
                monospace: options.monospace,
                italic: options.italic,
                weight: options.weight,
            },
            options.font_size,
            options.line_height,
            &options.fundamentals,
        );
        let span_attrs_owned = spans
            .iter()
            .map(|span| {
                self.build_text_attrs_owned(
                    &span.style,
                    options.font_size,
                    options.line_height,
                    &options.fundamentals,
                )
            })
            .collect::<Vec<_>>();

        {
            let width_px_opt = width_points_opt.map(|w| (w * scale).max(1.0));
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_wrap(if options.wrap {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            borrowed.set_size(width_px_opt, None);
            let rich_text = spans
                .iter()
                .zip(span_attrs_owned.iter())
                .map(|(span, attrs)| (span.text.as_str(), attrs.as_attrs()))
                .collect::<Vec<_>>();
            let default_attrs = default_attrs_owned.as_attrs();
            borrowed.set_rich_text(rich_text, &default_attrs, Shaping::Advanced, None);
            borrowed.shape_until_scroll(true);
        }

        let (mut measured_width_px, measured_height_px) = measure_buffer_pixels(&buffer);
        if let Some(width_points) = width_points_opt {
            measured_width_px = (width_points * scale).ceil() as usize;
        }

        self.prepare_text_layout_from_buffer(
            &buffer,
            measured_width_px.max(1),
            measured_height_px.max(1),
            scale,
            options.color,
            options.fundamentals.stem_darkening,
            &options.fundamentals,
        )
    }

    fn prepare_text_layout_from_buffer(
        &self,
        buffer: &Buffer,
        width_px: usize,
        height_px: usize,
        scale: f32,
        default_color: Color32,
        stem_darkening: bool,
        fundamentals: &TextFundamentals,
    ) -> PreparedTextLayout {
        let mut effective_fundamentals = fundamentals.clone();
        effective_fundamentals.stem_darkening = stem_darkening;
        let (glyphs, extra_width_points) = collect_prepared_glyphs_from_buffer(
            buffer,
            scale,
            default_color,
            &effective_fundamentals,
        );
        let approx_bytes = glyphs.len().saturating_mul(mem::size_of::<PreparedGlyph>());
        PreparedTextLayout {
            glyphs: Arc::from(glyphs),
            size_points: egui::vec2(
                width_px as f32 / scale + extra_width_points,
                height_px as f32 / scale,
            ),
            approx_bytes,
        }
    }

    fn build_text_scene_from_layout(
        &mut self,
        ctx: &Context,
        layout: &PreparedTextLayout,
        scale: f32,
    ) -> TextRenderScene {
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        let field_range_px = graphics_config.rasterization.field_range_px.max(1.0);
        let mut quads = Vec::with_capacity(layout.glyphs.len());
        let mut bounds: Option<Rect> = None;

        for glyph in layout.glyphs.iter() {
            let content_mode = self.resolved_glyph_content_mode(graphics_config, &glyph.cache_key);
            let raster_key = glyph
                .cache_key
                .for_content_mode(content_mode, field_range_px);
            let Some(atlas_entry) = self.glyph_atlas.resolve_sync(
                ctx,
                &mut self.font_system,
                &mut self.scale_context,
                raster_key,
                self.current_frame,
            ) else {
                continue;
            };

            let min = Pos2::new(
                glyph.offset_points.x + atlas_entry.placement_left_px as f32 / scale,
                glyph.offset_points.y - atlas_entry.placement_top_px as f32 / scale,
            );
            let size_points = egui::vec2(
                atlas_entry.size_px[0] as f32 / scale,
                atlas_entry.size_px[1] as f32 / scale,
            );
            let positions = quad_positions_from_min_size(min, size_points);
            let quad_bounds = rect_from_points(positions);
            bounds = Some(bounds.map_or(quad_bounds, |current| current.union(quad_bounds)));
            quads.push(TextAtlasQuad {
                atlas_page_index: atlas_entry.page_index,
                positions: positions.map(Into::into),
                uvs: uv_quad_points(atlas_entry.uv).map(Into::into),
                tint: if atlas_entry.is_color {
                    Color32::WHITE.into()
                } else {
                    glyph.color.into()
                },
                is_color: atlas_entry.is_color,
            });
        }

        TextRenderScene {
            quads,
            bounds: bounds.unwrap_or(Rect::NOTHING).into(),
            size_points: layout.size_points.into(),
        }
    }

    fn build_text_gpu_scene_from_layout(
        &mut self,
        layout: &PreparedTextLayout,
        scale: f32,
    ) -> TextGpuScene {
        let target_page_side_px =
            default_gpu_scene_page_side(self.resolved_graphics_config(self.max_texture_side_px));
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px);
        let mut pages = Vec::<CpuSceneAtlasPage>::new();
        let mut quads = Vec::with_capacity(layout.glyphs.len());
        let mut bounds: Option<Rect> = None;

        for glyph in layout.glyphs.iter() {
            let Some(atlas_glyph) = self.get_or_rasterize_gpu_scene_glyph(
                &glyph.cache_key,
                graphics_config.rasterization,
                graphics_config.atlas_padding_px,
            ) else {
                continue;
            };

            let allocation_size = size2(
                atlas_glyph.upload_image.size[0] as i32,
                atlas_glyph.upload_image.size[1] as i32,
            );
            let Some((page_index, allocation)) =
                allocate_cpu_scene_page_slot(&mut pages, target_page_side_px, allocation_size)
            else {
                continue;
            };

            let pos = [
                allocation.rectangle.min.x.max(0) as usize,
                allocation.rectangle.min.y.max(0) as usize,
            ];
            blit_color_image(
                &mut pages[page_index].image,
                &atlas_glyph.upload_image,
                pos[0],
                pos[1],
            );

            let page_size = pages[page_index].image.size;
            let uv = Rect::from_min_max(
                Pos2::new(
                    (pos[0] + graphics_config.atlas_padding_px) as f32 / page_size[0] as f32,
                    (pos[1] + graphics_config.atlas_padding_px) as f32 / page_size[1] as f32,
                ),
                Pos2::new(
                    (pos[0] + graphics_config.atlas_padding_px + atlas_glyph.size_px[0]) as f32
                        / page_size[0] as f32,
                    (pos[1] + graphics_config.atlas_padding_px + atlas_glyph.size_px[1]) as f32
                        / page_size[1] as f32,
                ),
            );

            let min = Pos2::new(
                glyph.offset_points.x + atlas_glyph.placement_left_px as f32 / scale,
                glyph.offset_points.y - atlas_glyph.placement_top_px as f32 / scale,
            );
            let size_points = egui::vec2(
                atlas_glyph.size_px[0] as f32 / scale,
                atlas_glyph.size_px[1] as f32 / scale,
            );
            let positions = quad_positions_from_min_size(min, size_points);
            let quad_bounds = rect_from_points(positions);
            bounds = Some(bounds.map_or(quad_bounds, |current| current.union(quad_bounds)));
            quads.push(TextGpuQuad {
                atlas_page_index: page_index,
                positions: positions.map(|point| [point.x, point.y]),
                uvs: uv_quad_points(uv).map(|point| [point.x, point.y]),
                tint_rgba: [
                    if atlas_glyph.is_color {
                        Color32::WHITE.r()
                    } else {
                        glyph.color.r()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.g()
                    } else {
                        glyph.color.g()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.b()
                    } else {
                        glyph.color.b()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.a()
                    } else {
                        glyph.color.a()
                    },
                ],
            });
        }

        let bounds = bounds.unwrap_or(Rect::NOTHING);
        TextGpuScene {
            atlas_pages: pages
                .iter()
                .enumerate()
                .map(|(page_index, page)| color_image_to_page_data(page_index, &page.image))
                .collect(),
            quads,
            bounds_min: [bounds.min.x, bounds.min.y],
            bounds_max: [bounds.max.x, bounds.max.y],
            size_points: [layout.size_points.x, layout.size_points.y],
            fingerprint: 0,
        }
    }

    fn build_text_scene_on_path(
        &mut self,
        ctx: &Context,
        layout: &PreparedTextLayout,
        fallback_advance_points: f32,
        line_height_points: f32,
        scale: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextRenderScene, TextPathError> {
        let path_layout = build_path_layout_from_prepared_layout(
            layout,
            fallback_advance_points,
            line_height_points,
            path,
            path_options,
        )?;
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        let field_range_px = graphics_config.rasterization.field_range_px.max(1.0);

        let mut quads = Vec::with_capacity(layout.glyphs.len());
        let mut bounds: Option<Rect> = None;

        for (glyph, path_glyph) in layout.glyphs.iter().zip(path_layout.glyphs.iter()) {
            let content_mode = self.resolved_glyph_content_mode(graphics_config, &glyph.cache_key);
            let raster_key = glyph
                .cache_key
                .for_content_mode(content_mode, field_range_px);
            let Some(atlas_entry) = self.glyph_atlas.resolve_sync(
                ctx,
                &mut self.font_system,
                &mut self.scale_context,
                raster_key,
                self.current_frame,
            ) else {
                continue;
            };

            let size_points = egui::vec2(
                atlas_entry.size_px[0] as f32 / scale,
                atlas_entry.size_px[1] as f32 / scale,
            );
            let origin_offset = egui::vec2(
                atlas_entry.placement_left_px as f32 / scale,
                -(atlas_entry.placement_top_px as f32) / scale,
            );
            let positions = rotated_quad_positions(
                egui_point_from_text(path_glyph.anchor),
                origin_offset,
                size_points,
                path_glyph.rotation_radians,
            );
            let quad_bounds = rect_from_points(positions);
            bounds = Some(bounds.map_or(quad_bounds, |current| current.union(quad_bounds)));
            quads.push(TextAtlasQuad {
                atlas_page_index: atlas_entry.page_index,
                positions: positions.map(Into::into),
                uvs: uv_quad_points(atlas_entry.uv).map(Into::into),
                tint: if atlas_entry.is_color {
                    Color32::WHITE.into()
                } else {
                    glyph.color.into()
                },
                is_color: atlas_entry.is_color,
            });
        }

        Ok(TextRenderScene {
            quads,
            bounds: bounds
                .unwrap_or(egui_rect_from_text(path_layout.bounds))
                .into(),
            size_points: layout.size_points.into(),
        })
    }

    fn build_text_gpu_scene_on_path(
        &mut self,
        layout: &PreparedTextLayout,
        fallback_advance_points: f32,
        line_height_points: f32,
        scale: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextGpuScene, TextPathError> {
        let path_layout = build_path_layout_from_prepared_layout(
            layout,
            fallback_advance_points,
            line_height_points,
            path,
            path_options,
        )?;
        let target_page_side_px =
            default_gpu_scene_page_side(self.resolved_graphics_config(self.max_texture_side_px));
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px);
        let mut pages = Vec::<CpuSceneAtlasPage>::new();
        let mut quads = Vec::with_capacity(layout.glyphs.len());
        let mut bounds: Option<Rect> = None;

        for (glyph, path_glyph) in layout.glyphs.iter().zip(path_layout.glyphs.iter()) {
            let Some(atlas_glyph) = self.get_or_rasterize_gpu_scene_glyph(
                &glyph.cache_key,
                graphics_config.rasterization,
                graphics_config.atlas_padding_px,
            ) else {
                continue;
            };

            let allocation_size = size2(
                atlas_glyph.upload_image.size[0] as i32,
                atlas_glyph.upload_image.size[1] as i32,
            );
            let Some((page_index, allocation)) =
                allocate_cpu_scene_page_slot(&mut pages, target_page_side_px, allocation_size)
            else {
                continue;
            };

            let pos = [
                allocation.rectangle.min.x.max(0) as usize,
                allocation.rectangle.min.y.max(0) as usize,
            ];
            blit_color_image(
                &mut pages[page_index].image,
                &atlas_glyph.upload_image,
                pos[0],
                pos[1],
            );

            let page_size = pages[page_index].image.size;
            let uv = Rect::from_min_max(
                Pos2::new(
                    (pos[0] + graphics_config.atlas_padding_px) as f32 / page_size[0] as f32,
                    (pos[1] + graphics_config.atlas_padding_px) as f32 / page_size[1] as f32,
                ),
                Pos2::new(
                    (pos[0] + graphics_config.atlas_padding_px + atlas_glyph.size_px[0]) as f32
                        / page_size[0] as f32,
                    (pos[1] + graphics_config.atlas_padding_px + atlas_glyph.size_px[1]) as f32
                        / page_size[1] as f32,
                ),
            );

            let size_points = egui::vec2(
                atlas_glyph.size_px[0] as f32 / scale,
                atlas_glyph.size_px[1] as f32 / scale,
            );
            let origin_offset = egui::vec2(
                atlas_glyph.placement_left_px as f32 / scale,
                -(atlas_glyph.placement_top_px as f32) / scale,
            );
            let positions = rotated_quad_positions(
                egui_point_from_text(path_glyph.anchor),
                origin_offset,
                size_points,
                path_glyph.rotation_radians,
            );
            let quad_bounds = rect_from_points(positions);
            bounds = Some(bounds.map_or(quad_bounds, |current| current.union(quad_bounds)));
            quads.push(TextGpuQuad {
                atlas_page_index: page_index,
                positions: positions.map(|point| [point.x, point.y]),
                uvs: uv_quad_points(uv).map(|point| [point.x, point.y]),
                tint_rgba: [
                    if atlas_glyph.is_color {
                        Color32::WHITE.r()
                    } else {
                        glyph.color.r()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.g()
                    } else {
                        glyph.color.g()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.b()
                    } else {
                        glyph.color.b()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.a()
                    } else {
                        glyph.color.a()
                    },
                ],
            });
        }

        let bounds = bounds.unwrap_or(egui_rect_from_text(path_layout.bounds));
        Ok(TextGpuScene {
            atlas_pages: pages
                .iter()
                .enumerate()
                .map(|(page_index, page)| color_image_to_page_data(page_index, &page.image))
                .collect(),
            quads,
            bounds_min: [bounds.min.x, bounds.min.y],
            bounds_max: [bounds.max.x, bounds.max.y],
            size_points: [layout.size_points.x, layout.size_points.y],
            fingerprint: 0,
        })
    }

    fn build_text_wgpu_scene_callback(
        &self,
        quads: &[PaintTextQuad],
    ) -> Option<TextWgpuSceneCallback> {
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        if graphics_config.renderer_backend != ResolvedTextRendererBackend::WgpuInstanced {
            return None;
        }
        let target_format = self.glyph_atlas.wgpu_render_state.as_ref()?.target_format;
        let mut grouped = FxHashMap::<usize, Vec<TextWgpuInstance>>::default();
        for quad in quads {
            grouped
                .entry(quad.page_index)
                .or_default()
                .push(TextWgpuInstance::from_quad(quad));
        }

        let mut page_indices = grouped.keys().copied().collect::<Vec<_>>();
        page_indices.sort_unstable();
        let mut batches = Vec::with_capacity(page_indices.len());
        for page_index in page_indices {
            let texture = self.glyph_atlas.native_texture_for_page(page_index)?;
            let instances = grouped.remove(&page_index).unwrap_or_default();
            if instances.is_empty() {
                continue;
            }
            batches.push(TextWgpuSceneBatchSource {
                texture,
                instances: Arc::from(instances.into_boxed_slice()),
            });
        }

        if batches.is_empty() {
            return None;
        }

        Some(TextWgpuSceneCallback {
            target_format,
            atlas_sampling: graphics_config.atlas_sampling,
            batches: Arc::from(batches.into_boxed_slice()),
            prepared: Arc::new(Mutex::new(TextWgpuPreparedScene::default())),
        })
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

    fn get_or_rasterize_gpu_scene_glyph(
        &mut self,
        cache_key: &GlyphRasterKey,
        rasterization: TextRasterizationConfig,
        padding_px: usize,
    ) -> Option<Arc<PreparedAtlasGlyph>> {
        if let Some(glyph) = self
            .gpu_scene_glyph_cache
            .write(|state| state.touch(cache_key).map(|entry| Arc::clone(&entry.value)))
        {
            return Some(glyph);
        }

        let glyph = Arc::new(rasterize_atlas_glyph(
            &mut self.font_system,
            &mut self.scale_context,
            cache_key,
            rasterization,
            padding_px,
        )?);
        self.gpu_scene_glyph_cache.write(|state| {
            let _ = state.insert(cache_key.clone(), Arc::clone(&glyph), glyph.approx_bytes);
        });
        Some(glyph)
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

    fn input_attrs_owned(&self, options: &InputOptions, scale: f32) -> AttrsOwned {
        let mut attrs = Attrs::new()
            .color(to_cosmic_color(options.text_color))
            .metrics(Metrics::new(
                (self.effective_font_size(options.font_size) * scale).max(1.0),
                (self.effective_line_height(options.line_height) * scale).max(1.0),
            ))
            .weight(Weight(self.effective_weight(400)));

        if options.monospace {
            attrs = attrs.family(Family::Monospace);
        } else if let Some(family) = self.ui_font_family.as_deref() {
            attrs = attrs.family(Family::Name(family));
        }
        if let Some(features) =
            compose_font_features(&self.open_type_feature_tags, &options.fundamentals)
        {
            attrs = attrs.font_features(features);
        }

        AttrsOwned::new(&attrs)
    }

    fn input_attrs_fingerprint(&self, options: &InputOptions, scale: f32) -> u64 {
        let mut hasher = new_fingerprint_hasher();
        "input_attrs".hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.text_color.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        self.ui_font_family.hash(&mut hasher);
        self.ui_font_size_scale.to_bits().hash(&mut hasher);
        self.ui_font_weight.hash(&mut hasher);
        self.open_type_features_enabled.hash(&mut hasher);
        self.open_type_features_to_enable.hash(&mut hasher);
        hasher.finish()
    }
}

fn parse_feature_tag_list(feature_tags_csv: &str) -> Vec<[u8; 4]> {
    let mut tags = BTreeSet::new();
    for token in feature_tags_csv.split(',') {
        let raw = token.trim();
        if raw.len() != 4 || !raw.is_ascii() {
            continue;
        }

        let mut tag = [0_u8; 4];
        for (index, byte) in raw.as_bytes().iter().enumerate() {
            tag[index] = byte.to_ascii_lowercase();
        }
        tags.insert(tag);
    }

    tags.into_iter().collect()
}

fn build_font_features_from_settings(
    settings: impl IntoIterator<Item = ([u8; 4], u16)>,
) -> Option<FontFeatures> {
    let mut features = FontFeatures::new();
    let mut any = false;
    for (tag, value) in settings {
        features.set(cosmic_text::FeatureTag::new(&tag), value.into());
        any = true;
    }
    any.then_some(features)
}

fn compose_font_features(
    global_feature_tags: &[[u8; 4]],
    fundamentals: &TextFundamentals,
) -> Option<FontFeatures> {
    let mut settings = std::collections::BTreeMap::<[u8; 4], u16>::new();
    for tag in global_feature_tags {
        settings.insert(*tag, 1);
    }
    match fundamentals.kerning {
        TextKerning::Auto => {}
        TextKerning::Normal => {
            settings.insert(*b"kern", 1);
        }
        TextKerning::None => {
            settings.insert(*b"kern", 0);
        }
    }
    settings.insert(*b"liga", u16::from(fundamentals.standard_ligatures));
    settings.insert(*b"calt", u16::from(fundamentals.contextual_alternates));
    settings.insert(*b"dlig", u16::from(fundamentals.discretionary_ligatures));
    settings.insert(*b"hlig", u16::from(fundamentals.historical_ligatures));
    settings.insert(*b"case", u16::from(fundamentals.case_sensitive_forms));
    settings.insert(*b"zero", u16::from(fundamentals.slashed_zero));
    settings.insert(*b"tnum", u16::from(fundamentals.tabular_numbers));
    for feature in &fundamentals.feature_settings {
        settings.insert(feature.tag, feature.value);
    }
    build_font_features_from_settings(settings)
}

fn build_font_features(tags: &[[u8; 4]]) -> FontFeatures {
    build_font_features_from_settings(tags.iter().copied().map(|tag| (tag, 1)))
        .unwrap_or_else(FontFeatures::new)
}

fn multiply_color32(a: Color32, b: Color32) -> Color32 {
    Color32::from_rgba_premultiplied(
        ((u16::from(a.r()) * u16::from(b.r())) / 255) as u8,
        ((u16::from(a.g()) * u16::from(b.g())) / 255) as u8,
        ((u16::from(a.b()) * u16::from(b.b())) / 255) as u8,
        ((u16::from(a.a()) * u16::from(b.a())) / 255) as u8,
    )
}

fn to_cosmic_color(color: Color32) -> Color {
    Color::rgba(color.r(), color.g(), color.b(), color.a())
}

fn to_cosmic_text_color(color: TextColor) -> Color {
    to_cosmic_color(color.into())
}

fn cosmic_to_egui_color(color: Color) -> Color32 {
    Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), color.a())
}

fn egui_vec_from_text(vector: TextVector) -> Vec2 {
    vector.into()
}

fn egui_rect_from_text(rect: TextRect) -> Rect {
    rect.into()
}

fn egui_point_from_text(point: TextPoint) -> Pos2 {
    point.into()
}

/// Find the glyph index and x-offset within that glyph for a cursor on the given layout run.
/// Replicates cosmic-text's private `cursor_glyph_opt` function.
fn editor_cursor_glyph_opt(cursor: &Cursor, run: &LayoutRun<'_>) -> Option<(usize, f32)> {
    if cursor.line != run.line_i {
        return None;
    }
    for (glyph_i, glyph) in run.glyphs.iter().enumerate() {
        if cursor.index == glyph.start {
            return Some((glyph_i, 0.0));
        } else if cursor.index > glyph.start && cursor.index < glyph.end {
            let cluster = &run.text[glyph.start..glyph.end];
            let total = cluster.grapheme_indices(true).count().max(1);
            let before = run.text[glyph.start..cursor.index]
                .grapheme_indices(true)
                .count();
            return Some((glyph_i, glyph.w * before as f32 / total as f32));
        }
    }
    if let Some(last) = run.glyphs.last() {
        if cursor.index == last.end {
            return Some((run.glyphs.len(), 0.0));
        }
    } else {
        // Empty run — cursor is at the start
        return Some((0, 0.0));
    }
    None
}

/// Pixel x-coordinate of the cursor within a layout run (in buffer-space, before scroll).
/// Returns None if the cursor is not on this run.
fn editor_cursor_x_in_run(
    cursor: &Cursor,
    run: &LayoutRun<'_>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<i32> {
    let (cursor_glyph, cursor_glyph_offset) = editor_cursor_glyph_opt(cursor, run)?;
    let prefixes = collect_glyph_spacing_prefixes_px(run.text, run.glyphs, fundamentals, scale);
    let x = run.glyphs.get(cursor_glyph).map_or_else(
        || {
            run.glyphs.last().map_or(0, |g| {
                let prefix_px = prefixes.last().copied().unwrap_or(0.0);
                let glyph_x = adjusted_glyph_x_px(g, prefix_px);
                if g.level.is_rtl() {
                    glyph_x as i32
                } else {
                    (glyph_x + g.w) as i32
                }
            })
        },
        |g| {
            let prefix_px = prefixes.get(cursor_glyph).copied().unwrap_or(0.0);
            let glyph_x = adjusted_glyph_x_px(g, prefix_px);
            if g.level.is_rtl() {
                (glyph_x + g.w - cursor_glyph_offset) as i32
            } else {
                (glyph_x + cursor_glyph_offset) as i32
            }
        },
    );
    Some(x)
}

/// Convert a selection pixel range on a single layout run to an egui Rect in screen space.
/// `line_top` is already scroll-adjusted (as returned by `layout_runs()`).
fn editor_sel_rect(
    min_x: i32,
    max_x: i32,
    line_top: f32,
    line_height: f32,
    horiz_scroll_px: f32,
    origin: Pos2,
    scale: f32,
) -> Rect {
    Rect::from_min_size(
        Pos2::new(
            (min_x as f32 - horiz_scroll_px) / scale + origin.x,
            line_top / scale + origin.y,
        ),
        Vec2::new((max_x - min_x).max(0) as f32 / scale, line_height / scale),
    )
}

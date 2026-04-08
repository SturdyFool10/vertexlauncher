use std::{
    cmp::Ordering,
    collections::{BTreeSet, VecDeque},
    hash::{Hash, Hasher},
    mem,
    sync::{Arc, Mutex, mpsc},
};

const GAMEPAD_SCROLL_DELTA_ID: &str = "textui_gamepad_scroll_delta";
const TEXT_WGPU_INSTANCED_SHADER: &str = include_str!("shaders/text_instanced.wgsl");

fn gamepad_scroll_delta(ctx: &egui::Context) -> egui::Vec2 {
    ctx.data_mut(|data| {
        data.get_temp::<egui::Vec2>(egui::Id::new(GAMEPAD_SCROLL_DELTA_ID))
            .unwrap_or(egui::Vec2::ZERO)
    })
}

fn apply_gamepad_scroll_if_focused(ui: &Ui, response: &Response) {
    if response.has_focus() {
        let delta = gamepad_scroll_delta(ui.ctx());
        if delta != egui::Vec2::ZERO {
            ui.scroll_with_delta(delta);
        }
    }
}

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
use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, Options as MdOptions, Parser, Tag, TagEnd,
};
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
mod button_options;
mod code_block_options;
mod input_options;
mod label_options;
mod markdown_options;
mod text_helpers;
mod tooltip_options;

use crate::{
    button_options::ButtonOptions, code_block_options::CodeBlockOptions,
    input_options::InputOptions, label_options::LabelOptions, markdown_options::MarkdownOptions,
    tooltip_options::TooltipOptions,
};

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
#[doc(hidden)]
pub use input_options::InputOptions as EguiInputOptions;
pub use advanced_text::DEFAULT_ELLIPSIS;

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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct GlyphRasterKey {
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
    fn new(
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
    fn display_scale(&self) -> f32 {
        f32::from_bits(self.display_scale_bits)
    }

    #[inline]
    fn stem_darkening(&self) -> bool {
        self.raster_flags & Self::STEM_DARKENING != 0
    }

    #[inline]
    fn content_mode(&self) -> GlyphContentMode {
        self.content_mode
    }

    #[inline]
    fn field_range_px(&self) -> f32 {
        f32::from_bits(self.field_range_bits)
    }

    fn for_content_mode(&self, content_mode: GlyphContentMode, field_range_px: f32) -> Self {
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
fn glyph_logical_font_size_points(glyph: &GlyphRasterKey) -> f32 {
    let ppem = f32::from_bits(glyph.cache_key.font_size_bits);
    let display_scale = glyph.display_scale().max(1.0);
    (ppem / display_scale).max(1.0)
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum GlyphContentMode {
    AlphaMask,
    Sdf,
    Msdf,
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

struct GlyphAtlas {
    entries: ThreadSafeLru<GlyphRasterKey, GlyphAtlasEntry>,
    pages: Vec<GlyphAtlasPage>,
    page_side_px: usize,
    padding_px: usize,
    sampling: TextAtlasSampling,
    rasterization: TextRasterizationConfig,
    wgpu_render_state: Option<EguiWgpuRenderState>,
    pending: FxHashSet<GlyphRasterKey>,
    ready: VecDeque<GlyphAtlasWorkerResponse>,
    generation: u64,
    tx: Option<mpsc::Sender<GlyphAtlasWorkerMessage>>,
    rx: Option<mpsc::Receiver<GlyphAtlasWorkerResponse>>,
}

struct GlyphAtlasPage {
    allocator: AtlasAllocator,
    content_mode: GlyphContentMode,
    texture: GlyphAtlasTexture,
    backing: ColorImage,
    dirty_rect: Option<DirtyAtlasRect>,
    live_glyphs: usize,
}

enum GlyphAtlasTexture {
    Egui(TextureHandle),
    Wgpu(NativeGlyphAtlasTexture),
}

struct NativeGlyphAtlasTexture {
    id: TextureId,
    texture: wgpu::Texture,
}

impl GlyphAtlasTexture {
    fn id(&self) -> TextureId {
        match self {
            Self::Egui(texture) => texture.id(),
            Self::Wgpu(texture) => texture.id,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DirtyAtlasRect {
    min: [usize; 2],
    max: [usize; 2],
}

impl DirtyAtlasRect {
    fn new(pos: [usize; 2], size: [usize; 2]) -> Self {
        Self {
            min: pos,
            max: [
                pos[0].saturating_add(size[0]),
                pos[1].saturating_add(size[1]),
            ],
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    fn size(self) -> [usize; 2] {
        [
            self.max[0].saturating_sub(self.min[0]),
            self.max[1].saturating_sub(self.min[1]),
        ]
    }
}

#[derive(Clone, Debug)]
struct GlyphAtlasEntry {
    page_index: usize,
    allocation_id: AllocId,
    atlas_min_px: [usize; 2],
    size_px: [usize; 2],
    placement_left_px: i32,
    placement_top_px: i32,
    is_color: bool,
    content_mode: GlyphContentMode,
    field_range_px: f32,
    last_used_frame: u64,
    approx_bytes: usize,
}

#[derive(Clone)]
struct ResolvedGlyphAtlasEntry {
    page_index: usize,
    uv: Rect,
    size_px: [usize; 2],
    placement_left_px: i32,
    placement_top_px: i32,
    is_color: bool,
    content_mode: GlyphContentMode,
    field_range_px: f32,
}

#[derive(Clone)]
struct PreparedAtlasGlyph {
    upload_image: ColorImage,
    size_px: [usize; 2],
    placement_left_px: i32,
    placement_top_px: i32,
    is_color: bool,
    content_mode: GlyphContentMode,
    field_range_px: f32,
    approx_bytes: usize,
}

#[derive(Clone, Debug)]
struct PaintTextQuad {
    page_index: usize,
    positions: [Pos2; 4],
    uvs: [Pos2; 4],
    tint: Color32,
    content_mode: GlyphContentMode,
    field_range_px: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextWgpuScreenUniform {
    screen_size_points: [f32; 2],
    _padding: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextWgpuInstance {
    pos0: [f32; 2],
    pos1: [f32; 2],
    pos2: [f32; 2],
    pos3: [f32; 2],
    uv0: [f32; 2],
    uv1: [f32; 2],
    uv2: [f32; 2],
    uv3: [f32; 2],
    color: [f32; 4],
    decode_mode: f32,
    field_range_px: f32,
    _padding: [f32; 2],
}

impl TextWgpuInstance {
    fn from_quad(quad: &PaintTextQuad) -> Self {
        Self {
            pos0: [quad.positions[0].x, quad.positions[0].y],
            pos1: [quad.positions[1].x, quad.positions[1].y],
            pos2: [quad.positions[2].x, quad.positions[2].y],
            pos3: [quad.positions[3].x, quad.positions[3].y],
            uv0: [quad.uvs[0].x, quad.uvs[0].y],
            uv1: [quad.uvs[1].x, quad.uvs[1].y],
            uv2: [quad.uvs[2].x, quad.uvs[2].y],
            uv3: [quad.uvs[3].x, quad.uvs[3].y],
            color: quad.tint.to_normalized_gamma_f32(),
            decode_mode: match quad.content_mode {
                GlyphContentMode::AlphaMask => 0.0,
                GlyphContentMode::Sdf => 1.0,
                GlyphContentMode::Msdf => 2.0,
            },
            field_range_px: quad.field_range_px,
            _padding: [0.0, 0.0],
        }
    }
}

#[derive(Clone)]
struct TextWgpuSceneBatchSource {
    texture: wgpu::Texture,
    instances: Arc<[TextWgpuInstance]>,
}

struct TextWgpuPreparedBatch {
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
}

#[derive(Default)]
struct TextWgpuPreparedScene {
    batches: Vec<TextWgpuPreparedBatch>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolvedTextRendererBackend {
    EguiMesh,
    WgpuInstanced,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ResolvedTextGraphicsConfig {
    renderer_backend: ResolvedTextRendererBackend,
    atlas_sampling: TextAtlasSampling,
    atlas_page_target_px: usize,
    atlas_padding_px: usize,
    rasterization: TextRasterizationConfig,
}

#[derive(Clone)]
struct TextWgpuSceneCallback {
    target_format: wgpu::TextureFormat,
    atlas_sampling: TextAtlasSampling,
    batches: Arc<[TextWgpuSceneBatchSource]>,
    prepared: Arc<Mutex<TextWgpuPreparedScene>>,
}

struct TextWgpuPipelineResources {
    target_format: wgpu::TextureFormat,
    atlas_sampling: TextAtlasSampling,
    pipeline: wgpu::RenderPipeline,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
}

struct CpuSceneAtlasPage {
    allocator: AtlasAllocator,
    image: ColorImage,
}

fn hash_text_fundamentals<H: Hasher>(fundamentals: &TextFundamentals, state: &mut H) {
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

fn shared_variation_settings(fundamentals: &TextFundamentals) -> Arc<[TextVariationSetting]> {
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
/// - `\u{201C}` (LEFT DOUBLE QUOTATION MARK) → `"`
/// - `\u{201D}` (RIGHT DOUBLE QUOTATION MARK) → `"`
/// - `\u{2018}` (LEFT SINGLE QUOTATION MARK) → `'`
/// - `\u{2019}` (RIGHT SINGLE QUOTATION MARK) → `'`
pub fn sanitize_for_clipboard(text: &str) -> String {
    // Fast path: if no replaceable chars exist, avoid allocation.
    let needs_work = text.contains('\u{2026}')
        || text.contains('\u{201C}')
        || text.contains('\u{201D}')
        || text.contains('\u{2018}')
        || text.contains('\u{2019}');
    if !needs_work {
        return text.to_owned();
    }
    text.replace('\u{2026}', "...")
        .replace('\u{201C}', "\"")
        .replace('\u{201D}', "\"")
        .replace('\u{2018}', "'")
        .replace('\u{2019}', "'")
}

/// Replace straight ASCII quotes with typographic curly quotes for display.
///
/// Uses simple heuristic: a quote preceded by whitespace, at the start of
/// text, or after an opening bracket/paren is an opening quote; otherwise
/// it is a closing quote.
pub fn apply_smart_quotes(text: &str) -> String {
    if !text.contains('"') && !text.contains('\'') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut prev = ' '; // treat start-of-string as whitespace
    for ch in text.chars() {
        match ch {
            '"' => {
                if is_opening_context(prev) {
                    out.push('\u{201C}'); // "
                } else {
                    out.push('\u{201D}'); // "
                }
            }
            '\'' => {
                // Avoid converting apostrophes inside words (e.g. "don't").
                // An opening single quote appears after whitespace or at SOL.
                if is_opening_context(prev) {
                    out.push('\u{2018}'); // '
                } else {
                    out.push('\u{2019}'); // '
                }
            }
            _ => out.push(ch),
        }
        prev = ch;
    }
    out
}

#[inline]
fn is_opening_context(prev: char) -> bool {
    prev.is_whitespace() || matches!(prev, '(' | '[' | '{' | '\u{2014}' | '\u{2013}' | '\0')
}

/// Copy text to the egui clipboard after sanitising typographic characters.
fn copy_sanitized(ctx: &Context, text: String) {
    ctx.copy_text(sanitize_for_clipboard(&text));
}

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

fn collect_glyph_spacing_prefixes_px(
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

fn adjusted_glyph_x_px(glyph: &LayoutGlyph, prefix_px: f32) -> f32 {
    glyph.x + prefix_px
}

fn adjusted_glyph_right_px(glyph: &LayoutGlyph, prefix_px: f32) -> f32 {
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

fn cursor_stops_for_glyphs(
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

fn hit_buffer_with_fundamentals(
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

fn collect_prepared_glyphs_from_buffer(
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

#[derive(Clone, Debug)]
enum AsyncRasterKind {
    Plain(String),
    Rich(Vec<RichSpan>),
}

#[derive(Clone, Debug)]
struct AsyncRasterRequest {
    key_hash: u64,
    kind: AsyncRasterKind,
    options: LabelOptions,
    width_points_opt: Option<f32>,
    scale: f32,
    typography: TypographySnapshot,
}

#[derive(Clone, Debug)]
struct AsyncRasterResponse {
    key_hash: u64,
    layout: PreparedTextLayout,
}

#[derive(Clone, Debug)]
struct TypographySnapshot {
    ui_font_family: Option<String>,
    ui_font_size_scale: f32,
    ui_font_weight: i32,
    open_type_feature_tags: Vec<[u8; 4]>,
}

struct AsyncRasterState {
    tx: Option<mpsc::Sender<AsyncRasterWorkerMessage>>,
    rx: Option<mpsc::Receiver<AsyncRasterResponse>>,
    pending: FxHashSet<u64>,
    cache: ThreadSafeLru<u64, AsyncRasterCacheEntry>,
}

#[derive(Clone, Debug)]
struct AsyncRasterCacheEntry {
    layout: Arc<PreparedTextLayout>,
    last_used_frame: u64,
}

enum AsyncRasterWorkerMessage {
    RegisterFont(Vec<u8>),
    Render(AsyncRasterRequest),
}

enum GlyphAtlasWorkerMessage {
    RegisterFont(Vec<u8>),
    Rasterize {
        generation: u64,
        cache_key: GlyphRasterKey,
        rasterization: TextRasterizationConfig,
        padding_px: usize,
    },
}

struct GlyphAtlasWorkerResponse {
    generation: u64,
    cache_key: GlyphRasterKey,
    glyph: Option<PreparedAtlasGlyph>,
}

/// Coarse operation kind used to group consecutive edits into a single undo
/// entry (so typing a word is one undo step rather than per-char).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum UndoOpKind {
    #[default]
    None,
    TextInsert,
    Delete,
    Paste,
    Cut,
}

#[derive(Clone, Debug)]
struct UndoEntry {
    text: String,
    cursor: Cursor,
    selection: Selection,
}

#[derive(Debug)]
struct InputState {
    editor: Editor<'static>,
    last_text: String,
    attrs_fingerprint: u64,
    multiline: bool,
    preferred_cursor_x_px: Option<f32>,
    scroll_metrics: EditorScrollMetrics,
    last_used_frame: u64,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    last_undo_op: UndoOpKind,
}

#[derive(Clone, Copy, Debug, Default)]
struct EditorScrollMetrics {
    current_horizontal_scroll_px: f32,
    max_horizontal_scroll_px: f32,
    current_vertical_scroll_px: f32,
    max_vertical_scroll_px: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct ViewerScrollbarTracks {
    horizontal: Option<Rect>,
    vertical: Option<Rect>,
}

impl ViewerScrollbarTracks {
    fn contains(self, pos: Pos2) -> bool {
        self.horizontal.is_some_and(|rect| rect.contains(pos))
            || self.vertical.is_some_and(|rect| rect.contains(pos))
    }
}

/// High-level text rendering engine built on cosmic-text + Swash.
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

        let (worker_tx, worker_rx) = mpsc::channel::<AsyncRasterWorkerMessage>();
        let (result_tx, result_rx) = mpsc::channel::<AsyncRasterResponse>();
        let _ = tokio_runtime::spawn_blocking_detached(move || {
            async_raster_worker_loop(worker_rx, result_tx)
        });
        let (worker_tx, result_rx) = (Some(worker_tx), Some(result_rx));
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
            async_raster: AsyncRasterState {
                tx: worker_tx,
                rx: result_rx,
                pending: FxHashSet::default(),
                cache: ThreadSafeLru::new(ASYNC_RASTER_CACHE_MAX_BYTES),
            },
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

    fn handle_input_events(
        &mut self,
        ui: &Ui,
        response: &Response,
        editor: &mut Editor<'static>,
        multiline: bool,
        content_rect: Rect,
        scale: f32,
        preferred_cursor_x_px: &mut Option<f32>,
        fundamentals: &TextFundamentals,
        process_keyboard: bool,
        scroll_metrics: &mut EditorScrollMetrics,
    ) -> bool {
        let mut changed = false;
        let modifiers = ui.ctx().input(|i| i.modifiers);
        let horizontal_scroll = editor_horizontal_scroll(editor);

        if let Some(pointer_pos) = response.interact_pointer_pos() {
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

        if process_keyboard {
            for event in &self.frame_events {
                match event {
                    TextInputEvent::Text(text) => {
                        let mut text = text.clone();
                        if !multiline {
                            text = text.replace(['\n', '\r'], "");
                        }
                        if !text.is_empty() {
                            *preferred_cursor_x_px = None;
                            editor.insert_string(&text, None);
                            changed = true;
                        }
                    }
                    TextInputEvent::Copy => {
                        if let Some(selection) = editor.copy_selection() {
                            copy_sanitized(ui.ctx(), selection);
                        }
                    }
                    TextInputEvent::Cut => {
                        if let Some(selection) = editor.copy_selection() {
                            copy_sanitized(ui.ctx(), selection);
                            changed |= editor.delete_selection();
                            if changed {
                                *preferred_cursor_x_px = None;
                            }
                        }
                    }
                    TextInputEvent::Paste(pasted) => {
                        let mut pasted = pasted.clone();
                        if !multiline {
                            pasted = pasted.replace(['\n', '\r'], " ");
                        }
                        if !pasted.is_empty() {
                            *preferred_cursor_x_px = None;
                            editor.insert_string(&pasted, None);
                            changed = true;
                        }
                    }
                    // Middle-click paste (X11 primary selection convention)
                    TextInputEvent::PointerButton {
                        button: TextPointerButton::Middle,
                        pressed: true,
                        ..
                    } if response.hovered()
                        || response.has_focus()
                        || response.is_pointer_button_down_on() =>
                    {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            if let Ok(paste_text) = cb.get_text() {
                                let paste_text = if multiline {
                                    paste_text
                                } else {
                                    paste_text.replace(['\n', '\r'], " ")
                                };
                                if !paste_text.is_empty() {
                                    *preferred_cursor_x_px = None;
                                    editor.insert_string(&paste_text, None);
                                    changed = true;
                                }
                            }
                        }
                    }
                    TextInputEvent::Key {
                        key,
                        pressed,
                        modifiers,
                    } if *pressed => {
                        changed |= handle_editor_key_event(
                            &mut self.font_system,
                            editor,
                            egui_key_from_text(*key),
                            egui_modifiers_from_text(*modifiers),
                            multiline,
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
            self.adjust_editor_horizontal_scroll(
                editor,
                0.0,
                scroll_metrics.max_horizontal_scroll_px,
            );
            *scroll_metrics = self.measure_editor_scroll_metrics(editor, fundamentals, scale);
        }

        changed
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

    fn typography_snapshot(&self) -> TypographySnapshot {
        TypographySnapshot {
            ui_font_family: self.ui_font_family.clone(),
            ui_font_size_scale: self.ui_font_size_scale,
            ui_font_weight: self.ui_font_weight,
            open_type_feature_tags: self.open_type_feature_tags.clone(),
        }
    }

    fn poll_async_raster_results(&mut self) {
        let mut should_reset_worker = false;
        let Some(rx) = self.async_raster.rx.as_ref() else {
            return;
        };
        let current_frame = self.current_frame;
        loop {
            match rx.try_recv() {
                Ok(response) => {
                    self.async_raster.pending.remove(&response.key_hash);
                    let layout = Arc::new(response.layout);
                    let approx_bytes = layout.approx_bytes;
                    self.async_raster.cache.write(|state| {
                        let _ = state.insert(
                            response.key_hash,
                            AsyncRasterCacheEntry {
                                layout,
                                last_used_frame: current_frame,
                            },
                            approx_bytes,
                        );
                    });
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    should_reset_worker = true;
                    break;
                }
            }
        }
        if should_reset_worker {
            self.async_raster.tx = None;
            self.async_raster.rx = None;
            self.async_raster.pending.clear();
        }
        self.enforce_async_raster_cache_budget();
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

    fn enforce_async_raster_cache_budget(&mut self) {
        self.async_raster.cache.write(|state| {
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

    fn get_or_queue_async_plain_layout(
        &mut self,
        key_hash: u64,
        text: String,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<Arc<PreparedTextLayout>> {
        let current_frame = self.current_frame;
        if let Some(layout) = self.async_raster.cache.write(|state| {
            let entry = state.touch(&key_hash)?;
            entry.value.last_used_frame = current_frame;
            Some(Arc::clone(&entry.value.layout))
        }) {
            return Some(layout);
        }
        let Some(tx) = self.async_raster.tx.as_ref().cloned() else {
            return Some(Arc::new(self.prepare_plain_text_layout(
                text.as_str(),
                options,
                width_points_opt,
                scale,
            )));
        };
        if self.async_raster.pending.insert(key_hash) {
            let request_text = text.clone();
            let request = AsyncRasterRequest {
                key_hash,
                kind: AsyncRasterKind::Plain(request_text),
                options: options.clone(),
                width_points_opt,
                scale,
                typography: self.typography_snapshot(),
            };
            if tx.send(AsyncRasterWorkerMessage::Render(request)).is_err() {
                self.async_raster.pending.remove(&key_hash);
                self.async_raster.tx = None;
                self.async_raster.rx = None;
                return Some(Arc::new(self.prepare_plain_text_layout(
                    text.as_str(),
                    options,
                    width_points_opt,
                    scale,
                )));
            }
        }
        None
    }

    fn get_or_queue_async_rich_layout(
        &mut self,
        key_hash: u64,
        spans: Vec<RichSpan>,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<Arc<PreparedTextLayout>> {
        let current_frame = self.current_frame;
        if let Some(layout) = self.async_raster.cache.write(|state| {
            let entry = state.touch(&key_hash)?;
            entry.value.last_used_frame = current_frame;
            Some(Arc::clone(&entry.value.layout))
        }) {
            return Some(layout);
        }
        let Some(tx) = self.async_raster.tx.as_ref().cloned() else {
            return Some(Arc::new(self.prepare_rich_text_layout(
                spans.as_slice(),
                options,
                width_points_opt,
                scale,
            )));
        };
        if self.async_raster.pending.insert(key_hash) {
            let request_spans = spans.clone();
            let request = AsyncRasterRequest {
                key_hash,
                kind: AsyncRasterKind::Rich(request_spans),
                options: options.clone(),
                width_points_opt,
                scale,
                typography: self.typography_snapshot(),
            };
            if tx.send(AsyncRasterWorkerMessage::Render(request)).is_err() {
                self.async_raster.pending.remove(&key_hash);
                self.async_raster.tx = None;
                self.async_raster.rx = None;
                return Some(Arc::new(self.prepare_rich_text_layout(
                    spans.as_slice(),
                    options,
                    width_points_opt,
                    scale,
                )));
            }
        }
        None
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

fn async_raster_worker_loop(
    rx: mpsc::Receiver<AsyncRasterWorkerMessage>,
    tx: mpsc::Sender<AsyncRasterResponse>,
) {
    let mut font_system = FontSystem::new();
    configure_text_font_defaults(&mut font_system);

    while let Ok(msg) = rx.recv() {
        match msg {
            AsyncRasterWorkerMessage::RegisterFont(bytes) => {
                font_system.db_mut().load_font_data(bytes);
            }
            AsyncRasterWorkerMessage::Render(req) => {
                let layout = async_prepare_text_layout(&mut font_system, &req);
                let _ = tx.send(AsyncRasterResponse {
                    key_hash: req.key_hash,
                    layout,
                });
            }
        }
    }
}

fn async_prepare_text_layout(
    font_system: &mut FontSystem,
    req: &AsyncRasterRequest,
) -> PreparedTextLayout {
    let metrics = Metrics::new(
        (req.options.font_size * req.typography.ui_font_size_scale * req.scale).max(1.0),
        (req.options.line_height * req.typography.ui_font_size_scale * req.scale).max(1.0),
    );
    let mut buffer = Buffer::new(font_system, metrics);
    let width_px_opt = req.width_points_opt.map(|w| (w * req.scale).max(1.0));
    {
        let mut borrowed = buffer.borrow_with(font_system);
        borrowed.set_wrap(if req.options.wrap {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        borrowed.set_size(width_px_opt, None);

        match &req.kind {
            AsyncRasterKind::Plain(text) => {
                let attrs_owned = async_build_text_attrs_owned(
                    req,
                    &SpanStyle {
                        color: req.options.color.into(),
                        monospace: req.options.monospace,
                        italic: req.options.italic,
                        weight: req.options.weight,
                    },
                );
                let attrs = attrs_owned.as_attrs();
                borrowed.set_text(text, &attrs, Shaping::Advanced, None);
            }
            AsyncRasterKind::Rich(spans) => {
                let default_attrs_owned = async_build_text_attrs_owned(
                    req,
                    &SpanStyle {
                        color: req.options.color.into(),
                        monospace: req.options.monospace,
                        italic: req.options.italic,
                        weight: req.options.weight,
                    },
                );
                let span_attrs_owned = spans
                    .iter()
                    .map(|span| async_build_text_attrs_owned(req, &span.style))
                    .collect::<Vec<_>>();
                let rich_text = spans
                    .iter()
                    .zip(span_attrs_owned.iter())
                    .map(|(span, attrs)| (span.text.as_str(), attrs.as_attrs()))
                    .collect::<Vec<_>>();
                let default_attrs = default_attrs_owned.as_attrs();
                borrowed.set_rich_text(rich_text, &default_attrs, Shaping::Advanced, None);
            }
        }
        borrowed.shape_until_scroll(true);
    }

    let (mut measured_width_px, measured_height_px) = measure_buffer_pixels(&buffer);
    if let Some(width_points) = req.width_points_opt {
        measured_width_px = (width_points * req.scale).ceil() as usize;
    }
    let width_px = measured_width_px.max(1);
    let height_px = measured_height_px.max(1);
    let (glyphs, extra_width_points) = collect_prepared_glyphs_from_buffer(
        &buffer,
        req.scale,
        req.options.color,
        &req.options.fundamentals,
    );

    PreparedTextLayout {
        approx_bytes: glyphs.len().saturating_mul(mem::size_of::<PreparedGlyph>()),
        glyphs: Arc::from(glyphs),
        size_points: egui::vec2(
            width_px as f32 / req.scale + extra_width_points,
            height_px as f32 / req.scale,
        ),
    }
}

fn async_build_text_attrs_owned(req: &AsyncRasterRequest, style: &SpanStyle) -> AttrsOwned {
    let effective_weight =
        (i32::from(style.weight) + (req.typography.ui_font_weight - 400)).clamp(100, 900) as u16;
    let mut attrs = Attrs::new()
        .color(to_cosmic_text_color(style.color))
        .weight(Weight(effective_weight))
        .metrics(Metrics::new(
            (req.options.font_size * req.typography.ui_font_size_scale).max(1.0),
            (req.options.line_height * req.typography.ui_font_size_scale).max(1.0),
        ));

    if style.monospace {
        attrs = attrs.family(Family::Monospace);
    } else if let Some(family) = req.typography.ui_font_family.as_deref() {
        attrs = attrs.family(Family::Name(family));
    }
    if style.italic {
        attrs = attrs.style(FontStyle::Italic);
    }
    if let Some(features) = compose_font_features(
        &req.typography.open_type_feature_tags,
        &req.options.fundamentals,
    ) {
        attrs = attrs.font_features(features);
    }
    AttrsOwned::new(&attrs)
}

fn build_font_features(tags: &[[u8; 4]]) -> FontFeatures {
    build_font_features_from_settings(tags.iter().copied().map(|tag| (tag, 1)))
        .unwrap_or_else(FontFeatures::new)
}

impl GlyphAtlas {
    fn new() -> Self {
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
            wgpu_render_state: None,
            pending: FxHashSet::default(),
            ready: VecDeque::new(),
            generation: 0,
            tx: Some(tx),
            rx: Some(result_rx),
        }
    }

    fn set_render_state(&mut self, render_state: Option<&EguiWgpuRenderState>) {
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

    fn set_page_side(&mut self, page_side_px: usize) {
        self.page_side_px = page_side_px.max(1);
    }

    fn set_sampling(&mut self, sampling: TextAtlasSampling) {
        self.sampling = sampling;
    }

    fn set_padding(&mut self, padding_px: usize) {
        self.padding_px = padding_px;
    }

    fn set_rasterization(&mut self, rasterization: TextRasterizationConfig) {
        self.rasterization = rasterization;
    }

    fn register_font(&self, bytes: Vec<u8>) {
        if let Some(tx) = self.tx.as_ref() {
            let _ = tx.send(GlyphAtlasWorkerMessage::RegisterFont(bytes));
        }
    }

    fn clear(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.pending.clear();
        self.ready.clear();
        let _ = self.entries.write(|state| state.clear());
        self.free_all_pages();
    }

    fn poll_ready(&mut self, ctx: &Context, current_frame: u64) {
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

    fn trim_stale(&mut self, current_frame: u64) {
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

    fn resolve_or_queue(
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

    fn resolve_sync(
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
            field_range_px: glyph.field_range_px,
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
            dirty_rect: None,
            live_glyphs: 0,
        });
        true
    }

    fn allocate_page_texture(&mut self, ctx: &Context, side: usize) -> GlyphAtlasTexture {
        if let Some(render_state) = self.wgpu_render_state.as_ref() {
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
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
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
            field_range_px: entry.field_range_px,
        }
    }

    fn page_snapshot(&self, page_index: usize) -> Option<TextAtlasPageSnapshot> {
        let page = self.pages.get(page_index)?;
        Some(TextAtlasPageSnapshot {
            page_index,
            size_px: page.backing.size,
            rgba8: page
                .backing
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect(),
        })
    }

    fn page_data(&self, page_index: usize) -> Option<TextAtlasPageData> {
        self.page_snapshot(page_index)
            .map(|snapshot| snapshot.to_rgba8())
    }

    fn texture_id_for_page(&self, page_index: usize) -> Option<TextureId> {
        self.pages.get(page_index).map(|page| page.texture.id())
    }

    fn native_texture_for_page(&self, page_index: usize) -> Option<wgpu::Texture> {
        let page = self.pages.get(page_index)?;
        match &page.texture {
            GlyphAtlasTexture::Wgpu(texture) => Some(texture.texture.clone()),
            GlyphAtlasTexture::Egui(_) => None,
        }
    }

    fn write_glyph(&mut self, page_index: usize, allocation: Allocation, glyph: &ColorImage) {
        let Some(page) = self.pages.get_mut(page_index) else {
            return;
        };

        let pos = [
            allocation.rectangle.min.x.max(0) as usize,
            allocation.rectangle.min.y.max(0) as usize,
        ];
        blit_color_image(&mut page.backing, glyph, pos[0], pos[1]);
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

fn rasterize_atlas_glyph(
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
        field_range_px: 0.0,
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

fn render_swash_outline_commands(
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

#[derive(Clone, Copy)]
struct FieldLineSegment {
    a: [f32; 2],
    b: [f32; 2],
    color_mask: u8,
}

#[derive(Default)]
struct FlattenedOutline {
    contours: Vec<Vec<[f32; 2]>>,
    segments: Vec<FieldLineSegment>,
    min: [f32; 2],
    max: [f32; 2],
}

impl FlattenedOutline {
    fn new() -> Self {
        Self {
            contours: Vec::new(),
            segments: Vec::new(),
            min: [f32::INFINITY, f32::INFINITY],
            max: [f32::NEG_INFINITY, f32::NEG_INFINITY],
        }
    }

    fn include_point(&mut self, point: [f32; 2]) {
        self.min[0] = self.min[0].min(point[0]);
        self.min[1] = self.min[1].min(point[1]);
        self.max[0] = self.max[0].max(point[0]);
        self.max[1] = self.max[1].max(point[1]);
    }
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
        field_range_px,
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

#[derive(Clone, Copy)]
struct GlyphErrorScore {
    total_error: f32,
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

fn blit_color_image(dest: &mut ColorImage, src: &ColorImage, dest_x: usize, dest_y: usize) {
    let dest_width = dest.size[0];
    for y in 0..src.size[1] {
        let target_y = dest_y + y;
        if target_y >= dest.size[1] {
            break;
        }
        let src_row = y * src.size[0];
        let dest_row = target_y * dest_width;
        for x in 0..src.size[0] {
            let target_x = dest_x + x;
            if target_x >= dest_width {
                break;
            }
            dest.pixels[dest_row + target_x] = src.pixels[src_row + x];
        }
    }
}

fn color_image_sub_image(src: &ColorImage, rect: DirtyAtlasRect) -> ColorImage {
    let size = rect.size();
    let mut image = ColorImage::filled(size, Color32::TRANSPARENT);
    let src_width = src.size[0];
    let dst_width = size[0];
    for y in 0..size[1] {
        let src_y = rect.min[1] + y;
        let src_row = src_y * src_width;
        let dst_row = y * dst_width;
        for x in 0..size[0] {
            let src_x = rect.min[0] + x;
            image.pixels[dst_row + x] = src.pixels[src_row + src_x];
        }
    }
    image
}

fn write_color_image_to_wgpu_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pos: [usize; 2],
    image: &ColorImage,
) {
    let size = wgpu::Extent3d {
        width: image.size[0] as u32,
        height: image.size[1] as u32,
        depth_or_array_layers: 1,
    };
    let mut bytes = Vec::with_capacity(image.pixels.len().saturating_mul(4));
    for pixel in &image.pixels {
        bytes.extend_from_slice(&pixel.to_array());
    }
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
            bytes_per_row: Some(4 * image.size[0] as u32),
            rows_per_image: Some(image.size[1] as u32),
        },
        size,
    );
}

impl TextWgpuPipelineResources {
    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        atlas_sampling: TextAtlasSampling,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("textui_instanced_shader"),
            source: wgpu::ShaderSource::Wgsl(TEXT_WGPU_INSTANCED_SHADER.into()),
        });
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("textui_instanced_uniform_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("textui_instanced_texture_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("textui_instanced_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu_filter_mode_for_sampling(atlas_sampling),
            min_filter: wgpu_filter_mode_for_sampling(atlas_sampling),
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let uniform = TextWgpuScreenUniform {
            screen_size_points: [1.0, 1.0],
            _padding: [0.0, 0.0],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("textui_instanced_uniform_buffer"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("textui_instanced_uniform_bg"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("textui_instanced_pipeline_layout"),
            bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let premultiplied_alpha = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("textui_instanced_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<TextWgpuInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x2,
                        3 => Float32x2,
                        4 => Float32x2,
                        5 => Float32x2,
                        6 => Float32x2,
                        7 => Float32x2,
                        8 => Float32x4,
                        9 => Float32,
                        10 => Float32
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(premultiplied_alpha),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: TEXT_WGPU_PASS_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: TEXT_WGPU_PASS_MSAA_SAMPLES.max(1),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });
        Self {
            target_format,
            atlas_sampling,
            pipeline,
            texture_bind_group_layout,
            sampler,
            uniform_buffer,
            uniform_bind_group,
        }
    }

    fn update_uniform(&self, queue: &wgpu::Queue, screen_size_points: [f32; 2]) {
        let uniform = TextWgpuScreenUniform {
            screen_size_points,
            _padding: [0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }
}

impl egui_wgpu::CallbackTrait for TextWgpuSceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = callback_resources
            .entry::<TextWgpuPipelineResources>()
            .or_insert_with(|| {
                TextWgpuPipelineResources::new(device, self.target_format, self.atlas_sampling)
            });
        if resources.target_format != self.target_format
            || resources.atlas_sampling != self.atlas_sampling
        {
            *resources =
                TextWgpuPipelineResources::new(device, self.target_format, self.atlas_sampling);
        }
        resources.update_uniform(
            queue,
            [
                screen_descriptor.size_in_pixels[0] as f32 / screen_descriptor.pixels_per_point,
                screen_descriptor.size_in_pixels[1] as f32 / screen_descriptor.pixels_per_point,
            ],
        );

        let mut prepared_batches = Vec::with_capacity(self.batches.len());
        for batch in self.batches.iter() {
            if batch.instances.is_empty() {
                continue;
            }
            let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("textui_instanced_instance_buffer"),
                contents: bytemuck::cast_slice(batch.instances.as_ref()),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let view = batch
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("textui_instanced_texture_bg"),
                layout: &resources.texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&resources.sampler),
                    },
                ],
            });
            prepared_batches.push(TextWgpuPreparedBatch {
                bind_group,
                instance_buffer,
                instance_count: batch.instances.len() as u32,
            });
        }

        if let Ok(mut prepared) = self.prepared.lock() {
            prepared.batches = prepared_batches;
        }

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<TextWgpuPipelineResources>() else {
            return;
        };
        let Ok(prepared) = self.prepared.lock() else {
            return;
        };
        if prepared.batches.is_empty() {
            return;
        }

        render_pass.set_viewport(
            0.0,
            0.0,
            info.screen_size_px[0] as f32,
            info.screen_size_px[1] as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
        for batch in &prepared.batches {
            render_pass.set_bind_group(1, &batch.bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.instance_buffer.slice(..));
            render_pass.draw(0..6, 0..batch.instance_count);
        }
    }
}

fn add_text_quad(
    mesh: &mut egui::epaint::Mesh,
    positions: [Pos2; 4],
    uvs: [Pos2; 4],
    tint: Color32,
) {
    let base = mesh.vertices.len() as u32;
    for index in 0..4 {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: positions[index],
            uv: uvs[index],
            color: tint,
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn paint_text_quads_fallback(
    glyph_atlas: &GlyphAtlas,
    painter: &egui::Painter,
    quads: &[PaintTextQuad],
) {
    let mut meshes: FxHashMap<TextureId, egui::epaint::Mesh> = FxHashMap::default();
    for quad in quads {
        let Some(texture_id) = glyph_atlas.texture_id_for_page(quad.page_index) else {
            continue;
        };
        let mesh = meshes
            .entry(texture_id)
            .or_insert_with(|| egui::epaint::Mesh::with_texture(texture_id));
        add_text_quad(mesh, quad.positions, quad.uvs, quad.tint);
    }

    for (_, mesh) in meshes {
        if !mesh.is_empty() {
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

fn map_scene_quads_to_rect(
    rect: Rect,
    uv: Rect,
    natural_size: Vec2,
    quads: &[TextAtlasQuad],
    tint: Color32,
) -> Vec<PaintTextQuad> {
    if natural_size.x.abs() <= f32::EPSILON || natural_size.y.abs() <= f32::EPSILON {
        return Vec::new();
    }

    let scale_x = rect.width() / natural_size.x;
    let scale_y = rect.height() / natural_size.y;
    let uv_width = uv.width();
    let uv_height = uv.height();

    quads
        .iter()
        .map(|quad| PaintTextQuad {
            page_index: quad.atlas_page_index,
            positions: quad.positions.map(|point| {
                Pos2::new(
                    rect.min.x + point.x * scale_x,
                    rect.min.y + point.y * scale_y,
                )
            }),
            uvs: quad.uvs.map(|point| {
                Pos2::new(
                    uv.min.x + point.x * uv_width,
                    uv.min.y + point.y * uv_height,
                )
            }),
            tint: multiply_color32(quad.tint.into(), tint),
            content_mode: GlyphContentMode::AlphaMask,
            field_range_px: 0.0,
        })
        .collect()
}

fn default_gpu_scene_page_side(graphics_config: ResolvedTextGraphicsConfig) -> usize {
    graphics_config.atlas_page_target_px.max(256)
}

fn gpu_scene_approx_bytes(scene: &TextGpuScene) -> usize {
    scene
        .atlas_pages
        .iter()
        .map(|p| p.rgba8.len())
        .sum::<usize>()
        + scene.quads.len() * std::mem::size_of::<TextGpuQuad>()
        + 64
}

fn allocate_cpu_scene_page_slot(
    pages: &mut Vec<CpuSceneAtlasPage>,
    target_page_side_px: usize,
    allocation_size: etagere::Size,
) -> Option<(usize, Allocation)> {
    for (page_index, page) in pages.iter_mut().enumerate() {
        if let Some(allocation) = page.allocator.allocate(allocation_size) {
            return Some((page_index, allocation));
        }
    }

    let side = target_page_side_px
        .max(allocation_size.width.max(1) as usize)
        .max(allocation_size.height.max(1) as usize);
    let mut allocator = AtlasAllocator::new(size2(side as i32, side as i32));
    let allocation = allocator.allocate(allocation_size)?;
    pages.push(CpuSceneAtlasPage {
        allocator,
        image: ColorImage::filled([side, side], Color32::TRANSPARENT),
    });
    Some((pages.len() - 1, allocation))
}

fn color_image_to_page_data(page_index: usize, image: &ColorImage) -> TextAtlasPageData {
    let mut rgba8 = Vec::with_capacity(image.pixels.len().saturating_mul(4));
    for pixel in &image.pixels {
        rgba8.extend_from_slice(&pixel.to_array());
    }
    TextAtlasPageData {
        page_index,
        size_px: image.size,
        rgba8,
    }
}

fn quad_positions_from_min_size(min: Pos2, size: Vec2) -> [Pos2; 4] {
    [
        min,
        Pos2::new(min.x + size.x, min.y),
        Pos2::new(min.x + size.x, min.y + size.y),
        Pos2::new(min.x, min.y + size.y),
    ]
}

fn rotated_quad_positions(
    anchor: Pos2,
    top_left_offset: Vec2,
    size_points: Vec2,
    rotation_radians: f32,
) -> [Pos2; 4] {
    let rotation = egui::emath::Rot2::from_angle(rotation_radians);
    [
        top_left_offset,
        top_left_offset + egui::vec2(size_points.x, 0.0),
        top_left_offset + size_points,
        top_left_offset + egui::vec2(0.0, size_points.y),
    ]
    .map(|offset| anchor + rotation * offset)
}

fn uv_quad_points(uv: Rect) -> [Pos2; 4] {
    [
        uv.min,
        Pos2::new(uv.max.x, uv.min.y),
        uv.max,
        Pos2::new(uv.min.x, uv.max.y),
    ]
}

fn rect_from_points(points: [Pos2; 4]) -> Rect {
    let mut min = points[0];
    let mut max = points[0];
    for point in &points[1..] {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
    }
    Rect::from_min_max(min, max)
}

fn build_path_layout_from_prepared_layout(
    layout: &PreparedTextLayout,
    fallback_advance_points: f32,
    line_height_points: f32,
    path: &TextPath,
    path_options: &TextPathOptions,
) -> Result<TextPathLayout, TextPathError> {
    if path.points.is_empty() {
        return Err(TextPathError::EmptyPath);
    }
    if layout.glyphs.is_empty() {
        return Err(TextPathError::EmptyText);
    }
    let path_length = text_path_length(path);
    if path_length <= f32::EPSILON {
        return Err(TextPathError::PathTooShort);
    }

    let baseline_y = layout.glyphs[0].offset_points.y;
    let mut glyphs = Vec::with_capacity(layout.glyphs.len());
    let mut bounds: Option<Rect> = None;
    for (index, glyph) in layout.glyphs.iter().enumerate() {
        let advance_points =
            estimated_glyph_advance_points(&layout.glyphs, index, fallback_advance_points);
        let distance =
            (path_options.start_offset_points + glyph.offset_points.x).clamp(0.0, path_length);
        let sample = sample_text_path(path, distance).ok_or(TextPathError::PathTooShort)?;
        let baseline_offset =
            glyph.offset_points.y - baseline_y + path_options.normal_offset_points;
        let anchor = sample.position + sample.normal * baseline_offset;
        let rotation_radians = if path_options.rotate_glyphs {
            sample.tangent.y.atan2(sample.tangent.x)
        } else {
            0.0
        };
        let glyph_rect = Rect::from_center_size(
            anchor,
            egui::vec2(advance_points.max(1.0), line_height_points.max(1.0)),
        );
        bounds = Some(bounds.map_or(glyph_rect, |current| current.union(glyph_rect)));
        glyphs.push(TextPathGlyph {
            anchor: anchor.into(),
            tangent: sample.tangent.into(),
            normal: sample.normal.into(),
            rotation_radians,
            local_offset: egui::vec2(0.0, baseline_offset).into(),
            advance_points,
            color: glyph.color.into(),
        });
    }

    Ok(TextPathLayout {
        glyphs,
        bounds: bounds.unwrap_or(Rect::NOTHING).into(),
        total_advance_points: layout.size_points.x,
        path_length_points: path_length,
    })
}

fn export_prepared_layout_as_shapes(
    layout: &PreparedTextLayout,
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    fallback_line_height: f32,
    rasterization: TextRasterizationConfig,
) -> VectorTextShape {
    let mut glyphs = Vec::with_capacity(layout.glyphs.len());
    let mut bounds: Option<Rect> = None;
    for glyph in layout.glyphs.iter() {
        let Some(shape) = export_vector_glyph_shape(
            font_system,
            scale_context,
            glyph,
            fallback_line_height,
            rasterization,
        ) else {
            continue;
        };
        bounds = Some(bounds.map_or(shape.bounds.into(), |current| {
            current.union(shape.bounds.into())
        }));
        glyphs.push(shape);
    }
    VectorTextShape {
        glyphs,
        bounds: bounds.unwrap_or(Rect::NOTHING).into(),
    }
}

#[derive(Clone, Copy)]
struct TextPathSample {
    position: Pos2,
    tangent: Vec2,
    normal: Vec2,
}

fn text_path_length(path: &TextPath) -> f32 {
    iter_text_path_segments(path)
        .map(|(from, to)| (to - from).length())
        .sum()
}

fn iter_text_path_segments(path: &TextPath) -> impl Iterator<Item = (Pos2, Pos2)> + '_ {
    let open_segments = path.points.windows(2).map(|points| {
        (
            egui_point_from_text(points[0]),
            egui_point_from_text(points[1]),
        )
    });
    let closing_segment = if path.closed && path.points.len() > 2 {
        Some((
            egui_point_from_text(*path.points.last().unwrap_or(&TextPoint::ZERO)),
            egui_point_from_text(path.points[0]),
        ))
    } else {
        None
    };
    open_segments.chain(closing_segment)
}

fn sample_text_path(path: &TextPath, distance_points: f32) -> Option<TextPathSample> {
    let mut remaining = distance_points.max(0.0);
    for (from, to) in iter_text_path_segments(path) {
        let delta = to - from;
        let segment_length = delta.length();
        if segment_length <= f32::EPSILON {
            continue;
        }
        let tangent = delta / segment_length;
        let normal = egui::vec2(-tangent.y, tangent.x);
        if remaining <= segment_length {
            let position = from + tangent * remaining;
            return Some(TextPathSample {
                position,
                tangent,
                normal,
            });
        }
        remaining -= segment_length;
    }
    let (&last_but_one, &last) = match path.points.as_slice() {
        [.., a, b] => (a, b),
        _ => return None,
    };
    let last_but_one = egui_point_from_text(last_but_one);
    let last = egui_point_from_text(last);
    let delta = last - last_but_one;
    let segment_length = delta.length();
    if segment_length <= f32::EPSILON {
        return None;
    }
    let tangent = delta / segment_length;
    Some(TextPathSample {
        position: last,
        tangent,
        normal: egui::vec2(-tangent.y, tangent.x),
    })
}

fn estimated_glyph_advance_points(
    glyphs: &[PreparedGlyph],
    index: usize,
    fallback_font_size: f32,
) -> f32 {
    if let Some(next) = glyphs.get(index + 1) {
        (next.offset_points.x - glyphs[index].offset_points.x).max(0.0)
    } else {
        fallback_font_size.max(1.0) * 0.5
    }
}

fn export_vector_glyph_shape(
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    glyph: &PreparedGlyph,
    fallback_line_height: f32,
    rasterization: TextRasterizationConfig,
) -> Option<VectorGlyphShape> {
    let commands =
        render_swash_outline_commands(font_system, scale_context, &glyph.cache_key, rasterization)?;
    let mut vector_commands = Vec::with_capacity(commands.len());
    let mut bounds: Option<Rect> = None;
    for command in commands.iter() {
        let mapped = map_outline_command_to_points(*command, glyph.offset_points);
        update_vector_shape_bounds(&mut bounds, &mapped);
        vector_commands.push(mapped);
    }
    Some(VectorGlyphShape {
        bounds: bounds
            .unwrap_or_else(|| {
                Rect::from_min_size(
                    Pos2::new(
                        glyph.offset_points.x,
                        glyph.offset_points.y - fallback_line_height,
                    ),
                    egui::vec2(1.0, fallback_line_height.max(1.0)),
                )
            })
            .into(),
        color: glyph.color.into(),
        commands: vector_commands,
    })
}

fn map_outline_command_to_points(
    command: swash::zeno::Command,
    glyph_origin: Vec2,
) -> VectorPathCommand {
    let map_point =
        |point: swash::zeno::Point| Pos2::new(glyph_origin.x + point.x, glyph_origin.y - point.y);
    match command {
        swash::zeno::Command::MoveTo(point) => VectorPathCommand::MoveTo(map_point(point).into()),
        swash::zeno::Command::LineTo(point) => VectorPathCommand::LineTo(map_point(point).into()),
        swash::zeno::Command::QuadTo(control, point) => {
            VectorPathCommand::QuadTo(map_point(control).into(), map_point(point).into())
        }
        swash::zeno::Command::CurveTo(control_a, control_b, point) => VectorPathCommand::CurveTo(
            map_point(control_a).into(),
            map_point(control_b).into(),
            map_point(point).into(),
        ),
        swash::zeno::Command::Close => VectorPathCommand::Close,
    }
}

fn update_vector_shape_bounds(bounds: &mut Option<Rect>, command: &VectorPathCommand) {
    let mut include_point = |point: Pos2| {
        let rect = Rect::from_min_max(point, point);
        *bounds = Some(bounds.map_or(rect, |current| current.union(rect)));
    };
    match command {
        VectorPathCommand::MoveTo(point) | VectorPathCommand::LineTo(point) => {
            include_point((*point).into())
        }
        VectorPathCommand::QuadTo(control, point) => {
            include_point((*control).into());
            include_point((*point).into());
        }
        VectorPathCommand::CurveTo(control_a, control_b, point) => {
            include_point((*control_a).into());
            include_point((*control_b).into());
            include_point((*point).into());
        }
        VectorPathCommand::Close => {}
    }
}

fn multiply_color32(a: Color32, b: Color32) -> Color32 {
    Color32::from_rgba_premultiplied(
        ((u16::from(a.r()) * u16::from(b.r())) / 255) as u8,
        ((u16::from(a.g()) * u16::from(b.g())) / 255) as u8,
        ((u16::from(a.b()) * u16::from(b.b())) / 255) as u8,
        ((u16::from(a.a()) * u16::from(b.a())) / 255) as u8,
    )
}

fn editor_to_string(editor: &Editor<'static>) -> String {
    let mut out = String::new();
    editor.with_buffer(|buffer| {
        for line in &buffer.lines {
            out.push_str(line.text());
            out.push_str(line.ending().as_str());
        }
    });
    out
}

fn editor_horizontal_scroll(editor: &Editor<'static>) -> f32 {
    editor.with_buffer(|buffer| buffer.scroll().horizontal.max(0.0))
}

fn clamp_cursor_to_editor(editor: &Editor<'static>, cursor: Cursor) -> Cursor {
    editor.with_buffer(|buffer| {
        let Some(last_line) = buffer.lines.len().checked_sub(1) else {
            return Cursor::new_with_affinity(0, 0, cursor.affinity);
        };
        let line = cursor.line.min(last_line);
        let index = cursor.index.min(buffer.lines[line].text().len());
        Cursor::new_with_affinity(line, index, cursor.affinity)
    })
}

fn clamp_selection_to_editor(editor: &Editor<'static>, selection: Selection) -> Selection {
    match selection {
        Selection::None => Selection::None,
        Selection::Normal(cursor) => Selection::Normal(clamp_cursor_to_editor(editor, cursor)),
        Selection::Line(cursor) => Selection::Line(clamp_cursor_to_editor(editor, cursor)),
        Selection::Word(cursor) => Selection::Word(clamp_cursor_to_editor(editor, cursor)),
    }
}

fn selection_anchor(selection: Selection) -> Option<Cursor> {
    match selection {
        Selection::None => None,
        Selection::Normal(cursor) | Selection::Line(cursor) | Selection::Word(cursor) => {
            Some(cursor)
        }
    }
}

fn cursor_x_for_layout_cursor(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    cursor: Cursor,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<f32> {
    let layout_cursor = buffer.layout_cursor(font_system, cursor)?;
    let line_text = buffer.lines.get(layout_cursor.line)?.text().to_owned();
    let layout = buffer.line_layout(font_system, layout_cursor.line)?;
    let layout_line = layout.get(layout_cursor.layout).or_else(|| layout.last())?;
    let stops = cursor_stops_for_glyphs(
        layout_cursor.line,
        &line_text,
        &layout_line.glyphs,
        fundamentals,
        scale,
    );
    stops
        .into_iter()
        .find(|(stop_cursor, _)| {
            stop_cursor.line == cursor.line && stop_cursor.index == cursor.index
        })
        .map(|(_, x)| x)
        .or_else(|| layout_line.glyphs.is_empty().then_some(0.0))
}

fn cursor_for_layout_line_x(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    line_i: usize,
    layout_i: usize,
    desired_x: f32,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<Cursor> {
    let line_text = buffer.lines.get(line_i)?.text().to_owned();
    let layout = buffer.line_layout(font_system, line_i)?;
    let layout_line = layout.get(layout_i).or_else(|| layout.last())?;
    let stops =
        cursor_stops_for_glyphs(line_i, &line_text, &layout_line.glyphs, fundamentals, scale);

    if let Some((first_cursor, first_x)) = stops.first().copied()
        && desired_x <= first_x
    {
        return Some(first_cursor);
    }

    for window in stops.windows(2) {
        let (left_cursor, left_x) = window[0];
        let (right_cursor, right_x) = window[1];
        let mid_x = (left_x + right_x) * 0.5;
        if desired_x <= mid_x {
            return Some(left_cursor);
        }
        if desired_x <= right_x {
            return Some(right_cursor);
        }
    }

    stops
        .last()
        .map(|(cursor, _)| *cursor)
        .or_else(|| Some(Cursor::new_with_affinity(line_i, 0, Affinity::After)))
}

fn adjacent_visual_layout_position(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    cursor: Cursor,
    direction: i32,
) -> Option<(usize, usize)> {
    let mut layout_cursor = buffer.layout_cursor(font_system, cursor)?;
    match direction.cmp(&0) {
        Ordering::Less => {
            if layout_cursor.layout > 0 {
                layout_cursor.layout -= 1;
            } else if layout_cursor.line > 0 {
                layout_cursor.line -= 1;
                let layout_count = buffer.line_layout(font_system, layout_cursor.line)?.len();
                layout_cursor.layout = layout_count.saturating_sub(1);
            } else {
                return None;
            }
        }
        Ordering::Greater => {
            let layout_count = buffer.line_layout(font_system, layout_cursor.line)?.len();
            if layout_cursor.layout + 1 < layout_count {
                layout_cursor.layout += 1;
            } else if layout_cursor.line + 1 < buffer.lines.len() {
                layout_cursor.line += 1;
                layout_cursor.layout = 0;
            } else {
                return None;
            }
        }
        Ordering::Equal => return Some((layout_cursor.line, layout_cursor.layout)),
    }
    Some((layout_cursor.line, layout_cursor.layout))
}

fn move_cursor_one_visual_line(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    direction: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    let current_cursor = editor.cursor();
    let desired_x = preferred_cursor_x_px.unwrap_or_else(|| {
        editor
            .with_buffer_mut(|buffer| {
                cursor_x_for_layout_cursor(buffer, font_system, current_cursor, fundamentals, scale)
            })
            .unwrap_or(0.0)
    });
    *preferred_cursor_x_px = Some(desired_x);

    let Some((target_line, target_layout)) = editor.with_buffer_mut(|buffer| {
        adjacent_visual_layout_position(buffer, font_system, current_cursor, direction)
    }) else {
        return false;
    };

    let Some(new_cursor) = editor.with_buffer_mut(|buffer| {
        cursor_for_layout_line_x(
            buffer,
            font_system,
            target_line,
            target_layout,
            desired_x,
            fundamentals,
            scale,
        )
    }) else {
        return false;
    };

    if new_cursor != current_cursor {
        editor.set_cursor(new_cursor);
        true
    } else {
        false
    }
}

fn handle_spacing_aware_vertical_motion(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    motion: Motion,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    match motion {
        Motion::Up => move_cursor_one_visual_line(
            font_system,
            editor,
            -1,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        ),
        Motion::Down => move_cursor_one_visual_line(
            font_system,
            editor,
            1,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        ),
        Motion::PageUp | Motion::PageDown | Motion::Vertical(_) => {
            let step_count = editor.with_buffer(|buffer| match motion {
                Motion::PageUp => buffer
                    .size()
                    .1
                    .map(|height| -(height as i32 / buffer.metrics().line_height as i32))
                    .unwrap_or(0),
                Motion::PageDown => buffer
                    .size()
                    .1
                    .map(|height| height as i32 / buffer.metrics().line_height as i32)
                    .unwrap_or(0),
                Motion::Vertical(px) => px / buffer.metrics().line_height as i32,
                _ => 0,
            });
            let direction = step_count.signum();
            let mut moved = false;
            for _ in 0..step_count.unsigned_abs() {
                if !move_cursor_one_visual_line(
                    font_system,
                    editor,
                    direction,
                    preferred_cursor_x_px,
                    fundamentals,
                    scale,
                ) {
                    break;
                }
                moved = true;
            }
            moved
        }
        _ => false,
    }
}

fn motion_uses_preferred_cursor_x(motion: Motion) -> bool {
    matches!(
        motion,
        Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown | Motion::Vertical(_)
    )
}

fn editor_hit_test(
    editor: &Editor<'static>,
    x: i32,
    y: i32,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<Cursor> {
    editor.with_buffer(|buffer| {
        hit_buffer_with_fundamentals(buffer, x as f32, y as f32, fundamentals, scale)
    })
}

fn click_editor_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let old_cursor = editor.cursor();
    let old_selection = editor.selection();
    editor.set_selection(Selection::None);
    if let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) {
        editor.set_cursor(new_cursor);
    }
    editor.cursor() != old_cursor || editor.selection() != old_selection
}

fn double_click_editor_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let old_cursor = editor.cursor();
    let old_selection = editor.selection();
    editor.set_selection(Selection::None);
    if let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) {
        editor.set_cursor(new_cursor);
        editor.set_selection(Selection::Word(editor.cursor()));
    }
    editor.cursor() != old_cursor || editor.selection() != old_selection
}

fn triple_click_editor_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let old_cursor = editor.cursor();
    let old_selection = editor.selection();
    editor.set_selection(Selection::None);
    if let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) {
        editor.set_cursor(new_cursor);
        editor.set_selection(Selection::Line(editor.cursor()));
    }
    editor.cursor() != old_cursor || editor.selection() != old_selection
}

fn drag_editor_selection_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let old_cursor = editor.cursor();
    let old_selection = editor.selection();
    if editor.selection() == Selection::None {
        editor.set_selection(Selection::Normal(editor.cursor()));
    }
    if let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) {
        editor.set_cursor(new_cursor);
    }
    editor.cursor() != old_cursor || editor.selection() != old_selection
}

fn extend_selection_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let anchor = selection_anchor(editor.selection()).unwrap_or_else(|| editor.cursor());
    let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) else {
        return false;
    };

    editor.set_cursor(new_cursor);
    if new_cursor == anchor {
        editor.set_selection(Selection::None);
    } else {
        editor.set_selection(Selection::Normal(anchor));
    }
    true
}

fn select_all(editor: &mut Editor<'static>) -> bool {
    let end = editor.with_buffer(|buffer| {
        let Some(line) = buffer.lines.len().checked_sub(1) else {
            return Cursor::new(0, 0);
        };
        Cursor::new(line, buffer.lines[line].text().len())
    });
    editor.set_selection(Selection::Normal(Cursor::new(0, 0)));
    editor.set_cursor(end);
    true
}

/// Classify a frame event as a text-modifying operation for undo grouping.
/// Returns `UndoOpKind::None` for non-modifying events (navigation, etc.).
fn classify_modify_op(event: &TextInputEvent) -> UndoOpKind {
    match event {
        TextInputEvent::Text(t) if !t.is_empty() => UndoOpKind::TextInsert,
        TextInputEvent::Paste(p) if !p.is_empty() => UndoOpKind::Paste,
        TextInputEvent::Cut => UndoOpKind::Cut,
        TextInputEvent::Key {
            key,
            pressed: true,
            modifiers,
        } => {
            let word_delete = (modifiers.alt || modifiers.ctrl || modifiers.mac_cmd)
                && matches!(key, TextKey::Backspace | TextKey::Delete);
            let emacs_delete =
                modifiers.ctrl && matches!(key, TextKey::H | TextKey::K | TextKey::U | TextKey::W);
            if matches!(key, TextKey::Backspace | TextKey::Delete) || word_delete || emacs_delete {
                UndoOpKind::Delete
            } else {
                UndoOpKind::None
            }
        }
        // Middle-click paste counts as Paste
        TextInputEvent::PointerButton {
            button: TextPointerButton::Middle,
            pressed: true,
            ..
        } => UndoOpKind::Paste,
        _ => UndoOpKind::None,
    }
}

/// True if the event is a cursor-navigation key (resets undo grouping so the
/// next insertion starts a new undo entry).
fn is_navigation_event(event: &TextInputEvent) -> bool {
    matches!(
        event,
        TextInputEvent::Key {
            key: TextKey::Left
                | TextKey::Right
                | TextKey::Up
                | TextKey::Down
                | TextKey::Home
                | TextKey::End
                | TextKey::PageUp
                | TextKey::PageDown,
            pressed: true,
            ..
        }
    )
}

/// Returns the first modifying op kind found in the event list, or None.
fn pending_modify_op(events: &[TextInputEvent]) -> UndoOpKind {
    events
        .iter()
        .map(classify_modify_op)
        .find(|op| *op != UndoOpKind::None)
        .unwrap_or(UndoOpKind::None)
}

/// Push an undo entry, capping the stack at UNDO_STACK_MAX.
fn push_undo(stack: &mut Vec<UndoEntry>, entry: UndoEntry) {
    if stack.len() >= UNDO_STACK_MAX {
        stack.remove(0);
    }
    stack.push(entry);
}

fn handle_editor_key_event(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
    multiline: bool,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    if modifiers.command && key == Key::A {
        *preferred_cursor_x_px = None;
        return select_all(editor);
    }

    if handle_editor_delete_shortcut(font_system, editor, key, modifiers) {
        *preferred_cursor_x_px = None;
        return true;
    }

    if cfg!(target_os = "macos") && modifiers.ctrl && !modifiers.shift {
        if let Some(motion) = mac_control_motion(key) {
            return handle_editor_motion_key(
                font_system,
                editor,
                key,
                modifiers,
                motion,
                preferred_cursor_x_px,
                fundamentals,
                scale,
            );
        }
    }

    let Some(action) = key_to_action(key, modifiers, multiline) else {
        return false;
    };

    match action {
        Action::Motion(motion) => handle_editor_motion_key(
            font_system,
            editor,
            key,
            modifiers,
            motion,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        ),
        _ => {
            *preferred_cursor_x_px = None;
            editor.borrow_with(font_system).action(action);
            true
        }
    }
}

fn handle_read_only_editor_key_event(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    if modifiers.command && key == Key::A {
        *preferred_cursor_x_px = None;
        return select_all(editor);
    }

    if cfg!(target_os = "macos") && modifiers.ctrl && !modifiers.shift {
        if let Some(motion) = mac_control_motion(key) {
            return handle_editor_motion_key(
                font_system,
                editor,
                key,
                modifiers,
                motion,
                preferred_cursor_x_px,
                fundamentals,
                scale,
            );
        }
    }

    let Some(action) = key_to_action(key, modifiers, true) else {
        if key == Key::Escape && editor.selection() != Selection::None {
            *preferred_cursor_x_px = None;
            editor.set_selection(Selection::None);
            return true;
        }
        return false;
    };

    match action {
        Action::Motion(motion) => handle_editor_motion_key(
            font_system,
            editor,
            key,
            modifiers,
            motion,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        ),
        Action::Escape => {
            if editor.selection() != Selection::None {
                *preferred_cursor_x_px = None;
                editor.set_selection(Selection::None);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn scroll_editor_to_buffer_end(font_system: &mut FontSystem, editor: &mut Editor<'static>) {
    editor.set_selection(Selection::None);
    editor
        .borrow_with(font_system)
        .action(Action::Motion(Motion::BufferEnd));
}

fn handle_editor_motion_key(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
    motion: Motion,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    if modifiers.shift {
        if editor.selection() == Selection::None {
            editor.set_selection(Selection::Normal(editor.cursor()));
        }
        if motion_uses_preferred_cursor_x(motion) {
            return handle_spacing_aware_vertical_motion(
                font_system,
                editor,
                motion,
                preferred_cursor_x_px,
                fundamentals,
                scale,
            );
        }
        *preferred_cursor_x_px = None;
        editor
            .borrow_with(font_system)
            .action(Action::Motion(motion));
        return true;
    }

    if let Some((start, end)) = editor.selection_bounds() {
        if modifiers.is_none() && key == Key::ArrowLeft {
            *preferred_cursor_x_px = None;
            editor.set_selection(Selection::None);
            editor.set_cursor(start);
            return true;
        }
        if modifiers.is_none() && key == Key::ArrowRight {
            *preferred_cursor_x_px = None;
            editor.set_selection(Selection::None);
            editor.set_cursor(end);
            return true;
        }
        editor.set_selection(Selection::None);
    }

    if motion_uses_preferred_cursor_x(motion) {
        handle_spacing_aware_vertical_motion(
            font_system,
            editor,
            motion,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        )
    } else {
        *preferred_cursor_x_px = None;
        editor
            .borrow_with(font_system)
            .action(Action::Motion(motion));
        true
    }
}

fn handle_editor_delete_shortcut(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
) -> bool {
    match key {
        Key::Backspace if modifiers.mac_cmd => delete_to_motion(font_system, editor, Motion::Home),
        Key::Backspace if modifiers.alt || modifiers.ctrl => {
            delete_to_motion(font_system, editor, Motion::PreviousWord)
        }
        Key::Delete if (!modifiers.shift || !cfg!(target_os = "windows")) && modifiers.mac_cmd => {
            delete_forward_to_motion(font_system, editor, Motion::End)
        }
        Key::Delete
            if (!modifiers.shift || !cfg!(target_os = "windows"))
                && (modifiers.alt || modifiers.ctrl) =>
        {
            delete_forward_to_motion(font_system, editor, Motion::NextWord)
        }
        Key::H if modifiers.ctrl => {
            editor.borrow_with(font_system).action(Action::Backspace);
            true
        }
        Key::K if modifiers.ctrl => delete_forward_to_motion(font_system, editor, Motion::End),
        Key::U if modifiers.ctrl => delete_to_motion(font_system, editor, Motion::Home),
        Key::W if modifiers.ctrl => delete_to_motion(font_system, editor, Motion::PreviousWord),
        _ => false,
    }
}

fn delete_to_motion(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    motion: Motion,
) -> bool {
    if editor.delete_selection() {
        return true;
    }

    let end = editor.cursor();
    let Some(start) = cursor_after_motion(font_system, editor, end, motion) else {
        return false;
    };
    delete_cursor_range(editor, start, end)
}

fn delete_forward_to_motion(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    motion: Motion,
) -> bool {
    if editor.delete_selection() {
        return true;
    }

    let start = editor.cursor();
    let Some(end) = cursor_after_motion(font_system, editor, start, motion) else {
        return false;
    };
    delete_cursor_range(editor, start, end)
}

fn cursor_after_motion(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    cursor: Cursor,
    motion: Motion,
) -> Option<Cursor> {
    editor.with_buffer_mut(|buffer| {
        let mut borrowed = buffer.borrow_with(font_system);
        borrowed
            .cursor_motion(cursor, None, motion)
            .map(|(next, _)| next)
    })
}

fn delete_cursor_range(editor: &mut Editor<'static>, first: Cursor, second: Cursor) -> bool {
    if first == second {
        return false;
    }

    let (start, end) = ordered_cursor_pair(first, second);
    editor.set_selection(Selection::None);
    editor.set_cursor(start);
    editor.delete_range(start, end);
    true
}

fn ordered_cursor_pair(first: Cursor, second: Cursor) -> (Cursor, Cursor) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn mac_control_motion(key: Key) -> Option<Motion> {
    match key {
        Key::A => Some(Motion::Home),
        Key::E => Some(Motion::End),
        Key::B => Some(Motion::Left),
        Key::F => Some(Motion::Right),
        Key::P => Some(Motion::Up),
        Key::N => Some(Motion::Down),
        _ => None,
    }
}

fn key_to_action(key: Key, modifiers: egui::Modifiers, multiline: bool) -> Option<Action> {
    match key {
        Key::ArrowLeft => Some(if modifiers.alt || modifiers.ctrl {
            Action::Motion(Motion::PreviousWord)
        } else if modifiers.mac_cmd {
            Action::Motion(Motion::Home)
        } else {
            Action::Motion(Motion::Left)
        }),
        Key::ArrowRight => Some(if modifiers.alt || modifiers.ctrl {
            Action::Motion(Motion::NextWord)
        } else if modifiers.mac_cmd {
            Action::Motion(Motion::End)
        } else {
            Action::Motion(Motion::Right)
        }),
        Key::ArrowUp => Some(if modifiers.command {
            Action::Motion(Motion::BufferStart)
        } else {
            Action::Motion(Motion::Up)
        }),
        Key::ArrowDown => Some(if modifiers.command {
            Action::Motion(Motion::BufferEnd)
        } else {
            Action::Motion(Motion::Down)
        }),
        Key::Home => Some(if modifiers.ctrl {
            Action::Motion(Motion::BufferStart)
        } else {
            Action::Motion(Motion::Home)
        }),
        Key::End => Some(if modifiers.ctrl {
            Action::Motion(Motion::BufferEnd)
        } else {
            Action::Motion(Motion::End)
        }),
        Key::PageUp => Some(Action::Motion(Motion::PageUp)),
        Key::PageDown => Some(Action::Motion(Motion::PageDown)),
        Key::Backspace => Some(Action::Backspace),
        Key::Delete => Some(Action::Delete),
        Key::Escape => Some(Action::Escape),
        Key::Enter if multiline => Some(Action::Enter),
        Key::Tab if multiline => Some(if modifiers.shift {
            Action::Unindent
        } else {
            Action::Indent
        }),
        _ => None,
    }
}

fn parse_markdown_blocks(markdown: &str) -> Vec<TextMarkdownBlock> {
    let parser = Parser::new_ext(markdown, MdOptions::all());

    let mut blocks = Vec::new();
    let mut text_buf = String::new();
    let mut current_heading: Option<HeadingLevel> = None;
    let mut in_code_block = false;
    let mut current_code_language: Option<String> = None;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                text_buf.clear();
                current_heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = current_heading.take() {
                    if !text_buf.trim().is_empty() {
                        blocks.push(TextMarkdownBlock::Heading {
                            level: text_markdown_heading_level(level),
                            text: text_buf.trim().to_owned(),
                        });
                    }
                    text_buf.clear();
                }
            }
            Event::Start(Tag::Paragraph) => {
                text_buf.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                if !text_buf.trim().is_empty() {
                    blocks.push(TextMarkdownBlock::Paragraph(text_buf.trim().to_owned()));
                }
                text_buf.clear();
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                text_buf.clear();
                current_code_language = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let token = lang.split_whitespace().next().unwrap_or_default();
                        if token.is_empty() {
                            None
                        } else {
                            Some(token.to_owned())
                        }
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                blocks.push(TextMarkdownBlock::Code {
                    language: current_code_language.take(),
                    text: text_buf.clone(),
                });
                text_buf.clear();
                in_code_block = false;
            }
            Event::Text(text) | Event::Code(text) => {
                text_buf.push_str(&text);
            }
            Event::SoftBreak | Event::HardBreak => {
                text_buf.push('\n');
            }
            Event::Start(Tag::Item) => {
                if !in_code_block {
                    if !text_buf.is_empty() {
                        text_buf.push('\n');
                    }
                    text_buf.push_str("- ");
                }
            }
            Event::Rule => {
                if !text_buf.trim().is_empty() {
                    blocks.push(TextMarkdownBlock::Paragraph(text_buf.trim().to_owned()));
                }
                text_buf.clear();
                blocks.push(TextMarkdownBlock::Paragraph("---".to_owned()));
            }
            _ => {}
        }
    }

    if !text_buf.trim().is_empty() {
        if in_code_block {
            blocks.push(TextMarkdownBlock::Code {
                language: current_code_language,
                text: text_buf,
            });
        } else if let Some(level) = current_heading {
            blocks.push(TextMarkdownBlock::Heading {
                level: text_markdown_heading_level(level),
                text: text_buf,
            });
        } else {
            blocks.push(TextMarkdownBlock::Paragraph(text_buf));
        }
    }

    blocks
}

fn text_markdown_heading_level(level: HeadingLevel) -> TextMarkdownHeadingLevel {
    match level {
        HeadingLevel::H1 => TextMarkdownHeadingLevel::H1,
        HeadingLevel::H2 => TextMarkdownHeadingLevel::H2,
        HeadingLevel::H3 => TextMarkdownHeadingLevel::H3,
        HeadingLevel::H4 => TextMarkdownHeadingLevel::H4,
        HeadingLevel::H5 => TextMarkdownHeadingLevel::H5,
        HeadingLevel::H6 => TextMarkdownHeadingLevel::H6,
    }
}

fn measure_buffer_pixels(buffer: &Buffer) -> (usize, usize) {
    let mut max_right = 0.0_f32;
    let mut max_bottom = 0.0_f32;

    for run in buffer.layout_runs() {
        max_bottom = max_bottom.max(run.line_top + run.line_height);
        for glyph in run.glyphs {
            max_right = max_right.max(glyph.x + glyph.w);
        }
    }

    if max_bottom <= 0.0 {
        max_bottom = buffer.metrics().line_height.max(1.0);
    }

    (
        max_right.ceil().max(1.0) as usize,
        max_bottom.ceil().max(1.0) as usize,
    )
}

fn measure_borrowed_buffer_scroll_metrics(
    buffer: &mut BorrowedWithFontSystem<'_, Buffer>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> EditorScrollMetrics {
    let metrics = buffer.metrics();
    let scroll = buffer.scroll();
    let mut max_right = 0.0_f32;
    let mut max_bottom = 0.0_f32;
    let mut line_top = 0.0_f32;
    let mut current_vertical_scroll_px = 0.0_f32;
    let line_count = buffer.lines.len();

    for line_i in 0..line_count {
        if line_i == scroll.line {
            current_vertical_scroll_px = line_top + scroll.vertical.max(0.0);
        }

        let line_text = buffer.lines[line_i].text().to_owned();
        let Some(layout_lines) = buffer.line_layout(line_i) else {
            continue;
        };
        for layout_line in layout_lines {
            let line_height = layout_line.line_height_opt.unwrap_or(metrics.line_height);
            max_bottom = max_bottom.max(line_top + line_height);
            let prefixes = collect_glyph_spacing_prefixes_px(
                &line_text,
                &layout_line.glyphs,
                fundamentals,
                scale,
            );
            for (glyph_index, glyph) in layout_line.glyphs.iter().enumerate() {
                max_right = max_right.max(adjusted_glyph_right_px(glyph, prefixes[glyph_index]));
            }
            line_top += line_height;
        }
    }

    if scroll.line >= line_count {
        current_vertical_scroll_px = max_bottom.max(0.0);
    }

    if max_bottom <= 0.0 {
        max_bottom = metrics.line_height.max(1.0);
    }

    let content_width_px = max_right.ceil().max(1.0);
    let content_height_px = max_bottom.ceil().max(1.0);
    let viewport_width_px = buffer.size().0.unwrap_or(content_width_px).max(1.0);
    let viewport_height_px = buffer.size().1.unwrap_or(content_height_px).max(1.0);
    let max_horizontal_scroll_px = (content_width_px - viewport_width_px).max(0.0);
    let max_vertical_scroll_px = (content_height_px - viewport_height_px).max(0.0);

    EditorScrollMetrics {
        current_horizontal_scroll_px: scroll.horizontal.clamp(0.0, max_horizontal_scroll_px),
        max_horizontal_scroll_px,
        current_vertical_scroll_px: current_vertical_scroll_px.clamp(0.0, max_vertical_scroll_px),
        max_vertical_scroll_px,
    }
}

fn clamp_borrowed_buffer_scroll(
    buffer: &mut BorrowedWithFontSystem<'_, Buffer>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> EditorScrollMetrics {
    let mut scroll_metrics = measure_borrowed_buffer_scroll_metrics(buffer, fundamentals, scale);
    let mut scroll = buffer.scroll();
    let clamped_horizontal = scroll
        .horizontal
        .clamp(0.0, scroll_metrics.max_horizontal_scroll_px);
    if (clamped_horizontal - scroll.horizontal).abs() > f32::EPSILON {
        scroll.horizontal = clamped_horizontal;
        buffer.set_scroll(scroll);
        buffer.shape_until_scroll(true);
    }
    scroll_metrics.current_horizontal_scroll_px = clamped_horizontal;
    scroll_metrics
}

fn viewer_scrollbar_track_rects(
    scroll_style: egui::style::ScrollStyle,
    widget_hovered: bool,
    widget_active: bool,
    content_rect: Rect,
    scroll_metrics: EditorScrollMetrics,
) -> ViewerScrollbarTracks {
    let show_horizontal = scroll_metrics.max_horizontal_scroll_px > f32::EPSILON;
    let show_vertical = scroll_metrics.max_vertical_scroll_px > f32::EPSILON;
    if !show_horizontal && !show_vertical {
        return ViewerScrollbarTracks::default();
    }

    let bar_width = if scroll_style.floating && !widget_hovered && !widget_active {
        scroll_style
            .floating_width
            .max(scroll_style.floating_allocated_width)
            .max(2.0)
    } else {
        scroll_style.bar_width.max(2.0)
    };
    let inner_margin = if scroll_style.floating {
        scroll_style.bar_inner_margin
    } else {
        scroll_style.bar_inner_margin.max(1.0)
    };
    let outer_margin = if scroll_style.floating {
        0.0
    } else {
        scroll_style.bar_outer_margin
    };

    ViewerScrollbarTracks {
        vertical: if show_vertical {
            let min_x = content_rect.max.x - outer_margin - bar_width;
            let max_x = content_rect.max.x - outer_margin;
            let max_y = if show_horizontal {
                content_rect.max.y - outer_margin - bar_width - inner_margin
            } else {
                content_rect.max.y - outer_margin
            };
            let min_y = content_rect.min.y + inner_margin;
            Some(Rect::from_min_max(
                Pos2::new(min_x, min_y),
                Pos2::new(max_x, max_y),
            ))
        } else {
            None
        },
        horizontal: if show_horizontal {
            let min_y = content_rect.max.y - outer_margin - bar_width;
            let max_y = content_rect.max.y - outer_margin;
            let max_x = if show_vertical {
                content_rect.max.x - outer_margin - bar_width - inner_margin
            } else {
                content_rect.max.x - outer_margin
            };
            let min_x = content_rect.min.x + inner_margin;
            Some(Rect::from_min_max(
                Pos2::new(min_x, min_y),
                Pos2::new(max_x, max_y),
            ))
        } else {
            None
        },
    }
}

fn viewer_visible_text_rect(
    content_rect: Rect,
    scroll_metrics: EditorScrollMetrics,
) -> Option<Rect> {
    let viewport_width = content_rect.width().max(1.0);
    let viewport_height = content_rect.height().max(1.0);
    let content_width = viewport_width + scroll_metrics.max_horizontal_scroll_px;
    let content_height = viewport_height + scroll_metrics.max_vertical_scroll_px;
    let visible_width =
        (content_width - scroll_metrics.current_horizontal_scroll_px).clamp(0.0, viewport_width);
    let visible_height =
        (content_height - scroll_metrics.current_vertical_scroll_px).clamp(0.0, viewport_height);

    if visible_width <= f32::EPSILON || visible_height <= f32::EPSILON {
        None
    } else {
        Some(Rect::from_min_size(
            content_rect.min,
            egui::vec2(visible_width, visible_height),
        ))
    }
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

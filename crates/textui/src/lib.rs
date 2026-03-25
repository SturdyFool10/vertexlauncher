use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::mpsc,
};

use cosmic_text::{
    Action, Attrs, AttrsOwned, BorrowedWithFontSystem, Buffer, Color, Cursor, Edit, Editor, Family,
    FontFeatures, FontSystem, Metrics, Motion, Selection, Shaping, Style as FontStyle, SwashCache,
    Weight, Wrap,
};
use egui::{
    self, Color32, ColorImage, Context, CornerRadius, Id, Key, Pos2, Rect, Response, Sense,
    TextureHandle, TextureOptions, Ui, Vec2,
};
use launcher_runtime as tokio_runtime;
use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, Options as MdOptions, Parser, Tag, TagEnd,
};
use skrifa::raw::{FontRef as SkrifaFontRef, TableProvider as _};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle as SyntectFontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use tracing::warn;

mod button_options;
mod code_block_options;
mod input_options;
mod label_options;
mod markdown_options;
mod text_helpers;
mod tooltip_options;

pub use button_options::ButtonOptions;
pub use code_block_options::CodeBlockOptions;
pub use input_options::InputOptions;
pub use label_options::LabelOptions;
pub use markdown_options::MarkdownOptions;
pub use text_helpers::{
    normalize_inline_whitespace, truncate_single_line_text_with_ellipsis,
    truncate_single_line_text_with_ellipsis_preserving_whitespace,
};
pub use tooltip_options::TooltipOptions;

const DEFAULT_OPEN_TYPE_FEATURE_TAGS: &str = "liga, calt";

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

/// A rasterized text texture with helpers for all paint scenarios.
///
/// Obtain via [`TextUi::prepare_label_texture`] or
/// [`TextUi::prepare_rich_text_texture`].  You can:
///
/// - Call `handle.paint(ui, rect)` for standard rendering.
/// - Call `handle.paint_tinted(ui, rect, tint)` for alpha-fade or colourisation.
/// - Call `handle.paint_uv(ui, rect, uv, tint)` for UV crop/flip/repeat.
/// - Pass `handle.texture.id()` into a wgpu `PaintCallback` to use the glyph
///   image as a shader mask, stencil, or any texture role — zero restrictions.
#[derive(Clone)]
pub struct TextTextureHandle {
    /// The egui texture backing the rasterized text.
    pub texture: TextureHandle,
    /// Logical (points) size of the rendered text content.
    pub size_points: Vec2,
}

impl TextTextureHandle {
    /// Paint the texture in `rect` with no tint (white = pass-through).
    pub fn paint(&self, ui: &Ui, rect: Rect) {
        let rect = snap_rect_to_pixel_grid(rect, ui.ctx().pixels_per_point());
        ui.painter().image(
            self.texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    /// Paint with a tint multiplier.  `Color32::WHITE` = unmodified.
    pub fn paint_tinted(&self, ui: &Ui, rect: Rect, tint: Color32) {
        let rect = snap_rect_to_pixel_grid(rect, ui.ctx().pixels_per_point());
        ui.painter().image(
            self.texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            tint,
        );
    }

    /// Paint a UV sub-region with a tint.  Full UV = `Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0,1.0))`.
    pub fn paint_uv(&self, ui: &Ui, rect: Rect, uv: Rect, tint: Color32) {
        let rect = snap_rect_to_pixel_grid(rect, ui.ctx().pixels_per_point());
        ui.painter().image(self.texture.id(), rect, uv, tint);
    }

    /// Paint on a specific egui `Painter` (e.g. a layer painter for overlays).
    pub fn paint_on(&self, painter: &egui::Painter, rect: Rect, tint: Color32) {
        let pixels_per_point = painter.ctx().pixels_per_point();
        let rect = snap_rect_to_pixel_grid(rect, pixels_per_point);
        painter.image(
            self.texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            tint,
        );
    }
}

#[derive(Clone, Debug)]
pub struct RichTextStyle {
    pub color: Color32,
    pub monospace: bool,
    pub italic: bool,
    pub weight: u16,
}

impl Default for RichTextStyle {
    fn default() -> Self {
        Self {
            color: Color32::WHITE,
            monospace: false,
            italic: false,
            weight: 400,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RichTextSpan {
    pub text: String,
    pub style: RichTextStyle,
}

type SpanStyle = RichTextStyle;
type RichSpan = RichTextSpan;

#[derive(Clone, Debug)]
struct RasterizedText {
    image: ColorImage,
    size_points: Vec2,
}

#[derive(Clone, Debug)]
struct RasterizedTile {
    image: ColorImage,
    offset_points: Vec2,
    size_points: Vec2,
}

struct TextureEntry {
    fingerprint: u64,
    texture: TextureHandle,
    size_points: Vec2,
    last_used_frame: u64,
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
    max_texture_side_px: usize,
    scale: f32,
    typography: TypographySnapshot,
}

#[derive(Clone, Debug)]
struct AsyncRasterResponse {
    key_hash: u64,
    raster: RasterizedText,
}

#[derive(Clone, Debug)]
struct TypographySnapshot {
    ui_font_family: Option<String>,
    ui_font_size_scale: f32,
    ui_font_weight: i32,
    open_type_features_enabled: bool,
    open_type_features_to_enable: String,
}

struct AsyncRasterState {
    tx: Option<mpsc::Sender<AsyncRasterWorkerMessage>>,
    rx: Option<mpsc::Receiver<AsyncRasterResponse>>,
    pending: HashSet<u64>,
    cache: HashMap<u64, RasterizedText>,
}

enum AsyncRasterWorkerMessage {
    RegisterFont(Vec<u8>),
    Render(AsyncRasterRequest),
}

#[derive(Debug)]
struct InputState {
    editor: Editor<'static>,
    last_text: String,
    attrs_fingerprint: u64,
    multiline: bool,
    scroll_metrics: EditorScrollMetrics,
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

#[derive(Clone, Debug)]
enum MarkdownBlock {
    Heading {
        level: HeadingLevel,
        text: String,
    },
    Paragraph(String),
    Code {
        language: Option<String>,
        text: String,
    },
}

/// High-level text rendering helper built on cosmic-text + egui textures.
pub struct TextUi {
    font_system: FontSystem,
    swash_cache: SwashCache,
    syntax_set: SyntaxSet,
    code_theme: Theme,
    textures: HashMap<Id, TextureEntry>,
    input_states: HashMap<Id, InputState>,
    ui_font_family: Option<String>,
    ui_font_size_scale: f32,
    ui_font_weight: i32,
    open_type_features_enabled: bool,
    open_type_features_to_enable: String,
    open_type_features: Option<FontFeatures>,
    async_raster: AsyncRasterState,
    current_frame: u64,
    max_texture_side_px: usize,
    /// Cache for parsed markdown blocks: Id → (fingerprint, blocks).
    /// Prevents re-parsing unchanged markdown every frame.
    markdown_cache: HashMap<Id, (u64, Vec<MarkdownBlock>)>,
}

impl Default for TextUi {
    fn default() -> Self {
        Self::new()
    }
}

impl TextUi {
    /// Creates a new text renderer and background async raster worker.
    pub fn new() -> Self {
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

        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            syntax_set,
            code_theme,
            textures: HashMap::new(),
            input_states: HashMap::new(),
            ui_font_family: None,
            ui_font_size_scale: 1.0,
            ui_font_weight: 400,
            open_type_features_enabled: false,
            open_type_features_to_enable: String::new(),
            open_type_features: None,
            async_raster: AsyncRasterState {
                tx: worker_tx,
                rx: result_rx,
                pending: HashSet::new(),
                cache: HashMap::new(),
            },
            current_frame: 0,
            max_texture_side_px: usize::MAX,
            markdown_cache: HashMap::new(),
        }
    }

    /// Performs per-frame maintenance and processes async raster results.
    pub fn begin_frame(&mut self, ctx: &Context) {
        self.current_frame = ctx.cumulative_frame_nr();
        let max_texture_side_px = ctx.input(|i| i.max_texture_side).max(1);
        if self.max_texture_side_px != max_texture_side_px {
            self.max_texture_side_px = max_texture_side_px;
            self.invalidate_text_caches(false);
        }
        self.textures
            .retain(|_, entry| self.current_frame.saturating_sub(entry.last_used_frame) <= 600);
        self.poll_async_raster_results();
    }

    /// Registers additional font bytes for rendering.
    ///
    /// This clears cached textures/input states so new faces are picked up.
    pub fn register_font_data(&mut self, bytes: Vec<u8>) {
        if let Some(tx) = self.async_raster.tx.as_ref() {
            let _ = tx.send(AsyncRasterWorkerMessage::RegisterFont(bytes.clone()));
        }
        self.font_system.db_mut().load_font_data(bytes);
        self.invalidate_text_caches(true);
    }

    /// Renders an asynchronously rasterized label.
    pub fn label_async(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        self.label_impl(ui, id_source, text, options, Sense::hover(), true)
    }

    /// Renders an asynchronously rasterized syntax-highlighted code block.
    pub fn code_block_async(
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
            self.highlight_code_spans(code, options.language.as_deref(), options.text_color);
        let label_options = LabelOptions {
            font_size: options.font_size,
            line_height: options.line_height,
            color: options.text_color,
            wrap: options.wrap,
            monospace: true,
            weight: 400,
            italic: false,
            padding: egui::Vec2::ZERO,
        };

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "code_async".hash(&mut hasher);
        code.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.text_color.hash(&mut hasher);
        options.background_color.hash(&mut hasher);
        options.language.hash(&mut hasher);
        width_points_opt
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();
        let texture_id = ui.make_persistent_id(id_source).with("textui_code");

        let raster = self.get_or_queue_async_rich_raster(
            fingerprint,
            spans,
            &label_options,
            width_points_opt,
            scale,
        );

        if let Some(raster) = raster {
            let desired_size = raster.size_points + options.padding * 2.0;
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

            let texture = self.update_texture(
                ui.ctx(),
                texture_id,
                fingerprint,
                raster.image,
                raster.size_points,
            );
            let image_rect = Rect::from_min_size(rect.min + options.padding, raster.size_points);
            paint_texture(ui, &texture, image_rect);
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
            && self.open_type_features == active_features
        {
            return;
        }

        self.open_type_features_enabled = enabled;
        self.open_type_features_to_enable = normalized_csv;
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
    pub fn label(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        self.label_impl(ui, id_source, text, options, Sense::hover(), false)
    }

    /// Renders a clickable label synchronously.
    pub fn clickable_label(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        self.label_impl(ui, id_source, text, options, Sense::click(), false)
    }

    /// Measures rendered size of text for the provided style options.
    pub fn measure_text_size(&mut self, ui: &Ui, text: &str, options: &LabelOptions) -> Vec2 {
        let scale = ui.ctx().pixels_per_point();
        let metrics = Metrics::new(
            (self.effective_font_size(options.font_size) * scale).max(1.0),
            (self.effective_line_height(options.line_height) * scale).max(1.0),
        );
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs_owned = self.build_text_attrs_owned(
            &SpanStyle {
                color: options.color,
                monospace: options.monospace,
                italic: options.italic,
                weight: options.weight,
            },
            options.font_size,
            options.line_height,
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
        egui::vec2(width_px as f32 / scale, height_px as f32 / scale)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Raw-texture API  — zero restrictions on how you consume the result
    //
    // Returns a `TextTextureHandle` containing the egui `TextureHandle` and
    // logical size.  Call `.paint()` for the standard path, `.paint_tinted()`
    // for alpha-fade/colourisation, `.paint_uv()` for UV-crop/flip, or pass
    // `.texture.id()` directly into a wgpu PaintCallback to use the glyph
    // image as a shader mask, stencil, or any other texture role.
    // ─────────────────────────────────────────────────────────────────────────

    /// Returns (or freshly rasterizes) a cached texture for plain text.
    ///
    /// The texture is **not** painted — you control every aspect of rendering.
    pub fn prepare_label_texture(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle {
        let scale = ctx.pixels_per_point();
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "prepare_label".hash(&mut hasher);
        text.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
        options.weight.hash(&mut hasher);
        options.italic.hash(&mut hasher);
        options.color.hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        let texture_id = egui::Id::new(id_source).with("textui_prepare_label");
        if let Some((texture, size_points)) = self.get_cached_texture(texture_id, fingerprint) {
            return TextTextureHandle {
                texture,
                size_points,
            };
        }

        let raster = self.rasterize_plain_text(text, options, binned_width, scale);
        let size_points = raster.size_points;
        let texture = self.update_texture(ctx, texture_id, fingerprint, raster.image, size_points);
        TextTextureHandle {
            texture,
            size_points,
        }
    }

    /// Returns (or freshly rasterizes) a cached texture for rich (multi-style) text.
    ///
    /// Same zero-restriction guarantees as [`prepare_label_texture`].
    pub fn prepare_rich_text_texture(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle {
        let scale = ctx.pixels_per_point();
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
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
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        let texture_id = egui::Id::new(id_source).with("textui_prepare_rich");
        if let Some((texture, size_points)) = self.get_cached_texture(texture_id, fingerprint) {
            return TextTextureHandle {
                texture,
                size_points,
            };
        }

        let raster = self.rasterize_rich_text(spans, options, binned_width, scale);
        let size_points = raster.size_points;
        let texture = self.update_texture(ctx, texture_id, fingerprint, raster.image, size_points);
        TextTextureHandle {
            texture,
            size_points,
        }
    }

    fn label_impl(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        sense: Sense,
        async_mode: bool,
    ) -> Response {
        let scale = ui.ctx().pixels_per_point();
        // Snap available_width to bin boundaries so sub-pixel jitter
        // (scrollbars appearing, fractional DPI) does not bust the cache for
        // every label on screen simultaneously.
        let width_points_opt = if options.wrap {
            Some(snap_width_to_bin(ui.available_width().max(1.0), scale))
        } else {
            None
        };

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "label".hash(&mut hasher);
        text.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
        options.weight.hash(&mut hasher);
        options.italic.hash(&mut hasher);
        options.color.hash(&mut hasher);
        width_points_opt
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();
        let texture_id = ui.make_persistent_id(id_source).with("textui_label");
        if let Some((texture, size_points)) = self.get_cached_texture(texture_id, fingerprint) {
            let desired_size = size_points + options.padding * 2.0;
            let (rect, response) = ui.allocate_exact_size(desired_size, sense);
            let image_rect = Rect::from_min_size(rect.min + options.padding, size_points);
            paint_texture(ui, &texture, image_rect);
            return response;
        }

        let raster = if async_mode {
            self.get_or_queue_async_plain_raster(
                fingerprint,
                text.to_owned(),
                options,
                width_points_opt,
                scale,
            )
        } else {
            Some(self.rasterize_plain_text(text, options, width_points_opt, scale))
        };
        let Some(raster) = raster else {
            let fallback_height = (options.line_height + options.padding.y * 2.0).max(20.0);
            let fallback_width = width_points_opt.unwrap_or_else(|| ui.available_width().max(1.0));
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(fallback_width, fallback_height), sense);
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), ui.visuals().faint_bg_color);
            ui.ctx().request_repaint();
            return response;
        };
        let desired_size = raster.size_points + options.padding * 2.0;
        let (rect, response) = ui.allocate_exact_size(desired_size, sense);

        let texture = self.update_texture(
            ui.ctx(),
            texture_id,
            fingerprint,
            raster.image,
            raster.size_points,
        );
        let image_rect = Rect::from_min_size(rect.min + options.padding, raster.size_points);
        paint_texture(ui, &texture, image_rect);

        response
    }

    /// Renders a button with text styles from [`ButtonOptions`].
    pub fn button(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &ButtonOptions,
    ) -> Response {
        self.button_impl(ui, id_source, text, false, options)
    }

    /// Renders a selectable button variant.
    pub fn selectable_button(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        selected: bool,
        options: &ButtonOptions,
    ) -> Response {
        self.button_impl(ui, id_source, text, selected, options)
    }

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

        // ── Text fingerprint excludes hover/active ─────────────────────────────
        // hover/active only change the background fill, never the text pixels.
        // Including them caused a re-rasterize on every pointer-enter/-leave.
        let text_tex_id = ui.make_persistent_id(id_source).with("button_text");
        let text_fingerprint = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            "textui_button".hash(&mut hasher);
            text.hash(&mut hasher);
            selected.hash(&mut hasher);
            options.font_size.to_bits().hash(&mut hasher);
            options.line_height.to_bits().hash(&mut hasher);
            options.text_color.hash(&mut hasher);
            self.hash_typography(&mut hasher);
            hasher.finish()
        };

        // ── Cache-first: rasterize only on miss ───────────────────────────────
        let (text_size, texture) =
            if let Some((tex, sz)) = self.get_cached_texture(text_tex_id, text_fingerprint) {
                (sz, tex)
            } else {
                let raster = self.rasterize_plain_text(text, &label_style, None, scale);
                let sz = raster.size_points;
                let tex =
                    self.update_texture(ui.ctx(), text_tex_id, text_fingerprint, raster.image, sz);
                (sz, tex)
            };

        let desired_size = egui::vec2(
            (text_size.x + options.padding.x * 2.0).max(options.min_size.x),
            (text_size.y + options.padding.y * 2.0).max(options.min_size.y),
        );

        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());

        let fill = if response.is_pointer_button_down_on() {
            options.fill_active
        } else if response.hovered() {
            options.fill_hovered
        } else if selected {
            options.fill_selected
        } else {
            options.fill
        };

        ui.painter()
            .rect_filled(rect, CornerRadius::same(options.corner_radius), fill);
        if options.stroke.width > 0.0 {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(options.corner_radius),
                options.stroke,
                egui::StrokeKind::Inside,
            );
        }

        let text_rect = Rect::from_center_size(rect.center(), text_size);
        paint_texture(ui, &texture, text_rect);

        response
    }

    /// Shows a tooltip while the provided response is hovered.
    pub fn tooltip_for_response(
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
        let tooltip_fingerprint = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            "textui_tooltip".hash(&mut hasher);
            text.hash(&mut hasher);
            options.text.font_size.to_bits().hash(&mut hasher);
            options.text.line_height.to_bits().hash(&mut hasher);
            options.text.color.hash(&mut hasher);
            width_points_opt
                .map(f32::to_bits)
                .unwrap_or(0)
                .hash(&mut hasher);
            self.hash_typography(&mut hasher);
            hasher.finish()
        };

        let (texture, raster_size) = if let Some((tex, sz)) =
            self.get_cached_texture(tooltip_tex_id, tooltip_fingerprint)
        {
            (tex, sz)
        } else {
            let raster = self.rasterize_plain_text(text, &options.text, width_points_opt, scale);
            let sz = raster.size_points;
            let tex = self.update_texture(
                ui.ctx(),
                tooltip_tex_id,
                tooltip_fingerprint,
                raster.image,
                sz,
            );
            (tex, sz)
        };

        let size = raster_size + options.padding * 2.0;
        let mut rect = Rect::from_min_size(pointer + options.offset, size);
        let min_y = ui.clip_rect().top();
        if rect.min.y < min_y {
            let delta = min_y - rect.min.y;
            rect = rect.translate(egui::vec2(0.0, delta));
        }

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

        let text_rect = Rect::from_min_size(rect.min + options.padding, raster_size);
        painter.image(
            texture.id(),
            text_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    /// Renders a syntax-highlighted code block synchronously.
    pub fn code_block(
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

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "code".hash(&mut hasher);
        code.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.text_color.hash(&mut hasher);
        options.background_color.hash(&mut hasher);
        options.language.hash(&mut hasher);
        width_points_opt
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();
        let texture_id = ui.make_persistent_id(id_source).with("textui_code");
        if let Some((texture, size_points)) = self.get_cached_texture(texture_id, fingerprint) {
            let desired_size = size_points + options.padding * 2.0;
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

            let image_rect = Rect::from_min_size(rect.min + options.padding, size_points);
            paint_texture(ui, &texture, image_rect);
            return response;
        }

        let spans =
            self.highlight_code_spans(code, options.language.as_deref(), options.text_color);

        let raster = self.rasterize_rich_text(
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
            },
            width_points_opt,
            scale,
        );

        let desired_size = raster.size_points + options.padding * 2.0;
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

        let texture = self.update_texture(
            ui.ctx(),
            texture_id,
            fingerprint,
            raster.image,
            raster.size_points,
        );
        let image_rect = Rect::from_min_size(rect.min + options.padding, raster.size_points);
        paint_texture(ui, &texture, image_rect);

        response
    }

    /// Renders simple markdown (headings, paragraphs, fenced code).
    pub fn markdown(
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
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
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

        let blocks = if let Some((fp, cached)) = self.markdown_cache.get(&cache_id) {
            if *fp == md_fingerprint {
                cached.clone()
            } else {
                let b = parse_markdown_blocks(markdown);
                self.markdown_cache
                    .insert(cache_id, (md_fingerprint, b.clone()));
                b
            }
        } else {
            let b = parse_markdown_blocks(markdown);
            self.markdown_cache
                .insert(cache_id, (md_fingerprint, b.clone()));
            b
        };

        ui.push_id(id_source, |ui| {
            for (index, block) in blocks.iter().enumerate() {
                match block {
                    MarkdownBlock::Heading { level, text } => {
                        let factor = match level {
                            HeadingLevel::H1 => options.heading_scale + 0.26,
                            HeadingLevel::H2 => options.heading_scale + 0.12,
                            HeadingLevel::H3 => options.heading_scale,
                            HeadingLevel::H4 => options.heading_scale - 0.08,
                            HeadingLevel::H5 => options.heading_scale - 0.12,
                            HeadingLevel::H6 => options.heading_scale - 0.16,
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
                        };
                        let _ = self.label(ui, ("md_h", index), text, &heading_style);
                    }
                    MarkdownBlock::Paragraph(text) => {
                        let _ = self.label(ui, ("md_p", index), text, &options.body);
                    }
                    MarkdownBlock::Code { language, text } => {
                        let mut code_options = options.code.clone();
                        code_options.language = language.clone();
                        let _ = self.code_block(ui, ("md_code", index), text, &code_options);
                    }
                }

                if index + 1 < blocks.len() {
                    ui.add_space(options.paragraph_spacing);
                }
            }
        });
    }

    /// Renders a single-line editable text field.
    pub fn singleline_input(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        self.input_widget(ui, id_source, text, options, false)
    }

    /// Renders a multi-line editable text field.
    pub fn multiline_input(
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
    pub fn multiline_rich_viewer(
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
        let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

        let has_focus = response.has_focus();
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

        let base_fingerprint =
            rich_viewer_texture_fingerprint(&state.editor, &text, spans, options, false, wrap);
        for (tile_index, tile) in self
            .rasterize_editor_tiled(
                &state.editor,
                options,
                content_width_px as usize,
                content_height_px as usize,
                scale,
                false,
                true,
            )
            .into_iter()
            .enumerate()
        {
            let texture = self.update_texture(
                ui.ctx(),
                id.with(("tile", tile_index)),
                {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    base_fingerprint.hash(&mut hasher);
                    tile_index.hash(&mut hasher);
                    hasher.finish()
                },
                tile.image,
                tile.size_points,
            );
            let tile_rect =
                Rect::from_min_size(content_rect.min + tile.offset_points, tile.size_points);
            paint_texture(ui, &texture, tile_rect);
        }

        state_changed |= self.sync_viewer_scrollbars(
            ui,
            id,
            &mut state.editor,
            content_rect,
            scale,
            &mut state.scroll_metrics,
        );

        self.input_states.insert(id, state);
        if state_changed {
            response.mark_changed();
        }

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
        let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

        if response.hovered() || response.has_focus() {
            ui.output_mut(|o| {
                o.cursor_icon = egui::CursorIcon::Text;
                o.mutable_text_under_cursor = true;
            });
        }

        if response.clicked() {
            response.request_focus();
        }

        let has_focus = response.has_focus();
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
            changed = self.handle_input_events(
                ui,
                &response,
                &mut state.editor,
                multiline,
                content_rect,
                scale,
                has_focus,
                &mut state.scroll_metrics,
            );

            if !multiline && ui.input(|i| i.key_pressed(Key::Enter)) {
                response.surrender_focus();
            }
        }

        let latest_text = editor_to_string(&state.editor);
        if latest_text != *text {
            *text = latest_text.clone();
            state.last_text = latest_text;
            changed = true;
        }

        if changed {
            response.mark_changed();
        }

        let image = self.rasterize_editor(
            &state.editor,
            options,
            content_width_px as usize,
            content_height_px as usize,
            has_focus,
        );

        let fingerprint = input_texture_fingerprint(&state.editor, text, options, has_focus);

        let texture = self.update_texture(
            ui.ctx(),
            id.with("tex"),
            fingerprint,
            image,
            content_rect.size(),
        );
        self.input_states.insert(id, state);

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

        paint_texture(ui, &texture, content_rect);

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
            scroll_metrics: EditorScrollMetrics::default(),
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
            scroll_metrics = clamp_borrowed_buffer_scroll(&mut borrowed);
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
            scroll_metrics = clamp_borrowed_buffer_scroll(&mut borrowed);
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
            scroll_metrics = clamp_borrowed_buffer_scroll(&mut borrowed);
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
            scroll_metrics = clamp_borrowed_buffer_scroll(&mut borrowed);
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
        process_keyboard: bool,
        pointer_over_scrollbar: bool,
        scroll_metrics: &mut EditorScrollMetrics,
    ) -> bool {
        let mut changed = false;
        let (modifiers, primary_pressed, smooth_scroll_delta, raw_scroll_delta) =
            ui.ctx().input(|i| {
                (
                    i.modifiers,
                    i.pointer.primary_pressed(),
                    i.smooth_scroll_delta,
                    i.raw_scroll_delta,
                )
            });
        let pointer_pressed_on_widget = primary_pressed && response.is_pointer_button_down_on();
        let horizontal_scroll = editor_horizontal_scroll(editor);

        if !pointer_over_scrollbar && let Some(pointer_pos) = response.interact_pointer_pos() {
            let x =
                (((pointer_pos.x - content_rect.min.x) * scale) + horizontal_scroll).round() as i32;
            let y = ((pointer_pos.y - content_rect.min.y) * scale).round() as i32;

            if response.triple_clicked() {
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::TripleClick { x, y });
                changed = true;
            } else if response.double_clicked() {
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::DoubleClick { x, y });
                changed = true;
            } else if pointer_pressed_on_widget {
                if modifiers.shift {
                    changed |= extend_selection_to_pointer(editor, x, y);
                } else {
                    editor
                        .borrow_with(&mut self.font_system)
                        .action(Action::Click { x, y });
                    changed = true;
                }
            } else if response.clicked() {
                if modifiers.shift {
                    changed |= extend_selection_to_pointer(editor, x, y);
                } else {
                    editor
                        .borrow_with(&mut self.font_system)
                        .action(Action::Click { x, y });
                    changed = true;
                }
            }

            if response.dragged() {
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::Drag { x, y });
                changed = true;
            }
        }

        if response.hovered() {
            let vertical_scroll_delta = if smooth_scroll_delta.y.abs() > f32::EPSILON {
                smooth_scroll_delta.y
            } else {
                raw_scroll_delta.y
            };
            let horizontal_scroll_delta = if smooth_scroll_delta.x.abs() > f32::EPSILON {
                smooth_scroll_delta.x
            } else if raw_scroll_delta.x.abs() > f32::EPSILON {
                raw_scroll_delta.x
            } else if modifiers.shift && smooth_scroll_delta.y.abs() > f32::EPSILON {
                smooth_scroll_delta.y
            } else if modifiers.shift {
                raw_scroll_delta.y
            } else {
                0.0
            };
            let horizontal_uses_vertical_wheel = modifiers.shift
                && smooth_scroll_delta.x.abs() <= f32::EPSILON
                && raw_scroll_delta.x.abs() <= f32::EPSILON
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
            let events = ui.ctx().input(|i| i.events.clone());
            for event in events {
                match event {
                    egui::Event::Copy | egui::Event::Cut => {
                        if let Some(selection) = editor.copy_selection() {
                            ui.ctx().copy_text(selection);
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } if pressed => {
                        changed |= handle_read_only_editor_key_event(
                            &mut self.font_system,
                            editor,
                            key,
                            modifiers,
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
            *scroll_metrics = self.measure_editor_scroll_metrics(editor);
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
    ) -> EditorScrollMetrics {
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            measure_borrowed_buffer_scroll_metrics(&mut borrowed)
        })
    }

    fn sync_viewer_scrollbars(
        &mut self,
        ui: &mut Ui,
        id: Id,
        editor: &mut Editor<'static>,
        content_rect: Rect,
        scale: f32,
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
        *scroll_metrics = self.measure_editor_scroll_metrics(editor);
        ui.ctx().request_repaint();
        true
    }

    fn rasterize_editor_tiled(
        &mut self,
        editor: &Editor<'static>,
        options: &InputOptions,
        width_px: usize,
        height_px: usize,
        scale: f32,
        has_focus: bool,
        show_selection_without_focus: bool,
    ) -> Vec<RasterizedTile> {
        let width_px = width_px.max(1);
        let height_px = height_px.max(1);
        let horizontal_scroll = editor_horizontal_scroll(editor).round() as i32;
        let tile_max_dim_px = self.max_texture_side_px.max(1);
        let tile_cols = width_px.div_ceil(tile_max_dim_px);
        let tile_rows = height_px.div_ceil(tile_max_dim_px);
        let tile_count = tile_cols * tile_rows;

        let mut tiles = Vec::with_capacity(tile_count);
        for row in 0..tile_rows {
            for col in 0..tile_cols {
                let origin_x = col * tile_max_dim_px;
                let origin_y = row * tile_max_dim_px;
                let tile_width = (width_px - origin_x).min(tile_max_dim_px);
                let tile_height = (height_px - origin_y).min(tile_max_dim_px);
                tiles.push((
                    origin_x,
                    origin_y,
                    ColorImage::new(
                        [tile_width, tile_height],
                        vec![Color32::TRANSPARENT; tile_width * tile_height],
                    ),
                ));
            }
        }

        let selection_visible =
            has_focus || (show_selection_without_focus && editor.selection() != Selection::None);
        editor.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            to_cosmic_color(options.text_color),
            if has_focus {
                to_cosmic_color(options.cursor_color)
            } else {
                to_cosmic_color(Color32::TRANSPARENT)
            },
            if selection_visible {
                to_cosmic_color(options.selection_color)
            } else {
                to_cosmic_color(Color32::TRANSPARENT)
            },
            if selection_visible {
                to_cosmic_color(options.selected_text_color)
            } else {
                to_cosmic_color(options.text_color)
            },
            |x, y, w, h, color| {
                blend_rect_into_tiles(
                    &mut tiles,
                    width_px,
                    height_px,
                    x - horizontal_scroll,
                    y,
                    w as i32,
                    h as i32,
                    cosmic_to_egui_color(color),
                );
            },
        );

        tiles
            .into_iter()
            .map(|(origin_x, origin_y, image)| RasterizedTile {
                offset_points: egui::vec2(origin_x as f32 / scale, origin_y as f32 / scale),
                size_points: egui::vec2(image.size[0] as f32 / scale, image.size[1] as f32 / scale),
                image,
            })
            .collect()
    }

    fn input_span_attrs_owned(
        &self,
        style: &RichTextStyle,
        options: &InputOptions,
        scale: f32,
    ) -> AttrsOwned {
        let mut attrs = Attrs::new()
            .color(to_cosmic_color(style.color))
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
        if let Some(features) = &self.open_type_features {
            attrs = attrs.font_features(features.clone());
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
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "rich_viewer_attrs".hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        scale.to_bits().hash(&mut hasher);
        wrap.hash(&mut hasher);
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
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::TripleClick { x, y });
                changed = true;
            } else if response.double_clicked() {
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::DoubleClick { x, y });
                changed = true;
            } else if response.clicked() {
                if modifiers.shift {
                    changed |= extend_selection_to_pointer(editor, x, y);
                } else {
                    editor
                        .borrow_with(&mut self.font_system)
                        .action(Action::Click { x, y });
                    changed = true;
                }
            }

            if response.dragged() {
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::Drag { x, y });
                changed = true;
            }
        }

        if process_keyboard {
            let events = ui.ctx().input(|i| i.events.clone());
            for event in events {
                match event {
                    egui::Event::Text(mut text) => {
                        if !multiline {
                            text = text.replace(['\n', '\r'], "");
                        }
                        if !text.is_empty() {
                            editor.insert_string(&text, None);
                            changed = true;
                        }
                    }
                    egui::Event::Copy => {
                        if let Some(selection) = editor.copy_selection() {
                            ui.ctx().copy_text(selection);
                        }
                    }
                    egui::Event::Cut => {
                        if let Some(selection) = editor.copy_selection() {
                            ui.ctx().copy_text(selection);
                            changed |= editor.delete_selection();
                        }
                    }
                    egui::Event::Paste(mut pasted) => {
                        if !multiline {
                            pasted = pasted.replace(['\n', '\r'], " ");
                        }
                        if !pasted.is_empty() {
                            editor.insert_string(&pasted, None);
                            changed = true;
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } if pressed => {
                        changed |= handle_editor_key_event(
                            &mut self.font_system,
                            editor,
                            key,
                            modifiers,
                            multiline,
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
            *scroll_metrics = self.measure_editor_scroll_metrics(editor);
        }

        changed
    }

    fn rasterize_editor(
        &mut self,
        editor: &Editor<'static>,
        options: &InputOptions,
        width_px: usize,
        height_px: usize,
        has_focus: bool,
    ) -> ColorImage {
        let horizontal_scroll = editor_horizontal_scroll(editor).round() as i32;
        let width_px = width_px.clamp(1, self.max_texture_side_px.max(1));
        let height_px = height_px.clamp(1, self.max_texture_side_px.max(1));
        let mut image = ColorImage::new(
            [width_px.max(1), height_px.max(1)],
            vec![Color32::TRANSPARENT; width_px.max(1) * height_px.max(1)],
        );

        editor.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            to_cosmic_color(options.text_color),
            if has_focus {
                to_cosmic_color(options.cursor_color)
            } else {
                to_cosmic_color(Color32::TRANSPARENT)
            },
            if has_focus {
                to_cosmic_color(options.selection_color)
            } else {
                to_cosmic_color(Color32::TRANSPARENT)
            },
            to_cosmic_color(options.selected_text_color),
            |x, y, w, h, color| {
                blend_rect(
                    &mut image,
                    x - horizontal_scroll,
                    y,
                    w as i32,
                    h as i32,
                    cosmic_to_egui_color(color),
                );
            },
        );

        image
    }

    fn highlight_code_spans(
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
                    color: fallback_color,
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
                                ),
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
                            color: fallback_color,
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

    fn rasterize_plain_text(
        &mut self,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> RasterizedText {
        let spans = vec![RichSpan {
            text: text.to_owned(),
            style: SpanStyle {
                color: options.color,
                monospace: options.monospace,
                italic: options.italic,
                weight: options.weight,
            },
        }];
        self.rasterize_rich_text(&spans, options, width_points_opt, scale)
    }

    fn rasterize_rich_text(
        &mut self,
        spans: &[RichSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> RasterizedText {
        let metrics = Metrics::new(
            (self.effective_font_size(options.font_size) * scale).max(1.0),
            (self.effective_line_height(options.line_height) * scale).max(1.0),
        );

        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let default_attrs_owned = self.build_text_attrs_owned(
            &SpanStyle {
                color: options.color,
                monospace: options.monospace,
                italic: options.italic,
                weight: options.weight,
            },
            options.font_size,
            options.line_height,
        );
        let span_attrs_owned = spans
            .iter()
            .map(|span| {
                self.build_text_attrs_owned(&span.style, options.font_size, options.line_height)
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

        let width_px = measured_width_px.clamp(1, self.max_texture_side_px.max(1));
        let height_px = measured_height_px.clamp(1, self.max_texture_side_px.max(1));

        let mut image = ColorImage::new(
            [width_px, height_px],
            vec![Color32::TRANSPARENT; width_px * height_px],
        );

        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            to_cosmic_color(options.color),
            |x, y, w, h, color| {
                blend_rect(
                    &mut image,
                    x,
                    y,
                    w as i32,
                    h as i32,
                    cosmic_to_egui_color(color),
                );
            },
        );

        RasterizedText {
            image,
            size_points: egui::vec2(width_px as f32 / scale, height_px as f32 / scale),
        }
    }

    fn typography_snapshot(&self) -> TypographySnapshot {
        TypographySnapshot {
            ui_font_family: self.ui_font_family.clone(),
            ui_font_size_scale: self.ui_font_size_scale,
            ui_font_weight: self.ui_font_weight,
            open_type_features_enabled: self.open_type_features_enabled,
            open_type_features_to_enable: self.open_type_features_to_enable.clone(),
        }
    }

    fn poll_async_raster_results(&mut self) {
        let mut should_reset_worker = false;
        let Some(rx) = self.async_raster.rx.as_ref() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(response) => {
                    self.async_raster.pending.remove(&response.key_hash);
                    self.async_raster
                        .cache
                        .insert(response.key_hash, response.raster);
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
    }

    fn invalidate_text_caches(&mut self, clear_input_states: bool) {
        self.textures.clear();
        self.async_raster.cache.clear();
        self.async_raster.pending.clear();
        self.markdown_cache.clear();
        if clear_input_states {
            self.input_states.clear();
        }
    }

    fn hash_typography<H: Hasher>(&self, state: &mut H) {
        self.ui_font_family.hash(state);
        self.ui_font_size_scale.to_bits().hash(state);
        self.ui_font_weight.hash(state);
        self.open_type_features_enabled.hash(state);
        self.open_type_features_to_enable.hash(state);
        self.max_texture_side_px.hash(state);
    }

    fn get_or_queue_async_plain_raster(
        &mut self,
        key_hash: u64,
        text: String,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<RasterizedText> {
        if let Some(raster) = self.async_raster.cache.get(&key_hash) {
            return Some(raster.clone());
        }
        let Some(tx) = self.async_raster.tx.as_ref().cloned() else {
            return Some(self.rasterize_plain_text(
                text.as_str(),
                options,
                width_points_opt,
                scale,
            ));
        };
        if self.async_raster.pending.insert(key_hash) {
            let request_text = text.clone();
            let request = AsyncRasterRequest {
                key_hash,
                kind: AsyncRasterKind::Plain(request_text),
                options: options.clone(),
                width_points_opt,
                max_texture_side_px: self.max_texture_side_px,
                scale,
                typography: self.typography_snapshot(),
            };
            if tx.send(AsyncRasterWorkerMessage::Render(request)).is_err() {
                self.async_raster.pending.remove(&key_hash);
                self.async_raster.tx = None;
                self.async_raster.rx = None;
                return Some(self.rasterize_plain_text(
                    text.as_str(),
                    options,
                    width_points_opt,
                    scale,
                ));
            }
        }
        None
    }

    fn get_or_queue_async_rich_raster(
        &mut self,
        key_hash: u64,
        spans: Vec<RichSpan>,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<RasterizedText> {
        if let Some(raster) = self.async_raster.cache.get(&key_hash) {
            return Some(raster.clone());
        }
        let Some(tx) = self.async_raster.tx.as_ref().cloned() else {
            return Some(self.rasterize_rich_text(
                spans.as_slice(),
                options,
                width_points_opt,
                scale,
            ));
        };
        if self.async_raster.pending.insert(key_hash) {
            let request_spans = spans.clone();
            let request = AsyncRasterRequest {
                key_hash,
                kind: AsyncRasterKind::Rich(request_spans),
                options: options.clone(),
                width_points_opt,
                max_texture_side_px: self.max_texture_side_px,
                scale,
                typography: self.typography_snapshot(),
            };
            if tx.send(AsyncRasterWorkerMessage::Render(request)).is_err() {
                self.async_raster.pending.remove(&key_hash);
                self.async_raster.tx = None;
                self.async_raster.rx = None;
                return Some(self.rasterize_rich_text(
                    spans.as_slice(),
                    options,
                    width_points_opt,
                    scale,
                ));
            }
        }
        None
    }

    fn get_cached_texture(&mut self, id: Id, fingerprint: u64) -> Option<(TextureHandle, Vec2)> {
        let entry = self.textures.get_mut(&id)?;
        if entry.fingerprint != fingerprint {
            return None;
        }
        entry.last_used_frame = self.current_frame;
        Some((entry.texture.clone(), entry.size_points))
    }

    fn update_texture(
        &mut self,
        ctx: &Context,
        id: Id,
        fingerprint: u64,
        image: ColorImage,
        size_points: Vec2,
    ) -> TextureHandle {
        let entry = self.textures.entry(id).or_insert_with(|| TextureEntry {
            fingerprint: 0,
            texture: ctx.load_texture(
                format!("textui_texture_{id:?}"),
                image.clone(),
                TextureOptions::LINEAR,
            ),
            size_points,
            last_used_frame: self.current_frame,
        });

        if entry.fingerprint != fingerprint || entry.texture.size() != image.size {
            entry
                .texture
                .set(egui::ImageData::Color(image.into()), TextureOptions::LINEAR);
            entry.fingerprint = fingerprint;
        }

        entry.size_points = size_points;
        entry.last_used_frame = self.current_frame;
        entry.texture.clone()
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
    ) -> AttrsOwned {
        let mut attrs = Attrs::new()
            .color(to_cosmic_color(style.color))
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
        if let Some(features) = &self.open_type_features {
            attrs = attrs.font_features(features.clone());
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
        if let Some(features) = &self.open_type_features {
            attrs = attrs.font_features(features.clone());
        }

        AttrsOwned::new(&attrs)
    }

    fn input_attrs_fingerprint(&self, options: &InputOptions, scale: f32) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "input_attrs".hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.text_color.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
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

fn async_raster_worker_loop(
    rx: mpsc::Receiver<AsyncRasterWorkerMessage>,
    tx: mpsc::Sender<AsyncRasterResponse>,
) {
    let mut font_system = FontSystem::new();
    let mut swash_cache = SwashCache::new();

    while let Ok(msg) = rx.recv() {
        match msg {
            AsyncRasterWorkerMessage::RegisterFont(bytes) => {
                font_system.db_mut().load_font_data(bytes);
            }
            AsyncRasterWorkerMessage::Render(req) => {
                let raster = async_rasterize_request(&mut font_system, &mut swash_cache, &req);
                let _ = tx.send(AsyncRasterResponse {
                    key_hash: req.key_hash,
                    raster,
                });
            }
        }
    }
}

fn async_rasterize_request(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    req: &AsyncRasterRequest,
) -> RasterizedText {
    let metrics = Metrics::new(
        (req.options.font_size * req.typography.ui_font_size_scale * req.scale).max(1.0),
        (req.options.line_height * req.typography.ui_font_size_scale * req.scale).max(1.0),
    );
    let mut buffer = Buffer::new(font_system, metrics);
    let width_px_opt = req.width_points_opt.map(|w| (w * req.scale).max(1.0));
    let feature_tags = if req.typography.open_type_features_enabled {
        parse_feature_tag_list(&req.typography.open_type_features_to_enable)
    } else {
        Vec::new()
    };
    let features = if feature_tags.is_empty() {
        None
    } else {
        Some(build_font_features(&feature_tags))
    };

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
                        color: req.options.color,
                        monospace: req.options.monospace,
                        italic: req.options.italic,
                        weight: req.options.weight,
                    },
                    features.clone(),
                );
                let attrs = attrs_owned.as_attrs();
                borrowed.set_text(text, &attrs, Shaping::Advanced, None);
            }
            AsyncRasterKind::Rich(spans) => {
                let default_attrs_owned = async_build_text_attrs_owned(
                    req,
                    &SpanStyle {
                        color: req.options.color,
                        monospace: req.options.monospace,
                        italic: req.options.italic,
                        weight: req.options.weight,
                    },
                    features.clone(),
                );
                let span_attrs_owned = spans
                    .iter()
                    .map(|span| async_build_text_attrs_owned(req, &span.style, features.clone()))
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
    let width_px = measured_width_px.clamp(1, req.max_texture_side_px.max(1));
    let height_px = measured_height_px.clamp(1, req.max_texture_side_px.max(1));

    let mut image = ColorImage::new(
        [width_px, height_px],
        vec![Color32::TRANSPARENT; width_px * height_px],
    );
    buffer.draw(
        font_system,
        swash_cache,
        to_cosmic_color(req.options.color),
        |x, y, w, h, color| {
            blend_rect(
                &mut image,
                x,
                y,
                w as i32,
                h as i32,
                cosmic_to_egui_color(color),
            );
        },
    );

    RasterizedText {
        image,
        size_points: egui::vec2(width_px as f32 / req.scale, height_px as f32 / req.scale),
    }
}

fn async_build_text_attrs_owned(
    req: &AsyncRasterRequest,
    style: &SpanStyle,
    features: Option<FontFeatures>,
) -> AttrsOwned {
    let effective_weight =
        (i32::from(style.weight) + (req.typography.ui_font_weight - 400)).clamp(100, 900) as u16;
    let mut attrs = Attrs::new()
        .color(to_cosmic_color(style.color))
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
    if let Some(features) = features {
        attrs = attrs.font_features(features);
    }
    AttrsOwned::new(&attrs)
}

fn build_font_features(tags: &[[u8; 4]]) -> FontFeatures {
    let mut features = FontFeatures::new();
    for tag in tags {
        features.set(cosmic_text::FeatureTag::new(tag), 1);
    }
    features
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

fn input_texture_fingerprint(
    editor: &Editor<'static>,
    text: &str,
    options: &InputOptions,
    has_focus: bool,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "input".hash(&mut hasher);
    text.hash(&mut hasher);
    options.font_size.to_bits().hash(&mut hasher);
    options.line_height.to_bits().hash(&mut hasher);
    options.text_color.hash(&mut hasher);
    options.cursor_color.hash(&mut hasher);
    options.selection_color.hash(&mut hasher);
    options.selected_text_color.hash(&mut hasher);
    has_focus.hash(&mut hasher);
    hash_cursor(editor.cursor(), &mut hasher);
    hash_selection(editor.selection(), &mut hasher);
    editor.with_buffer(|buffer| hash_scroll(buffer.scroll(), &mut hasher));
    hasher.finish()
}

fn rich_viewer_texture_fingerprint(
    editor: &Editor<'static>,
    text: &str,
    spans: &[RichTextSpan],
    options: &InputOptions,
    has_focus: bool,
    wrap: bool,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "rich_viewer".hash(&mut hasher);
    text.hash(&mut hasher);
    options.font_size.to_bits().hash(&mut hasher);
    options.line_height.to_bits().hash(&mut hasher);
    options.text_color.hash(&mut hasher);
    options.cursor_color.hash(&mut hasher);
    options.selection_color.hash(&mut hasher);
    options.selected_text_color.hash(&mut hasher);
    wrap.hash(&mut hasher);
    has_focus.hash(&mut hasher);
    for span in spans {
        span.text.hash(&mut hasher);
        span.style.color.hash(&mut hasher);
        span.style.monospace.hash(&mut hasher);
        span.style.italic.hash(&mut hasher);
        span.style.weight.hash(&mut hasher);
    }
    hash_cursor(editor.cursor(), &mut hasher);
    hash_selection(editor.selection(), &mut hasher);
    editor.with_buffer(|buffer| hash_scroll(buffer.scroll(), &mut hasher));
    hasher.finish()
}

fn hash_cursor<H: Hasher>(cursor: Cursor, state: &mut H) {
    cursor.line.hash(state);
    cursor.index.hash(state);
    format!("{:?}", cursor.affinity).hash(state);
}

fn hash_scroll<H: Hasher>(scroll: cosmic_text::Scroll, state: &mut H) {
    scroll.line.hash(state);
    scroll.vertical.to_bits().hash(state);
    scroll.horizontal.to_bits().hash(state);
}

fn hash_selection<H: Hasher>(selection: Selection, state: &mut H) {
    match selection {
        Selection::None => {
            0_u8.hash(state);
        }
        Selection::Normal(cursor) => {
            1_u8.hash(state);
            hash_cursor(cursor, state);
        }
        Selection::Line(cursor) => {
            2_u8.hash(state);
            hash_cursor(cursor, state);
        }
        Selection::Word(cursor) => {
            3_u8.hash(state);
            hash_cursor(cursor, state);
        }
    }
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

fn extend_selection_to_pointer(editor: &mut Editor<'static>, x: i32, y: i32) -> bool {
    let anchor = selection_anchor(editor.selection()).unwrap_or_else(|| editor.cursor());
    let Some(new_cursor) = editor.with_buffer(|buffer| buffer.hit(x as f32, y as f32)) else {
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

fn handle_editor_key_event(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
    multiline: bool,
) -> bool {
    if modifiers.command && key == Key::A {
        return select_all(editor);
    }

    if handle_editor_delete_shortcut(font_system, editor, key, modifiers) {
        return true;
    }

    if cfg!(target_os = "macos") && modifiers.ctrl && !modifiers.shift {
        if let Some(motion) = mac_control_motion(key) {
            return handle_editor_motion_key(font_system, editor, key, modifiers, motion);
        }
    }

    let Some(action) = key_to_action(key, modifiers, multiline) else {
        return false;
    };

    match action {
        Action::Motion(motion) => {
            handle_editor_motion_key(font_system, editor, key, modifiers, motion)
        }
        _ => {
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
) -> bool {
    if modifiers.command && key == Key::A {
        return select_all(editor);
    }

    if cfg!(target_os = "macos") && modifiers.ctrl && !modifiers.shift {
        if let Some(motion) = mac_control_motion(key) {
            return handle_editor_motion_key(font_system, editor, key, modifiers, motion);
        }
    }

    let Some(action) = key_to_action(key, modifiers, true) else {
        if key == Key::Escape && editor.selection() != Selection::None {
            editor.set_selection(Selection::None);
            return true;
        }
        return false;
    };

    match action {
        Action::Motion(motion) => {
            handle_editor_motion_key(font_system, editor, key, modifiers, motion)
        }
        Action::Escape => {
            if editor.selection() != Selection::None {
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
) -> bool {
    if modifiers.shift {
        if editor.selection() == Selection::None {
            editor.set_selection(Selection::Normal(editor.cursor()));
        }
        editor
            .borrow_with(font_system)
            .action(Action::Motion(motion));
        return true;
    }

    if let Some((start, end)) = editor.selection_bounds() {
        if modifiers.is_none() && key == Key::ArrowLeft {
            editor.set_selection(Selection::None);
            editor.set_cursor(start);
            return true;
        }
        if modifiers.is_none() && key == Key::ArrowRight {
            editor.set_selection(Selection::None);
            editor.set_cursor(end);
            return true;
        }
        editor.set_selection(Selection::None);
    }

    editor
        .borrow_with(font_system)
        .action(Action::Motion(motion));
    true
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

fn parse_markdown_blocks(markdown: &str) -> Vec<MarkdownBlock> {
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
                        blocks.push(MarkdownBlock::Heading {
                            level,
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
                    blocks.push(MarkdownBlock::Paragraph(text_buf.trim().to_owned()));
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
                blocks.push(MarkdownBlock::Code {
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
                    blocks.push(MarkdownBlock::Paragraph(text_buf.trim().to_owned()));
                }
                text_buf.clear();
                blocks.push(MarkdownBlock::Paragraph("---".to_owned()));
            }
            _ => {}
        }
    }

    if !text_buf.trim().is_empty() {
        if in_code_block {
            blocks.push(MarkdownBlock::Code {
                language: current_code_language,
                text: text_buf,
            });
        } else if let Some(level) = current_heading {
            blocks.push(MarkdownBlock::Heading {
                level,
                text: text_buf,
            });
        } else {
            blocks.push(MarkdownBlock::Paragraph(text_buf));
        }
    }

    blocks
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

        let Some(layout_lines) = buffer.line_layout(line_i) else {
            continue;
        };
        for layout_line in layout_lines {
            let line_height = layout_line.line_height_opt.unwrap_or(metrics.line_height);
            max_bottom = max_bottom.max(line_top + line_height);
            for glyph in &layout_line.glyphs {
                max_right = max_right.max(glyph.x + glyph.w);
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
) -> EditorScrollMetrics {
    let mut scroll_metrics = measure_borrowed_buffer_scroll_metrics(buffer);
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

fn paint_texture(ui: &Ui, texture: &TextureHandle, rect: Rect) {
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn to_cosmic_color(color: Color32) -> Color {
    Color::rgba(color.r(), color.g(), color.b(), color.a())
}

fn cosmic_to_egui_color(color: Color) -> Color32 {
    Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), color.a())
}

fn blend_rect(image: &mut ColorImage, x: i32, y: i32, w: i32, h: i32, src: Color32) {
    let width = image.size[0] as i32;
    let height = image.size[1] as i32;

    let x0 = x.max(0).min(width);
    let y0 = y.max(0).min(height);
    let x1 = (x + w).max(0).min(width);
    let y1 = (y + h).max(0).min(height);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    for py in y0..y1 {
        for px in x0..x1 {
            let index = (py as usize) * image.size[0] + px as usize;
            let dst = image.pixels[index];
            image.pixels[index] = alpha_blend(src, dst);
        }
    }
}

fn blend_rect_into_tiles(
    tiles: &mut [(usize, usize, ColorImage)],
    total_width: usize,
    total_height: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    src: Color32,
) {
    let total_width = total_width as i32;
    let total_height = total_height as i32;
    let x0 = x.max(0).min(total_width);
    let y0 = y.max(0).min(total_height);
    let x1 = (x + w).max(0).min(total_width);
    let y1 = (y + h).max(0).min(total_height);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    for (origin_x, origin_y, image) in tiles.iter_mut() {
        let tile_x0 = *origin_x as i32;
        let tile_y0 = *origin_y as i32;
        let tile_x1 = tile_x0 + image.size[0] as i32;
        let tile_y1 = tile_y0 + image.size[1] as i32;

        let overlap_x0 = x0.max(tile_x0);
        let overlap_y0 = y0.max(tile_y0);
        let overlap_x1 = x1.min(tile_x1);
        let overlap_y1 = y1.min(tile_y1);
        if overlap_x0 >= overlap_x1 || overlap_y0 >= overlap_y1 {
            continue;
        }

        blend_rect(
            image,
            overlap_x0 - tile_x0,
            overlap_y0 - tile_y0,
            overlap_x1 - overlap_x0,
            overlap_y1 - overlap_y0,
            src,
        );
    }
}

fn alpha_blend(src: Color32, dst: Color32) -> Color32 {
    if src.a() == 255 {
        return src;
    }
    if src.a() == 0 {
        return dst;
    }

    let sa = src.a() as f32 / 255.0;
    let da = dst.a() as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= f32::EPSILON {
        return Color32::TRANSPARENT;
    }

    let sr = src.r() as f32 / 255.0;
    let sg = src.g() as f32 / 255.0;
    let sb = src.b() as f32 / 255.0;

    let dr = dst.r() as f32 / 255.0;
    let dg = dst.g() as f32 / 255.0;
    let db = dst.b() as f32 / 255.0;

    let out_r = (sr * sa + dr * da * (1.0 - sa)) / out_a;
    let out_g = (sg * sa + dg * da * (1.0 - sa)) / out_a;
    let out_b = (sb * sa + db * da * (1.0 - sa)) / out_a;

    Color32::from_rgba_unmultiplied(
        (out_r.clamp(0.0, 1.0) * 255.0) as u8,
        (out_g.clamp(0.0, 1.0) * 255.0) as u8,
        (out_b.clamp(0.0, 1.0) * 255.0) as u8,
        (out_a.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

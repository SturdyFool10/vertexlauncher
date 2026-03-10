use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::mpsc,
};

use cosmic_text::{
    Action, Attrs, AttrsOwned, Buffer, Color, Edit, Editor, Family, FontFeatures, FontSystem,
    Metrics, Motion, Shaping, Style as FontStyle, SwashCache, Weight, Wrap,
};
use egui::{
    self, Color32, ColorImage, Context, CornerRadius, Id, Key, Pos2, Rect, Response, Sense, Stroke,
    TextureHandle, TextureOptions, Ui, Vec2,
};
use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, Options as MdOptions, Parser, Tag, TagEnd,
};
use skrifa::raw::{FontRef as SkrifaFontRef, TableProvider as _};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle as SyntectFontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use tracing::warn;

const DEFAULT_OPEN_TYPE_FEATURE_TAGS: &str = "liga, calt";

/// Styling options for plain/rich labels.
#[derive(Clone, Debug)]
pub struct LabelOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub color: Color32,
    pub wrap: bool,
    pub monospace: bool,
    pub weight: u16,
    pub italic: bool,
    pub padding: Vec2,
}

impl Default for LabelOptions {
    fn default() -> Self {
        Self {
            font_size: 18.0,
            line_height: 24.0,
            color: Color32::WHITE,
            wrap: true,
            monospace: false,
            weight: 400,
            italic: false,
            padding: egui::vec2(0.0, 0.0),
        }
    }
}

/// Styling options for syntax-highlighted code blocks.
#[derive(Clone, Debug)]
pub struct CodeBlockOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub text_color: Color32,
    pub background_color: Color32,
    pub stroke: Stroke,
    pub wrap: bool,
    pub language: Option<String>,
    pub padding: Vec2,
    pub corner_radius: u8,
}

impl Default for CodeBlockOptions {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            line_height: 22.0,
            text_color: Color32::from_rgb(230, 230, 230),
            background_color: Color32::from_rgb(16, 18, 22),
            stroke: Stroke::new(1.0, Color32::from_rgb(36, 40, 48)),
            wrap: true,
            language: None,
            padding: egui::vec2(10.0, 10.0),
            corner_radius: 8,
        }
    }
}

/// Markdown rendering options.
#[derive(Clone, Debug)]
pub struct MarkdownOptions {
    pub body: LabelOptions,
    pub heading_scale: f32,
    pub paragraph_spacing: f32,
    pub code: CodeBlockOptions,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self {
            body: LabelOptions::default(),
            heading_scale: 1.28,
            paragraph_spacing: 8.0,
            code: CodeBlockOptions::default(),
        }
    }
}

/// Styling/behavior options for single/multi-line text inputs.
#[derive(Clone, Debug)]
pub struct InputOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub text_color: Color32,
    pub cursor_color: Color32,
    pub selection_color: Color32,
    pub selected_text_color: Color32,
    pub background_color: Color32,
    pub stroke: Stroke,
    pub padding: Vec2,
    pub monospace: bool,
    pub min_width: f32,
    pub desired_width: Option<f32>,
    pub desired_rows: usize,
}

impl Default for InputOptions {
    fn default() -> Self {
        Self {
            font_size: 18.0,
            line_height: 24.0,
            text_color: Color32::WHITE,
            cursor_color: Color32::from_rgb(90, 170, 255),
            selection_color: Color32::from_rgba_premultiplied(90, 170, 255, 80),
            selected_text_color: Color32::WHITE,
            background_color: Color32::from_rgb(11, 13, 16),
            stroke: Stroke::new(1.0, Color32::from_rgb(45, 50, 60)),
            padding: egui::vec2(8.0, 6.0),
            monospace: false,
            min_width: 64.0,
            desired_width: None,
            desired_rows: 5,
        }
    }
}

/// Styling options for button widgets.
#[derive(Clone, Debug)]
pub struct ButtonOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub text_color: Color32,
    pub fill: Color32,
    pub fill_hovered: Color32,
    pub fill_active: Color32,
    pub fill_selected: Color32,
    pub stroke: Stroke,
    pub corner_radius: u8,
    pub padding: Vec2,
    pub min_size: Vec2,
}

impl Default for ButtonOptions {
    fn default() -> Self {
        Self {
            font_size: 18.0,
            line_height: 24.0,
            text_color: Color32::WHITE,
            fill: Color32::from_rgb(24, 28, 34),
            fill_hovered: Color32::from_rgb(30, 35, 42),
            fill_active: Color32::from_rgb(36, 43, 52),
            fill_selected: Color32::from_rgb(40, 56, 74),
            stroke: Stroke::new(1.0, Color32::from_rgb(52, 58, 68)),
            corner_radius: 8,
            padding: egui::vec2(10.0, 6.0),
            min_size: egui::vec2(88.0, 30.0),
        }
    }
}

/// Styling options for tooltip overlays.
#[derive(Clone, Debug)]
pub struct TooltipOptions {
    pub text: LabelOptions,
    pub background: Color32,
    pub stroke: Stroke,
    pub corner_radius: u8,
    pub padding: Vec2,
    pub offset: Vec2,
}

impl Default for TooltipOptions {
    fn default() -> Self {
        let mut text = LabelOptions::default();
        text.font_size = 14.0;
        text.line_height = 18.0;
        text.wrap = true;

        Self {
            text,
            background: Color32::from_rgba_premultiplied(14, 16, 20, 245),
            stroke: Stroke::new(1.0, Color32::from_rgb(42, 48, 58)),
            corner_radius: 6,
            padding: egui::vec2(8.0, 6.0),
            offset: egui::vec2(10.0, 6.0),
        }
    }
}

#[derive(Clone, Debug)]
struct SpanStyle {
    color: Color32,
    monospace: bool,
    italic: bool,
    weight: u16,
}

#[derive(Clone, Debug)]
struct RichSpan {
    text: String,
    style: SpanStyle,
}

#[derive(Clone, Debug)]
struct RasterizedText {
    image: ColorImage,
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
    multiline: bool,
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
        let worker_spawn = std::thread::Builder::new()
            .name("textui-async-raster-worker".to_owned())
            .spawn(move || async_raster_worker_loop(worker_rx, result_tx));
        let (worker_tx, result_rx) = match worker_spawn {
            Ok(_) => (Some(worker_tx), Some(result_rx)),
            Err(error) => {
                warn!(
                    target: "vertexlauncher/textui",
                    error = %error,
                    "failed to spawn textui async raster worker; falling back to synchronous text rasterization"
                );
                (None, None)
            }
        };

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
        }
    }

    /// Performs per-frame maintenance and processes async raster results.
    pub fn begin_frame(&mut self, ctx: &Context) {
        self.current_frame = ctx.cumulative_frame_nr();
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
        self.textures.clear();
        self.input_states.clear();
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
            Some((ui.available_width() - options.padding.x * 2.0).max(1.0))
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
        self.textures.clear();
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
        self.textures.clear();
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
        let width_points_opt = if options.wrap {
            Some(ui.available_width().max(1.0))
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
        let raster = self.rasterize_plain_text(text, &label_style, None, scale);
        let text_size = raster.size_points;
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

        let texture = self.update_texture(
            ui.ctx(),
            ui.make_persistent_id(id_source).with("button_text"),
            {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                "textui_button".hash(&mut hasher);
                text.hash(&mut hasher);
                selected.hash(&mut hasher);
                options.font_size.to_bits().hash(&mut hasher);
                options.line_height.to_bits().hash(&mut hasher);
                options.text_color.hash(&mut hasher);
                response.hovered().hash(&mut hasher);
                response.is_pointer_button_down_on().hash(&mut hasher);
                hasher.finish()
            },
            raster.image,
            raster.size_points,
        );

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
        let width_points_opt =
            Some(320.0_f32.min(ui.ctx().input(|i| i.content_rect().width() * 0.35)));
        let raster = self.rasterize_plain_text(text, &options.text, width_points_opt, scale);
        let size = raster.size_points + options.padding * 2.0;
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

        let texture = self.update_texture(
            ui.ctx(),
            ui.make_persistent_id(&id_source).with("tooltip_text"),
            {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                "textui_tooltip".hash(&mut hasher);
                text.hash(&mut hasher);
                options.text.font_size.to_bits().hash(&mut hasher);
                options.text.line_height.to_bits().hash(&mut hasher);
                options.text.color.hash(&mut hasher);
                hasher.finish()
            },
            raster.image,
            raster.size_points,
        );

        let text_rect = Rect::from_min_size(rect.min + options.padding, raster.size_points);
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
            Some((ui.available_width() - options.padding.x * 2.0).max(1.0))
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
        let blocks = parse_markdown_blocks(markdown);
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

        if response.hovered() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Text);
        }

        if response.clicked() {
            response.request_focus();
        }

        let has_focus = response.has_focus();
        let scale = ui.ctx().pixels_per_point();
        let content_rect = rect.shrink2(options.padding);
        let content_width_px = (content_rect.width() * scale).max(1.0);
        let content_height_px = (content_rect.height() * scale).max(1.0);

        let mut state = self
            .input_states
            .remove(&id)
            .unwrap_or_else(|| Self::new_input_state(&mut self.font_system, text, multiline));

        if state.multiline != multiline {
            state = Self::new_input_state(&mut self.font_system, text, multiline);
        }

        if !has_focus && state.last_text != *text {
            self.replace_editor_text(
                &mut state.editor,
                text,
                options,
                multiline,
                content_width_px,
                content_height_px,
                scale,
            );
            state.last_text.clone_from(text);
        }

        self.configure_editor(
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

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "input".hash(&mut hasher);
        text.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.text_color.hash(&mut hasher);
        options.background_color.hash(&mut hasher);
        has_focus.hash(&mut hasher);
        let cursor = state.editor.cursor();
        cursor.line.hash(&mut hasher);
        cursor.index.hash(&mut hasher);
        format!("{:?}", cursor.affinity).hash(&mut hasher);
        let fingerprint = hasher.finish();

        let texture = self.update_texture(
            ui.ctx(),
            id.with("tex"),
            fingerprint,
            image,
            content_rect.size(),
        );
        self.input_states.insert(id, state);

        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), options.background_color);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            options.stroke,
            egui::StrokeKind::Inside,
        );

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
            multiline,
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
    ) {
        let attrs_owned = self.input_attrs_owned(options, scale);
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
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
            borrowed.shape_until_scroll(true);
        });
    }

    fn configure_editor(
        &mut self,
        editor: &mut Editor<'static>,
        options: &InputOptions,
        multiline: bool,
        width_px: f32,
        height_px: f32,
        scale: f32,
    ) {
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
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
        });
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
    ) -> bool {
        let mut changed = false;

        if let Some(pointer_pos) = response.interact_pointer_pos() {
            let x = ((pointer_pos.x - content_rect.min.x) * scale).round() as i32;
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
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::Click { x, y });
                changed = true;
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
                    egui::Event::Text(text) => {
                        for ch in text.chars() {
                            if !multiline && (ch == '\n' || ch == '\r') {
                                continue;
                            }
                            editor
                                .borrow_with(&mut self.font_system)
                                .action(Action::Insert(ch));
                            changed = true;
                        }
                    }
                    egui::Event::Paste(mut pasted) => {
                        if !multiline {
                            pasted = pasted.replace(['\n', '\r'], " ");
                        }
                        for ch in pasted.chars() {
                            editor
                                .borrow_with(&mut self.font_system)
                                .action(Action::Insert(ch));
                            changed = true;
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } if pressed => {
                        if let Some(action) = key_to_action(key, modifiers, multiline) {
                            editor.borrow_with(&mut self.font_system).action(action);
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        if changed {
            editor
                .borrow_with(&mut self.font_system)
                .shape_as_needed(false);
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
                    x,
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

        let width_px = measured_width_px.max(1);
        let height_px = measured_height_px.max(1);

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
    let width_px = measured_width_px.max(1);
    let height_px = measured_height_px.max(1);

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

fn key_to_action(key: Key, modifiers: egui::Modifiers, multiline: bool) -> Option<Action> {
    let command = modifiers.command || modifiers.ctrl;
    match key {
        Key::ArrowLeft => Some(if command {
            Action::Motion(Motion::PreviousWord)
        } else {
            Action::Motion(Motion::Left)
        }),
        Key::ArrowRight => Some(if command {
            Action::Motion(Motion::NextWord)
        } else {
            Action::Motion(Motion::Right)
        }),
        Key::ArrowUp => Some(Action::Motion(Motion::Up)),
        Key::ArrowDown => Some(Action::Motion(Motion::Down)),
        Key::Home => Some(if command {
            Action::Motion(Motion::BufferStart)
        } else {
            Action::Motion(Motion::Home)
        }),
        Key::End => Some(if command {
            Action::Motion(Motion::BufferEnd)
        } else {
            Action::Motion(Motion::End)
        }),
        Key::PageUp => Some(Action::Motion(Motion::PageUp)),
        Key::PageDown => Some(Action::Motion(Motion::PageDown)),
        Key::Backspace => Some(Action::Backspace),
        Key::Delete => Some(Action::Delete),
        Key::Enter if multiline => Some(Action::Enter),
        Key::Tab => Some(Action::Insert('\t')),
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

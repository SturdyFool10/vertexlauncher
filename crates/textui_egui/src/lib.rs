mod button_options;
mod code_block_options;
mod input_options;
mod label_options;
mod markdown_options;
mod text_helpers;
mod tooltip_options;

use egui::{
    Color32, Context, CornerRadius, Id, Painter, Rect, Response, Sense, TextureHandle, TextureId,
    TextureOptions, Ui, Vec2,
};
use egui_wgpu::RenderState;
use std::{
    collections::HashMap,
    hash::DefaultHasher,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
};
use textui::{
    TextAtlasPageData, TextAtlasSampling, TextFrameInfo, TextFrameOutput, TextGpuScene,
    TextInputEvent, TextKey, TextMarkdownBlock, TextMarkdownHeadingLevel, TextModifiers, TextPath,
    TextPathError, TextPathLayout, TextPathOptions, TextPointerButton, TextRenderScene, TextUi,
};

pub use button_options::ButtonOptions;
pub use code_block_options::CodeBlockOptions;
pub use input_options::InputOptions;
pub use label_options::LabelOptions;
pub use markdown_options::MarkdownOptions;
pub use text_helpers::{
    TruncatedText, normalize_inline_whitespace, truncate_single_line_text_with_ellipsis,
    truncate_single_line_text_with_ellipsis_detailed,
    truncate_single_line_text_with_ellipsis_preserving_whitespace,
    truncate_single_line_text_with_ellipsis_preserving_whitespace_detailed,
};
pub use textui::{RichTextSpan, RichTextStyle, TextColor};
pub use tooltip_options::TooltipOptions;

#[derive(Clone)]
pub struct TextTextureHandle {
    scene: Arc<TextGpuScene>,
    pub size_points: Vec2,
}

impl TextTextureHandle {
    pub fn scene(&self) -> &TextGpuScene {
        &self.scene
    }

    pub fn into_scene(self) -> Arc<TextGpuScene> {
        self.scene
    }

    pub fn paint(&self, text_ui: &mut TextUi, ui: &Ui, rect: Rect) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        paint_gpu_scene_in_rect(text_ui, &painter, rect, &self.scene, Color32::WHITE);
    }

    pub fn paint_tinted(&self, text_ui: &mut TextUi, ui: &Ui, rect: Rect, tint: egui::Color32) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        paint_gpu_scene_in_rect(text_ui, &painter, rect, &self.scene, tint);
    }

    pub fn paint_on(
        &self,
        text_ui: &mut TextUi,
        painter: &egui::Painter,
        rect: Rect,
        tint: egui::Color32,
    ) {
        paint_gpu_scene_in_rect(text_ui, painter, rect, &self.scene, tint);
    }
}

const GPU_SCENE_TEXTURE_CACHE_ID: &str = "textui_egui_gpu_scene_texture_cache";
const GPU_SCENE_TEXTURE_CACHE_STALE_FRAMES: u64 = 600;
const RETAINED_GPU_SCENE_CACHE_ID: &str = "textui_egui_retained_gpu_scene_cache";
const RETAINED_GPU_SCENE_CACHE_STALE_FRAMES: u64 = 600;
const WIDTH_BIN_PX: f32 = 16.0;

#[derive(Clone)]
struct CachedGpuSceneTexture {
    handle: TextureHandle,
    last_used_frame: u64,
}

#[derive(Default)]
struct GpuSceneTextureCacheState {
    entries: HashMap<u64, CachedGpuSceneTexture>,
    /// Frame on which `entries.retain` was last run. Ensures the O(N) eviction
    /// scan runs at most once per frame rather than once per text element.
    last_eviction_frame: u64,
}

type GpuSceneTextureCache = Arc<Mutex<GpuSceneTextureCacheState>>;

#[derive(Clone)]
struct CachedRetainedGpuScene {
    fingerprint: u64,
    scene: Arc<TextGpuScene>,
    last_used_frame: u64,
}

#[derive(Default)]
struct RetainedGpuSceneCacheState {
    entries: HashMap<Id, CachedRetainedGpuScene>,
    last_eviction_frame: u64,
}

type RetainedGpuSceneCache = Arc<Mutex<RetainedGpuSceneCacheState>>;

fn snap_rect_to_pixel_grid(rect: Rect, pixels_per_point: f32) -> Rect {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return rect;
    }
    let snap = |value: f32| (value * pixels_per_point).round() / pixels_per_point;
    Rect::from_min_max(
        egui::pos2(snap(rect.min.x), snap(rect.min.y)),
        egui::pos2(snap(rect.max.x), snap(rect.max.y)),
    )
}

fn snap_width_to_bin(width_points: f32, scale: f32) -> f32 {
    let width_px = (width_points * scale).round();
    let snapped_px = (width_px / WIDTH_BIN_PX).floor() * WIDTH_BIN_PX;
    (snapped_px / scale).max(1.0)
}

fn normalize_wrapped_width(width_points_opt: Option<f32>, scale: f32) -> Option<f32> {
    width_points_opt.map(|width| snap_width_to_bin(width.max(1.0), scale))
}

fn texture_options_for_sampling(sampling: TextAtlasSampling) -> TextureOptions {
    match sampling {
        TextAtlasSampling::Linear => TextureOptions::LINEAR,
        TextAtlasSampling::Nearest => TextureOptions::NEAREST,
    }
}

fn gpu_scene_texture_cache(ctx: &Context) -> GpuSceneTextureCache {
    ctx.data_mut(|data| {
        let id = Id::new(GPU_SCENE_TEXTURE_CACHE_ID);
        if let Some(cache) = data.get_temp::<GpuSceneTextureCache>(id) {
            cache
        } else {
            let cache = Arc::new(Mutex::new(GpuSceneTextureCacheState::default()));
            data.insert_temp(id, Arc::clone(&cache));
            cache
        }
    })
}

fn retained_gpu_scene_cache(ctx: &Context) -> RetainedGpuSceneCache {
    ctx.data_mut(|data| {
        let id = Id::new(RETAINED_GPU_SCENE_CACHE_ID);
        if let Some(cache) = data.get_temp::<RetainedGpuSceneCache>(id) {
            cache
        } else {
            let cache = Arc::new(Mutex::new(RetainedGpuSceneCacheState::default()));
            data.insert_temp(id, Arc::clone(&cache));
            cache
        }
    })
}

fn retained_gpu_scene(
    ctx: &Context,
    cache_id: Id,
    fingerprint: u64,
    build_scene: impl FnOnce() -> Option<Arc<TextGpuScene>>,
) -> Option<Arc<TextGpuScene>> {
    let current_frame = ctx.cumulative_frame_nr();
    let cache = retained_gpu_scene_cache(ctx);
    {
        let mut cache_guard = cache
            .lock()
            .expect("textui_egui retained scene cache poisoned");
        if current_frame > cache_guard.last_eviction_frame {
            cache_guard.last_eviction_frame = current_frame;
            cache_guard.entries.retain(|_, entry| {
                current_frame.saturating_sub(entry.last_used_frame)
                    <= RETAINED_GPU_SCENE_CACHE_STALE_FRAMES
            });
        }
        if let Some(entry) = cache_guard.entries.get_mut(&cache_id)
            && entry.fingerprint == fingerprint
        {
            entry.last_used_frame = current_frame;
            return Some(Arc::clone(&entry.scene));
        }
    }

    let scene = build_scene()?;
    let mut cache_guard = cache
        .lock()
        .expect("textui_egui retained scene cache poisoned");
    cache_guard.entries.insert(
        cache_id,
        CachedRetainedGpuScene {
            fingerprint,
            scene: Arc::clone(&scene),
            last_used_frame: current_frame,
        },
    );
    Some(scene)
}

fn hash_text_fundamentals(hasher: &mut DefaultHasher, fundamentals: &textui::TextFundamentals) {
    fundamentals.kerning.hash(hasher);
    fundamentals.stem_darkening.hash(hasher);
    fundamentals.standard_ligatures.hash(hasher);
    fundamentals.contextual_alternates.hash(hasher);
    fundamentals.discretionary_ligatures.hash(hasher);
    fundamentals.historical_ligatures.hash(hasher);
    fundamentals.case_sensitive_forms.hash(hasher);
    fundamentals.slashed_zero.hash(hasher);
    fundamentals.tabular_numbers.hash(hasher);
    fundamentals.smart_quotes.hash(hasher);
    fundamentals.letter_spacing_points.to_bits().hash(hasher);
    fundamentals.word_spacing_points.to_bits().hash(hasher);
    fundamentals.letter_spacing_floor.to_bits().hash(hasher);
    fundamentals.feature_settings.hash(hasher);
    fundamentals.variation_settings.hash(hasher);
}

fn hash_label_options(hasher: &mut DefaultHasher, options: &LabelOptions) {
    options.font_size.to_bits().hash(hasher);
    options.line_height.to_bits().hash(hasher);
    options.color.hash(hasher);
    options.wrap.hash(hasher);
    options.monospace.hash(hasher);
    options.weight.hash(hasher);
    options.italic.hash(hasher);
    options.padding.x.to_bits().hash(hasher);
    options.padding.y.to_bits().hash(hasher);
    hash_text_fundamentals(hasher, &options.fundamentals);
    options.ellipsis.hash(hasher);
}

fn hash_label_scene_request(
    text: &str,
    options: &LabelOptions,
    width_points_opt: Option<f32>,
    scale: f32,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    "label_scene".hash(&mut hasher);
    text.hash(&mut hasher);
    hash_label_options(&mut hasher, options);
    width_points_opt.map(f32::to_bits).hash(&mut hasher);
    scale.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn hash_code_block_scene_request(
    code: &str,
    options: &CodeBlockOptions,
    width_points_opt: Option<f32>,
    scale: f32,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    "code_block_scene".hash(&mut hasher);
    code.hash(&mut hasher);
    options.font_size.to_bits().hash(&mut hasher);
    options.line_height.to_bits().hash(&mut hasher);
    options.text_color.hash(&mut hasher);
    options.wrap.hash(&mut hasher);
    options.language.hash(&mut hasher);
    options.padding.x.to_bits().hash(&mut hasher);
    options.padding.y.to_bits().hash(&mut hasher);
    hash_text_fundamentals(&mut hasher, &options.fundamentals);
    width_points_opt.map(f32::to_bits).hash(&mut hasher);
    scale.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn hash_rich_text_scene_request(
    spans: &[RichTextSpan],
    options: &LabelOptions,
    width_points_opt: Option<f32>,
    scale: f32,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    "rich_text_scene".hash(&mut hasher);
    for span in spans {
        span.text.hash(&mut hasher);
        span.style.color.hash(&mut hasher);
        span.style.monospace.hash(&mut hasher);
        span.style.italic.hash(&mut hasher);
        span.style.weight.hash(&mut hasher);
    }
    hash_label_options(&mut hasher, options);
    width_points_opt.map(f32::to_bits).hash(&mut hasher);
    scale.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn hash_text_render_scene(scene: &TextRenderScene) -> u64 {
    let mut hasher = DefaultHasher::new();
    "text_render_scene".hash(&mut hasher);
    scene.bounds.min.x.to_bits().hash(&mut hasher);
    scene.bounds.min.y.to_bits().hash(&mut hasher);
    scene.bounds.max.x.to_bits().hash(&mut hasher);
    scene.bounds.max.y.to_bits().hash(&mut hasher);
    scene.size_points.x.to_bits().hash(&mut hasher);
    scene.size_points.y.to_bits().hash(&mut hasher);
    for quad in &scene.quads {
        quad.atlas_page_index.hash(&mut hasher);
        for point in quad.positions {
            point.x.to_bits().hash(&mut hasher);
            point.y.to_bits().hash(&mut hasher);
        }
        for point in quad.uvs {
            point.x.to_bits().hash(&mut hasher);
            point.y.to_bits().hash(&mut hasher);
        }
        quad.tint.hash(&mut hasher);
        quad.is_color.hash(&mut hasher);
    }
    hasher.finish()
}

fn retained_gpu_scene_for_render_scene(
    text_ui: &TextUi,
    ctx: &Context,
    scene: &TextRenderScene,
) -> Arc<TextGpuScene> {
    let fingerprint = hash_text_render_scene(scene);
    retained_gpu_scene(
        ctx,
        Id::new(("textui_render_scene", fingerprint)),
        fingerprint,
        || {
            let mut gpu_scene = Arc::new(text_ui.gpu_scene_for_scene(scene));
            if let Some(scene) = Arc::get_mut(&mut gpu_scene) {
                scene.fingerprint = fingerprint;
            }
            Some(gpu_scene)
        },
    )
    .expect("text render scene conversion should always produce a gpu scene")
}

fn hash_gpu_scene_page(page: &TextAtlasPageData, sampling: TextAtlasSampling) -> u64 {
    let mut hasher = DefaultHasher::new();
    page.content_hash.hash(&mut hasher);
    match sampling {
        TextAtlasSampling::Linear => 0_u8,
        TextAtlasSampling::Nearest => 1_u8,
    }
    .hash(&mut hasher);
    hasher.finish()
}

fn texture_ids_for_gpu_scene(
    text_ui: &TextUi,
    ctx: &Context,
    scene: &TextGpuScene,
) -> HashMap<usize, TextureId> {
    let sampling = text_ui.graphics_config().atlas_sampling;
    let current_frame = ctx.cumulative_frame_nr();
    let cache = gpu_scene_texture_cache(ctx);
    let mut texture_ids = HashMap::new();
    let mut cache_guard = cache.lock().expect("textui_egui texture cache poisoned");
    if current_frame > cache_guard.last_eviction_frame {
        cache_guard.last_eviction_frame = current_frame;
        cache_guard.entries.retain(|_, entry| {
            current_frame.saturating_sub(entry.last_used_frame)
                <= GPU_SCENE_TEXTURE_CACHE_STALE_FRAMES
        });
    }

    for page in &scene.atlas_pages {
        let key = hash_gpu_scene_page(page, sampling);
        let entry = cache_guard
            .entries
            .entry(key)
            .or_insert_with(|| CachedGpuSceneTexture {
                handle: ctx.load_texture(
                    format!("textui_egui_gpu_scene_{key:016x}"),
                    egui::ColorImage::from_rgba_premultiplied(page.size_px, &page.rgba8),
                    texture_options_for_sampling(sampling),
                ),
                last_used_frame: current_frame,
            });
        entry.last_used_frame = current_frame;
        texture_ids.insert(page.page_index, entry.handle.id());
    }

    texture_ids
}

fn text_modifiers(modifiers: egui::Modifiers) -> TextModifiers {
    TextModifiers {
        alt: modifiers.alt,
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        command: modifiers.command,
        mac_cmd: modifiers.mac_cmd,
    }
}

fn text_key(key: egui::Key) -> Option<TextKey> {
    Some(match key {
        egui::Key::A => TextKey::A,
        egui::Key::ArrowDown => TextKey::Down,
        egui::Key::ArrowLeft => TextKey::Left,
        egui::Key::ArrowRight => TextKey::Right,
        egui::Key::ArrowUp => TextKey::Up,
        egui::Key::B => TextKey::B,
        egui::Key::Backspace => TextKey::Backspace,
        egui::Key::Delete => TextKey::Delete,
        egui::Key::E => TextKey::E,
        egui::Key::End => TextKey::End,
        egui::Key::Enter => TextKey::Enter,
        egui::Key::Escape => TextKey::Escape,
        egui::Key::F => TextKey::F,
        egui::Key::H => TextKey::H,
        egui::Key::Home => TextKey::Home,
        egui::Key::K => TextKey::K,
        egui::Key::N => TextKey::N,
        egui::Key::P => TextKey::P,
        egui::Key::PageDown => TextKey::PageDown,
        egui::Key::PageUp => TextKey::PageUp,
        egui::Key::Tab => TextKey::Tab,
        egui::Key::U => TextKey::U,
        egui::Key::W => TextKey::W,
        egui::Key::Y => TextKey::Y,
        egui::Key::Z => TextKey::Z,
        _ => return None,
    })
}

fn text_pointer_button(button: egui::PointerButton) -> TextPointerButton {
    match button {
        egui::PointerButton::Primary => TextPointerButton::Primary,
        egui::PointerButton::Secondary => TextPointerButton::Secondary,
        egui::PointerButton::Middle => TextPointerButton::Middle,
        egui::PointerButton::Extra1 => TextPointerButton::Extra1,
        egui::PointerButton::Extra2 => TextPointerButton::Extra2,
    }
}

fn translate_input_events(events: &[egui::Event]) -> Vec<TextInputEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            egui::Event::Text(text) => Some(TextInputEvent::Text(text.clone())),
            egui::Event::Copy => Some(TextInputEvent::Copy),
            egui::Event::Cut => Some(TextInputEvent::Cut),
            egui::Event::Paste(text) => Some(TextInputEvent::Paste(text.clone())),
            egui::Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } => text_key(*key).map(|key| TextInputEvent::Key {
                key,
                pressed: *pressed,
                modifiers: text_modifiers(*modifiers),
            }),
            egui::Event::PointerButton {
                button,
                pressed,
                modifiers,
                ..
            } => Some(TextInputEvent::PointerButton {
                button: text_pointer_button(*button),
                pressed: *pressed,
                modifiers: text_modifiers(*modifiers),
            }),
            _ => None,
        })
        .collect()
}

fn add_gpu_quad(
    mesh: &mut egui::epaint::Mesh,
    positions: [egui::Pos2; 4],
    uvs: [egui::Pos2; 4],
    tint: Color32,
) {
    let start = mesh.vertices.len() as u32;
    for (pos, uv) in positions.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv,
            color: tint,
        });
    }
    mesh.indices
        .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}

/// Optional affine transform applied to quad positions during painting.
/// When `None`, positions are used as-is (absolute mode).
struct PaintTransform {
    offset: [f32; 2],
    scale: [f32; 2],
}

fn paint_gpu_scene_impl(
    text_ui: &TextUi,
    painter: &Painter,
    scene: &TextGpuScene,
    tint: Color32,
    transform: Option<&PaintTransform>,
) {
    let texture_ids = texture_ids_for_gpu_scene(text_ui, painter.ctx(), scene);
    let draw_options = if let Some(t) = transform {
        textui::TextGpuSceneDrawOptions {
            offset: textui::TextPoint::new(t.offset[0], t.offset[1]),
            scale: textui::TextVector::new(t.scale[0], t.scale[1]),
            tint: tint.into(),
        }
    } else {
        textui::TextGpuSceneDrawOptions {
            offset: textui::TextPoint::ZERO,
            scale: textui::TextVector::splat(1.0),
            tint: tint.into(),
        }
    };
    for batch in text_ui
        .prepare_gpu_scene_draw_batches(scene, draw_options)
        .iter()
    {
        let Some(texture_id) = texture_ids.get(&batch.page_index).copied() else {
            continue;
        };
        let mut mesh = egui::epaint::Mesh::with_texture(texture_id);
        for quad in batch.quads.iter() {
            let positions = quad.positions.map(|point| egui::pos2(point[0], point[1]));
            let uvs = quad.uvs.map(|point| egui::pos2(point[0], point[1]));
            let final_tint = Color32::from_rgba_premultiplied(
                quad.tint_rgba[0],
                quad.tint_rgba[1],
                quad.tint_rgba[2],
                quad.tint_rgba[3],
            );
            add_gpu_quad(&mut mesh, positions, uvs, final_tint);
        }
        if !mesh.is_empty() {
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

fn paint_gpu_scene_absolute(
    text_ui: &TextUi,
    painter: &Painter,
    scene: &TextGpuScene,
    tint: Color32,
) {
    paint_gpu_scene_impl(text_ui, painter, scene, tint, None);
}

fn paint_gpu_scene_in_rect(
    text_ui: &TextUi,
    painter: &Painter,
    rect: Rect,
    scene: &TextGpuScene,
    tint: Color32,
) {
    let size = egui::vec2(scene.size_points[0], scene.size_points[1]);
    if size.x.abs() <= f32::EPSILON || size.y.abs() <= f32::EPSILON {
        return;
    }

    let rect = snap_rect_to_pixel_grid(rect, painter.pixels_per_point());
    let transform = PaintTransform {
        offset: [rect.min.x, rect.min.y],
        scale: [rect.width() / size.x, rect.height() / size.y],
    };
    paint_gpu_scene_impl(text_ui, painter, scene, tint, Some(&transform));
}

pub trait TextUiEguiExt {
    fn label_async<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response;
    fn code_block_async<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response;
    fn label<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response;
    fn clickable_label<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response;
    fn measure_text_size(&mut self, ui: &Ui, text: &str, options: &LabelOptions) -> Vec2;
    fn prepare_label_texture<H: Hash>(
        &mut self,
        ctx: &Context,
        id_source: H,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle;
    fn prepare_rich_text_texture<H: Hash>(
        &mut self,
        ctx: &Context,
        id_source: H,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle;
    fn paint_label_on_path<H: Hash>(
        &mut self,
        painter: &Painter,
        id_source: H,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError>;
    fn paint_rich_text_on_path<H: Hash>(
        &mut self,
        painter: &Painter,
        id_source: H,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError>;
    fn paint_scene_in_rect(&mut self, painter: &Painter, rect: Rect, scene: &TextRenderScene);
    fn paint_scene_in_rect_tinted(
        &mut self,
        painter: &Painter,
        rect: Rect,
        scene: &TextRenderScene,
        tint: egui::Color32,
    );
    fn button<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &ButtonOptions,
    ) -> Response;
    fn selectable_button<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        selected: bool,
        options: &ButtonOptions,
    ) -> Response;
    fn tooltip_for_response<H: Hash>(
        &mut self,
        ui: &Ui,
        id_source: H,
        response: &Response,
        text: &str,
        options: &TooltipOptions,
    );
    fn code_block<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response;
    fn markdown<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        markdown: &str,
        options: &MarkdownOptions,
    );
    fn singleline_input<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &mut String,
        options: &InputOptions,
    ) -> Response;
    fn multiline_input<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &mut String,
        options: &InputOptions,
    ) -> Response;
    fn multiline_rich_viewer<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        spans: &[RichTextSpan],
        options: &InputOptions,
        stick_to_bottom: bool,
        wrap: bool,
    ) -> Response;
}

fn label_impl(
    text_ui: &mut TextUi,
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
        display_text = textui::apply_smart_quotes(text);
        display_text.as_str()
    } else {
        text
    };

    let scale = ui.ctx().pixels_per_point();
    let width_points_opt =
        normalize_wrapped_width(options.wrap.then(|| ui.available_width().max(1.0)), scale);
    let cache_id = ui.make_persistent_id((&id_source, "textui_label_retained_scene"));
    let fingerprint = hash_label_scene_request(text, options, width_points_opt, scale);
    let text_options = options.to_text_label_options();
    let scene_opt = retained_gpu_scene(ui.ctx(), cache_id, fingerprint, || {
        if async_mode {
            text_ui.prepare_label_gpu_scene_async_at_scale(
                &id_source,
                text,
                &text_options,
                width_points_opt,
                scale,
            )
        } else {
            Some(text_ui.prepare_label_gpu_scene_at_scale(
                &id_source,
                text,
                &text_options,
                width_points_opt,
                scale,
            ))
        }
    });

    let Some(scene) = scene_opt else {
        let fallback_height = (options.line_height + options.padding.y * 2.0).max(20.0);
        let fallback_width = width_points_opt.unwrap_or_else(|| ui.available_width().max(1.0));
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(fallback_width, fallback_height), sense);
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), ui.visuals().faint_bg_color);
        ui.ctx().request_repaint();
        return response;
    };

    let scene_size = egui::vec2(scene.size_points[0], scene.size_points[1]);
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
    paint_gpu_scene_in_rect(text_ui, &painter, image_rect, &scene, Color32::WHITE);
    response
}

fn code_block_impl(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    code: &str,
    options: &CodeBlockOptions,
    async_mode: bool,
) -> Response {
    let scale = ui.ctx().pixels_per_point();
    let width_points_opt = normalize_wrapped_width(
        if options.wrap {
            Some((ui.available_width() - options.padding.x * 2.0).max(1.0))
        } else {
            None
        },
        scale,
    );
    let cache_id = ui.make_persistent_id((&id_source, "textui_code_block_retained_scene"));
    let fingerprint = hash_code_block_scene_request(code, options, width_points_opt, scale);
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
    let scene_opt = retained_gpu_scene(ui.ctx(), cache_id, fingerprint, || {
        let spans = text_ui.highlight_code_spans(
            code,
            options.language.as_deref(),
            options.text_color.into(),
        );
        let text_options = label_options.to_text_label_options();
        if async_mode {
            text_ui.prepare_rich_text_gpu_scene_async_at_scale(
                &id_source,
                &spans,
                &text_options,
                width_points_opt,
                scale,
            )
        } else {
            Some(text_ui.prepare_rich_text_gpu_scene_at_scale(
                &id_source,
                &spans,
                &text_options,
                width_points_opt,
                scale,
            ))
        }
    });

    let Some(scene) = scene_opt else {
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
        return response;
    };

    let scene_size = egui::vec2(scene.size_points[0], scene.size_points[1]);
    let desired_size = scene_size + options.padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(options.corner_radius),
        options.background_color,
    );
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
    paint_gpu_scene_in_rect(text_ui, &painter, image_rect, &scene, Color32::WHITE);
    response
}

fn markdown_impl(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    markdown: &str,
    options: &MarkdownOptions,
) {
    let mut hasher = DefaultHasher::new();
    "markdown_blocks".hash(&mut hasher);
    markdown.hash(&mut hasher);
    options.heading_scale.to_bits().hash(&mut hasher);
    options.paragraph_spacing.to_bits().hash(&mut hasher);
    options.body.font_size.to_bits().hash(&mut hasher);
    options.body.line_height.to_bits().hash(&mut hasher);
    options.body.color.hash(&mut hasher);
    options.code.font_size.to_bits().hash(&mut hasher);
    let fingerprint = hasher.finish();
    let blocks = text_ui.parse_markdown_blocks_cached(id_source, markdown, fingerprint);

    ui.push_id("textui_markdown", |ui| {
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
                    let _ = label_impl(
                        text_ui,
                        ui,
                        ("md_h", index),
                        text,
                        &heading_style,
                        Sense::hover(),
                        false,
                    );
                }
                TextMarkdownBlock::Paragraph(text) => {
                    let _ = label_impl(
                        text_ui,
                        ui,
                        ("md_p", index),
                        text,
                        &options.body,
                        Sense::hover(),
                        false,
                    );
                }
                TextMarkdownBlock::Code { language, text } => {
                    let mut code_options = options.code.clone();
                    code_options.language = language.clone();
                    let _ = code_block_impl(
                        text_ui,
                        ui,
                        ("md_code", index),
                        text,
                        &code_options,
                        false,
                    );
                }
            }

            if index + 1 < blocks.len() {
                ui.add_space(options.paragraph_spacing);
            }
        }
    });
}

fn selectable_button_impl(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    text: &str,
    selected: bool,
    options: &ButtonOptions,
) -> Response {
    let label_style = LabelOptions {
        font_size: options.font_size,
        line_height: options.line_height,
        color: options.text_color,
        wrap: false,
        monospace: false,
        weight: 400,
        italic: false,
        padding: egui::Vec2::ZERO,
        fundamentals: Default::default(),
        ..LabelOptions::default()
    };
    let scale = ui.ctx().pixels_per_point();
    let cache_id = ui.make_persistent_id((&id_source, "textui_button_retained_scene"));
    let fingerprint = hash_label_scene_request(text, &label_style, None, scale);
    let scene = retained_gpu_scene(ui.ctx(), cache_id, fingerprint, || {
        Some(text_ui.prepare_label_gpu_scene_at_scale(
            (&id_source, "button_text"),
            text,
            &label_style.to_text_label_options(),
            None,
            scale,
        ))
    })
    .expect("synchronous textui button scene should always be available");
    let text_size = egui::vec2(scene.size_points[0], scene.size_points[1]);
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
    paint_gpu_scene_in_rect(text_ui, &painter, text_rect, &scene, Color32::WHITE);
    response
}

fn tooltip_impl(
    text_ui: &mut TextUi,
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
    let width_points_opt = normalize_wrapped_width(
        Some(320.0_f32.min(ui.ctx().input(|i| i.content_rect().width() * 0.35))),
        scale,
    );
    let cache_id = ui.make_persistent_id((&id_source, "textui_tooltip_retained_scene"));
    let fingerprint = hash_label_scene_request(text, &options.text, width_points_opt, scale);
    let scene = retained_gpu_scene(ui.ctx(), cache_id, fingerprint, || {
        Some(text_ui.prepare_label_gpu_scene_at_scale(
            (&id_source, "tooltip"),
            text,
            &options.text.to_text_label_options(),
            width_points_opt,
            scale,
        ))
    })
    .expect("synchronous textui tooltip scene should always be available");
    let raster_size = egui::vec2(scene.size_points[0], scene.size_points[1]);
    let size = raster_size + options.padding * 2.0;
    let mut rect = Rect::from_min_size(pointer + options.offset, size);
    let min_y = ui.clip_rect().top();
    if rect.min.y < min_y {
        rect = rect.translate(egui::vec2(0.0, min_y - rect.min.y));
    }
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
    paint_gpu_scene_in_rect(text_ui, &painter, text_rect, &scene, Color32::WHITE);
}

impl TextUiEguiExt for TextUi {
    fn label_async<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        label_impl(self, ui, id_source, text, options, Sense::hover(), true)
    }

    fn code_block_async<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response {
        code_block_impl(self, ui, id_source, code, options, true)
    }

    fn label<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        label_impl(self, ui, id_source, text, options, Sense::hover(), false)
    }

    fn clickable_label<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        label_impl(self, ui, id_source, text, options, Sense::click(), false)
    }

    fn measure_text_size(&mut self, ui: &Ui, text: &str, options: &LabelOptions) -> Vec2 {
        self.measure_text_size_at_scale(
            ui.ctx().pixels_per_point(),
            text,
            &options.to_text_label_options(),
        )
        .into()
    }

    fn prepare_label_texture<H: Hash>(
        &mut self,
        ctx: &Context,
        id_source: H,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle {
        let scale = ctx.pixels_per_point();
        let width_points_opt = normalize_wrapped_width(width_points_opt, scale);
        let fingerprint = hash_label_scene_request(text, options, width_points_opt, scale);
        let scene = retained_gpu_scene(
            ctx,
            Id::new((&id_source, "textui_prepare_label_texture_scene")),
            fingerprint,
            || {
                Some(self.prepare_label_gpu_scene_at_scale(
                    &id_source,
                    text,
                    &options.to_text_label_options(),
                    width_points_opt,
                    scale,
                ))
            },
        )
        .expect("synchronous label texture scene should always be available");
        TextTextureHandle {
            size_points: egui::vec2(scene.size_points[0], scene.size_points[1]),
            scene,
        }
    }

    fn prepare_rich_text_texture<H: Hash>(
        &mut self,
        ctx: &Context,
        id_source: H,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle {
        let scale = ctx.pixels_per_point();
        let width_points_opt = normalize_wrapped_width(width_points_opt, scale);
        let fingerprint = hash_rich_text_scene_request(spans, options, width_points_opt, scale);
        let scene = retained_gpu_scene(
            ctx,
            Id::new((&id_source, "textui_prepare_rich_texture_scene")),
            fingerprint,
            || {
                Some(self.prepare_rich_text_gpu_scene_at_scale(
                    &id_source,
                    spans,
                    &options.to_text_label_options(),
                    width_points_opt,
                    scale,
                ))
            },
        )
        .expect("synchronous rich text texture scene should always be available");
        TextTextureHandle {
            size_points: egui::vec2(scene.size_points[0], scene.size_points[1]),
            scene,
        }
    }

    fn paint_label_on_path<H: Hash>(
        &mut self,
        painter: &Painter,
        id_source: H,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let scene = self.prepare_label_path_gpu_scene_at_scale(
            &id_source,
            text,
            &options.to_text_label_options(),
            width_points_opt,
            painter.pixels_per_point(),
            path,
            path_options,
        )?;
        let layout = self.prepare_label_path_layout(
            text,
            &options.to_text_label_options(),
            width_points_opt,
            path,
            path_options,
        )?;
        paint_gpu_scene_absolute(self, painter, &scene, Color32::WHITE);
        Ok(layout)
    }

    fn paint_rich_text_on_path<H: Hash>(
        &mut self,
        painter: &Painter,
        id_source: H,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let scene = self.prepare_rich_text_path_gpu_scene_at_scale(
            &id_source,
            spans,
            &options.to_text_label_options(),
            width_points_opt,
            painter.pixels_per_point(),
            path,
            path_options,
        )?;
        let layout = self.prepare_rich_text_path_layout(
            spans,
            &options.to_text_label_options(),
            width_points_opt,
            path,
            path_options,
        )?;
        paint_gpu_scene_absolute(self, painter, &scene, Color32::WHITE);
        Ok(layout)
    }

    fn paint_scene_in_rect(&mut self, painter: &Painter, rect: Rect, scene: &TextRenderScene) {
        let gpu_scene = retained_gpu_scene_for_render_scene(self, painter.ctx(), scene);
        paint_gpu_scene_in_rect(self, painter, rect, &gpu_scene, Color32::WHITE)
    }

    fn paint_scene_in_rect_tinted(
        &mut self,
        painter: &Painter,
        rect: Rect,
        scene: &TextRenderScene,
        tint: egui::Color32,
    ) {
        let gpu_scene = retained_gpu_scene_for_render_scene(self, painter.ctx(), scene);
        paint_gpu_scene_in_rect(self, painter, rect, &gpu_scene, tint)
    }

    fn button<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &ButtonOptions,
    ) -> Response {
        selectable_button_impl(self, ui, id_source, text, false, options)
    }

    fn selectable_button<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        selected: bool,
        options: &ButtonOptions,
    ) -> Response {
        selectable_button_impl(self, ui, id_source, text, selected, options)
    }

    fn tooltip_for_response<H: Hash>(
        &mut self,
        ui: &Ui,
        id_source: H,
        response: &Response,
        text: &str,
        options: &TooltipOptions,
    ) {
        tooltip_impl(self, ui, id_source, response, text, options)
    }

    fn code_block<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response {
        code_block_impl(self, ui, id_source, code, options, false)
    }

    fn markdown<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        markdown: &str,
        options: &MarkdownOptions,
    ) {
        markdown_impl(self, ui, id_source, markdown, options)
    }

    fn singleline_input<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        TextUi::egui_singleline_input(self, ui, id_source, text, &options.to_core_input_options())
    }

    fn multiline_input<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        TextUi::egui_multiline_input(self, ui, id_source, text, &options.to_core_input_options())
    }

    fn multiline_rich_viewer<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        spans: &[RichTextSpan],
        options: &InputOptions,
        stick_to_bottom: bool,
        wrap: bool,
    ) -> Response {
        TextUi::egui_multiline_rich_viewer(
            self,
            ui,
            id_source,
            spans,
            &options.to_core_input_options(),
            stick_to_bottom,
            wrap,
        )
    }
}

pub mod prelude {
    pub use super::{
        ButtonOptions, CodeBlockOptions, InputOptions, LabelOptions, MarkdownOptions, RichTextSpan,
        RichTextStyle, TextColor, TextTextureHandle, TextUiEguiExt, TooltipOptions, TruncatedText,
        normalize_inline_whitespace, truncate_single_line_text_with_ellipsis,
        truncate_single_line_text_with_ellipsis_detailed,
        truncate_single_line_text_with_ellipsis_preserving_whitespace,
        truncate_single_line_text_with_ellipsis_preserving_whitespace_detailed,
    };
}

const GAMEPAD_SCROLL_DELTA_ID: &str = "textui_gamepad_scroll_delta";
const GAMEPAD_SCROLL_TARGETS_ID: &str = "textui_gamepad_scroll_targets";
const GAMEPAD_SCROLL_FRAME_ID: &str = "textui_gamepad_scroll_frame";

#[derive(Clone, Copy)]
struct GamepadScrollTarget {
    id: Id,
    rect: Rect,
    content_size: Vec2,
}

impl GamepadScrollTarget {
    fn max_offset(&self) -> Vec2 {
        Vec2::new(
            (self.content_size.x - self.rect.width()).max(0.0),
            (self.content_size.y - self.rect.height()).max(0.0),
        )
    }

    fn can_scroll_h(&self) -> bool {
        self.max_offset().x > 0.5
    }

    fn can_scroll_v(&self) -> bool {
        self.max_offset().y > 0.5
    }
}

pub fn begin_frame(
    text_ui: &mut TextUi,
    ctx: &Context,
    render_state: Option<&RenderState>,
) -> TextFrameOutput {
    text_ui.egui_set_render_state(render_state);
    text_ui.begin_frame_info(TextFrameInfo::new(
        ctx.cumulative_frame_nr(),
        ctx.input(|i| i.max_texture_side).max(1),
    ));
    text_ui.set_frame_input_events(ctx.input(|i| translate_input_events(&i.events)));
    text_ui.egui_flush_frame(ctx)
}

pub fn set_gamepad_scroll_delta(ctx: &Context, delta: Vec2) {
    ctx.data_mut(|data| data.insert_temp(Id::new(GAMEPAD_SCROLL_DELTA_ID), delta));
}

pub fn gamepad_scroll_delta(ctx: &Context) -> Vec2 {
    ctx.data_mut(|data| {
        data.get_temp::<Vec2>(Id::new(GAMEPAD_SCROLL_DELTA_ID))
            .unwrap_or(Vec2::ZERO)
    })
}

fn ensure_gamepad_scroll_targets_fresh(ctx: &Context) {
    let current = ctx.cumulative_frame_nr();
    let frame_key = Id::new(GAMEPAD_SCROLL_FRAME_ID);
    let last = ctx.data(|d| d.get_temp::<u64>(frame_key).unwrap_or(u64::MAX));
    if current != last {
        ctx.data_mut(|d| {
            d.remove::<Vec<GamepadScrollTarget>>(Id::new(GAMEPAD_SCROLL_TARGETS_ID));
            d.insert_temp(frame_key, current);
        });
    }
}

fn register_gamepad_scroll_target(ctx: &Context, id: Id, rect: Rect, content_size: Vec2) {
    ctx.data_mut(|data| {
        let key = Id::new(GAMEPAD_SCROLL_TARGETS_ID);
        let mut targets = data
            .get_temp::<Vec<GamepadScrollTarget>>(key)
            .unwrap_or_default();
        targets.retain(|target| target.id != id);
        targets.push(GamepadScrollTarget {
            id,
            rect,
            content_size,
        });
        data.insert_temp(key, targets);
    });
}

pub fn make_gamepad_scrollable<R>(ctx: &Context, output: &egui::scroll_area::ScrollAreaOutput<R>) {
    ensure_gamepad_scroll_targets_fresh(ctx);
    register_gamepad_scroll_target(ctx, output.id, output.inner_rect, output.content_size);
}

pub fn gamepad_scroll<R>(
    scroll_area: egui::ScrollArea,
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    let output = scroll_area.show(ui, add_contents);
    make_gamepad_scrollable(ui.ctx(), &output);
    output
}

pub fn apply_gamepad_scroll_to_focused_target(ctx: &Context, delta: Vec2) -> bool {
    if delta == Vec2::ZERO {
        return false;
    }

    let Some(focused_id) = ctx.memory(|memory| memory.focused()) else {
        return false;
    };

    let focused_screen_rect = ctx.read_response(focused_id).map(|r| r.rect);
    let targets = ctx.data_mut(|data| {
        data.get_temp::<Vec<GamepadScrollTarget>>(Id::new(GAMEPAD_SCROLL_TARGETS_ID))
            .unwrap_or_default()
    });

    let sort_by_area = |a: &GamepadScrollTarget, b: &GamepadScrollTarget| {
        let a_area = a.rect.width() * a.rect.height();
        let b_area = b.rect.width() * b.rect.height();
        a_area
            .partial_cmp(&b_area)
            .unwrap_or(std::cmp::Ordering::Equal)
    };

    let mut candidates: Vec<GamepadScrollTarget> = if let Some(fr) = focused_screen_rect {
        let fp = fr.center();
        let positional: Vec<_> = targets
            .iter()
            .copied()
            .filter(|t| t.rect.contains(fp) || t.rect.intersects(fr))
            .collect();
        if positional.is_empty() {
            targets
        } else {
            positional
        }
    } else {
        targets
    };
    candidates.sort_by(sort_by_area);

    if let Some(pos) = candidates.iter().position(|t| t.id == focused_id) {
        let direct = candidates.remove(pos);
        candidates.insert(0, direct);
    }

    let mut need_x = delta.x != 0.0;
    let mut need_y = delta.y != 0.0;
    let mut applied = false;

    for target in &candidates {
        if !need_x && !need_y {
            break;
        }

        let max_offset = target.max_offset();
        let can_x = need_x && target.can_scroll_h();
        let can_y = need_y && target.can_scroll_v();

        if !can_x && !can_y {
            continue;
        }

        let mut state = egui::scroll_area::State::load(ctx, target.id).unwrap_or_default();
        let mut changed = false;

        if can_x {
            let new_x = (state.offset.x - delta.x).clamp(0.0, max_offset.x);
            if new_x != state.offset.x {
                state.offset.x = new_x;
                need_x = false;
                applied = true;
                changed = true;
            }
        }
        if can_y {
            let new_y = (state.offset.y - delta.y).clamp(0.0, max_offset.y);
            if new_y != state.offset.y {
                state.offset.y = new_y;
                need_y = false;
                applied = true;
                changed = true;
            }
        }

        if changed {
            state.store(ctx, target.id);
        }
    }

    applied
}

pub fn apply_gamepad_scroll_to_registered_id(ctx: &Context, scroll_id: Id, delta: Vec2) -> bool {
    if delta == Vec2::ZERO {
        return false;
    }
    let targets = ctx.data(|d| {
        d.get_temp::<Vec<GamepadScrollTarget>>(Id::new(GAMEPAD_SCROLL_TARGETS_ID))
            .unwrap_or_default()
    });
    let Some(target) = targets.iter().find(|t| t.id == scroll_id).copied() else {
        return false;
    };
    let max_offset = target.max_offset();
    let mut state = egui::scroll_area::State::load(ctx, scroll_id).unwrap_or_default();
    let mut changed = false;
    if delta.x != 0.0 && target.can_scroll_h() {
        let new_x = (state.offset.x - delta.x).clamp(0.0, max_offset.x);
        if new_x != state.offset.x {
            state.offset.x = new_x;
            changed = true;
        }
    }
    if delta.y != 0.0 && target.can_scroll_v() {
        let new_y = (state.offset.y - delta.y).clamp(0.0, max_offset.y);
        if new_y != state.offset.y {
            state.offset.y = new_y;
            changed = true;
        }
    }
    if changed {
        state.store(ctx, scroll_id);
    }
    changed
}

pub fn apply_gamepad_scroll_if_focused(ui: &Ui, response: &Response) {
    if response.has_focus() {
        let delta = gamepad_scroll_delta(ui.ctx());
        if delta != Vec2::ZERO {
            ui.scroll_with_delta(delta);
        }
    }
}

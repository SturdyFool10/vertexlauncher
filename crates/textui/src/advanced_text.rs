use egui::{Color32, Pos2, Rect, Vec2};
use std::fmt::Write as _;

#[path = "advanced_text/rich_text_span.rs"]
mod rich_text_span;
#[path = "advanced_text/rich_text_style.rs"]
mod rich_text_style;
#[path = "advanced_text/text_atlas_page_data.rs"]
mod text_atlas_page_data;
#[path = "advanced_text/text_atlas_page_snapshot.rs"]
mod text_atlas_page_snapshot;
#[path = "advanced_text/text_atlas_quad.rs"]
mod text_atlas_quad;
#[path = "advanced_text/text_atlas_sampling.rs"]
mod text_atlas_sampling;
#[path = "advanced_text/text_color.rs"]
mod text_color;
#[path = "advanced_text/text_feature_setting.rs"]
mod text_feature_setting;
#[path = "advanced_text/text_frame_info.rs"]
mod text_frame_info;
#[path = "advanced_text/text_frame_output.rs"]
mod text_frame_output;
#[path = "advanced_text/text_fundamentals.rs"]
mod text_fundamentals;
#[path = "advanced_text/text_glyph_raster_mode.rs"]
mod text_glyph_raster_mode;
#[path = "advanced_text/text_gpu_power_preference.rs"]
mod text_gpu_power_preference;
#[path = "advanced_text/text_gpu_quad.rs"]
mod text_gpu_quad;
#[path = "advanced_text/text_gpu_scene.rs"]
mod text_gpu_scene;
#[path = "advanced_text/text_gpu_scene_draw_options.rs"]
mod text_gpu_scene_draw_options;
#[path = "advanced_text/text_gpu_scene_page_batch.rs"]
mod text_gpu_scene_page_batch;
#[path = "advanced_text/text_graphics_api.rs"]
mod text_graphics_api;
#[path = "advanced_text/text_graphics_config.rs"]
mod text_graphics_config;
#[path = "advanced_text/text_hinting_mode.rs"]
mod text_hinting_mode;
#[path = "advanced_text/text_input_event.rs"]
mod text_input_event;
#[path = "advanced_text/text_kerning.rs"]
mod text_kerning;
#[path = "advanced_text/text_key.rs"]
mod text_key;
#[path = "advanced_text/text_label_options.rs"]
mod text_label_options;
#[path = "advanced_text/text_markdown_block.rs"]
mod text_markdown_block;
#[path = "advanced_text/text_markdown_heading_level.rs"]
mod text_markdown_heading_level;
#[path = "advanced_text/text_modifiers.rs"]
mod text_modifiers;
#[path = "advanced_text/text_optical_sizing_mode.rs"]
mod text_optical_sizing_mode;
#[path = "advanced_text/text_path.rs"]
mod text_path;
#[path = "advanced_text/text_path_error.rs"]
mod text_path_error;
#[path = "advanced_text/text_path_glyph.rs"]
mod text_path_glyph;
#[path = "advanced_text/text_path_layout.rs"]
mod text_path_layout;
#[path = "advanced_text/text_path_options.rs"]
mod text_path_options;
#[path = "advanced_text/text_point.rs"]
mod text_point;
#[path = "advanced_text/text_pointer_button.rs"]
mod text_pointer_button;
#[path = "advanced_text/text_rasterization_config.rs"]
mod text_rasterization_config;
#[path = "advanced_text/text_rect.rs"]
mod text_rect;
#[path = "advanced_text/text_render_scene.rs"]
mod text_render_scene;
#[path = "advanced_text/text_renderer_backend.rs"]
mod text_renderer_backend;
#[path = "advanced_text/text_rendering_policy.rs"]
mod text_rendering_policy;
#[path = "advanced_text/text_stem_darkening_mode.rs"]
mod text_stem_darkening_mode;
#[path = "advanced_text/text_variation_setting.rs"]
mod text_variation_setting;
#[path = "advanced_text/text_vector.rs"]
mod text_vector;
#[path = "advanced_text/vector_glyph_shape.rs"]
mod vector_glyph_shape;
#[path = "advanced_text/vector_path_command.rs"]
mod vector_path_command;
#[path = "advanced_text/vector_text_shape.rs"]
mod vector_text_shape;

pub use self::rich_text_span::RichTextSpan;
pub use self::rich_text_style::RichTextStyle;
pub use self::text_atlas_page_data::TextAtlasPageData;
pub use self::text_atlas_page_snapshot::TextAtlasPageSnapshot;
pub use self::text_atlas_quad::TextAtlasQuad;
pub use self::text_atlas_sampling::TextAtlasSampling;
pub use self::text_color::TextColor;
pub use self::text_feature_setting::TextFeatureSetting;
pub use self::text_frame_info::TextFrameInfo;
pub use self::text_frame_output::TextFrameOutput;
pub use self::text_fundamentals::TextFundamentals;
pub use self::text_glyph_raster_mode::TextGlyphRasterMode;
pub use self::text_gpu_power_preference::TextGpuPowerPreference;
pub use self::text_gpu_quad::TextGpuQuad;
pub use self::text_gpu_scene::TextGpuScene;
pub use self::text_gpu_scene_draw_options::TextGpuSceneDrawOptions;
pub use self::text_gpu_scene_page_batch::TextGpuScenePageBatch;
pub use self::text_graphics_api::TextGraphicsApi;
pub use self::text_graphics_config::TextGraphicsConfig;
pub use self::text_hinting_mode::TextHintingMode;
pub use self::text_input_event::TextInputEvent;
pub use self::text_kerning::TextKerning;
pub use self::text_key::TextKey;
pub use self::text_label_options::TextLabelOptions;
pub use self::text_markdown_block::TextMarkdownBlock;
pub use self::text_markdown_heading_level::TextMarkdownHeadingLevel;
pub use self::text_modifiers::TextModifiers;
pub use self::text_optical_sizing_mode::TextOpticalSizingMode;
pub use self::text_path::TextPath;
pub use self::text_path_error::TextPathError;
pub use self::text_path_glyph::TextPathGlyph;
pub use self::text_path_layout::TextPathLayout;
pub use self::text_path_options::TextPathOptions;
pub use self::text_point::TextPoint;
pub use self::text_pointer_button::TextPointerButton;
pub use self::text_rasterization_config::TextRasterizationConfig;
pub use self::text_rect::TextRect;
pub use self::text_render_scene::TextRenderScene;
pub use self::text_renderer_backend::TextRendererBackend;
pub use self::text_rendering_policy::TextRenderingPolicy;
pub use self::text_stem_darkening_mode::TextStemDarkeningMode;
pub use self::text_variation_setting::TextVariationSetting;
pub use self::text_vector::TextVector;
pub use self::vector_glyph_shape::VectorGlyphShape;
pub use self::vector_path_command::VectorPathCommand;
pub use self::vector_text_shape::VectorTextShape;

/// Declares the rendering expectation for a text run.
///
/// This enum is the formal rendering policy described in the typography design
/// document.  It allows call sites to express whether a text run **must** stay
/// on the GPU path, **prefers** the GPU path, or **may** fall back to software
/// rendering when required.

/// Default ellipsis string used for text truncation.
pub const DEFAULT_ELLIPSIS: &str = "\u{2026}";

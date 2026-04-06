use egui::{Color32, Pos2, Rect, Vec2};
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TextColor {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl TextColor {
    pub const TRANSPARENT: Self = Self::from_rgba8(0, 0, 0, 0);
    pub const WHITE: Self = Self::from_rgba8(255, 255, 255, 255);

    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn r(self) -> u8 {
        self.r
    }

    pub const fn g(self) -> u8 {
        self.g
    }

    pub const fn b(self) -> u8 {
        self.b
    }

    pub const fn a(self) -> u8 {
        self.a
    }

    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn to_normalized_gamma_f32(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }
}

impl From<Color32> for TextColor {
    fn from(value: Color32) -> Self {
        Self::from_rgba8(value.r(), value.g(), value.b(), value.a())
    }
}

impl From<TextColor> for Color32 {
    fn from(value: TextColor) -> Self {
        Color32::from_rgba_premultiplied(value.r, value.g, value.b, value.a)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextPoint {
    pub x: f32,
    pub y: f32,
}

impl TextPoint {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<Pos2> for TextPoint {
    fn from(value: Pos2) -> Self {
        Self::new(value.x, value.y)
    }
}

impl From<TextPoint> for Pos2 {
    fn from(value: TextPoint) -> Self {
        Pos2::new(value.x, value.y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextVector {
    pub x: f32,
    pub y: f32,
}

impl TextVector {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub const fn splat(value: f32) -> Self {
        Self::new(value, value)
    }
}

impl From<Vec2> for TextVector {
    fn from(value: Vec2) -> Self {
        Self::new(value.x, value.y)
    }
}

impl From<TextVector> for Vec2 {
    fn from(value: TextVector) -> Self {
        Vec2::new(value.x, value.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRect {
    pub min: TextPoint,
    pub max: TextPoint,
}

impl TextRect {
    pub const NOTHING: Self = Self::from_min_max(
        TextPoint::new(f32::INFINITY, f32::INFINITY),
        TextPoint::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
    );

    pub const fn from_min_max(min: TextPoint, max: TextPoint) -> Self {
        Self { min, max }
    }

    pub fn from_min_size(min: TextPoint, size: TextVector) -> Self {
        Self::from_min_max(min, TextPoint::new(min.x + size.x, min.y + size.y))
    }

    pub fn from_center_size(center: TextPoint, size: TextVector) -> Self {
        let half = TextVector::new(size.x * 0.5, size.y * 0.5);
        Self::from_min_max(
            TextPoint::new(center.x - half.x, center.y - half.y),
            TextPoint::new(center.x + half.x, center.y + half.y),
        )
    }

    pub fn width(self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn union(self, other: Self) -> Self {
        if self == Self::NOTHING {
            return other;
        }
        if other == Self::NOTHING {
            return self;
        }
        Self::from_min_max(
            TextPoint::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            TextPoint::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        )
    }
}

impl Default for TextRect {
    fn default() -> Self {
        Self::NOTHING
    }
}

impl From<Rect> for TextRect {
    fn from(value: Rect) -> Self {
        Self::from_min_max(value.min.into(), value.max.into())
    }
}

impl From<TextRect> for Rect {
    fn from(value: TextRect) -> Self {
        Rect::from_min_max(value.min.into(), value.max.into())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextKerning {
    Auto,
    Normal,
    None,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextFeatureSetting {
    pub tag: [u8; 4],
    pub value: u16,
}

impl TextFeatureSetting {
    pub const fn new(tag: [u8; 4], value: u16) -> Self {
        Self { tag, value }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextVariationSetting {
    pub tag: [u8; 4],
    value_bits: u32,
}

impl TextVariationSetting {
    pub const fn from_bits(tag: [u8; 4], value_bits: u32) -> Self {
        Self { tag, value_bits }
    }

    pub fn new(tag: [u8; 4], value: f32) -> Self {
        Self::from_bits(tag, value.to_bits())
    }

    pub const fn value_bits(self) -> u32 {
        self.value_bits
    }

    pub fn value(self) -> f32 {
        f32::from_bits(self.value_bits)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextFundamentals {
    pub kerning: TextKerning,
    pub stem_darkening: bool,
    pub standard_ligatures: bool,
    pub contextual_alternates: bool,
    pub discretionary_ligatures: bool,
    pub historical_ligatures: bool,
    pub case_sensitive_forms: bool,
    pub slashed_zero: bool,
    pub tabular_numbers: bool,
    pub letter_spacing_points: f32,
    pub word_spacing_points: f32,
    pub feature_settings: Vec<TextFeatureSetting>,
    pub variation_settings: Vec<TextVariationSetting>,
}

impl Default for TextFundamentals {
    fn default() -> Self {
        Self {
            kerning: TextKerning::Auto,
            stem_darkening: true,
            standard_ligatures: true,
            contextual_alternates: true,
            discretionary_ligatures: false,
            historical_ligatures: false,
            case_sensitive_forms: false,
            slashed_zero: false,
            tabular_numbers: false,
            letter_spacing_points: 0.0,
            word_spacing_points: 0.0,
            feature_settings: Vec::new(),
            variation_settings: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextPath {
    pub points: Vec<TextPoint>,
    pub closed: bool,
}

impl TextPath {
    pub fn new(points: impl Into<Vec<TextPoint>>) -> Self {
        Self {
            points: points.into(),
            closed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPathOptions {
    pub start_offset_points: f32,
    pub normal_offset_points: f32,
    pub rotate_glyphs: bool,
}

impl Default for TextPathOptions {
    fn default() -> Self {
        Self {
            start_offset_points: 0.0,
            normal_offset_points: 0.0,
            rotate_glyphs: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextPathError {
    EmptyPath,
    PathTooShort,
    EmptyText,
}

#[derive(Clone, Debug)]
pub struct TextPathGlyph {
    pub anchor: TextPoint,
    pub tangent: TextVector,
    pub normal: TextVector,
    pub rotation_radians: f32,
    pub local_offset: TextVector,
    pub advance_points: f32,
    pub color: TextColor,
}

#[derive(Clone, Debug)]
pub struct TextPathLayout {
    pub glyphs: Vec<TextPathGlyph>,
    pub bounds: TextRect,
    pub total_advance_points: f32,
    pub path_length_points: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VectorPathCommand {
    MoveTo(TextPoint),
    LineTo(TextPoint),
    QuadTo(TextPoint, TextPoint),
    CurveTo(TextPoint, TextPoint, TextPoint),
    Close,
}

#[derive(Clone, Debug)]
pub struct VectorGlyphShape {
    pub bounds: TextRect,
    pub color: TextColor,
    pub commands: Vec<VectorPathCommand>,
}

#[derive(Clone, Debug)]
pub struct VectorTextShape {
    pub glyphs: Vec<VectorGlyphShape>,
    pub bounds: TextRect,
}

#[derive(Clone, Debug)]
pub struct TextAtlasPageSnapshot {
    pub page_index: usize,
    pub size_px: [usize; 2],
    pub rgba8: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct RichTextStyle {
    pub color: TextColor,
    pub monospace: bool,
    pub italic: bool,
    pub weight: u16,
}

impl Default for RichTextStyle {
    fn default() -> Self {
        Self {
            color: TextColor::WHITE,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextRendererBackend {
    Auto,
    EguiMesh,
    WgpuInstanced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextAtlasSampling {
    Linear,
    Nearest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextGraphicsApi {
    Auto,
    Vulkan,
    Metal,
    Dx12,
    Gl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextGpuPowerPreference {
    Auto,
    LowPower,
    HighPerformance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextHintingMode {
    Auto,
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextStemDarkeningMode {
    Auto,
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextOpticalSizingMode {
    Auto,
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRasterizationConfig {
    pub hinting: TextHintingMode,
    pub stem_darkening: TextStemDarkeningMode,
    pub optical_sizing: TextOpticalSizingMode,
    pub stem_darkening_min_ppem: f32,
    pub stem_darkening_max_ppem: f32,
    pub stem_darkening_max_strength: f32,
}

impl Default for TextRasterizationConfig {
    fn default() -> Self {
        Self {
            hinting: TextHintingMode::Auto,
            stem_darkening: TextStemDarkeningMode::Auto,
            optical_sizing: TextOpticalSizingMode::Auto,
            stem_darkening_min_ppem: 10.0,
            stem_darkening_max_ppem: 50.0,
            stem_darkening_max_strength: 0.4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGraphicsConfig {
    pub renderer_backend: TextRendererBackend,
    pub atlas_sampling: TextAtlasSampling,
    pub atlas_page_target_px: usize,
    pub atlas_padding_px: usize,
    pub graphics_api: TextGraphicsApi,
    pub gpu_power_preference: TextGpuPowerPreference,
    pub rasterization: TextRasterizationConfig,
}

impl Default for TextGraphicsConfig {
    fn default() -> Self {
        Self {
            renderer_backend: TextRendererBackend::Auto,
            atlas_sampling: TextAtlasSampling::Linear,
            atlas_page_target_px: 1024,
            atlas_padding_px: 1,
            graphics_api: TextGraphicsApi::Auto,
            gpu_power_preference: TextGpuPowerPreference::Auto,
            rasterization: TextRasterizationConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextFrameInfo {
    pub frame_number: u64,
    pub max_texture_side_px: usize,
}

impl TextFrameInfo {
    pub const fn new(frame_number: u64, max_texture_side_px: usize) -> Self {
        Self {
            frame_number,
            max_texture_side_px,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextFrameOutput {
    pub needs_repaint: bool,
}

#[derive(Clone, Debug)]
pub struct TextAtlasPageData {
    pub page_index: usize,
    pub size_px: [usize; 2],
    pub rgba8: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct TextAtlasQuad {
    pub atlas_page_index: usize,
    pub positions: [TextPoint; 4],
    pub uvs: [TextPoint; 4],
    pub tint: TextColor,
    pub is_color: bool,
}

#[derive(Clone, Debug)]
pub struct TextRenderScene {
    pub quads: Vec<TextAtlasQuad>,
    pub bounds: TextRect,
    pub size_points: TextVector,
}

#[derive(Clone, Debug)]
pub struct TextGpuQuad {
    pub atlas_page_index: usize,
    pub positions: [[f32; 2]; 4],
    pub uvs: [[f32; 2]; 4],
    pub tint_rgba: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct TextGpuScene {
    pub atlas_pages: Vec<TextAtlasPageData>,
    pub quads: Vec<TextGpuQuad>,
    pub bounds_min: [f32; 2],
    pub bounds_max: [f32; 2],
    pub size_points: [f32; 2],
}

impl TextRenderScene {
    pub fn atlas_page_indices(&self) -> Vec<usize> {
        let mut page_indices = self
            .quads
            .iter()
            .map(|quad| quad.atlas_page_index)
            .collect::<Vec<_>>();
        page_indices.sort_unstable();
        page_indices.dedup();
        page_indices
    }

    pub fn to_gpu_scene(&self, atlas_pages: Vec<TextAtlasPageData>) -> TextGpuScene {
        TextGpuScene {
            atlas_pages,
            quads: self
                .quads
                .iter()
                .map(|quad| TextGpuQuad {
                    atlas_page_index: quad.atlas_page_index,
                    positions: quad.positions.map(|point| [point.x, point.y]),
                    uvs: quad.uvs.map(|point| [point.x, point.y]),
                    tint_rgba: quad.tint.to_array(),
                })
                .collect(),
            bounds_min: [self.bounds.min.x, self.bounds.min.y],
            bounds_max: [self.bounds.max.x, self.bounds.max.y],
            size_points: [self.size_points.x, self.size_points.y],
        }
    }
}

impl TextAtlasPageSnapshot {
    pub fn to_rgba8(&self) -> TextAtlasPageData {
        TextAtlasPageData {
            page_index: self.page_index,
            size_px: self.size_px,
            rgba8: self.rgba8.clone(),
        }
    }
}

impl VectorGlyphShape {
    pub fn to_svg_path_data(&self) -> String {
        let mut path = String::new();
        for command in &self.commands {
            match command {
                VectorPathCommand::MoveTo(point) => {
                    let _ = write!(path, "M{} {} ", point.x, point.y);
                }
                VectorPathCommand::LineTo(point) => {
                    let _ = write!(path, "L{} {} ", point.x, point.y);
                }
                VectorPathCommand::QuadTo(control, point) => {
                    let _ = write!(
                        path,
                        "Q{} {} {} {} ",
                        control.x, control.y, point.x, point.y
                    );
                }
                VectorPathCommand::CurveTo(control_a, control_b, point) => {
                    let _ = write!(
                        path,
                        "C{} {} {} {} {} {} ",
                        control_a.x, control_a.y, control_b.x, control_b.y, point.x, point.y
                    );
                }
                VectorPathCommand::Close => path.push_str("Z "),
            }
        }
        path.trim_end().to_owned()
    }
}

impl VectorTextShape {
    pub fn to_svg_document(&self) -> String {
        let bounds = if self.bounds.width() > 0.0 && self.bounds.height() > 0.0 {
            self.bounds
        } else {
            TextRect::from_min_size(TextPoint::ZERO, TextVector::splat(1.0))
        };
        let mut svg = String::new();
        let _ = write!(
            svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}">"#,
            bounds.min.x,
            bounds.min.y,
            bounds.width().max(1.0),
            bounds.height().max(1.0)
        );
        for glyph in &self.glyphs {
            let _ = write!(
                svg,
                r#"<path d="{}" fill="{}"/>"#,
                glyph.to_svg_path_data(),
                svg_color(glyph.color)
            );
        }
        svg.push_str("</svg>");
        svg
    }
}

fn svg_color(color: TextColor) -> String {
    let alpha = color.a() as f32 / 255.0;
    format!(
        "rgba({}, {}, {}, {:.3})",
        color.r(),
        color.g(),
        color.b(),
        alpha
    )
}

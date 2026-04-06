use egui::{Color32, Pos2, Rect, Vec2};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextKerning {
    Auto,
    Normal,
    None,
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
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextPath {
    pub points: Vec<Pos2>,
    pub closed: bool,
}

impl TextPath {
    pub fn new(points: impl Into<Vec<Pos2>>) -> Self {
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
    pub anchor: Pos2,
    pub tangent: Vec2,
    pub normal: Vec2,
    pub rotation_radians: f32,
    pub local_offset: Vec2,
    pub advance_points: f32,
    pub color: Color32,
}

#[derive(Clone, Debug)]
pub struct TextPathLayout {
    pub glyphs: Vec<TextPathGlyph>,
    pub bounds: Rect,
    pub total_advance_points: f32,
    pub path_length_points: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VectorPathCommand {
    MoveTo(Pos2),
    LineTo(Pos2),
    QuadTo(Pos2, Pos2),
    CurveTo(Pos2, Pos2, Pos2),
    Close,
}

#[derive(Clone, Debug)]
pub struct VectorGlyphShape {
    pub bounds: Rect,
    pub color: Color32,
    pub commands: Vec<VectorPathCommand>,
}

#[derive(Clone, Debug)]
pub struct VectorTextShape {
    pub glyphs: Vec<VectorGlyphShape>,
    pub bounds: Rect,
}

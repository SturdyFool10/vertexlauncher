use egui::{Color32, Pos2, Rect, Vec2};
use std::fmt::Write as _;

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
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0))
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

fn svg_color(color: Color32) -> String {
    let alpha = color.a() as f32 / 255.0;
    format!(
        "rgba({}, {}, {}, {:.3})",
        color.r(),
        color.g(),
        color.b(),
        alpha
    )
}

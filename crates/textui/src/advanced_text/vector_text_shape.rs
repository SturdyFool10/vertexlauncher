use super::*;

#[derive(Clone, Debug)]
pub struct VectorTextShape {
    pub glyphs: Vec<VectorGlyphShape>,
    pub bounds: TextRect,
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

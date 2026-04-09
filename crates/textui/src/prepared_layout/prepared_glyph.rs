use super::*;

#[derive(Clone, Debug)]
pub(crate) struct PreparedGlyph {
    pub(crate) cache_key: GlyphRasterKey,
    pub(crate) offset_points: Vec2,
    pub(crate) color: Color32,
}

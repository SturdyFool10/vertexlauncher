use super::*;

#[derive(Clone, Debug)]
pub(crate) struct PreparedTextLayout {
    pub(crate) glyphs: Arc<[PreparedGlyph]>,
    pub(crate) size_points: Vec2,
    pub(crate) approx_bytes: usize,
}

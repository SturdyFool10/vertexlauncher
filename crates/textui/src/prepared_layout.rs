use super::*;

#[derive(Clone, Debug)]
pub(crate) struct PreparedTextLayout {
    pub(crate) glyphs: Arc<[PreparedGlyph]>,
    pub(crate) size_points: Vec2,
    pub(crate) approx_bytes: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedGlyph {
    pub(crate) cache_key: GlyphRasterKey,
    pub(crate) offset_points: Vec2,
    pub(crate) color: Color32,
}

pub(crate) struct PreparedTextCacheEntry {
    pub(crate) fingerprint: u64,
    pub(crate) layout: Arc<PreparedTextLayout>,
    pub(crate) last_used_frame: u64,
}

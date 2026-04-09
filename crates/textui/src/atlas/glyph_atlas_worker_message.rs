use super::*;

pub(super) enum GlyphAtlasWorkerMessage {
    RegisterFont(Vec<u8>),
    Rasterize {
        generation: u64,
        cache_key: GlyphRasterKey,
        rasterization: TextRasterizationConfig,
        padding_px: usize,
    },
}

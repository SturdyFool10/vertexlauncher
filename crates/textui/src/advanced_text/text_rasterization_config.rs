use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRasterizationConfig {
    pub glyph_raster_mode: TextGlyphRasterMode,
    pub hinting: TextHintingMode,
    pub stem_darkening: TextStemDarkeningMode,
    pub optical_sizing: TextOpticalSizingMode,
    pub field_range_px: f32,
    pub stem_darkening_min_ppem: f32,
    pub stem_darkening_max_ppem: f32,
    pub stem_darkening_max_strength: f32,
}

impl Default for TextRasterizationConfig {
    fn default() -> Self {
        Self {
            glyph_raster_mode: TextGlyphRasterMode::Auto,
            hinting: TextHintingMode::Auto,
            stem_darkening: TextStemDarkeningMode::Auto,
            optical_sizing: TextOpticalSizingMode::Auto,
            field_range_px: 8.0,
            stem_darkening_min_ppem: 14.0,
            stem_darkening_max_ppem: 28.0,
            stem_darkening_max_strength: 0.22,
        }
    }
}

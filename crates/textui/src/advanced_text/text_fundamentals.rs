use super::*;

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
    pub smart_quotes: bool,
    pub letter_spacing_points: f32,
    pub word_spacing_points: f32,
    pub letter_spacing_floor: f32,
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
            smart_quotes: true,
            letter_spacing_points: 0.0,
            word_spacing_points: 0.0,
            letter_spacing_floor: -0.5,
            feature_settings: Vec::new(),
            variation_settings: Vec::new(),
        }
    }
}

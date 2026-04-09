use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct DiscoverUiMetrics {
    pub(crate) card_min_width: f32,
    pub(crate) card_image_height: f32,
    pub(crate) estimated_card_base_height: f32,
}

impl DiscoverUiMetrics {
    pub(crate) fn from_ui(ui: &Ui) -> Self {
        let metrics = UiMetrics::from_ui(ui, 860.0);
        Self {
            card_min_width: metrics.scaled_width(0.18, 220.0, 300.0),
            card_image_height: metrics.scaled_height(0.15, 112.0, 160.0),
            estimated_card_base_height: metrics.scaled_height(0.24, 188.0, 236.0),
        }
    }
}

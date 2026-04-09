use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct HomeUiMetrics {
    pub(crate) tab_height: f32,
    pub(crate) instance_row_height: f32,
    pub(crate) activity_row_height: f32,
    pub(crate) screenshot_overlay_button_size: f32,
    pub(crate) screenshot_min_column_width: f32,
    pub(crate) action_button_width: f32,
}

impl HomeUiMetrics {
    pub(crate) fn from_ui(ui: &Ui) -> Self {
        let metrics = UiMetrics::from_ui(ui, 820.0);
        Self {
            tab_height: metrics.scaled_height(0.045, 34.0, 40.0),
            instance_row_height: metrics.scaled_height(0.04, 34.0, 42.0),
            activity_row_height: metrics.scaled_height(0.062, 50.0, 62.0),
            screenshot_overlay_button_size: metrics.scaled_width(0.022, 24.0, 30.0),
            screenshot_min_column_width: metrics.scaled_width(0.24, 180.0, 320.0),
            action_button_width: metrics.scaled_width(0.075, 92.0, 120.0),
        }
    }
}

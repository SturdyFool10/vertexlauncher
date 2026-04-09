use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct ContentBrowserUiMetrics {
    pub(super) action_button_width: f32,
    pub(super) action_button_height: f32,
    pub(super) download_progress_width: f32,
    pub(super) result_thumbnail_size: f32,
}

impl ContentBrowserUiMetrics {
    pub(super) fn from_ui(ui: &Ui) -> Self {
        let metrics = UiMetrics::from_ui(ui, 860.0);
        Self {
            action_button_width: metrics.scaled_width(0.02, TILE_ACTION_BUTTON_WIDTH, 34.0),
            action_button_height: metrics.scaled_height(0.036, TILE_ACTION_BUTTON_HEIGHT, 34.0),
            download_progress_width: metrics.scaled_width(
                0.08,
                TILE_DOWNLOAD_PROGRESS_WIDTH,
                124.0,
            ),
            result_thumbnail_size: metrics.scaled_width(0.075, 84.0, 108.0),
        }
    }
}

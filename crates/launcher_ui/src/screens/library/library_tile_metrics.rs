use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct LibraryTileMetrics {
    pub(super) tile_width: f32,
    pub(super) tile_height: f32,
    pub(super) thumbnail_height: f32,
    pub(super) centered_thumbnail_width: f32,
    pub(super) name_scroll_height: f32,
    pub(super) description_scroll_height: f32,
}

impl LibraryTileMetrics {
    pub(super) fn from_ui(ui: &Ui) -> (UiMetrics, usize, Self) {
        let metrics = UiMetrics::from_ui(ui, LIBRARY_GRID_COMPACT_THRESHOLD);
        let available_width = ui.available_width().max(1.0);
        let gap = style::SPACE_XL;
        let min_tile_width = if metrics.compact { 220.0 } else { 260.0 };
        let max_columns = if metrics.compact { 2 } else { 4 };
        let (columns, tile_width) =
            metrics.columns(available_width, min_tile_width, gap, max_columns);
        let thumbnail_height = (tile_width * 0.5).clamp(120.0, 170.0);
        let tile_height = (thumbnail_height
            + (tile_width * 0.56)
            + style::CONTROL_HEIGHT_LG
            + TILE_DELETE_BUTTON_HEIGHT
            + style::SPACE_XL * 3.0)
            .clamp(340.0, 470.0);
        let name_scroll_height = (tile_height * 0.14).clamp(44.0, 72.0);
        let description_scroll_height = (tile_height * 0.22).clamp(68.0, 120.0);
        (
            metrics,
            columns,
            Self {
                tile_width,
                tile_height,
                thumbnail_height,
                centered_thumbnail_width: (tile_width * 0.74).min(220.0),
                name_scroll_height,
                description_scroll_height,
            },
        )
    }
}

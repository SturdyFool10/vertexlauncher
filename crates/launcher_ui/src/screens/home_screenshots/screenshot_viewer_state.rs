use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ScreenshotViewerState {
    pub(crate) screenshot_key: String,
    /// Snapshot of the entry's metadata and path as of the last time it was found in the gallery.
    /// Used to retain and display the image during rescans when `screenshots` is temporarily empty.
    pub(crate) entry_snapshot: ScreenshotEntry,
    pub(crate) zoom: f32,
    pub(crate) pan_uv: egui::Vec2,
}

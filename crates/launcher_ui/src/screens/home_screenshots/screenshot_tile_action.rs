#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ScreenshotTileAction {
    pub(super) open_viewer: bool,
    pub(super) request_delete: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ScreenshotOverlayButtonResult {
    pub(super) clicked: bool,
    pub(super) contains_pointer: bool,
}

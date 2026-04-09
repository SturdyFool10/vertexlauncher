use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ScreenshotOverlayResult {
    pub(super) action: Option<ScreenshotOverlayAction>,
    pub(super) contains_pointer: bool,
}

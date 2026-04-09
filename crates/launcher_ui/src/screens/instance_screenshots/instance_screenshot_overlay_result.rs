use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct InstanceScreenshotOverlayResult {
    pub(super) action: Option<InstanceScreenshotOverlayAction>,
    pub(super) contains_pointer: bool,
}

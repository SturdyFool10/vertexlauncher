#[derive(Clone, Copy, Debug, Default)]
pub(super) struct InstanceScreenshotOverlayButtonResult {
    pub(super) clicked: bool,
    pub(super) contains_pointer: bool,
}

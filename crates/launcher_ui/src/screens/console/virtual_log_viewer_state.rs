#[derive(Clone, Debug, Default)]
pub(super) struct VirtualLogViewerState {
    pub(super) initialized: bool,
    pub(super) max_line_width: f32,
    pub(super) follow_bottom: bool,
}

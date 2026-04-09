#[derive(Clone, Copy, Debug)]
pub(crate) struct DiscoverTileRenderResult {
    pub(crate) clicked: bool,
    pub(crate) measured_height: f32,
}

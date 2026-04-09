use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedTextGraphicsConfig {
    pub(crate) renderer_backend: ResolvedTextRendererBackend,
    pub(crate) atlas_sampling: TextAtlasSampling,
    pub(crate) atlas_page_target_px: usize,
    pub(crate) atlas_padding_px: usize,
    pub(crate) rasterization: TextRasterizationConfig,
}

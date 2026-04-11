use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedTextGraphicsConfig {
    pub(crate) renderer_backend: ResolvedTextRendererBackend,
    pub(crate) atlas_sampling: TextAtlasSampling,
    pub(crate) atlas_page_target_px: usize,
    pub(crate) atlas_padding_px: usize,
    pub(crate) rasterization: TextRasterizationConfig,
    /// When true, outputting to HDR surface - shader passes through in scene-linear space.
    /// When false, applies tone mapping + sRGB encode for SDR output.
    pub(crate) output_is_hdr: bool,
}

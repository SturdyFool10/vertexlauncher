#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResolvedTextRendererBackend {
    EguiMesh,
    WgpuInstanced,
}

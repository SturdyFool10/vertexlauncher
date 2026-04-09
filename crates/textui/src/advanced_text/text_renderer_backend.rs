#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextRendererBackend {
    Auto,
    EguiMesh,
    WgpuInstanced,
}

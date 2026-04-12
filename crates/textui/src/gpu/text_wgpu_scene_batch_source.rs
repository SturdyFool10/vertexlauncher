use super::*;

#[derive(Clone)]
pub(crate) struct TextWgpuSceneBatchSource {
    pub(crate) atlas_generation: u64,
    pub(crate) page_index: usize,
    pub(crate) texture: wgpu::Texture,
    pub(crate) instances: Arc<[TextWgpuInstance]>,
}

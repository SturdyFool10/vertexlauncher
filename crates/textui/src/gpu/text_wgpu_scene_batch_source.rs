use super::*;

#[derive(Clone)]
pub(crate) struct TextWgpuSceneBatchSource {
    pub(crate) texture: wgpu::Texture,
    pub(crate) instances: Arc<[TextWgpuInstance]>,
}

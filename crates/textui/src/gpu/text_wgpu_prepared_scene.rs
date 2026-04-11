use super::*;

#[derive(Default)]
pub(crate) struct TextWgpuReusableInstanceBuffer {
    pub(crate) buffer: Option<wgpu::Buffer>,
    pub(crate) capacity_bytes: u64,
}

#[derive(Default)]
pub(crate) struct TextWgpuPreparedScene {
    pub(crate) batches: Vec<TextWgpuPreparedBatch>,
}

pub(crate) struct TextWgpuPreparedBatch {
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) instance_buffer: wgpu::Buffer,
    pub(crate) instance_buffer_offset: u64,
    pub(crate) instance_count: u32,
}

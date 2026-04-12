use super::*;
use std::collections::HashMap;

pub(crate) struct TextWgpuCachedTextureBinding {
    pub(crate) bind_group: wgpu::BindGroup,
}

#[derive(Default)]
pub(crate) struct TextWgpuTextureBindingCache {
    pub(crate) entries: HashMap<(u64, usize), TextWgpuCachedTextureBinding>,
}

#[derive(Default)]
pub(crate) struct TextWgpuPreparedScene {
    pub(crate) instance_buffer: Option<wgpu::Buffer>,
    pub(crate) batches: Vec<TextWgpuPreparedBatch>,
}

use super::*;

pub(super) enum TextureSlot {
    Skin,
    Cape,
}

pub(super) struct UploadedPreviewTexture {
    pub(super) hash: u64,
    pub(super) size: [u32; 2],
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) _texture: wgpu::Texture,
}

/// Allocates a sampled render target used by post-process passes.
///
/// Zero-sized dimensions are clamped to `1x1`, and `sample_count` values below `1`
/// are clamped to `1`.
///
/// This function does not panic.
pub(super) fn create_sampled_render_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    sample_count: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: sample_count.max(1),
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = create_preview_texture_bind_group(device, layout, sampler, &view, label);
    (texture, view, bind_group)
}

pub(super) fn create_skin_preview_sampler(
    device: &wgpu::Device,
    label: &'static str,
) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: SKIN_PREVIEW_ANISOTROPY_CLAMP,
        ..Default::default()
    })
}

pub(super) fn create_preview_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
    label: &'static str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

pub(super) fn create_preview_color_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    sample_count: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: sample_count.max(1),
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

pub(super) fn create_preview_depth_texture(
    device: &wgpu::Device,
    size: [u32; 2],
    sample_count: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: sample_count.max(1),
        dimension: wgpu::TextureDimension::D2,
        format: SKIN_PREVIEW_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

use super::*;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct GpuPreviewVertex {
    pub(super) pos_points: [f32; 2],
    pub(super) camera_z: f32,
    pub(super) uv: [f32; 2],
    pub(super) color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct GpuPreviewUniform {
    pub(super) screen_size_points: [f32; 2],
    pub(super) _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct GpuPreviewScalarUniform {
    pub(super) value: [f32; 4],
}

pub(super) struct GpuPreviewSceneBatch {
    pub(super) weight: f32,
    pub(super) skin_vertices: Vec<GpuPreviewVertex>,
    pub(super) skin_indices: Vec<u32>,
    pub(super) cape_vertices: Vec<GpuPreviewVertex>,
    pub(super) cape_indices: Vec<u32>,
}

pub(super) struct PreparedGpuPreviewSceneBatch {
    pub(super) _weight_buffer: wgpu::Buffer,
    pub(super) weight_bind_group: wgpu::BindGroup,
    pub(super) skin_vertex_buffer: wgpu::Buffer,
    pub(super) skin_index_buffer: wgpu::Buffer,
    pub(super) cape_vertex_buffer: wgpu::Buffer,
    pub(super) cape_index_buffer: wgpu::Buffer,
    pub(super) skin_index_count: u32,
    pub(super) cape_index_count: u32,
}

/// Converts projected preview triangles into GPU-ready skin and cape batches.
///
/// `weight` should be finite and usually falls within `0.0..=1.0`, although the
/// accumulation pass accepts larger finite values.
///
/// This function does not panic.
pub(super) fn build_preview_scene_batch(
    triangles: &[RenderTriangle],
    weight: f32,
) -> GpuPreviewSceneBatch {
    let mut skin_vertices = Vec::with_capacity(triangles.len() * 3);
    let mut skin_indices = Vec::with_capacity(triangles.len() * 3);
    let mut cape_vertices = Vec::new();
    let mut cape_indices = Vec::new();

    for tri in triangles {
        let target = match tri.texture {
            TriangleTexture::Skin => (&mut skin_vertices, &mut skin_indices),
            TriangleTexture::Cape => (&mut cape_vertices, &mut cape_indices),
        };
        let base = target.0.len() as u32;
        for i in 0..3 {
            target.0.push(GpuPreviewVertex {
                pos_points: [tri.pos[i].x, tri.pos[i].y],
                camera_z: tri.depth[i].max(SKIN_PREVIEW_NEAR + 0.000_1),
                uv: [tri.uv[i].x, tri.uv[i].y],
                color: tri.color.to_normalized_gamma_f32(),
            });
        }
        target
            .1
            .extend_from_slice(&[base, base.saturating_add(1), base.saturating_add(2)]);
    }

    GpuPreviewSceneBatch {
        weight,
        skin_vertices,
        skin_indices,
        cape_vertices,
        cape_indices,
    }
}

/// Uploads one scene batch to GPU buffers and bind groups.
///
/// `batch` may contain empty vertex or index lists; in that case placeholder
/// buffers are created so the render pipeline layout stays valid.
///
/// This function does not panic.
pub(super) fn prepare_preview_scene_batch_buffers(
    device: &wgpu::Device,
    scalar_uniform_bind_group_layout: &wgpu::BindGroupLayout,
    batch: &GpuPreviewSceneBatch,
) -> PreparedGpuPreviewSceneBatch {
    let weight_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("skins-preview-batch-weight-buffer"),
        contents: bytemuck::bytes_of(&GpuPreviewScalarUniform {
            value: [batch.weight, 0.0, 0.0, 0.0],
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let weight_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("skins-preview-batch-weight-bind-group"),
        layout: scalar_uniform_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: weight_buffer.as_entire_binding(),
        }],
    });

    PreparedGpuPreviewSceneBatch {
        _weight_buffer: weight_buffer,
        weight_bind_group,
        skin_vertex_buffer: create_preview_vertex_buffer(
            device,
            "skins-preview-batch-skin-vertex-buffer",
            &batch.skin_vertices,
        ),
        skin_index_buffer: create_preview_index_buffer(
            device,
            "skins-preview-batch-skin-index-buffer",
            &batch.skin_indices,
        ),
        cape_vertex_buffer: create_preview_vertex_buffer(
            device,
            "skins-preview-batch-cape-vertex-buffer",
            &batch.cape_vertices,
        ),
        cape_index_buffer: create_preview_index_buffer(
            device,
            "skins-preview-batch-cape-index-buffer",
            &batch.cape_indices,
        ),
        skin_index_count: batch.skin_indices.len() as u32,
        cape_index_count: batch.cape_indices.len() as u32,
    }
}

fn create_preview_vertex_buffer(
    device: &wgpu::Device,
    label: &'static str,
    vertices: &[GpuPreviewVertex],
) -> wgpu::Buffer {
    if vertices.is_empty() {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<GpuPreviewVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        })
    } else {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }
}

fn create_preview_index_buffer(
    device: &wgpu::Device,
    label: &'static str,
    indices: &[u32],
) -> wgpu::Buffer {
    if indices.is_empty() {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<u32>() as u64,
            usage: wgpu::BufferUsages::INDEX,
            mapped_at_creation: false,
        })
    } else {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        })
    }
}

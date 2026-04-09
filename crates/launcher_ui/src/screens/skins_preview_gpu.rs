use super::*;

#[path = "skins_preview_gpu_effects.rs"]
mod skins_preview_gpu_effects;
#[path = "skins_preview_gpu_geometry.rs"]
mod skins_preview_gpu_geometry;
#[path = "skins_preview_gpu_post_process.rs"]
mod skins_preview_gpu_post_process;

pub(super) use self::skins_preview_gpu_effects::PreviewHistory;
pub(super) use self::skins_preview_gpu_geometry::{
    ElytraWingUvs, add_cape_triangles, add_elytra_triangles,
};
use self::skins_preview_gpu_post_process::SkinPreviewPostProcessWgpuCallback;

pub(super) fn render_motion_blur_wgpu_scene(
    ui: &Ui,
    rect: Rect,
    scenes: &[WeightedPreviewScene],
    skin_sample: Arc<RgbaImage>,
    cape_sample: Option<Arc<RgbaImage>>,
    target_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
    present_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
) {
    let callback = SkinPreviewPostProcessWgpuCallback::from_weighted_scenes(
        scenes,
        skin_sample,
        cape_sample,
        target_format,
        scene_msaa_samples,
        present_msaa_samples,
        preview_aa_mode,
        preview_texel_aa_mode,
    );
    let callback_shape = egui_wgpu::Callback::new_paint_callback(rect, callback);
    ui.painter().add(callback_shape);
}

pub(super) fn render_depth_buffered_scene(
    ui: &Ui,
    painter: &egui::Painter,
    rect: Rect,
    triangles: &[RenderTriangle],
    skin_texture: &TextureHandle,
    cape_texture: Option<&TextureHandle>,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    wgpu_target_format: Option<wgpu::TextureFormat>,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
    preview_texture: &mut Option<TextureHandle>,
    preview_history: &mut Option<PreviewHistory>,
) {
    let Some(target_format) = wgpu_target_format else {
        paint_scene_fallback_mesh(painter, triangles, skin_texture, cape_texture);
        return;
    };
    let Some(skin_sample) = skin_sample else {
        paint_scene_fallback_mesh(painter, triangles, skin_texture, cape_texture);
        return;
    };

    let callback = SkinPreviewPostProcessWgpuCallback::from_scene(
        triangles,
        skin_sample,
        cape_sample,
        target_format,
        if preview_aa_mode == SkinPreviewAaMode::Msaa {
            preview_msaa_samples.max(1)
        } else {
            1
        },
        preview_msaa_samples.max(1),
        preview_aa_mode,
        preview_texel_aa_mode,
    );
    let callback_shape = egui_wgpu::Callback::new_paint_callback(rect, callback);
    ui.painter().add(callback_shape);
    let _ = (preview_texture, preview_history);
}

fn paint_scene_fallback_mesh(
    painter: &egui::Painter,
    triangles: &[RenderTriangle],
    skin_texture: &TextureHandle,
    cape_texture: Option<&TextureHandle>,
) {
    for tri in triangles {
        let texture_id = match tri.texture {
            TriangleTexture::Skin => skin_texture.id(),
            TriangleTexture::Cape => match cape_texture {
                Some(texture) => texture.id(),
                None => continue,
            },
        };
        let mut mesh = egui::epaint::Mesh::with_texture(texture_id);
        mesh.vertices.push(egui::epaint::Vertex {
            pos: tri.pos[0],
            uv: tri.uv[0],
            color: tri.color,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: tri.pos[1],
            uv: tri.uv[1],
            color: tri.color,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: tri.pos[2],
            uv: tri.uv[2],
            color: tri.color,
        });
        mesh.indices.extend_from_slice(&[0, 1, 2]);
        painter.add(egui::Shape::mesh(mesh));
    }
}

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

pub(super) fn build_gpu_preview_scene_batch(
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

pub(super) fn prepare_gpu_preview_scene_batch(
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

pub(super) fn create_preview_vertex_buffer(
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

pub(super) fn create_preview_index_buffer(
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

pub(super) fn create_preview_render_texture(
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
        mipmap_filter: wgpu::FilterMode::Linear,
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

pub(super) fn preview_mip_level_count(size: [u32; 2]) -> u32 {
    size[0].max(size[1]).max(1).ilog2() + 1
}

pub(super) fn write_preview_texture_mips(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    image: &RgbaImage,
) {
    let mut mip_image = image.clone();
    let mip_level_count = preview_mip_level_count([image.width(), image.height()]);

    for mip_level in 0..mip_level_count {
        let width = mip_image.width().max(1);
        let height = mip_image.height().max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            mip_image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        if mip_level + 1 < mip_level_count {
            let next_width = (width / 2).max(1);
            let next_height = (height / 2).max(1);
            mip_image = resize_preview_mip(&mip_image, next_width, next_height);
        }
    }
}

fn resize_preview_mip(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let mut premultiplied = image.clone();
    for pixel in premultiplied.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        pixel[0] = ((u16::from(pixel[0]) * alpha + 127) / 255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * alpha + 127) / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * alpha + 127) / 255) as u8;
    }

    let mut resized = image::imageops::resize(&premultiplied, width, height, FilterType::Triangle);
    for pixel in resized.pixels_mut() {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            continue;
        }

        let scale = 255.0 / f32::from(alpha);
        pixel[0] = (f32::from(pixel[0]) * scale).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (f32::from(pixel[1]) * scale).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (f32::from(pixel[2]) * scale).round().clamp(0.0, 255.0) as u8;
    }

    resized
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

pub(super) fn hash_rgba_image(image: &RgbaImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.width().hash(&mut hasher);
    image.height().hash(&mut hasher);
    image.as_raw().hash(&mut hasher);
    hasher.finish()
}

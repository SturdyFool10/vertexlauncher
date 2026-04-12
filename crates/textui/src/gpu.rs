use super::*;

#[path = "gpu/cpu_scene_atlas_page.rs"]
mod cpu_scene_atlas_page;
#[path = "gpu/resolved_text_graphics_config.rs"]
mod resolved_text_graphics_config;
#[path = "gpu/resolved_text_renderer_backend.rs"]
mod resolved_text_renderer_backend;
#[path = "gpu/text_wgpu_instance.rs"]
mod text_wgpu_instance;
#[path = "gpu/text_wgpu_pipeline_resources.rs"]
mod text_wgpu_pipeline_resources;
#[path = "gpu/text_wgpu_prepared_batch.rs"]
mod text_wgpu_prepared_batch;
#[path = "gpu/text_wgpu_prepared_scene.rs"]
mod text_wgpu_prepared_scene;
#[path = "gpu/text_wgpu_scene_batch_source.rs"]
mod text_wgpu_scene_batch_source;
#[path = "gpu/text_wgpu_scene_callback.rs"]
mod text_wgpu_scene_callback;
#[path = "gpu/text_wgpu_screen_uniform.rs"]
mod text_wgpu_screen_uniform;

pub(super) use self::cpu_scene_atlas_page::CpuSceneAtlasPage;
pub(super) use self::resolved_text_graphics_config::ResolvedTextGraphicsConfig;
pub(super) use self::resolved_text_renderer_backend::ResolvedTextRendererBackend;
pub(super) use self::text_wgpu_instance::TextWgpuInstance;
use self::text_wgpu_pipeline_resources::TextWgpuPipelineResources;
use self::text_wgpu_prepared_batch::TextWgpuPreparedBatch;
pub(super) use self::text_wgpu_prepared_scene::{
    TextWgpuCachedTextureBinding, TextWgpuPreparedScene, TextWgpuTextureBindingCache,
};
pub(super) use self::text_wgpu_scene_batch_source::TextWgpuSceneBatchSource;
pub(super) use self::text_wgpu_scene_callback::TextWgpuSceneCallback;
use self::text_wgpu_screen_uniform::TextWgpuScreenUniform;

impl TextWgpuPipelineResources {
    pub(super) fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        atlas_sampling: TextAtlasSampling,
        linear_pipeline: bool,
        output_is_hdr: bool,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("textui_instanced_shader"),
            source: wgpu::ShaderSource::Wgsl(TEXT_WGPU_INSTANCED_SHADER.into()),
        });
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("textui_instanced_uniform_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("textui_instanced_texture_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("textui_instanced_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu_filter_mode_for_sampling(atlas_sampling),
            min_filter: wgpu_filter_mode_for_sampling(atlas_sampling),
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uniform = TextWgpuScreenUniform {
            screen_size_points: [1.0, 1.0],
            output_is_hdr: if output_is_hdr { 1.0 } else { 0.0 },
            _pad0: 0.0,
            _pad1: [0.0; 2],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("textui_instanced_uniform_buffer"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("textui_instanced_uniform_bg"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("textui_instanced_pipeline_layout"),
            bind_group_layouts: &[
                Some(&uniform_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let premultiplied_alpha = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("textui_instanced_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<TextWgpuInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x2,
                        3 => Float32x2,
                        4 => Float32x2,
                        5 => Float32x2,
                        6 => Unorm8x4,
                        7 => Uint32
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(premultiplied_alpha),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: TEXT_WGPU_PASS_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: TEXT_WGPU_PASS_MSAA_SAMPLES.max(1),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            target_format,
            atlas_sampling,
            linear_pipeline,
            output_is_hdr,
            pipeline,
            texture_bind_group_layout,
            sampler,
            uniform_buffer,
            uniform_bind_group,
        }
    }

    pub(super) fn update_uniform(
        &self,
        queue: &wgpu::Queue,
        screen_size_points: [f32; 2],
        output_is_hdr: bool,
    ) {
        let uniform = TextWgpuScreenUniform {
            screen_size_points,
            output_is_hdr: if output_is_hdr { 1.0 } else { 0.0 },
            _pad0: 0.0,
            _pad1: [0.0; 2],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }
}

pub(super) fn add_text_quad(
    mesh: &mut egui::epaint::Mesh,
    positions: [Pos2; 4],
    uvs: [Pos2; 4],
    tint: Color32,
) {
    let base = mesh.vertices.len() as u32;
    for index in 0..4 {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: positions[index],
            uv: uvs[index],
            color: tint,
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

pub(super) fn paint_text_quads_fallback(
    glyph_atlas: &GlyphAtlas,
    painter: &egui::Painter,
    quads: &[PaintTextQuad],
) {
    let mut meshes: FxHashMap<TextureId, egui::epaint::Mesh> = FxHashMap::default();
    for quad in quads {
        let Some(texture_id) = glyph_atlas.texture_id_for_page(quad.page_index) else {
            continue;
        };
        let mesh = meshes
            .entry(texture_id)
            .or_insert_with(|| egui::epaint::Mesh::with_texture(texture_id));
        add_text_quad(mesh, quad.positions, quad.uvs, quad.tint);
    }

    for (_, mesh) in meshes {
        if !mesh.is_empty() {
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

pub(super) fn map_scene_quads_to_rect(
    rect: Rect,
    uv: Rect,
    natural_size: Vec2,
    quads: &[TextAtlasQuad],
    tint: Color32,
) -> Vec<PaintTextQuad> {
    if natural_size.x.abs() <= f32::EPSILON || natural_size.y.abs() <= f32::EPSILON {
        return Vec::new();
    }

    let scale_x = rect.width() / natural_size.x;
    let scale_y = rect.height() / natural_size.y;
    let uv_width = uv.width();
    let uv_height = uv.height();

    quads
        .iter()
        .map(|quad| PaintTextQuad {
            page_index: quad.atlas_page_index,
            positions: quad.positions.map(|point| {
                Pos2::new(
                    rect.min.x + point.x * scale_x,
                    rect.min.y + point.y * scale_y,
                )
            }),
            uvs: quad.uvs.map(|point| {
                Pos2::new(
                    uv.min.x + point.x * uv_width,
                    uv.min.y + point.y * uv_height,
                )
            }),
            tint: multiply_color32(quad.tint.into(), tint),
            content_mode: GlyphContentMode::AlphaMask,
        })
        .collect()
}

pub(super) fn default_gpu_scene_page_side(graphics_config: ResolvedTextGraphicsConfig) -> usize {
    graphics_config.atlas_page_target_px.max(256)
}

pub(super) fn gpu_scene_approx_bytes(scene: &TextGpuScene) -> usize {
    scene
        .atlas_pages
        .iter()
        .map(|p| p.rgba8.len())
        .sum::<usize>()
        + scene.quads.len() * std::mem::size_of::<TextGpuQuad>()
        + 64
}

pub(super) fn gpu_scene_page_batches_approx_bytes(batches: &[TextGpuScenePageBatch]) -> usize {
    batches
        .iter()
        .map(|batch| batch.quads.len() * std::mem::size_of::<TextGpuQuad>())
        .sum::<usize>()
        + batches.len() * std::mem::size_of::<TextGpuScenePageBatch>()
        + 64
}

pub(super) fn allocate_cpu_scene_page_slot(
    pages: &mut Vec<CpuSceneAtlasPage>,
    pool: &mut Vec<CpuSceneAtlasPage>,
    target_page_side_px: usize,
    allocation_size: etagere::Size,
) -> Option<(usize, Allocation)> {
    for (page_index, page) in pages.iter_mut().enumerate() {
        if let Some(allocation) = page.allocator.allocate(allocation_size) {
            return Some((page_index, allocation));
        }
    }

    let side = target_page_side_px
        .max(allocation_size.width.max(1) as usize)
        .max(allocation_size.height.max(1) as usize);
    let mut page = if let Some(mut pooled) = pool.pop() {
        pooled.reset_for_size(side);
        pooled
    } else {
        CpuSceneAtlasPage::new_for_size(side)
    };
    let allocation = page.allocator.allocate(allocation_size)?;
    pages.push(page);
    Some((pages.len() - 1, allocation))
}

/// Blit a ColorImage into a page's raw RGBA8 Vec buffer.
pub(super) fn blit_to_page(
    dest_rgba8: &mut Vec<u8>,
    dest_size: [usize; 2],
    src: &ColorImage,
    dest_x: usize,
    dest_y: usize,
) {
    let copy_width = src.size[0].min(dest_size[0].saturating_sub(dest_x));
    if copy_width == 0 {
        return;
    }
    for y in 0..src.size[1] {
        let target_y = dest_y + y;
        if target_y >= dest_size[1] {
            break;
        }
        // Safety: Color32 is repr(C) [u8; 4] — valid cast to bytes.
        let src_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                src.pixels.as_ptr().add(y * src.size[0]) as *const u8,
                copy_width * 4,
            )
        };
        let dest_start = (target_y * dest_size[0] + dest_x) * 4;
        dest_rgba8[dest_start..dest_start + copy_width * 4].copy_from_slice(src_bytes);
    }
}

/// Convert a CpuSceneAtlasPage to TextAtlasPageData using a pre-computed content hash.
/// Allocates one Arc<[u8]> with a memcpy; the Vec remains owned by the pool page so
/// reset_for_size can zero it in-place next frame without Arc refcount contention.
pub(super) fn cpu_page_to_page_data(
    page: &CpuSceneAtlasPage,
    page_index: usize,
    content_hash: u64,
) -> TextAtlasPageData {
    TextAtlasPageData {
        page_index,
        size_px: page.size,
        content_hash,
        rgba8: Arc::from(page.rgba8.as_slice()),
    }
}

pub(super) fn color_image_to_page_data(page_index: usize, image: &ColorImage) -> TextAtlasPageData {
    // Safety: Color32 is repr(C) struct([u8; 4]) — identical layout to 4 contiguous u8 bytes.
    // This avoids an intermediate Vec allocation and the Box→Arc realloc.
    let rgba8_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(image.pixels.as_ptr() as *const u8, image.pixels.len() * 4)
    };
    let mut hasher = rustc_hash::FxHasher::default();
    page_index.hash(&mut hasher);
    image.size.hash(&mut hasher);
    hasher.write(rgba8_bytes);
    TextAtlasPageData {
        page_index,
        size_px: image.size,
        content_hash: hasher.finish(),
        rgba8: Arc::from(rgba8_bytes),
    }
}

pub(super) fn quad_positions_from_min_size(min: Pos2, size: Vec2) -> [Pos2; 4] {
    [
        min,
        Pos2::new(min.x + size.x, min.y),
        Pos2::new(min.x + size.x, min.y + size.y),
        Pos2::new(min.x, min.y + size.y),
    ]
}

pub(super) fn rotated_quad_positions(
    anchor: Pos2,
    top_left_offset: Vec2,
    size_points: Vec2,
    rotation_radians: f32,
) -> [Pos2; 4] {
    let rotation = egui::emath::Rot2::from_angle(rotation_radians);
    [
        top_left_offset,
        top_left_offset + egui::vec2(size_points.x, 0.0),
        top_left_offset + size_points,
        top_left_offset + egui::vec2(0.0, size_points.y),
    ]
    .map(|offset| anchor + rotation * offset)
}

pub(super) fn uv_quad_points(uv: Rect) -> [Pos2; 4] {
    [
        uv.min,
        Pos2::new(uv.max.x, uv.min.y),
        uv.max,
        Pos2::new(uv.min.x, uv.max.y),
    ]
}

pub(super) fn rect_from_points(points: [Pos2; 4]) -> Rect {
    let mut min = points[0];
    let mut max = points[0];
    for point in &points[1..] {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
    }
    Rect::from_min_max(min, max)
}

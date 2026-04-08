use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct TextWgpuScreenUniform {
    pub(super) screen_size_points: [f32; 2],
    pub(super) _padding: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct TextWgpuInstance {
    pub(super) pos0: [f32; 2],
    pub(super) pos1: [f32; 2],
    pub(super) pos2: [f32; 2],
    pub(super) pos3: [f32; 2],
    pub(super) uv0: [f32; 2],
    pub(super) uv1: [f32; 2],
    pub(super) uv2: [f32; 2],
    pub(super) uv3: [f32; 2],
    pub(super) color: [f32; 4],
    pub(super) decode_mode: f32,
    pub(super) field_range_px: f32,
    pub(super) _padding: [f32; 2],
}

impl TextWgpuInstance {
    pub(super) fn from_quad(quad: &PaintTextQuad) -> Self {
        Self {
            pos0: [quad.positions[0].x, quad.positions[0].y],
            pos1: [quad.positions[1].x, quad.positions[1].y],
            pos2: [quad.positions[2].x, quad.positions[2].y],
            pos3: [quad.positions[3].x, quad.positions[3].y],
            uv0: [quad.uvs[0].x, quad.uvs[0].y],
            uv1: [quad.uvs[1].x, quad.uvs[1].y],
            uv2: [quad.uvs[2].x, quad.uvs[2].y],
            uv3: [quad.uvs[3].x, quad.uvs[3].y],
            color: quad.tint.to_normalized_gamma_f32(),
            decode_mode: match quad.content_mode {
                GlyphContentMode::AlphaMask => 0.0,
                GlyphContentMode::Sdf => 1.0,
                GlyphContentMode::Msdf => 2.0,
            },
            field_range_px: quad.field_range_px,
            _padding: [0.0, 0.0],
        }
    }
}

#[derive(Clone)]
pub(super) struct TextWgpuSceneBatchSource {
    pub(super) texture: wgpu::Texture,
    pub(super) instances: Arc<[TextWgpuInstance]>,
}

pub(super) struct TextWgpuPreparedBatch {
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) instance_buffer: wgpu::Buffer,
    pub(super) instance_count: u32,
}

#[derive(Default)]
pub(super) struct TextWgpuPreparedScene {
    pub(super) batches: Vec<TextWgpuPreparedBatch>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResolvedTextRendererBackend {
    EguiMesh,
    WgpuInstanced,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ResolvedTextGraphicsConfig {
    pub(super) renderer_backend: ResolvedTextRendererBackend,
    pub(super) atlas_sampling: TextAtlasSampling,
    pub(super) atlas_page_target_px: usize,
    pub(super) atlas_padding_px: usize,
    pub(super) rasterization: TextRasterizationConfig,
}

#[derive(Clone)]
pub(super) struct TextWgpuSceneCallback {
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) atlas_sampling: TextAtlasSampling,
    pub(super) batches: Arc<[TextWgpuSceneBatchSource]>,
    pub(super) prepared: Arc<Mutex<TextWgpuPreparedScene>>,
}

pub(super) struct TextWgpuPipelineResources {
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) atlas_sampling: TextAtlasSampling,
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) texture_bind_group_layout: wgpu::BindGroupLayout,
    pub(super) sampler: wgpu::Sampler,
    pub(super) uniform_buffer: wgpu::Buffer,
    pub(super) uniform_bind_group: wgpu::BindGroup,
}

pub(super) struct CpuSceneAtlasPage {
    pub(super) allocator: AtlasAllocator,
    pub(super) image: ColorImage,
}

impl TextWgpuPipelineResources {
    pub(super) fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        atlas_sampling: TextAtlasSampling,
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
                    visibility: wgpu::ShaderStages::VERTEX,
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
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let uniform = TextWgpuScreenUniform {
            screen_size_points: [1.0, 1.0],
            _padding: [0.0, 0.0],
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
            bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout],
            push_constant_ranges: &[],
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
                        6 => Float32x2,
                        7 => Float32x2,
                        8 => Float32x4,
                        9 => Float32,
                        10 => Float32
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
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: TEXT_WGPU_PASS_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: TEXT_WGPU_PASS_MSAA_SAMPLES.max(1),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });
        Self {
            target_format,
            atlas_sampling,
            pipeline,
            texture_bind_group_layout,
            sampler,
            uniform_buffer,
            uniform_bind_group,
        }
    }

    pub(super) fn update_uniform(&self, queue: &wgpu::Queue, screen_size_points: [f32; 2]) {
        let uniform = TextWgpuScreenUniform {
            screen_size_points,
            _padding: [0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }
}

impl egui_wgpu::CallbackTrait for TextWgpuSceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = callback_resources
            .entry::<TextWgpuPipelineResources>()
            .or_insert_with(|| {
                TextWgpuPipelineResources::new(device, self.target_format, self.atlas_sampling)
            });
        if resources.target_format != self.target_format
            || resources.atlas_sampling != self.atlas_sampling
        {
            *resources =
                TextWgpuPipelineResources::new(device, self.target_format, self.atlas_sampling);
        }
        resources.update_uniform(
            queue,
            [
                screen_descriptor.size_in_pixels[0] as f32 / screen_descriptor.pixels_per_point,
                screen_descriptor.size_in_pixels[1] as f32 / screen_descriptor.pixels_per_point,
            ],
        );

        let mut prepared_batches = Vec::with_capacity(self.batches.len());
        for batch in self.batches.iter() {
            if batch.instances.is_empty() {
                continue;
            }
            let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("textui_instanced_instance_buffer"),
                contents: bytemuck::cast_slice(batch.instances.as_ref()),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let view = batch
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("textui_instanced_texture_bg"),
                layout: &resources.texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&resources.sampler),
                    },
                ],
            });
            prepared_batches.push(TextWgpuPreparedBatch {
                bind_group,
                instance_buffer,
                instance_count: batch.instances.len() as u32,
            });
        }

        if let Ok(mut prepared) = self.prepared.lock() {
            prepared.batches = prepared_batches;
        }

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<TextWgpuPipelineResources>() else {
            return;
        };
        let Ok(prepared) = self.prepared.lock() else {
            return;
        };
        if prepared.batches.is_empty() {
            return;
        }

        render_pass.set_viewport(
            0.0,
            0.0,
            info.screen_size_px[0] as f32,
            info.screen_size_px[1] as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
        for batch in &prepared.batches {
            render_pass.set_bind_group(1, &batch.bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.instance_buffer.slice(..));
            render_pass.draw(0..6, 0..batch.instance_count);
        }
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
            field_range_px: 0.0,
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

pub(super) fn allocate_cpu_scene_page_slot(
    pages: &mut Vec<CpuSceneAtlasPage>,
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
    let mut allocator = AtlasAllocator::new(size2(side as i32, side as i32));
    let allocation = allocator.allocate(allocation_size)?;
    pages.push(CpuSceneAtlasPage {
        allocator,
        image: ColorImage::filled([side, side], Color32::TRANSPARENT),
    });
    Some((pages.len() - 1, allocation))
}

pub(super) fn color_image_to_page_data(page_index: usize, image: &ColorImage) -> TextAtlasPageData {
    let mut rgba8 = Vec::with_capacity(image.pixels.len().saturating_mul(4));
    for pixel in &image.pixels {
        rgba8.extend_from_slice(&pixel.to_array());
    }
    TextAtlasPageData {
        page_index,
        size_px: image.size,
        rgba8,
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

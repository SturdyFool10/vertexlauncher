use super::*;

pub(super) struct SkinPreviewPostProcessWgpuCallback {
    scene_batches: Vec<GpuPreviewSceneBatch>,
    skin_sample: Arc<RgbaImage>,
    cape_sample: Option<Arc<RgbaImage>>,
    skin_hash: u64,
    cape_hash: Option<u64>,
    target_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
    present_msaa_samples: u32,
    aa_mode: SkinPreviewAaMode,
    texel_aa_mode: SkinPreviewTexelAaMode,
}

impl SkinPreviewPostProcessWgpuCallback {
    pub(super) fn from_scene(
        triangles: &[RenderTriangle],
        skin_sample: Arc<RgbaImage>,
        cape_sample: Option<Arc<RgbaImage>>,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
        aa_mode: SkinPreviewAaMode,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) -> Self {
        Self {
            scene_batches: vec![build_gpu_preview_scene_batch(triangles, 1.0)],
            skin_hash: hash_rgba_image(&skin_sample),
            cape_hash: cape_sample
                .as_ref()
                .map(|image| hash_rgba_image(image.as_ref())),
            skin_sample,
            cape_sample,
            target_format,
            scene_msaa_samples,
            present_msaa_samples,
            aa_mode,
            texel_aa_mode,
        }
    }

    pub(super) fn from_weighted_scenes(
        scenes: &[WeightedPreviewScene],
        skin_sample: Arc<RgbaImage>,
        cape_sample: Option<Arc<RgbaImage>>,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
        aa_mode: SkinPreviewAaMode,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) -> Self {
        let scene_batches = scenes
            .iter()
            .map(|scene| build_gpu_preview_scene_batch(&scene.triangles, scene.weight))
            .collect();
        Self {
            scene_batches,
            skin_hash: hash_rgba_image(&skin_sample),
            cape_hash: cape_sample
                .as_ref()
                .map(|image| hash_rgba_image(image.as_ref())),
            skin_sample,
            cape_sample,
            target_format,
            scene_msaa_samples,
            present_msaa_samples,
            aa_mode,
            texel_aa_mode,
        }
    }
}

impl egui_wgpu::CallbackTrait for SkinPreviewPostProcessWgpuCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = callback_resources
            .entry::<SkinPreviewPostProcessWgpuResources>()
            .or_insert_with(|| {
                SkinPreviewPostProcessWgpuResources::new(
                    device,
                    self.target_format,
                    self.scene_msaa_samples,
                    self.present_msaa_samples,
                )
            });
        if resources.target_format != self.target_format
            || resources.scene_msaa_samples != self.scene_msaa_samples
            || resources.present_msaa_samples != self.present_msaa_samples
        {
            *resources = SkinPreviewPostProcessWgpuResources::new(
                device,
                self.target_format,
                self.scene_msaa_samples,
                self.present_msaa_samples,
            );
        }

        resources.update_scene_uniform(
            queue,
            [
                screen_descriptor.size_in_pixels[0] as f32 / screen_descriptor.pixels_per_point,
                screen_descriptor.size_in_pixels[1] as f32 / screen_descriptor.pixels_per_point,
            ],
        );
        resources.update_scene_texture_aa_mode(queue, self.texel_aa_mode);
        resources.ensure_render_targets(device, screen_descriptor.size_in_pixels);
        resources.update_texture(
            device,
            queue,
            TextureSlot::Skin,
            self.skin_hash,
            &self.skin_sample,
        );
        if let (Some(cape_hash), Some(cape_sample)) = (self.cape_hash, self.cape_sample.as_ref()) {
            resources.update_texture(device, queue, TextureSlot::Cape, cape_hash, cape_sample);
        } else {
            resources.cape_texture = None;
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("skins-preview-post-process-encoder"),
        });

        let use_smaa = self.aa_mode == SkinPreviewAaMode::Smaa;
        let use_fxaa = matches!(
            self.aa_mode,
            SkinPreviewAaMode::Fxaa | SkinPreviewAaMode::FxaaTaa
        );
        let use_taa = matches!(
            self.aa_mode,
            SkinPreviewAaMode::Taa | SkinPreviewAaMode::FxaaTaa
        );
        let use_fxaa_after_taa = self.aa_mode == SkinPreviewAaMode::FxaaTaa;
        resources.present_source = PresentSource::Accumulation;

        for (index, batch) in self.scene_batches.iter().enumerate() {
            let prepared_batch = prepare_gpu_preview_scene_batch(
                device,
                &resources.scalar_uniform_bind_group_layout,
                batch,
            );

            {
                let color_attachment =
                    resources.scene_color_attachment(index == 0 || self.scene_batches.len() == 1);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skins-preview-scene-pass"),
                    color_attachments: &[Some(color_attachment)],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: resources.scene_depth_view(),
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                resources.paint_scene(&mut pass, &prepared_batch);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skins-preview-accumulation-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &resources.accumulation_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: if index == 0 {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&resources.accumulate_pipeline);
                pass.set_bind_group(0, &resources.scene_resolve_bind_group, &[]);
                pass.set_bind_group(1, &prepared_batch.weight_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        if use_smaa {
            resources.apply_smaa(&mut encoder);
        } else if use_fxaa && !use_taa {
            resources.apply_fxaa(&mut encoder, PresentSource::Accumulation);
            resources.present_source = PresentSource::PostProcess;
        } else if use_taa {
            let taa_source = resources.apply_taa(
                &mut encoder,
                queue,
                if use_fxaa_after_taa { 0.22 } else { 0.35 },
            );
            if use_fxaa_after_taa {
                resources.present_source = resources.apply_fxaa(&mut encoder, taa_source);
            } else {
                resources.present_source = taa_source;
            }
        } else {
            resources.taa_history_valid = false;
        }

        vec![encoder.finish()]
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<SkinPreviewPostProcessWgpuResources>()
        else {
            return;
        };
        let viewport = info.viewport_in_pixels();
        render_pass.set_viewport(
            viewport.left_px as f32,
            viewport.top_px as f32,
            viewport.width_px as f32,
            viewport.height_px as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&resources.present_pipeline);
        let bind_group = match resources.present_source {
            PresentSource::Accumulation => &resources.accumulation_bind_group,
            PresentSource::PostProcess => &resources.post_process_bind_group,
        };
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[derive(Clone, Copy)]
enum PresentSource {
    Accumulation,
    PostProcess,
}

struct SkinPreviewPostProcessWgpuResources {
    scene_pipeline: wgpu::RenderPipeline,
    accumulate_pipeline: wgpu::RenderPipeline,
    smaa_pipeline: wgpu::RenderPipeline,
    fxaa_pipeline: wgpu::RenderPipeline,
    taa_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    texture_sampler: wgpu::Sampler,
    uniform_bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    scalar_uniform_bind_group_layout: wgpu::BindGroupLayout,
    scalar_uniform_bind_group: wgpu::BindGroup,
    scalar_uniform_buffer: wgpu::Buffer,
    skin_texture: Option<UploadedPreviewTexture>,
    cape_texture: Option<UploadedPreviewTexture>,
    accumulation_texture: wgpu::Texture,
    accumulation_view: wgpu::TextureView,
    accumulation_bind_group: wgpu::BindGroup,
    scene_resolve_texture: wgpu::Texture,
    scene_resolve_view: wgpu::TextureView,
    scene_resolve_bind_group: wgpu::BindGroup,
    scene_msaa_texture: Option<wgpu::Texture>,
    scene_msaa_view: Option<wgpu::TextureView>,
    scene_depth_texture: wgpu::Texture,
    scene_depth_view: wgpu::TextureView,
    post_process_texture: wgpu::Texture,
    post_process_view: wgpu::TextureView,
    post_process_bind_group: wgpu::BindGroup,
    taa_history_texture: wgpu::Texture,
    taa_history_view: wgpu::TextureView,
    taa_history_bind_group: wgpu::BindGroup,
    taa_history_valid: bool,
    render_target_size: [u32; 2],
    target_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
    present_msaa_samples: u32,
    present_source: PresentSource,
}

impl SkinPreviewPostProcessWgpuResources {
    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
    ) -> Self {
        const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-post-scene-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_post_scene.wgsl"
            ))),
        });
        let accumulate_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-accumulate-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_accumulate.wgsl"
            ))),
        });
        let fxaa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-fxaa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_fxaa.wgsl"
            ))),
        });
        let smaa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-smaa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_smaa.wgsl"
            ))),
        });
        let taa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-taa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_taa.wgsl"
            ))),
        });
        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-present-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_present.wgsl"
            ))),
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skins-preview-post-texture-layout"),
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
        let texture_sampler =
            create_skin_preview_sampler(device, "skins-preview-post-texture-sampler");
        let scene_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skins-preview-post-scene-uniform-layout"),
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
        let scalar_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skins-preview-post-scalar-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skins-preview-post-scene-uniform-buffer"),
            size: std::mem::size_of::<GpuPreviewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skins-preview-post-scene-uniform-bind-group"),
            layout: &scene_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let scalar_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skins-preview-post-scalar-uniform-buffer"),
            size: std::mem::size_of::<GpuPreviewScalarUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scalar_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skins-preview-post-scalar-bind-group"),
            layout: &scalar_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scalar_uniform_buffer.as_entire_binding(),
            }],
        });

        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("skins-preview-post-scene-layout"),
                bind_group_layouts: &[
                    &texture_bind_group_layout,
                    &scene_uniform_layout,
                    &scalar_uniform_layout,
                ],
                push_constant_ranges: &[],
            });
        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-post-scene-pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuPreviewVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32,
                        2 => Float32x2,
                        3 => Float32x4
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SKIN_PREVIEW_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: scene_msaa_samples.max(1),
                mask: !0,
                alpha_to_coverage_enabled: scene_msaa_samples > 1,
            },
            multiview: None,
            cache: None,
        });

        let fullscreen_vertex = wgpu::VertexState {
            module: &accumulate_shader,
            entry_point: Some("vs_fullscreen"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        };

        let accumulate_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-accumulate-layout"),
            bind_group_layouts: &[&texture_bind_group_layout, &scalar_uniform_layout],
            push_constant_ranges: &[],
        });
        let accumulate_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-accumulate-pipeline"),
            layout: Some(&accumulate_layout),
            vertex: fullscreen_vertex.clone(),
            fragment: Some(wgpu::FragmentState {
                module: &accumulate_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let smaa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-smaa-layout"),
            bind_group_layouts: &[&texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let smaa_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-smaa-pipeline"),
            layout: Some(&smaa_layout),
            vertex: wgpu::VertexState {
                module: &smaa_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &smaa_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let fxaa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-fxaa-layout"),
            bind_group_layouts: &[&texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let fxaa_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-fxaa-pipeline"),
            layout: Some(&fxaa_layout),
            vertex: wgpu::VertexState {
                module: &fxaa_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &fxaa_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let taa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-taa-layout"),
            bind_group_layouts: &[
                &texture_bind_group_layout,
                &texture_bind_group_layout,
                &scalar_uniform_layout,
            ],
            push_constant_ranges: &[],
        });
        let taa_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-taa-pipeline"),
            layout: Some(&taa_layout),
            vertex: wgpu::VertexState {
                module: &taa_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &taa_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let present_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-present-layout"),
            bind_group_layouts: &[&texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let present_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-present-pipeline"),
            layout: Some(&present_layout),
            vertex: wgpu::VertexState {
                module: &present_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &present_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState {
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
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SKIN_PREVIEW_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: present_msaa_samples.max(1),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let (accumulation_texture, accumulation_view, accumulation_bind_group) =
            create_preview_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-accumulation",
            );
        let (scene_resolve_texture, scene_resolve_view, scene_resolve_bind_group) =
            create_preview_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-scene-resolve",
            );
        let (post_process_texture, post_process_view, post_process_bind_group) =
            create_preview_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-post-process",
            );
        let (taa_history_texture, taa_history_view, taa_history_bind_group) =
            create_preview_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-taa-history",
            );
        let (scene_depth_texture, scene_depth_view) = create_preview_depth_texture(
            device,
            [1, 1],
            scene_msaa_samples.max(1),
            "skins-preview-scene-depth",
        );
        let (scene_msaa_texture, scene_msaa_view) = if scene_msaa_samples > 1 {
            let (texture, view) = create_preview_color_texture(
                device,
                OFFSCREEN_FORMAT,
                [1, 1],
                scene_msaa_samples.max(1),
                "skins-preview-scene-msaa",
            );
            (Some(texture), Some(view))
        } else {
            (None, None)
        };

        Self {
            scene_pipeline,
            accumulate_pipeline,
            smaa_pipeline,
            fxaa_pipeline,
            taa_pipeline,
            present_pipeline,
            texture_bind_group_layout,
            texture_sampler,
            uniform_bind_group,
            uniform_buffer,
            scalar_uniform_bind_group_layout: scalar_uniform_layout,
            scalar_uniform_bind_group,
            scalar_uniform_buffer,
            skin_texture: None,
            cape_texture: None,
            accumulation_texture,
            accumulation_view,
            accumulation_bind_group,
            scene_resolve_texture,
            scene_resolve_view,
            scene_resolve_bind_group,
            scene_msaa_texture,
            scene_msaa_view,
            scene_depth_texture,
            scene_depth_view,
            post_process_texture,
            post_process_view,
            post_process_bind_group,
            taa_history_texture,
            taa_history_view,
            taa_history_bind_group,
            taa_history_valid: false,
            render_target_size: [1, 1],
            target_format,
            scene_msaa_samples: scene_msaa_samples.max(1),
            present_msaa_samples: present_msaa_samples.max(1),
            present_source: PresentSource::Accumulation,
        }
    }

    fn ensure_render_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        if self.render_target_size == size {
            return;
        }
        self.render_target_size = size;
        self.taa_history_valid = false;

        const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
        let (accumulation_texture, accumulation_view, accumulation_bind_group) =
            create_preview_render_texture(
                device,
                &self.texture_bind_group_layout,
                &self.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-accumulation",
            );
        self.accumulation_texture = accumulation_texture;
        self.accumulation_view = accumulation_view;
        self.accumulation_bind_group = accumulation_bind_group;

        let (scene_resolve_texture, scene_resolve_view, scene_resolve_bind_group) =
            create_preview_render_texture(
                device,
                &self.texture_bind_group_layout,
                &self.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-scene-resolve",
            );
        self.scene_resolve_texture = scene_resolve_texture;
        self.scene_resolve_view = scene_resolve_view;
        self.scene_resolve_bind_group = scene_resolve_bind_group;

        let (post_process_texture, post_process_view, post_process_bind_group) =
            create_preview_render_texture(
                device,
                &self.texture_bind_group_layout,
                &self.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-post-process",
            );
        self.post_process_texture = post_process_texture;
        self.post_process_view = post_process_view;
        self.post_process_bind_group = post_process_bind_group;

        let (taa_history_texture, taa_history_view, taa_history_bind_group) =
            create_preview_render_texture(
                device,
                &self.texture_bind_group_layout,
                &self.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-taa-history",
            );
        self.taa_history_texture = taa_history_texture;
        self.taa_history_view = taa_history_view;
        self.taa_history_bind_group = taa_history_bind_group;

        let (scene_depth_texture, scene_depth_view) = create_preview_depth_texture(
            device,
            size,
            self.scene_msaa_samples.max(1),
            "skins-preview-scene-depth",
        );
        self.scene_depth_texture = scene_depth_texture;
        self.scene_depth_view = scene_depth_view;

        if self.scene_msaa_samples > 1 {
            let (texture, view) = create_preview_color_texture(
                device,
                OFFSCREEN_FORMAT,
                size,
                self.scene_msaa_samples.max(1),
                "skins-preview-scene-msaa",
            );
            self.scene_msaa_texture = Some(texture);
            self.scene_msaa_view = Some(view);
        } else {
            self.scene_msaa_texture = None;
            self.scene_msaa_view = None;
        }
    }

    fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: TextureSlot,
        hash: u64,
        image: &RgbaImage,
    ) {
        let size = [image.width(), image.height()];
        let target = match slot {
            TextureSlot::Skin => &mut self.skin_texture,
            TextureSlot::Cape => &mut self.cape_texture,
        };

        if target
            .as_ref()
            .is_some_and(|uploaded| uploaded.hash == hash && uploaded.size == size)
        {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("skins-preview-post-source-texture"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: preview_mip_level_count(size),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        write_preview_texture_mips(queue, &texture, image);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = create_preview_texture_bind_group(
            device,
            &self.texture_bind_group_layout,
            &self.texture_sampler,
            &view,
            "skins-preview-post-source-bind-group",
        );

        *target = Some(UploadedPreviewTexture {
            hash,
            size,
            bind_group,
            _texture: texture,
        });
    }

    fn update_scene_uniform(&self, queue: &wgpu::Queue, screen_size_points: [f32; 2]) {
        let uniform = GpuPreviewUniform {
            screen_size_points,
            _pad: [0.0; 2],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    fn update_scene_texture_aa_mode(
        &self,
        queue: &wgpu::Queue,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) {
        queue.write_buffer(
            &self.scalar_uniform_buffer,
            0,
            bytemuck::bytes_of(&GpuPreviewScalarUniform {
                value: [
                    if texel_aa_mode == SkinPreviewTexelAaMode::TexelBoundary {
                        1.0
                    } else {
                        0.0
                    },
                    0.0,
                    0.0,
                    0.0,
                ],
            }),
        );
    }

    fn scene_color_attachment(&self, _clear: bool) -> wgpu::RenderPassColorAttachment<'_> {
        if let Some(view) = self.scene_msaa_view.as_ref() {
            wgpu::RenderPassColorAttachment {
                view,
                resolve_target: Some(&self.scene_resolve_view),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            }
        } else {
            wgpu::RenderPassColorAttachment {
                view: &self.scene_resolve_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            }
        }
    }

    fn scene_depth_view(&self) -> &wgpu::TextureView {
        &self.scene_depth_view
    }

    fn paint_scene(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        batch: &PreparedGpuPreviewSceneBatch,
    ) {
        render_pass.set_pipeline(&self.scene_pipeline);
        render_pass.set_bind_group(1, &self.uniform_bind_group, &[]);
        render_pass.set_bind_group(2, &self.scalar_uniform_bind_group, &[]);

        if let Some(texture) = self.skin_texture.as_ref() {
            render_pass.set_bind_group(0, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.skin_vertex_buffer.slice(..));
            render_pass
                .set_index_buffer(batch.skin_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..batch.skin_index_count, 0, 0..1);
        }
        if let Some(texture) = self.cape_texture.as_ref() {
            render_pass.set_bind_group(0, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.cape_vertex_buffer.slice(..));
            render_pass
                .set_index_buffer(batch.cape_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..batch.cape_index_count, 0, 0..1);
        }
    }

    fn render_target_extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.render_target_size[0].max(1),
            height: self.render_target_size[1].max(1),
            depth_or_array_layers: 1,
        }
    }

    fn apply_smaa(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("skins-preview-smaa-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.post_process_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.smaa_pipeline);
        pass.set_bind_group(0, &self.accumulation_bind_group, &[]);
        pass.draw(0..3, 0..1);
        self.present_source = PresentSource::PostProcess;
        self.taa_history_valid = false;
    }

    fn apply_fxaa(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: PresentSource,
    ) -> PresentSource {
        let (source_bind_group, target_view, present_source, label) = match source {
            PresentSource::Accumulation => (
                &self.accumulation_bind_group,
                &self.post_process_view,
                PresentSource::PostProcess,
                "skins-preview-fxaa-pass",
            ),
            PresentSource::PostProcess => (
                &self.post_process_bind_group,
                &self.accumulation_view,
                PresentSource::Accumulation,
                "skins-preview-fxaa-after-taa-pass",
            ),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.fxaa_pipeline);
        pass.set_bind_group(0, source_bind_group, &[]);
        pass.draw(0..3, 0..1);
        present_source
    }

    fn apply_taa(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        taa_scalar: f32,
    ) -> PresentSource {
        let mut taa_source = PresentSource::Accumulation;
        if self.taa_history_valid {
            queue.write_buffer(
                &self.scalar_uniform_buffer,
                0,
                bytemuck::bytes_of(&GpuPreviewScalarUniform {
                    value: [taa_scalar, 0.0, 0.0, 0.0],
                }),
            );
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skins-preview-taa-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.post_process_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&self.taa_pipeline);
                pass.set_bind_group(0, &self.accumulation_bind_group, &[]);
                pass.set_bind_group(1, &self.taa_history_bind_group, &[]);
                pass.set_bind_group(2, &self.scalar_uniform_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.post_process_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.taa_history_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                self.render_target_extent(),
            );
            taa_source = PresentSource::PostProcess;
        } else {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.accumulation_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.taa_history_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                self.render_target_extent(),
            );
        }
        self.taa_history_valid = true;
        taa_source
    }
}

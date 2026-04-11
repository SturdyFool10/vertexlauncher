use super::*;

impl SkinPreviewPostProcessWgpuResources {
    pub(super) fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
    ) -> Self {
        // Linear FP16 for all intermediate render targets: holds HDR values without
        // clamping and gives the tone-mapping present pass a wide working range.
        const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

        let scene_shader = create_post_process_shader_module(
            device,
            "skins-preview-post-scene-shader",
            include_str!(concat!(
                env!("OUT_DIR"),
                "/shaders/skin_preview_post_scene.wgsl"
            )),
        );
        let accumulate_shader = create_post_process_shader_module(
            device,
            "skins-preview-accumulate-shader",
            include_str!(concat!(
                env!("OUT_DIR"),
                "/shaders/skin_preview_accumulate.wgsl"
            )),
        );
        let fxaa_shader = create_post_process_shader_module(
            device,
            "skins-preview-fxaa-shader",
            include_str!(concat!(env!("OUT_DIR"), "/shaders/skin_preview_fxaa.wgsl")),
        );
        let ssao_shader = create_post_process_shader_module(
            device,
            "skins-preview-ssao-shader",
            include_str!(concat!(env!("OUT_DIR"), "/shaders/skin_preview_ssao.wgsl")),
        );
        let smaa_shader = create_post_process_shader_module(
            device,
            "skins-preview-smaa-shader",
            include_str!(concat!(env!("OUT_DIR"), "/shaders/skin_preview_smaa.wgsl")),
        );
        let taa_shader = create_post_process_shader_module(
            device,
            "skins-preview-taa-shader",
            include_str!(concat!(env!("OUT_DIR"), "/shaders/skin_preview_taa.wgsl")),
        );
        let present_shader = create_post_process_shader_module(
            device,
            "skins-preview-present-shader",
            include_str!(concat!(
                env!("OUT_DIR"),
                "/shaders/skin_preview_present.wgsl"
            )),
        );

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
                    Some(&texture_bind_group_layout),
                    Some(&scene_uniform_layout),
                    Some(&scalar_uniform_layout),
                ],
                immediate_size: 0,
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
                targets: &[
                    // SV_Target0: scene color
                    Some(wgpu::ColorTargetState {
                        format: OFFSCREEN_FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    // SV_Target1: linearised window-space depth (r channel)
                    Some(wgpu::ColorTargetState {
                        format: OFFSCREEN_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SKIN_PREVIEW_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: scene_msaa_samples.max(1),
                mask: !0,
                alpha_to_coverage_enabled: scene_msaa_samples > 1,
            },
            multiview_mask: None,
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
            bind_group_layouts: &[
                Some(&texture_bind_group_layout),
                Some(&scalar_uniform_layout),
            ],
            immediate_size: 0,
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
            multiview_mask: None,
            cache: None,
        });

        let smaa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-smaa-layout"),
            bind_group_layouts: &[Some(&texture_bind_group_layout)],
            immediate_size: 0,
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
            multiview_mask: None,
            cache: None,
        });

        let fxaa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-fxaa-layout"),
            bind_group_layouts: &[Some(&texture_bind_group_layout)],
            immediate_size: 0,
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
            multiview_mask: None,
            cache: None,
        });

        // SSAO reads scene color from group 0 and depth-linear from group 1.
        let ssao_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-ssao-layout"),
            bind_group_layouts: &[
                Some(&texture_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let ssao_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-ssao-pipeline"),
            layout: Some(&ssao_layout),
            vertex: wgpu::VertexState {
                module: &ssao_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &ssao_shader,
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
            multiview_mask: None,
            cache: None,
        });

        let taa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-taa-layout"),
            bind_group_layouts: &[
                Some(&texture_bind_group_layout),
                Some(&texture_bind_group_layout),
                Some(&scalar_uniform_layout),
            ],
            immediate_size: 0,
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
            multiview_mask: None,
            cache: None,
        });

        // Present reads only the source texture — no uniform needed.
        let present_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-present-layout"),
            bind_group_layouts: &[Some(&texture_bind_group_layout)],
            immediate_size: 0,
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: present_msaa_samples.max(1),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        let (accumulation_texture, accumulation_view, accumulation_bind_group) =
            create_sampled_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-accumulation",
            );
        let (scene_resolve_texture, scene_resolve_view, scene_resolve_bind_group) =
            create_sampled_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-scene-resolve",
            );
        let (post_process_texture, post_process_view, post_process_bind_group) =
            create_sampled_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-post-process",
            );
        let (taa_history_texture, taa_history_view, taa_history_bind_group) =
            create_sampled_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-taa-history",
            );
        let (scene_depth_linear_texture, scene_depth_linear_view, scene_depth_linear_bind_group) =
            create_sampled_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-scene-depth-linear",
            );
        let (scene_depth_render_texture, scene_depth_render_view) = create_preview_depth_texture(
            device,
            [1, 1],
            scene_msaa_samples.max(1),
            "skins-preview-scene-depth",
        );
        let scene_depth =
            DepthAttachmentSet::new(scene_depth_render_texture, scene_depth_render_view);
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
            msaa_resolver: vertex_3d::MsaaResolvePool::new(device),
            shader_modules: SkinPreviewPostProcessShaderModules {
                scene_pipeline,
                accumulate_pipeline,
                ssao_pipeline,
                smaa_pipeline,
                fxaa_pipeline,
                taa_pipeline,
                present_pipeline,
                texture_bind_group_layout,
                texture_sampler,
            },
            uniforms: SkinPreviewPostProcessUniformResources {
                uniform_bind_group,
                uniform_buffer,
                scalar_uniform_bind_group_layout: scalar_uniform_layout,
                scalar_uniform_bind_group,
                scalar_uniform_buffer,
            },
            source_textures: SkinPreviewPostProcessSourceTextures::default(),
            render_targets: SkinPreviewPostProcessRenderTargets {
                accumulation_texture,
                accumulation_view,
                accumulation_bind_group,
                scene_resolve_texture,
                scene_resolve_view,
                scene_resolve_bind_group,
                scene_msaa_texture,
                scene_msaa_view,
                scene_depth,
                scene_depth_linear_texture,
                scene_depth_linear_view,
                scene_depth_linear_bind_group,
                scene_depth_linear_msaa_view: None,
                post_process_texture,
                post_process_view,
                post_process_bind_group,
                taa_history_texture,
                taa_history_view,
                taa_history_bind_group,
                taa_history_valid: false,
                render_target_size: [1, 1],
                present_source: PresentSource::Accumulation,
            },
            vertex3d_runtime: SkinPreviewVertex3dRuntime::new(
                device,
                target_format,
                scene_msaa_samples.max(1),
            ),
            cached_scene_plan: None,
            cached_scene_plan_batch_count: 0,
            cached_scene_plan_msaa_samples: 0,
            target_format,
            scene_msaa_samples: scene_msaa_samples.max(1),
            present_msaa_samples: present_msaa_samples.max(1),
        }
    }
}

fn create_post_process_shader_module(
    device: &wgpu::Device,
    label: &'static str,
    wgsl_source: &str,
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(wgsl_source.into()),
    })
}

use super::*;

impl SkinPreviewPostProcessWgpuResources {
    pub(super) fn ensure_render_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        let rebuilt =
            self.vertex3d_runtime
                .sync(device, size, self.target_format, self.scene_msaa_samples);
        if !rebuilt && self.render_targets.render_target_size == size {
            return;
        }
        self.render_targets.render_target_size = size;
        self.render_targets.taa_history_valid = false;

        let attachment = |handle: &str| {
            self.vertex3d_runtime
                .attachment(handle)
                .unwrap_or_else(|| panic!("missing vertex3d attachment: {handle}"))
        };

        let accumulation_attachment = attachment("accumulation");
        let accumulation_texture = accumulation_attachment.texture.clone();
        let accumulation_view = accumulation_attachment.view.clone();
        let accumulation_bind_group = create_preview_texture_bind_group(
            device,
            &self.shader_modules.texture_bind_group_layout,
            &self.shader_modules.texture_sampler,
            &accumulation_view,
            "skins-preview-accumulation",
        );
        self.render_targets.accumulation_texture = accumulation_texture;
        self.render_targets.accumulation_view = accumulation_view;
        self.render_targets.accumulation_bind_group = accumulation_bind_group;

        let scene_color_attachment = attachment("scene_color");
        let scene_resolve_texture = scene_color_attachment.texture.clone();
        let scene_resolve_view = scene_color_attachment.view.clone();
        let scene_resolve_bind_group = create_preview_texture_bind_group(
            device,
            &self.shader_modules.texture_bind_group_layout,
            &self.shader_modules.texture_sampler,
            &scene_resolve_view,
            "skins-preview-scene-resolve",
        );
        self.render_targets.scene_resolve_texture = scene_resolve_texture;
        self.render_targets.scene_resolve_view = scene_resolve_view;
        self.render_targets.scene_resolve_bind_group = scene_resolve_bind_group;

        let post_process_attachment = attachment("post_process");
        let post_process_texture = post_process_attachment.texture.clone();
        let post_process_view = post_process_attachment.view.clone();
        let post_process_bind_group = create_preview_texture_bind_group(
            device,
            &self.shader_modules.texture_bind_group_layout,
            &self.shader_modules.texture_sampler,
            &post_process_view,
            "skins-preview-post-process",
        );
        self.render_targets.post_process_texture = post_process_texture;
        self.render_targets.post_process_view = post_process_view;
        self.render_targets.post_process_bind_group = post_process_bind_group;

        let taa_history_attachment = attachment("taa_history");
        let taa_history_texture = taa_history_attachment.texture.clone();
        let taa_history_view = taa_history_attachment.view.clone();
        let taa_history_bind_group = create_preview_texture_bind_group(
            device,
            &self.shader_modules.texture_bind_group_layout,
            &self.shader_modules.texture_sampler,
            &taa_history_view,
            "skins-preview-taa-history",
        );
        self.render_targets.taa_history_texture = taa_history_texture;
        self.render_targets.taa_history_view = taa_history_view;
        self.render_targets.taa_history_bind_group = taa_history_bind_group;

        let scene_depth_attachment = attachment("scene_depth");
        let scene_depth_texture = scene_depth_attachment.texture.clone();
        let scene_depth_view = scene_depth_attachment.view.clone();
        let scene_depth_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skins-preview-scene-depth"),
            layout: &self.shader_modules.depth_texture_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&scene_depth_view),
            }],
        });
        self.render_targets.scene_depth_texture = scene_depth_texture;
        self.render_targets.scene_depth_view = scene_depth_view;
        self.render_targets.scene_depth_bind_group = scene_depth_bind_group;

        let scene_msaa_attachment = self.vertex3d_runtime.attachment("scene_msaa");
        self.render_targets.scene_msaa_texture =
            scene_msaa_attachment.map(|attachment| attachment.texture.clone());
        self.render_targets.scene_msaa_view =
            scene_msaa_attachment.map(|attachment| attachment.view.clone());
    }

    pub(super) fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: TextureSlot,
        hash: u64,
        image: &RgbaImage,
    ) {
        let size = [image.width(), image.height()];
        let target = match slot {
            TextureSlot::Skin => &mut self.source_textures.skin_texture,
            TextureSlot::Cape => &mut self.source_textures.cape_texture,
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

        upload_preview_texture_mips(queue, &texture, image);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = create_preview_texture_bind_group(
            device,
            &self.shader_modules.texture_bind_group_layout,
            &self.shader_modules.texture_sampler,
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

    pub(super) fn update_scene_uniform(&self, queue: &wgpu::Queue, screen_size_points: [f32; 2]) {
        let uniform = GpuPreviewUniform {
            screen_size_points,
            _pad: [0.0; 2],
        };
        queue.write_buffer(
            &self.uniforms.uniform_buffer,
            0,
            bytemuck::bytes_of(&uniform),
        );
    }

    pub(super) fn update_scene_texture_aa_mode(
        &self,
        queue: &wgpu::Queue,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) {
        queue.write_buffer(
            &self.uniforms.scalar_uniform_buffer,
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
}

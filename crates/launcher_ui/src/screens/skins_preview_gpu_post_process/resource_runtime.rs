use super::*;

impl SkinPreviewPostProcessWgpuResources {
    pub(super) fn ensure_render_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        if self.render_targets.render_target_size == size {
            return;
        }
        self.render_targets.render_target_size = size;
        self.render_targets.taa_history_valid = false;

        const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
        let (accumulation_texture, accumulation_view, accumulation_bind_group) =
            create_sampled_render_texture(
                device,
                &self.shader_modules.texture_bind_group_layout,
                &self.shader_modules.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-accumulation",
            );
        self.render_targets.accumulation_texture = accumulation_texture;
        self.render_targets.accumulation_view = accumulation_view;
        self.render_targets.accumulation_bind_group = accumulation_bind_group;

        let (scene_resolve_texture, scene_resolve_view, scene_resolve_bind_group) =
            create_sampled_render_texture(
                device,
                &self.shader_modules.texture_bind_group_layout,
                &self.shader_modules.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-scene-resolve",
            );
        self.render_targets.scene_resolve_texture = scene_resolve_texture;
        self.render_targets.scene_resolve_view = scene_resolve_view;
        self.render_targets.scene_resolve_bind_group = scene_resolve_bind_group;

        let (post_process_texture, post_process_view, post_process_bind_group) =
            create_sampled_render_texture(
                device,
                &self.shader_modules.texture_bind_group_layout,
                &self.shader_modules.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-post-process",
            );
        self.render_targets.post_process_texture = post_process_texture;
        self.render_targets.post_process_view = post_process_view;
        self.render_targets.post_process_bind_group = post_process_bind_group;

        let (taa_history_texture, taa_history_view, taa_history_bind_group) =
            create_sampled_render_texture(
                device,
                &self.shader_modules.texture_bind_group_layout,
                &self.shader_modules.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-taa-history",
            );
        self.render_targets.taa_history_texture = taa_history_texture;
        self.render_targets.taa_history_view = taa_history_view;
        self.render_targets.taa_history_bind_group = taa_history_bind_group;

        let (scene_depth_texture, scene_depth_view) = create_preview_depth_texture(
            device,
            size,
            self.scene_msaa_samples.max(1),
            "skins-preview-scene-depth",
        );
        self.render_targets.scene_depth_texture = scene_depth_texture;
        self.render_targets.scene_depth_view = scene_depth_view;

        if self.scene_msaa_samples > 1 {
            let (texture, view) = create_preview_color_texture(
                device,
                OFFSCREEN_FORMAT,
                size,
                self.scene_msaa_samples.max(1),
                "skins-preview-scene-msaa",
            );
            self.render_targets.scene_msaa_texture = Some(texture);
            self.render_targets.scene_msaa_view = Some(view);
        } else {
            self.render_targets.scene_msaa_texture = None;
            self.render_targets.scene_msaa_view = None;
        }
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

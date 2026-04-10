use super::*;

impl SkinPreviewPostProcessWgpuResources {
    pub(super) fn apply_smaa(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("skins-preview-smaa-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.render_targets.post_process_view,
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
            multiview_mask: None,
        });
        pass.set_pipeline(&self.shader_modules.smaa_pipeline);
        pass.set_bind_group(0, &self.render_targets.accumulation_bind_group, &[]);
        pass.draw(0..3, 0..1);
        self.render_targets.present_source = PresentSource::PostProcess;
        self.render_targets.taa_history_valid = false;
    }

    pub(super) fn apply_fxaa(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: PresentSource,
    ) -> PresentSource {
        let (source_bind_group, target_view, present_source, label) = match source {
            PresentSource::Accumulation => (
                &self.render_targets.accumulation_bind_group,
                &self.render_targets.post_process_view,
                PresentSource::PostProcess,
                "skins-preview-fxaa-pass",
            ),
            PresentSource::PostProcess => (
                &self.render_targets.post_process_bind_group,
                &self.render_targets.accumulation_view,
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
            multiview_mask: None,
        });
        pass.set_pipeline(&self.shader_modules.fxaa_pipeline);
        pass.set_bind_group(0, source_bind_group, &[]);
        pass.draw(0..3, 0..1);
        present_source
    }

    pub(super) fn apply_taa(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        taa_scalar: f32,
    ) -> PresentSource {
        let mut taa_source = PresentSource::Accumulation;
        if self.render_targets.taa_history_valid {
            queue.write_buffer(
                &self.uniforms.scalar_uniform_buffer,
                0,
                bytemuck::bytes_of(&GpuPreviewScalarUniform {
                    value: [taa_scalar, 0.0, 0.0, 0.0],
                }),
            );
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skins-preview-taa-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.render_targets.post_process_view,
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
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.shader_modules.taa_pipeline);
                pass.set_bind_group(0, &self.render_targets.accumulation_bind_group, &[]);
                pass.set_bind_group(1, &self.render_targets.taa_history_bind_group, &[]);
                pass.set_bind_group(2, &self.uniforms.scalar_uniform_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.render_targets.post_process_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.render_targets.taa_history_texture,
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
                    texture: &self.render_targets.accumulation_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.render_targets.taa_history_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                self.render_target_extent(),
            );
        }
        self.render_targets.taa_history_valid = true;
        taa_source
    }
}

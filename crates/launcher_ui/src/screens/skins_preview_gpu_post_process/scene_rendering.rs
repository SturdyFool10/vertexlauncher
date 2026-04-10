use super::*;

impl SkinPreviewPostProcessWgpuResources {
    pub(super) fn scene_color_attachment(
        &self,
        _clear: bool,
    ) -> wgpu::RenderPassColorAttachment<'_> {
        if let Some(view) = self.render_targets.scene_msaa_view.as_ref() {
            wgpu::RenderPassColorAttachment {
                view,
                resolve_target: Some(&self.render_targets.scene_resolve_view),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            }
        } else {
            wgpu::RenderPassColorAttachment {
                view: &self.render_targets.scene_resolve_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            }
        }
    }

    pub(super) fn scene_depth_view(&self) -> &wgpu::TextureView {
        &self.render_targets.scene_depth_view
    }

    pub(super) fn paint_scene(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        batch: &PreparedGpuPreviewSceneBatch,
    ) {
        render_pass.set_pipeline(&self.shader_modules.scene_pipeline);
        render_pass.set_bind_group(1, &self.uniforms.uniform_bind_group, &[]);
        render_pass.set_bind_group(2, &self.uniforms.scalar_uniform_bind_group, &[]);

        if let Some(texture) = self.source_textures.skin_texture.as_ref() {
            render_pass.set_bind_group(0, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.skin_vertex_buffer.slice(..));
            render_pass
                .set_index_buffer(batch.skin_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..batch.skin_index_count, 0, 0..1);
        }
        if let Some(texture) = self.source_textures.cape_texture.as_ref() {
            render_pass.set_bind_group(0, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.cape_vertex_buffer.slice(..));
            render_pass
                .set_index_buffer(batch.cape_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..batch.cape_index_count, 0, 0..1);
        }
    }

    pub(super) fn render_target_extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.render_targets.render_target_size[0].max(1),
            height: self.render_targets.render_target_size[1].max(1),
            depth_or_array_layers: 1,
        }
    }
}

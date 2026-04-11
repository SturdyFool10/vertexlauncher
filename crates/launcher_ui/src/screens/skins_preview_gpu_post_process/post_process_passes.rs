use super::*;

impl SkinPreviewPostProcessWgpuResources {
    pub(super) fn execute_vertex3d_post_process_plan(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        plan: &Vertex3dPostProcessPlan,
    ) {
        let _graph = &plan.frame_graph;
        for op in &plan.operations {
            match op {
                Vertex3dPostProcessOp::Fullscreen {
                    name,
                    pipeline,
                    source,
                    history,
                    target,
                    taa_scalar,
                } => self.execute_fullscreen_pass(
                    encoder,
                    queue,
                    name,
                    *pipeline,
                    source,
                    *history,
                    target,
                    *taa_scalar,
                ),
                Vertex3dPostProcessOp::Copy {
                    name: _name,
                    source,
                    target,
                } => self.copy_attachment_to_attachment(encoder, source, target),
                Vertex3dPostProcessOp::Present { source } => {
                    self.render_targets.present_source = present_source_from_handle(source);
                }
                Vertex3dPostProcessOp::SetTaaHistoryValid(value) => {
                    self.render_targets.taa_history_valid = *value;
                }
            }
        }
    }

    fn execute_fullscreen_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        name: &str,
        pipeline: Vertex3dPostProcessPipeline,
        source: &str,
        history: Option<&str>,
        target: &str,
        taa_scalar: Option<f32>,
    ) {
        if let Some(taa_scalar) = taa_scalar {
            queue.write_buffer(
                &self.uniforms.scalar_uniform_buffer,
                0,
                bytemuck::bytes_of(&GpuPreviewScalarUniform {
                    value: [taa_scalar, 0.0, 0.0, 0.0],
                }),
            );
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(name),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.attachment_view(target),
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

        pass.set_pipeline(match pipeline {
            Vertex3dPostProcessPipeline::Ssao => &self.shader_modules.ssao_pipeline,
            Vertex3dPostProcessPipeline::Smaa => &self.shader_modules.smaa_pipeline,
            Vertex3dPostProcessPipeline::Fxaa => &self.shader_modules.fxaa_pipeline,
            Vertex3dPostProcessPipeline::Taa => &self.shader_modules.taa_pipeline,
        });
        pass.set_bind_group(0, self.attachment_bind_group(source), &[]);
        if let Some(history) = history {
            pass.set_bind_group(1, self.attachment_bind_group(history), &[]);
            pass.set_bind_group(2, &self.uniforms.scalar_uniform_bind_group, &[]);
        }
        pass.draw(0..3, 0..1);
    }

    fn copy_attachment_to_attachment(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &str,
        target: &str,
    ) {
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: self.attachment_texture(source),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: self.attachment_texture(target),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            self.render_target_extent(),
        );
    }

    fn attachment_bind_group(&self, handle: &str) -> &wgpu::BindGroup {
        match handle {
            "accumulation" => &self.render_targets.accumulation_bind_group,
            "post_process" => &self.render_targets.post_process_bind_group,
            "taa_history" => &self.render_targets.taa_history_bind_group,
            "scene_depth_linear" => &self.render_targets.scene_depth_linear_bind_group,
            _ => panic!("unsupported post-process attachment bind group: {handle}"),
        }
    }

    fn attachment_view(&self, handle: &str) -> &wgpu::TextureView {
        match handle {
            "accumulation" => &self.render_targets.accumulation_view,
            "post_process" => &self.render_targets.post_process_view,
            "taa_history" => &self.render_targets.taa_history_view,
            _ => panic!("unsupported post-process attachment view: {handle}"),
        }
    }

    fn attachment_texture(&self, handle: &str) -> &wgpu::Texture {
        match handle {
            "accumulation" => &self.render_targets.accumulation_texture,
            "post_process" => &self.render_targets.post_process_texture,
            "taa_history" => &self.render_targets.taa_history_texture,
            _ => panic!("unsupported post-process attachment texture: {handle}"),
        }
    }
}

fn present_source_from_handle(handle: &str) -> PresentSource {
    match handle {
        "accumulation" => PresentSource::Accumulation,
        "post_process" => PresentSource::PostProcess,
        _ => panic!("unsupported post-process present source handle: {handle}"),
    }
}

use super::*;

impl SkinPreviewPostProcessWgpuResources {
    pub(super) fn execute_vertex3d_scene_plan(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        prepared_batches: &[PreparedGpuPreviewSceneBatch],
        plan: &Vertex3dScenePlan,
    ) {
        let _graph = &plan.frame_graph;
        for op in &plan.operations {
            match *op {
                Vertex3dSceneOp::SceneDraw {
                    batch_index,
                    clear_depth,
                } => {
                    let batch = prepared_batches
                        .get(batch_index)
                        .unwrap_or_else(|| panic!("missing prepared scene batch: {batch_index}"));
                    let color_attachment = self.scene_color_attachment(clear_depth);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("skins-preview-scene-pass"),
                        color_attachments: &[Some(color_attachment)],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: self.scene_depth_view(),
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    self.paint_scene(&mut pass, batch);
                }
                Vertex3dSceneOp::DepthResolve { .. } => {
                    // Auto-resolve any MSAA attachments that have a `{name}_resolve` partner.
                    // Detects scene_depth → scene_depth_resolve (and any future pairs).
                    self.msaa_resolver
                        .auto_resolve(encoder, device, self.vertex3d_runtime.pool());
                }
                Vertex3dSceneOp::Accumulate {
                    batch_index,
                    clear_target,
                } => {
                    let batch = prepared_batches
                        .get(batch_index)
                        .unwrap_or_else(|| panic!("missing prepared scene batch: {batch_index}"));
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("skins-preview-accumulation-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.render_targets.accumulation_view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: if clear_target {
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
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&self.shader_modules.accumulate_pipeline);
                    pass.set_bind_group(0, &self.render_targets.scene_resolve_bind_group, &[]);
                    pass.set_bind_group(1, &batch.weight_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }
    }

    pub(super) fn scene_color_attachment(
        &self,
        clear: bool,
    ) -> wgpu::RenderPassColorAttachment<'_> {
        if let Some(view) = self.render_targets.scene_msaa_view.as_ref() {
            wgpu::RenderPassColorAttachment {
                view,
                resolve_target: Some(&self.render_targets.scene_resolve_view),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            }
        } else {
            wgpu::RenderPassColorAttachment {
                view: &self.render_targets.scene_resolve_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            }
        }
    }

    pub(super) fn scene_depth_view(&self) -> &wgpu::TextureView {
        &self.render_targets.scene_depth.render_view
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

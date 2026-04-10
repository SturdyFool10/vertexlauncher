use super::*;

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
            resources.source_textures.cape_texture = None;
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
        resources.render_targets.present_source = PresentSource::Accumulation;

        for (index, batch) in self.scene_batches.iter().enumerate() {
            let prepared_batch = prepare_preview_scene_batch_buffers(
                device,
                &resources.uniforms.scalar_uniform_bind_group_layout,
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
                        view: &resources.render_targets.accumulation_view,
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
                pass.set_pipeline(&resources.shader_modules.accumulate_pipeline);
                pass.set_bind_group(0, &resources.render_targets.scene_resolve_bind_group, &[]);
                pass.set_bind_group(1, &prepared_batch.weight_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        if use_smaa {
            resources.apply_smaa(&mut encoder);
        } else if use_fxaa && !use_taa {
            resources.apply_fxaa(&mut encoder, PresentSource::Accumulation);
            resources.render_targets.present_source = PresentSource::PostProcess;
        } else if use_taa {
            let taa_source = resources.apply_taa(
                &mut encoder,
                queue,
                if use_fxaa_after_taa { 0.22 } else { 0.35 },
            );
            if use_fxaa_after_taa {
                resources.render_targets.present_source =
                    resources.apply_fxaa(&mut encoder, taa_source);
            } else {
                resources.render_targets.present_source = taa_source;
            }
        } else {
            resources.render_targets.taa_history_valid = false;
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
        render_pass.set_pipeline(&resources.shader_modules.present_pipeline);
        let bind_group = match resources.render_targets.present_source {
            PresentSource::Accumulation => &resources.render_targets.accumulation_bind_group,
            PresentSource::PostProcess => &resources.render_targets.post_process_bind_group,
        };
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

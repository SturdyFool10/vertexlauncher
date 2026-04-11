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
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
            if let (Some(cape_hash), Some(cape_sample)) =
                (self.cape_hash, self.cape_sample.as_ref())
            {
                resources.update_texture(device, queue, TextureSlot::Cape, cape_hash, cape_sample);
            } else {
                resources.source_textures.cape_texture = None;
            }

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("skins-preview-post-process-encoder"),
            });

            resources.render_targets.present_source = PresentSource::Accumulation;

            let prepared_batches = self
                .scene_batches
                .iter()
                .map(|batch| {
                    prepare_preview_scene_batch_buffers(
                        device,
                        &resources.uniforms.scalar_uniform_bind_group_layout,
                        batch,
                    )
                })
                .collect::<Vec<_>>();
            let scene_plan =
                Vertex3dScenePlan::build(prepared_batches.len(), self.scene_msaa_samples);
            resources.execute_vertex3d_scene_plan(
                device,
                &mut encoder,
                &prepared_batches,
                &scene_plan,
            );

            let post_plan = Vertex3dPostProcessPlan::build(
                self.aa_mode,
                resources.render_targets.taa_history_valid,
            );
            resources.execute_vertex3d_post_process_plan(&mut encoder, queue, &post_plan);

            vec![encoder.finish()]
        })) {
            Ok(command_buffers) => command_buffers,
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    "Skin preview WGPU callback panicked during prepare; skipping this frame."
                );
                Vec::new()
            }
        }
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
        }));

        if result.is_err() {
            tracing::error!(
                target: "vertexlauncher/skins",
                "Skin preview WGPU callback panicked during paint; skipping this frame."
            );
        }
    }
}

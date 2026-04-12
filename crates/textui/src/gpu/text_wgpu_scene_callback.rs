use super::*;

#[derive(Clone)]
pub(crate) struct TextWgpuSceneCallback {
    pub(crate) target_format: wgpu::TextureFormat,
    pub(crate) atlas_sampling: TextAtlasSampling,
    pub(crate) linear_pipeline: bool,
    pub(crate) output_is_hdr: bool,
    pub(crate) batches: Arc<[TextWgpuSceneBatchSource]>,
    pub(crate) prepared: Arc<Mutex<TextWgpuPreparedScene>>,
}

impl egui_wgpu::CallbackTrait for TextWgpuSceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (texture_bind_group_layout, sampler) = {
                let resources = callback_resources
                    .entry::<TextWgpuPipelineResources>()
                    .or_insert_with(|| {
                        TextWgpuPipelineResources::new(
                            device,
                            self.target_format,
                            self.atlas_sampling,
                            self.linear_pipeline,
                            self.output_is_hdr,
                        )
                    });
                if resources.target_format != self.target_format
                    || resources.atlas_sampling != self.atlas_sampling
                    || resources.linear_pipeline != self.linear_pipeline
                    || resources.output_is_hdr != self.output_is_hdr
                {
                    *resources = TextWgpuPipelineResources::new(
                        device,
                        self.target_format,
                        self.atlas_sampling,
                        self.linear_pipeline,
                        self.output_is_hdr,
                    );
                }
                resources.update_uniform(
                    queue,
                    [
                        screen_descriptor.size_in_pixels[0] as f32
                            / screen_descriptor.pixels_per_point,
                        screen_descriptor.size_in_pixels[1] as f32
                            / screen_descriptor.pixels_per_point,
                    ],
                    self.output_is_hdr,
                );
                (
                    resources.texture_bind_group_layout.clone(),
                    resources.sampler.clone(),
                )
            };
            let total_instance_bytes = self
                .batches
                .iter()
                .map(|batch| {
                    (batch.instances.len() * std::mem::size_of::<TextWgpuInstance>()) as u64
                })
                .sum::<u64>();
            let texture_binding_cache = callback_resources
                .entry::<TextWgpuTextureBindingCache>()
                .or_insert_with(TextWgpuTextureBindingCache::default);
            if let Some(current_generation) =
                self.batches.first().map(|batch| batch.atlas_generation)
            {
                texture_binding_cache
                    .entries
                    .retain(|(generation, _), _| *generation == current_generation);
            }

            let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("textui_instanced_instance_buffer"),
                size: total_instance_bytes.max(1),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut prepared_batches = Vec::with_capacity(self.batches.len());
            let mut instance_buffer_offset = 0_u64;
            for batch in self.batches.iter() {
                if batch.instances.is_empty() {
                    continue;
                }
                let instance_bytes = bytemuck::cast_slice(batch.instances.as_ref());
                queue.write_buffer(&instance_buffer, instance_buffer_offset, instance_bytes);
                let cache_key = (batch.atlas_generation, batch.page_index);
                let bind_group = texture_binding_cache
                    .entries
                    .entry(cache_key)
                    .or_insert_with(|| {
                        let view = batch
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("textui_instanced_texture_bg"),
                            layout: &texture_bind_group_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(&view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::Sampler(&sampler),
                                },
                            ],
                        });
                        TextWgpuCachedTextureBinding { bind_group }
                    })
                    .bind_group
                    .clone();
                prepared_batches.push(TextWgpuPreparedBatch {
                    bind_group,
                    instance_buffer: instance_buffer.clone(),
                    instance_buffer_offset,
                    instance_count: batch.instances.len() as u32,
                });
                instance_buffer_offset += instance_bytes.len() as u64;
            }

            if let Ok(mut prepared) = self.prepared.lock() {
                prepared.instance_buffer = Some(instance_buffer);
                prepared.batches = prepared_batches;
            }

            Vec::new()
        })) {
            Ok(command_buffers) => command_buffers,
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/textui",
                    "Text WGPU callback panicked during prepare; skipping this frame."
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
            let Some(resources) = callback_resources.get::<TextWgpuPipelineResources>() else {
                return;
            };
            let Ok(prepared) = self.prepared.lock() else {
                return;
            };
            if prepared.batches.is_empty() {
                return;
            }

            render_pass.set_viewport(
                0.0,
                0.0,
                info.screen_size_px[0] as f32,
                info.screen_size_px[1] as f32,
                0.0,
                1.0,
            );
            render_pass.set_pipeline(&resources.pipeline);
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            for batch in &prepared.batches {
                render_pass.set_bind_group(1, &batch.bind_group, &[]);
                let instance_stride = std::mem::size_of::<TextWgpuInstance>() as u64;
                let instance_byte_len = u64::from(batch.instance_count) * instance_stride;
                render_pass.set_vertex_buffer(
                    0,
                    batch.instance_buffer.slice(
                        batch.instance_buffer_offset
                            ..batch.instance_buffer_offset + instance_byte_len,
                    ),
                );
                render_pass.draw(0..4, 0..batch.instance_count);
            }
        }));

        if result.is_err() {
            tracing::error!(
                target: "vertexlauncher/textui",
                "Text WGPU callback panicked during paint; skipping this frame."
            );
        }
    }
}

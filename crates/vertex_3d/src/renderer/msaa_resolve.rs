//! Automatic MSAA resolve subsystem for the vertex_3d renderer.
//!
//! When a render attachment has `samples > 1` and a corresponding `{name}_resolve`
//! attachment exists in the pool with `samples == 1`, [`MsaaResolvePool::auto_resolve`]
//! detects the pair and inserts the appropriate GPU resolve commands.
//!
//! - **Color attachments** are resolved via a render-pass `resolve_target` (zero draw calls).
//! - **Depth attachments** are resolved via a full-screen shader that reads sample 0 with
//!   `textureLoad` and writes the result through `@builtin(frag_depth)`.
//!
//! The naming convention (`{name}` + `{name}_resolve`) is the only contract between
//! producers and consumers of multisampled attachments. No manual bookkeeping is required.

use std::collections::HashMap;

use super::{AttachmentPool, RenderTargetHandle};

const DEPTH_RESOLVE_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_depth_multisampled_2d;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle: covers [-1,1]^2 NDC with 3 vertices.
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

struct DepthOut {
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> DepthOut {
    let coords = vec2<i32>(i32(pos.x), i32(pos.y));
    return DepthOut(textureLoad(src, coords, 0));
}
"#;

/// Bind group layout for a `texture_depth_multisampled_2d` input.
/// Kept alive so pipelines can share it.
struct DepthMsaaLayout {
    layout: wgpu::BindGroupLayout,
}

impl DepthMsaaLayout {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("msaa-resolve-depth-input-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: true,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            }],
        });
        Self { layout }
    }
}

/// Detects multisampled attachments in an [`AttachmentPool`] and automatically inserts
/// resolve passes into a command encoder before they are consumed as shader inputs.
///
/// Create once per renderer, reuse every frame. The first call for a given depth
/// format compiles and caches the resolve pipeline for that format.
pub struct MsaaResolvePool {
    depth_input_layout: DepthMsaaLayout,
    shader: wgpu::ShaderModule,
    /// One pipeline per depth format, created lazily.
    depth_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl MsaaResolvePool {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("msaa-depth-resolve-shader"),
            source: wgpu::ShaderSource::Wgsl(DEPTH_RESOLVE_WGSL.into()),
        });
        Self {
            depth_input_layout: DepthMsaaLayout::new(device),
            shader,
            depth_pipelines: HashMap::new(),
        }
    }

    /// Scans `pool` for MSAA attachments and resolves each one into its `{name}_resolve`
    /// partner if present.
    ///
    /// This should be called after all scene draw passes that write to MSAA attachments
    /// and before any post-process pass that reads from the resolved versions.
    pub fn auto_resolve(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        pool: &AttachmentPool,
    ) {
        // Collect MSAA sources that have a matching resolve target.
        let pairs: Vec<(RenderTargetHandle, RenderTargetHandle)> = pool
            .iter()
            .filter(|att| att.samples > 1)
            .filter_map(|src| {
                let resolve_handle =
                    RenderTargetHandle::new(format!("{}_resolve", src.handle.as_str()));
                pool.get(&resolve_handle)?;
                Some((src.handle.clone(), resolve_handle))
            })
            .collect();

        for (src_handle, dst_handle) in pairs {
            let src = pool.get(&src_handle).unwrap();
            let dst = pool.get(&dst_handle).unwrap();

            if src.format.is_depth_stencil_format() {
                self.resolve_depth(encoder, device, &src.texture, &src.view, &dst.view, src.format);
            } else {
                Self::resolve_color(encoder, &src.view, &dst.view, src.samples, src.format);
            }
        }
    }

    /// Resolves a multisampled **color** texture into a single-sample target.
    ///
    /// Uses the render-pass `resolve_target` mechanism — no draw calls are issued.
    /// The MSAA texture is loaded but its contents are discarded after resolve.
    pub fn resolve_color(
        encoder: &mut wgpu::CommandEncoder,
        src_msaa_view: &wgpu::TextureView,
        dst_view: &wgpu::TextureView,
        _src_sample_count: u32,
        _format: wgpu::TextureFormat,
    ) {
        // A render pass with resolve_target resolves implicitly on pass end.
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("msaa-color-resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: src_msaa_view,
                resolve_target: Some(dst_view),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // No draw calls — the resolve happens automatically when the pass ends.
    }

    /// Resolves a multisampled **depth** texture into a single-sample target using a
    /// full-screen shader that picks sample index 0.
    ///
    /// WebGPU does not support depth-stencil `resolve_target`, so this is done via a
    /// fragment shader writing `@builtin(frag_depth)`.
    pub fn resolve_depth(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        src_texture: &wgpu::Texture,
        src_view: &wgpu::TextureView,
        dst_view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        // Ensure the pipeline exists, ending the mutable borrow before immutable borrows below.
        if !self.depth_pipelines.contains_key(&format) {
            let pipeline = Self::build_depth_pipeline(
                device,
                &self.depth_input_layout.layout,
                &self.shader,
                format,
            );
            self.depth_pipelines.insert(format, pipeline);
        }
        let pipeline = &self.depth_pipelines[&format];

        let src_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("msaa-depth-resolve-src"),
            layout: &self.depth_input_layout.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(src_view),
            }],
        });

        let _ = src_texture; // kept alive by its view

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("msaa-depth-resolve"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: dst_view,
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

        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &src_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn build_depth_pipeline(
        device: &wgpu::Device,
        input_layout: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("msaa-depth-resolve-pipeline-layout"),
            bind_group_layouts: &[Some(input_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("msaa-depth-resolve-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

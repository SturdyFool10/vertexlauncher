use std::collections::BTreeMap;

use super::*;
use vertex_3d::{
    AttachmentLifecycle, AttachmentPool, AttachmentTexture, GraphAttachment, ReflectionSnapshot,
    RenderTargetConfig, RenderTargetType, RendererConfig, RendererRuntime, ShaderGraphDescriptor,
    SurfaceConfig,
};

const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const SLANG_SOURCES: [&str; 6] = [
    include_str!("../shaders/skin_preview_post_scene.slang"),
    include_str!("../shaders/skin_preview_accumulate.slang"),
    include_str!("../shaders/skin_preview_fxaa.slang"),
    include_str!("../shaders/skin_preview_smaa.slang"),
    include_str!("../shaders/skin_preview_taa.slang"),
    include_str!("../shaders/skin_preview_present.slang"),
];

pub(super) struct SkinPreviewVertex3dRuntime {
    runtime: RendererRuntime,
    attachments: AttachmentPool,
    surface_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
}

impl SkinPreviewVertex3dRuntime {
    pub(super) fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
    ) -> Self {
        let config = build_renderer_config([1, 1], surface_format, scene_msaa_samples.max(1));
        let mut runtime = RendererRuntime::new(config);
        let mut attachments = AttachmentPool::default();
        attachments.rebuild(device, runtime.config());
        runtime.clear_rebuild_flags();
        Self {
            runtime,
            attachments,
            surface_format,
            scene_msaa_samples: scene_msaa_samples.max(1),
        }
    }

    pub(super) fn sync(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        surface_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
    ) -> bool {
        let size = [size[0].max(1), size[1].max(1)];
        let scene_msaa_samples = scene_msaa_samples.max(1);
        let rebuild_attachments;

        if self.scene_msaa_samples != scene_msaa_samples {
            self.scene_msaa_samples = scene_msaa_samples;
            self.surface_format = surface_format;
            self.runtime = RendererRuntime::new(build_renderer_config(
                size,
                surface_format,
                scene_msaa_samples,
            ));
            rebuild_attachments = true;
        } else {
            if self.surface_format != surface_format {
                self.surface_format = surface_format;
                self.runtime.set_surface_format(surface_format);
            }
            self.runtime.resize(size[0], size[1]);
            rebuild_attachments = !self.runtime.pending_rebuild().is_empty();
        }

        if rebuild_attachments {
            self.attachments.rebuild(device, self.runtime.config());
            self.runtime.clear_rebuild_flags();
        }

        rebuild_attachments
    }

    pub(super) fn attachment(&self, handle: &str) -> Option<&AttachmentTexture> {
        self.attachments.get(&handle.into())
    }

    pub(super) fn pool(&self) -> &AttachmentPool {
        &self.attachments
    }
}

fn build_renderer_config(
    size: [u32; 2],
    surface_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
) -> RendererConfig {
    let surface = SurfaceConfig::new(size[0], size[1], surface_format);
    let mut graph = collect_reflection_graph([size[0], size[1]]);

    if !graph.attachments.contains_key(&"scene_depth".into()) {
        graph = graph.with_attachment(
            GraphAttachment::new(
                "scene_depth",
                RenderTargetConfig::new(RenderTargetType::Depth, size[0], size[1])
                    .with_samples(scene_msaa_samples),
            )
            .with_lifecycle(AttachmentLifecycle::Transient),
        );
    }
    if !graph
        .attachments
        .contains_key(&"scene_depth_resolve".into())
    {
        graph = graph.with_attachment(
            GraphAttachment::new(
                "scene_depth_resolve",
                RenderTargetConfig::new(RenderTargetType::Depth, size[0], size[1]),
            )
            .with_lifecycle(AttachmentLifecycle::Transient),
        );
    }
    if !graph.attachments.contains_key(&"taa_history".into()) {
        graph = graph.with_attachment(
            GraphAttachment::new(
                "taa_history",
                RenderTargetConfig::new(RenderTargetType::Lighting, size[0], size[1]),
            )
            .with_lifecycle(AttachmentLifecycle::History),
        );
    }
    if scene_msaa_samples > 1 {
        graph = graph.with_attachment(
            GraphAttachment::new(
                "scene_msaa",
                RenderTargetConfig::new(RenderTargetType::Lighting, size[0], size[1])
                    .with_samples(scene_msaa_samples),
            )
            .with_lifecycle(AttachmentLifecycle::Transient),
        );
    }

    let mut config = RendererConfig::new(surface)
        .with_graph(graph)
        .with_msaa_samples(1);
    for handle in [
        "scene_color",
        "scene_msaa",
        "accumulation",
        "post_process",
        "taa_history",
    ] {
        if config.graph.attachments.contains_key(&handle.into()) {
            config.set_format_override(handle, OFFSCREEN_FORMAT);
        }
    }
    config.set_format_override("scene_depth", SKIN_PREVIEW_DEPTH_FORMAT);
    config.set_format_override("scene_depth_resolve", SKIN_PREVIEW_DEPTH_FORMAT);
    config
}

fn collect_reflection_graph(size: [u32; 2]) -> ShaderGraphDescriptor {
    let mut targets = BTreeMap::<String, vertex_3d::ReflectedRenderTarget>::new();

    for source in SLANG_SOURCES {
        let snapshot = ReflectionSnapshot::from_slang_source(source);
        for target in snapshot.render_targets {
            if target.handle == "present" {
                continue;
            }
            targets.entry(target.handle.clone()).or_insert(target);
        }
    }

    let mut graph = ShaderGraphDescriptor::new();
    for target in targets.values() {
        graph = graph.with_attachment(
            target.to_graph_attachment(vertex_3d::RenderTargetScale::Full, (size[0], size[1])),
        );
    }
    graph
}

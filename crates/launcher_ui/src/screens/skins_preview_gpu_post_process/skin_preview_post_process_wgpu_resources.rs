use super::*;

pub(super) struct SkinPreviewPostProcessWgpuResources {
    pub(super) shader_modules: SkinPreviewPostProcessShaderModules,
    pub(super) uniforms: SkinPreviewPostProcessUniformResources,
    pub(super) source_textures: SkinPreviewPostProcessSourceTextures,
    pub(super) render_targets: SkinPreviewPostProcessRenderTargets,
    pub(super) vertex3d_runtime: SkinPreviewVertex3dRuntime,
    pub(super) cached_scene_plan: Option<std::sync::Arc<Vertex3dScenePlan>>,
    pub(super) cached_scene_plan_batch_count: usize,
    pub(super) cached_scene_plan_msaa_samples: u32,
    pub(super) msaa_resolver: vertex_3d::MsaaResolvePool,
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) scene_msaa_samples: u32,
    pub(super) present_msaa_samples: u32,
}

use super::*;

pub(super) struct SkinPreviewPostProcessWgpuResources {
    pub(super) shader_modules: SkinPreviewPostProcessShaderModules,
    pub(super) uniforms: SkinPreviewPostProcessUniformResources,
    pub(super) source_textures: SkinPreviewPostProcessSourceTextures,
    pub(super) render_targets: SkinPreviewPostProcessRenderTargets,
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) scene_msaa_samples: u32,
    pub(super) present_msaa_samples: u32,
}

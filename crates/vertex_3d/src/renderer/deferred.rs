//! Minimal deferred renderer scaffold built from compiled shaders, reflection, and frame graphs.

use std::collections::BTreeMap;

use super::{
    AttachmentPool, FrameGraph, FrameGraphPlan, ReflectionBindGroupSet, RendererConfig,
    ShaderResourceTable, SurfaceConfig,
};
use crate::shader::{
    BuiltPipelineLayout, CompiledShaderProgram, PipelineLayoutPlan, ShaderCompileError, ShaderKind,
};

/// Deferred pass template naming which compiled stages a pass needs.
#[derive(Debug, Clone)]
pub struct DeferredRenderPipelineTemplate {
    pub pass_name: String,
    pub vertex_stage: Option<ShaderKind>,
    pub fragment_stage: Option<ShaderKind>,
}

impl DeferredRenderPipelineTemplate {
    pub fn fullscreen(pass_name: impl Into<String>) -> Self {
        Self {
            pass_name: pass_name.into(),
            vertex_stage: Some(ShaderKind::Vertex),
            fragment_stage: Some(ShaderKind::Fragment),
        }
    }
}

/// Runtime state for one deferred pass. This owns shader modules and shares the pipeline layout.
#[derive(Debug)]
pub struct DeferredPassRuntime {
    pub template: DeferredRenderPipelineTemplate,
    pub vertex_module: Option<wgpu::ShaderModule>,
    pub fragment_module: Option<wgpu::ShaderModule>,
}

/// Device-backed deferred renderer scaffold.
#[derive(Debug)]
pub struct DeferredRenderer {
    pub surface: SurfaceConfig,
    pub config: RendererConfig,
    pub frame_graph: FrameGraphPlan,
    pub pipeline_layout: BuiltPipelineLayout,
    pub attachments: AttachmentPool,
    pub passes: Vec<DeferredPassRuntime>,
}

impl DeferredRenderer {
    pub fn from_compiled_program(
        device: &wgpu::Device,
        surface: SurfaceConfig,
        compiled: &CompiledShaderProgram,
        frame_graph: FrameGraph,
        pass_templates: Vec<DeferredRenderPipelineTemplate>,
    ) -> Result<Self, DeferredRendererError> {
        let pipeline_plan = PipelineLayoutPlan::from_reflection(&compiled.reflection)?;
        let pipeline_layout =
            pipeline_plan.create_wgpu_layout(device, Some(&compiled.program.name))?;
        let shader_modules = compiled.create_shader_modules(device)?;
        let config = RendererConfig::for_reflection(surface.clone(), &compiled.reflection);
        let frame_graph = frame_graph.plan();
        let mut attachments = AttachmentPool::default();
        attachments.rebuild(device, &config);

        let passes = pass_templates
            .into_iter()
            .map(|template| {
                let vertex_module = template
                    .vertex_stage
                    .and_then(|kind| shader_modules.get(&kind))
                    .cloned();
                let fragment_module = template
                    .fragment_stage
                    .and_then(|kind| shader_modules.get(&kind))
                    .cloned();
                DeferredPassRuntime {
                    template,
                    vertex_module,
                    fragment_module,
                }
            })
            .collect();

        Ok(Self {
            surface,
            config,
            frame_graph,
            pipeline_layout,
            attachments,
            passes,
        })
    }

    pub fn shader_modules_by_stage(&self) -> BTreeMap<&str, (bool, bool)> {
        self.passes
            .iter()
            .map(|pass| {
                (
                    pass.template.pass_name.as_str(),
                    (pass.vertex_module.is_some(), pass.fragment_module.is_some()),
                )
            })
            .collect()
    }

    pub fn build_bind_groups(
        &self,
        device: &wgpu::Device,
        compiled: &CompiledShaderProgram,
        resources: &ShaderResourceTable<'_>,
    ) -> Result<ReflectionBindGroupSet, DeferredRendererError> {
        let layout_plan = PipelineLayoutPlan::from_reflection(&compiled.reflection)?;
        Ok(ReflectionBindGroupSet::build(
            device,
            &compiled.reflection,
            &layout_plan,
            &self.pipeline_layout,
            resources,
        )?)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DeferredRendererError {
    #[error(transparent)]
    PipelineLayout(#[from] crate::shader::PipelineLayoutPlanError),

    #[error(transparent)]
    Compile(#[from] ShaderCompileError),

    #[error(transparent)]
    BindGroups(#[from] super::resources::BindGroupBuildError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::{
        ReflectedRenderTarget, ReflectedResource, ReflectedResourceType, ReflectedStage,
        ReflectionPassthroughCompiler, ShaderBackendTarget, ShaderCompileRequest, ShaderCompiler,
        ShaderSourceLanguage, StageSource,
    };

    #[test]
    fn deferred_renderer_uses_reflection_for_graph_and_layout() {
        let compiler = ReflectionPassthroughCompiler::new(crate::shader::ReflectionSnapshot {
            stages: vec![
                ReflectedStage::new(ShaderKind::Vertex, "vs_main"),
                ReflectedStage::new(ShaderKind::Fragment, "fs_main"),
            ],
            resources: vec![ReflectedResource::new(
                "camera",
                0,
                ReflectedResourceType::UniformBuffer,
            )],
            render_targets: vec![ReflectedRenderTarget::new("g_albedo")],
        });
        let compiled = compiler
            .compile_program(
                &ShaderCompileRequest::new(
                    "deferred",
                    ShaderSourceLanguage::Slang,
                    ShaderBackendTarget::Wgsl,
                )
                .with_stage(StageSource::inline(ShaderKind::Vertex, "@vertex"))
                .with_stage(StageSource::inline(
                    ShaderKind::Fragment,
                    "@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
                )),
            )
            .expect("compile");

        let graph = FrameGraph::from_reflection(&compiled.reflection);
        assert_eq!(graph.plan().attachments.len(), 1);
    }
}

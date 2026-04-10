//! # Vertex 3D Framework
//!
//! A reusable 3D rendering framework providing:
//! - Comprehensive shader system with all 17+ stages
//! - HDR rendering with FP16/FP32 precision
//! - Deferred rendering GBuffer support
//! - Mesh geometry and vertex buffers
//! - Configurable post-processing pipeline
//! - Flexible uniform system
//! - Camera and projection utilities
//! - Math types (Vec2, Vec3, Mat4) from glam

pub mod camera;
pub mod mesh;
pub mod renderer;
pub mod shader;

// Re-export glam for convenience
pub use glam::{Mat4, Quat, Vec2, Vec3};

// Re-export main types for convenience
pub use camera::Camera;
pub use mesh::{Mesh, Vertex};
pub use renderer::{
    AdapterPreference, AttachmentLifecycle, AttachmentPool, AttachmentTexture, BindGroupBuildError,
    DeferredPassRuntime, DeferredRenderPipelineTemplate, DeferredRenderer, DeferredRendererError,
    DerivedRendererState, FrameGraph, FrameGraphAttachmentPlan, FrameGraphPass, FrameGraphPlan,
    FrameGraphUsage, GraphAttachment, NamedBindGroup, ReflectionBindGroupSet, RenderTargetHandle,
    RenderTargetScale, RendererConfig, RendererRebuildFlags, RendererRuntime,
    ShaderBindingResource, ShaderGraphDescriptor, ShaderResourceTable, SurfaceConfig,
};

// Re-export shader system types
pub use shader::{
    BindGroupLayoutPlan, BindingTypePlan, BufferPrecision, BuiltPipelineLayout, Colorspace,
    CompiledShaderProgram, CompiledShaderStage, GBufferType, HdrConfig, PipelineFlags,
    PipelineLayoutPlan, PipelineLayoutPlanError, PipelineResourceBindingPlan,
    ReflectedRenderTarget, ReflectedResource, ReflectedResourceType, ReflectedStage,
    ReflectedTextureDimension, ReflectionPassthroughCompiler, ReflectionSnapshot,
    RenderTargetConfig, RenderTargetType, ResourceBinding, ResourceType, ShaderBackendTarget,
    ShaderCompileError, ShaderCompileRequest, ShaderCompileSource, ShaderCompiler, ShaderKind,
    ShaderProgram, ShaderSourceLanguage, ShaderStage, ShaderStageConfig, SlangCompiler,
    StageSource,
};

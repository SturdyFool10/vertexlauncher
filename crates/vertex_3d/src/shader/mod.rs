//! # Shader System Module
//!
//! A comprehensive shader system designed for modern rendering pipelines with support for:
//! - All 17+ shader stages (rasterization, compute, mesh shading, ray tracing)
//! - Multiple render targets and buffers (albedo, depth, normals, motion vectors)
//! - SLANG reflection integration (planned architecture)
//! - Runtime source compilation via `&str` or `String`
//! - HDR rendering with FP16/FP32 precision
//! - Deferred rendering GBuffer support

pub mod compiler;
pub mod config;
pub mod gbuffer;
pub mod hdr;
pub mod kind;
pub mod pipeline;
pub mod program;
pub mod reflection;
pub mod stage;
pub mod standard_library;

// ============================================================================
// Re-exports for convenient access
// ============================================================================

/// Shader kinds and stages.
pub use kind::ShaderKind;

/// Shader stage configuration types.
pub use config::{PipelineFlags, ResourceBinding, ResourceType, ShaderStageConfig};

/// Compiler abstraction and compiled shader products.
pub use compiler::{
    CompiledShaderProgram, CompiledShaderStage, ReflectionPassthroughCompiler, ShaderBackendTarget,
    ShaderCompileError, ShaderCompileRequest, ShaderCompileSource, ShaderCompiler,
    ShaderSourceLanguage, SlangCompiler, StageSource,
};

/// GBuffer and render target types.
pub use gbuffer::{GBufferType, RenderTargetConfig, RenderTargetType};

/// HDR configuration types.
pub use hdr::{BufferPrecision, Colorspace, HdrConfig};

/// Main shader types.
pub use pipeline::{
    BindGroupLayoutPlan, BindingTypePlan, BuiltPipelineLayout, PipelineLayoutPlan,
    PipelineLayoutPlanError, PipelineResourceBindingPlan,
};
pub use program::ShaderProgram;
pub use reflection::{
    ReflectedRenderTarget, ReflectedResource, ReflectedResourceRole, ReflectedResourceType,
    ReflectedStage, ReflectedTextureDimension, ReflectionSnapshot,
};
pub use stage::ShaderStage;
pub use standard_library::{
    StandardShaderImport, resolve_standard_import_path, standard_library_dir, standard_module_path,
};

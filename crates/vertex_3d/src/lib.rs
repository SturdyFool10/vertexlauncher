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

pub mod asset;
pub mod camera;
pub mod material;
pub mod mesh;
pub mod renderer;
pub mod scene;
pub mod shader;

// Re-export glam for convenience
pub use glam::{Mat4, Quat, Vec2, Vec3};

// Re-export main types for convenience
pub use asset::{
    AssetHandle, MeshAsset, MeshHandle, RenderAssetLibrary, ShaderAsset, ShaderAssetBuildError,
    ShaderAssetDesc, ShaderHandle, TextureAsset, TextureHandle,
};
pub use camera::Camera;
pub use material::{
    AlphaMode, Material, MaterialHandle, MaterialModel, MaterialParameters, MaterialTextures,
    MaterialValue, PbrMaterial, UnlitMaterial,
};
pub use mesh::{Mesh, Vertex};
pub use renderer::{
    AdapterPreference, AdapterSelector, AttachmentLifecycle, AttachmentPool, AttachmentTexture,
    AvailableAdapter, BindGroupBuildError, DeferredPassRuntime, DeferredRenderPipelineTemplate,
    DeferredRenderer, DeferredRendererError, DerivedRendererState, FrameGraph,
    FrameGraphAttachmentPlan, FrameGraphPass, FrameGraphPlan, FrameGraphUsage, GraphAttachment,
    MsaaResolvePool, NamedBindGroup, ReflectionBindGroupSet, RenderTargetHandle, RenderTargetScale,
    RendererConfig, RendererRebuildFlags, RendererRuntime, ScenePipelineConfig, SceneRenderer,
    SceneRendererError, SceneSubmissionQueue, ShaderBindingResource, ShaderGraphDescriptor,
    ShaderResourceTable, SubmissionError, SurfaceConfig, describe_adapter_slice,
    enumerate_adapters, select_adapter_from_slice, select_adapter_slot,
};
pub use scene::{DrawPacket, RenderObject, Scene, Transform};

// Re-export shader system types
pub use shader::{
    BindGroupLayoutPlan, BindingTypePlan, BufferPrecision, BuiltPipelineLayout, Colorspace,
    CompiledShaderProgram, CompiledShaderStage, GBufferType, HdrConfig, PipelineFlags,
    PipelineLayoutPlan, PipelineLayoutPlanError, PipelineResourceBindingPlan,
    ReflectedRenderTarget, ReflectedResource, ReflectedResourceRole, ReflectedResourceType,
    ReflectedStage, ReflectedTextureDimension, ReflectionPassthroughCompiler, ReflectionSnapshot,
    RenderTargetConfig, RenderTargetType, ResourceBinding, ResourceType, ShaderBackendTarget,
    ShaderCompileError, ShaderCompileRequest, ShaderCompileSource, ShaderCompiler, ShaderKind,
    ShaderProgram, ShaderSourceLanguage, ShaderStage, ShaderStageConfig, SlangCompiler,
    StageSource, StandardShaderImport, resolve_standard_import_path, standard_library_dir,
    standard_module_path,
};

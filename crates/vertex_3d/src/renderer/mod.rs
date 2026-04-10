//! Renderer runtime state for deferred pipelines.
//!
//! This module separates user intent from derived GPU attachment state so
//! resizing, format changes, adapter swaps, and shader-graph updates all flow
//! through one rebuild path.

pub mod adapter_selection;
pub mod config;
pub mod deferred;
pub mod frame_graph;
pub mod msaa_resolve;
pub mod resources;
pub mod scene_renderer;
pub mod submission;

pub use adapter_selection::{
    AdapterSelector, AvailableAdapter, describe_adapter_slice, enumerate_adapters,
    select_adapter_from_slice, select_adapter_slot,
};
pub use config::{
    AdapterPreference, AttachmentLifecycle, DerivedRendererState, GraphAttachment,
    RenderTargetHandle, RenderTargetScale, RendererConfig, RendererRebuildFlags, RendererRuntime,
    ShaderGraphDescriptor, SurfaceConfig,
};
pub use deferred::{
    DeferredPassRuntime, DeferredRenderPipelineTemplate, DeferredRenderer, DeferredRendererError,
};
pub use frame_graph::{
    FrameGraph, FrameGraphAttachmentPlan, FrameGraphPass, FrameGraphPlan, FrameGraphUsage,
};
pub use msaa_resolve::MsaaResolvePool;
pub use resources::{
    AttachmentPool, AttachmentTexture, BindGroupBuildError, NamedBindGroup, ReflectionBindGroupSet,
    ShaderBindingResource, ShaderResourceTable,
};
pub use scene_renderer::{
    ExternalShaderBindGroup, ScenePipelineConfig, SceneRenderer, SceneRendererError,
};
pub use submission::{
    DrawBatch, FrameUploadArena, GpuInstanceData, GpuMesh, GpuResourceRegistry, GpuTexture,
    QueuedSceneSubmission, SceneSubmissionQueue, SubmissionError, UploadAllocation,
};

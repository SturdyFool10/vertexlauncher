//! Renderer runtime state for deferred pipelines.
//!
//! This module separates user intent from derived GPU attachment state so
//! resizing, format changes, adapter swaps, and shader-graph updates all flow
//! through one rebuild path.

pub mod config;
pub mod deferred;
pub mod frame_graph;
pub mod resources;

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
pub use resources::{
    AttachmentPool, AttachmentTexture, BindGroupBuildError, NamedBindGroup, ReflectionBindGroupSet,
    ShaderBindingResource, ShaderResourceTable,
};

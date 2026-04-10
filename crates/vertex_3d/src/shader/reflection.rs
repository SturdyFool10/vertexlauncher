//! Normalized shader reflection data derived from Slang or other frontends.
//!
//! Slang reflection is available through its compilation API rather than `slangc`
//! directly, so this module provides an engine-facing snapshot format that any
//! compiler integration can fill.

use serde::{Deserialize, Serialize};

use super::{
    PipelineFlags, RenderTargetConfig, RenderTargetType, ResourceBinding, ResourceType, ShaderKind,
};
use crate::renderer::{AttachmentLifecycle, GraphAttachment, RenderTargetHandle, RenderTargetScale};

/// Serializable snapshot of a compiled shader program's reflection data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReflectionSnapshot {
    pub stages: Vec<ReflectedStage>,
    pub resources: Vec<ReflectedResource>,
    pub render_targets: Vec<ReflectedRenderTarget>,
}

impl ReflectionSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_json_str(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    pub fn to_json_string_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn stage(&self, kind: ShaderKind) -> Option<&ReflectedStage> {
        self.stages.iter().find(|stage| stage.kind == kind)
    }

    pub fn resources_for_stage(
        &self,
        kind: ShaderKind,
    ) -> impl Iterator<Item = &ReflectedResource> + '_ {
        self.resources.iter().filter(move |resource| {
            resource.stages.is_empty() || resource.stages.contains(&kind)
        })
    }

    pub fn inferred_render_targets(
        &self,
        fallback_size: (u32, u32),
    ) -> Vec<RenderTargetConfig> {
        self.render_targets
            .iter()
            .map(|target| target.to_render_target_config(fallback_size))
            .collect()
    }

    pub fn inferred_graph_attachments(
        &self,
        default_scale: RenderTargetScale,
        fallback_size: (u32, u32),
    ) -> Vec<GraphAttachment> {
        self.render_targets
            .iter()
            .map(|target| target.to_graph_attachment(default_scale, fallback_size))
            .collect()
    }
}

/// Entry-point level reflection for a single stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectedStage {
    pub kind: ShaderKind,
    pub entry_point: String,
    #[serde(default)]
    pub writes_gbuffer: bool,
    #[serde(default = "default_pipeline_flags")]
    pub pipeline_flags: u32,
}

impl ReflectedStage {
    pub fn new(kind: ShaderKind, entry_point: impl Into<String>) -> Self {
        Self {
            kind,
            entry_point: entry_point.into(),
            writes_gbuffer: false,
            pipeline_flags: default_pipeline_flags(),
        }
    }

    pub fn flags(&self) -> PipelineFlags {
        PipelineFlags::from_bits(self.pipeline_flags)
    }
}

/// Reflected shader-visible resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectedResource {
    pub name: String,
    pub slot: u32,
    #[serde(default)]
    pub space: u32,
    #[serde(rename = "type")]
    pub resource_type: ReflectedResourceType,
    #[serde(default)]
    pub stages: Vec<ShaderKind>,
    #[serde(default)]
    pub texture_dimension: Option<ReflectedTextureDimension>,
}

impl ReflectedResource {
    pub fn new(
        name: impl Into<String>,
        slot: u32,
        resource_type: ReflectedResourceType,
    ) -> Self {
        Self {
            name: name.into(),
            slot,
            space: 0,
            resource_type,
            stages: Vec::new(),
            texture_dimension: None,
        }
    }

    pub fn to_resource_binding(&self) -> ResourceBinding {
        ResourceBinding::new(
            self.name.clone(),
            self.slot,
            self.resource_type.to_resource_type(),
        )
        .with_space(self.space)
    }
}

/// Normalized resource categories derived from shader reflection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectedResourceType {
    UniformBuffer,
    StorageBuffer,
    Texture,
    Sampler,
    CombinedTextureSampler,
}

impl ReflectedResourceType {
    pub fn to_resource_type(self) -> ResourceType {
        match self {
            ReflectedResourceType::UniformBuffer => ResourceType::UniformBuffer,
            ReflectedResourceType::StorageBuffer => ResourceType::StorageBuffer,
            ReflectedResourceType::Texture => ResourceType::Texture,
            ReflectedResourceType::Sampler => ResourceType::Sampler,
            ReflectedResourceType::CombinedTextureSampler => ResourceType::CombinedTextureSampler,
        }
    }
}

/// Texture dimensionality hints from reflection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectedTextureDimension {
    D1,
    D2,
    D2Array,
    Cube,
    D3,
}

/// Render target / output attachment reflected from the shader graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectedRenderTarget {
    pub handle: String,
    #[serde(default)]
    pub target_type: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub mip_levels: Option<u32>,
    #[serde(default)]
    pub samples: Option<u32>,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub lifecycle: Option<AttachmentLifecycle>,
}

impl ReflectedRenderTarget {
    pub fn new(handle: impl Into<String>) -> Self {
        Self {
            handle: handle.into(),
            target_type: None,
            width: None,
            height: None,
            mip_levels: None,
            samples: None,
            scale: None,
            lifecycle: None,
        }
    }

    pub fn to_render_target_config(&self, fallback_size: (u32, u32)) -> RenderTargetConfig {
        let target_type = self
            .target_type
            .as_deref()
            .map(RenderTargetType::from_reflection_name)
            .unwrap_or_else(|| RenderTargetType::from_reflection_name(self.handle.as_str()));

        let mut config = RenderTargetConfig::new(
            target_type,
            self.width.unwrap_or(fallback_size.0).max(1),
            self.height.unwrap_or(fallback_size.1).max(1),
        );
        if let Some(levels) = self.mip_levels {
            config = config.with_mip_levels(levels.max(1));
        }
        if let Some(samples) = self.samples {
            config = config.with_samples(samples.max(1));
        }
        config
    }

    pub fn to_graph_attachment(
        &self,
        default_scale: RenderTargetScale,
        fallback_size: (u32, u32),
    ) -> GraphAttachment {
        let target = self.to_render_target_config(fallback_size);
        let scale = self
            .scale
            .map(RenderTargetScale::Dynamic)
            .unwrap_or(default_scale);
        GraphAttachment::new(RenderTargetHandle::new(self.handle.clone()), target)
            .with_scale(scale)
            .with_lifecycle(self.lifecycle.unwrap_or(AttachmentLifecycle::Persistent))
    }
}

fn default_pipeline_flags() -> u32 {
    PipelineFlags::default().bits()
}

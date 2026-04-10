//! Normalized shader reflection data derived from Slang or other frontends.
//!
//! Slang reflection is available through its compilation API rather than `slangc`
//! directly, so this module provides an engine-facing snapshot format that any
//! compiler integration can fill.

use serde::{Deserialize, Serialize};

use super::{
    PipelineFlags, RenderTargetConfig, RenderTargetType, ResourceBinding, ResourceType, ShaderKind,
};
use crate::renderer::{
    AttachmentLifecycle, GraphAttachment, RenderTargetHandle, RenderTargetScale,
};

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

    pub fn from_slang_source(source: &str) -> Self {
        let stages = parse_stages(source);
        let stage_bodies = parse_stage_bodies(source, &stages);
        let mut resources = parse_resources(source);
        for resource in &mut resources {
            resource.stages = stage_bodies
                .iter()
                .filter_map(|(kind, body)| body.contains(resource.name.as_str()).then_some(*kind))
                .collect();
        }
        Self {
            stages,
            resources,
            render_targets: parse_target_annotations(source),
        }
    }

    pub fn stage(&self, kind: ShaderKind) -> Option<&ReflectedStage> {
        self.stages.iter().find(|stage| stage.kind == kind)
    }

    pub fn resources_for_stage(
        &self,
        kind: ShaderKind,
    ) -> impl Iterator<Item = &ReflectedResource> + '_ {
        self.resources
            .iter()
            .filter(move |resource| resource.stages.is_empty() || resource.stages.contains(&kind))
    }

    pub fn inferred_render_targets(&self, fallback_size: (u32, u32)) -> Vec<RenderTargetConfig> {
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
    pub fn new(name: impl Into<String>, slot: u32, resource_type: ReflectedResourceType) -> Self {
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

fn parse_stages(source: &str) -> Vec<ReflectedStage> {
    let mut stages = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    for index in 0..lines.len() {
        let line = lines[index].trim();
        let Some(kind) = parse_shader_attribute(line) else {
            continue;
        };
        for candidate in lines.iter().skip(index + 1) {
            let candidate = candidate.trim();
            if candidate.is_empty() || candidate.starts_with("//") {
                continue;
            }
            if let Some(entry_point) = parse_function_name(candidate) {
                stages.push(ReflectedStage::new(kind, entry_point));
                break;
            }
        }
    }
    stages
}

fn parse_stage_bodies(source: &str, stages: &[ReflectedStage]) -> Vec<(ShaderKind, String)> {
    stages
        .iter()
        .filter_map(|stage| {
            let function_marker = format!("{}(", stage.entry_point);
            let start = source.find(&function_marker)?;
            let brace_start = source[start..].find('{')? + start;
            let brace_end = find_matching_brace(source, brace_start)?;
            Some((stage.kind, source[brace_start..=brace_end].to_string()))
        })
        .collect()
}

fn parse_resources(source: &str) -> Vec<ReflectedResource> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("[[vk::binding(") {
                return None;
            }
            let binding_end = line.find(")]]")?;
            let binding_spec = &line["[[vk::binding(".len()..binding_end];
            let mut binding_parts = binding_spec.split(',').map(str::trim);
            let slot = binding_parts.next()?.parse().ok()?;
            let space = binding_parts.next()?.parse().ok()?;

            let declaration = line[binding_end + 3..].trim();
            let declaration = declaration.strip_suffix(';').unwrap_or(declaration);
            let mut tokens = declaration.split_whitespace();
            let type_token = tokens.next()?;
            let name = tokens.next()?.trim().to_string();
            let (resource_type, texture_dimension) = classify_resource_type(type_token);

            Some(ReflectedResource {
                name,
                slot,
                space,
                resource_type,
                stages: Vec::new(),
                texture_dimension,
            })
        })
        .collect()
}

fn parse_target_annotations(source: &str) -> Vec<ReflectedRenderTarget> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let annotation = line.strip_prefix("// @vertex3d.target ")?;
            let mut target = ReflectedRenderTarget::new("");
            for pair in annotation.split_whitespace() {
                let Some((key, value)) = pair.split_once('=') else {
                    continue;
                };
                match key {
                    "handle" => target.handle = value.to_string(),
                    "type" => target.target_type = Some(value.to_string()),
                    "width" => target.width = value.parse().ok(),
                    "height" => target.height = value.parse().ok(),
                    "mips" => target.mip_levels = value.parse().ok(),
                    "samples" => target.samples = value.parse().ok(),
                    "scale" => target.scale = value.parse().ok(),
                    "lifecycle" => {
                        target.lifecycle = match value {
                            "persistent" => Some(AttachmentLifecycle::Persistent),
                            "transient" => Some(AttachmentLifecycle::Transient),
                            "history" => Some(AttachmentLifecycle::History),
                            _ => None,
                        }
                    }
                    _ => {}
                }
            }
            (!target.handle.is_empty()).then_some(target)
        })
        .collect()
}

fn parse_shader_attribute(line: &str) -> Option<ShaderKind> {
    let value = line.strip_prefix("[shader(\"")?;
    let value = value.strip_suffix("\")]")?;
    match value {
        "vertex" => Some(ShaderKind::Vertex),
        "fragment" => Some(ShaderKind::Fragment),
        "compute" => Some(ShaderKind::Compute),
        _ => None,
    }
}

fn parse_function_name(line: &str) -> Option<String> {
    let open = line.find('(')?;
    let prefix = &line[..open];
    prefix.split_whitespace().last().map(str::to_string)
}

fn find_matching_brace(source: &str, brace_start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in source[brace_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(brace_start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn classify_resource_type(
    type_token: &str,
) -> (ReflectedResourceType, Option<ReflectedTextureDimension>) {
    if type_token.starts_with("ConstantBuffer<") {
        (ReflectedResourceType::UniformBuffer, None)
    } else if type_token.starts_with("StructuredBuffer<")
        || type_token.starts_with("RWStructuredBuffer<")
    {
        (ReflectedResourceType::StorageBuffer, None)
    } else if type_token.starts_with("SamplerState") {
        (ReflectedResourceType::Sampler, None)
    } else if type_token.starts_with("Texture1D") {
        (
            ReflectedResourceType::Texture,
            Some(ReflectedTextureDimension::D1),
        )
    } else if type_token.starts_with("Texture2DArray") {
        (
            ReflectedResourceType::Texture,
            Some(ReflectedTextureDimension::D2Array),
        )
    } else if type_token.starts_with("Texture2D") {
        (
            ReflectedResourceType::Texture,
            Some(ReflectedTextureDimension::D2),
        )
    } else if type_token.starts_with("TextureCube") {
        (
            ReflectedResourceType::Texture,
            Some(ReflectedTextureDimension::Cube),
        )
    } else if type_token.starts_with("Texture3D") {
        (
            ReflectedResourceType::Texture,
            Some(ReflectedTextureDimension::D3),
        )
    } else {
        (ReflectedResourceType::Texture, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slang_source_in_memory() {
        let source = r#"
// @vertex3d.target handle=accumulation type=lighting lifecycle=transient
[[vk::binding(0, 0)]] Texture2D<float4> source_tex;
[[vk::binding(1, 0)]] SamplerState source_sampler;
[shader("vertex")]
FullscreenOut vs_main(uint vertex_index : SV_VertexID) { return (FullscreenOut)0; }
[shader("fragment")]
float4 fs_main(float4 pos : SV_Position) : SV_Target { return source_tex.Load(int3(0,0,0)); }
"#;
        let snapshot = ReflectionSnapshot::from_slang_source(source);
        assert_eq!(snapshot.stages.len(), 2);
        assert_eq!(snapshot.resources.len(), 2);
        assert_eq!(snapshot.render_targets.len(), 1);
        assert_eq!(snapshot.render_targets[0].handle, "accumulation");
    }
}

//! Pass graph planning for deferred rendering.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AttachmentLifecycle, GraphAttachment, RenderTargetHandle, RenderTargetScale,
    ShaderGraphDescriptor,
};
use crate::shader::{ReflectionSnapshot, RenderTargetConfig, RenderTargetType};

/// Generic graph resource identifier for images and buffers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphResourceHandle(pub String);

impl GraphResourceHandle {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for GraphResourceHandle {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for GraphResourceHandle {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Coarse pass classification for scheduling and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameGraphPassKind {
    #[default]
    Raster,
    Compute,
    Copy,
    Present,
}

/// Resource category visible to the graph scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphResourceKind {
    Image,
    Buffer,
}

/// Intended GPU usage for a graph resource within a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphResourceUsage {
    SampledRead,
    StorageRead,
    StorageWrite,
    UniformRead,
    VertexRead,
    IndexRead,
    IndirectRead,
    RenderAttachmentWrite,
    DepthAttachmentWrite,
    CopySrc,
    CopyDst,
    Present,
}

impl GraphResourceUsage {
    pub fn is_write(self) -> bool {
        matches!(
            self,
            Self::StorageWrite
                | Self::RenderAttachmentWrite
                | Self::DepthAttachmentWrite
                | Self::CopyDst
                | Self::Present
        )
    }
}

/// One image/buffer access declared by a pass.
#[derive(Debug, Clone)]
pub struct FrameGraphResourceAccess {
    pub handle: GraphResourceHandle,
    pub kind: GraphResourceKind,
    pub usage: GraphResourceUsage,
}

impl FrameGraphResourceAccess {
    pub fn image(handle: impl Into<GraphResourceHandle>, usage: GraphResourceUsage) -> Self {
        Self {
            handle: handle.into(),
            kind: GraphResourceKind::Image,
            usage,
        }
    }

    pub fn buffer(handle: impl Into<GraphResourceHandle>, usage: GraphResourceUsage) -> Self {
        Self {
            handle: handle.into(),
            kind: GraphResourceKind::Buffer,
            usage,
        }
    }
}

/// A renderer pass with explicit attachment reads and writes.
#[derive(Debug, Clone)]
pub struct FrameGraphPass {
    pub name: String,
    pub kind: FrameGraphPassKind,
    pub reads: BTreeSet<RenderTargetHandle>,
    pub writes: Vec<FrameGraphUsage>,
    pub accesses: Vec<FrameGraphResourceAccess>,
}

impl FrameGraphPass {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: FrameGraphPassKind::Raster,
            reads: BTreeSet::new(),
            writes: Vec::new(),
            accesses: Vec::new(),
        }
    }

    pub fn with_kind(mut self, kind: FrameGraphPassKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn reads(mut self, handle: impl Into<RenderTargetHandle>) -> Self {
        self.reads.insert(handle.into());
        self
    }

    pub fn writes(mut self, usage: FrameGraphUsage) -> Self {
        self.writes.push(usage);
        self
    }

    pub fn read_image(
        mut self,
        handle: impl Into<GraphResourceHandle>,
        usage: GraphResourceUsage,
    ) -> Self {
        self.accesses
            .push(FrameGraphResourceAccess::image(handle, usage));
        self
    }

    pub fn sample_image(self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.read_image(handle, GraphResourceUsage::SampledRead)
    }

    pub fn write_image(
        mut self,
        handle: impl Into<GraphResourceHandle>,
        usage: GraphResourceUsage,
    ) -> Self {
        self.accesses
            .push(FrameGraphResourceAccess::image(handle, usage));
        self
    }

    pub fn read_buffer(
        mut self,
        handle: impl Into<GraphResourceHandle>,
        usage: GraphResourceUsage,
    ) -> Self {
        self.accesses
            .push(FrameGraphResourceAccess::buffer(handle, usage));
        self
    }

    pub fn write_buffer(
        mut self,
        handle: impl Into<GraphResourceHandle>,
        usage: GraphResourceUsage,
    ) -> Self {
        self.accesses
            .push(FrameGraphResourceAccess::buffer(handle, usage));
        self
    }
}

/// One declared write target in the frame graph.
#[derive(Debug, Clone)]
pub struct FrameGraphUsage {
    pub handle: RenderTargetHandle,
    pub target_type: RenderTargetType,
    pub scale: RenderTargetScale,
    pub lifecycle: AttachmentLifecycle,
}

impl FrameGraphUsage {
    pub fn new(handle: impl Into<RenderTargetHandle>, target_type: RenderTargetType) -> Self {
        Self {
            handle: handle.into(),
            target_type,
            scale: RenderTargetScale::Full,
            lifecycle: AttachmentLifecycle::Transient,
        }
    }

    pub fn with_scale(mut self, scale: RenderTargetScale) -> Self {
        self.scale = scale;
        self
    }

    pub fn with_lifecycle(mut self, lifecycle: AttachmentLifecycle) -> Self {
        self.lifecycle = lifecycle;
        self
    }
}

/// Ordered frame graph definition.
#[derive(Debug, Clone, Default)]
pub struct FrameGraph {
    pub passes: Vec<FrameGraphPass>,
}

impl FrameGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_pass(mut self, pass: FrameGraphPass) -> Self {
        self.passes.push(pass);
        self
    }

    pub fn plan(&self) -> FrameGraphPlan {
        let mut attachments: BTreeMap<RenderTargetHandle, FrameGraphAttachmentPlan> =
            BTreeMap::new();
        let mut resources: BTreeMap<GraphResourceHandle, FrameGraphResourcePlan> = BTreeMap::new();
        for (pass_index, pass) in self.passes.iter().enumerate() {
            for read in &pass.reads {
                attachments
                    .entry(read.clone())
                    .or_insert_with(|| FrameGraphAttachmentPlan::new(read.clone()))
                    .consumers
                    .push(pass_index);
            }
            for write in &pass.writes {
                let plan = attachments
                    .entry(write.handle.clone())
                    .or_insert_with(|| FrameGraphAttachmentPlan::new(write.handle.clone()));
                plan.target_type = Some(write.target_type.clone());
                plan.scale = write.scale;
                plan.lifecycle = write.lifecycle;
                plan.producers.push(pass_index);
            }
            for access in &pass.accesses {
                let plan = resources.entry(access.handle.clone()).or_insert_with(|| {
                    FrameGraphResourcePlan {
                        handle: access.handle.clone(),
                        kind: access.kind,
                        first_use: pass_index,
                        last_use: pass_index,
                        producers: Vec::new(),
                        consumers: Vec::new(),
                        usages: Vec::new(),
                    }
                });
                plan.kind = access.kind;
                plan.first_use = plan.first_use.min(pass_index);
                plan.last_use = plan.last_use.max(pass_index);
                if access.usage.is_write() {
                    plan.producers.push(pass_index);
                } else {
                    plan.consumers.push(pass_index);
                }
                plan.usages.push((pass_index, access.usage));
            }
        }
        FrameGraphPlan {
            passes: self.passes.clone(),
            attachments,
            resources,
        }
    }

    pub fn infer_shader_graph(&self, fallback_size: (u32, u32)) -> ShaderGraphDescriptor {
        let plan = self.plan();
        let mut graph = ShaderGraphDescriptor::new();
        for attachment in plan.attachments.values() {
            let target_type = attachment.target_type.clone().unwrap_or_else(|| {
                RenderTargetType::Custom(attachment.handle.as_str().to_string())
            });
            let target = RenderTargetConfig::new(target_type, fallback_size.0, fallback_size.1);
            graph = graph.with_attachment(
                GraphAttachment::new(attachment.handle.clone(), target)
                    .with_scale(attachment.scale)
                    .with_lifecycle(attachment.lifecycle),
            );
        }
        graph
    }

    pub fn from_reflection(reflection: &ReflectionSnapshot) -> Self {
        let mut pass = FrameGraphPass::new("reflected_outputs");
        for target in &reflection.render_targets {
            let target_type = target
                .target_type
                .as_deref()
                .map(RenderTargetType::from_reflection_name)
                .unwrap_or_else(|| RenderTargetType::from_reflection_name(target.handle.as_str()));
            let mut usage = FrameGraphUsage::new(target.handle.clone(), target_type);
            if let Some(scale) = target.scale {
                usage = usage.with_scale(RenderTargetScale::Dynamic(scale));
            }
            if let Some(lifecycle) = target.lifecycle {
                usage = usage.with_lifecycle(lifecycle);
            }
            pass = pass.writes(usage).write_image(
                target.handle.clone(),
                GraphResourceUsage::RenderAttachmentWrite,
            );
        }
        Self::new().with_pass(pass)
    }
}

/// Planned usage summary for one attachment across all passes.
#[derive(Debug, Clone)]
pub struct FrameGraphAttachmentPlan {
    pub handle: RenderTargetHandle,
    pub target_type: Option<RenderTargetType>,
    pub scale: RenderTargetScale,
    pub lifecycle: AttachmentLifecycle,
    pub producers: Vec<usize>,
    pub consumers: Vec<usize>,
}

impl FrameGraphAttachmentPlan {
    fn new(handle: RenderTargetHandle) -> Self {
        Self {
            handle,
            target_type: None,
            scale: RenderTargetScale::Full,
            lifecycle: AttachmentLifecycle::Transient,
            producers: Vec::new(),
            consumers: Vec::new(),
        }
    }
}

/// Planned pass execution order and attachment usage summary.
#[derive(Debug, Clone)]
pub struct FrameGraphPlan {
    pub passes: Vec<FrameGraphPass>,
    pub attachments: BTreeMap<RenderTargetHandle, FrameGraphAttachmentPlan>,
    pub resources: BTreeMap<GraphResourceHandle, FrameGraphResourcePlan>,
}

/// Planned access summary for one image or buffer across all passes.
#[derive(Debug, Clone)]
pub struct FrameGraphResourcePlan {
    pub handle: GraphResourceHandle,
    pub kind: GraphResourceKind,
    pub first_use: usize,
    pub last_use: usize,
    pub producers: Vec<usize>,
    pub consumers: Vec<usize>,
    pub usages: Vec<(usize, GraphResourceUsage)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_graph_infers_attachment_dependencies() {
        let graph = FrameGraph::new()
            .with_pass(
                FrameGraphPass::new("gbuffer")
                    .writes(FrameGraphUsage::new("g_albedo", RenderTargetType::Albedo))
                    .writes(FrameGraphUsage::new("g_depth", RenderTargetType::Depth)),
            )
            .with_pass(
                FrameGraphPass::new("lighting")
                    .reads("g_albedo")
                    .reads("g_depth")
                    .writes(FrameGraphUsage::new("lighting", RenderTargetType::Lighting)),
            );

        let plan = graph.plan();
        let albedo = plan
            .attachments
            .get(&RenderTargetHandle::new("g_albedo"))
            .expect("albedo");
        assert_eq!(albedo.producers, vec![0]);
        assert_eq!(albedo.consumers, vec![1]);
    }

    #[test]
    fn pass_graph_tracks_generic_image_and_buffer_dependencies() {
        let graph = FrameGraph::new()
            .with_pass(
                FrameGraphPass::new("cull")
                    .with_kind(FrameGraphPassKind::Compute)
                    .read_buffer("instance_input", GraphResourceUsage::StorageRead)
                    .write_buffer("visible_instances", GraphResourceUsage::StorageWrite),
            )
            .with_pass(
                FrameGraphPass::new("shade")
                    .with_kind(FrameGraphPassKind::Raster)
                    .read_buffer("visible_instances", GraphResourceUsage::VertexRead)
                    .sample_image("scene_color")
                    .write_image(
                        "lighting_history",
                        GraphResourceUsage::RenderAttachmentWrite,
                    ),
            )
            .with_pass(
                FrameGraphPass::new("present")
                    .with_kind(FrameGraphPassKind::Present)
                    .sample_image("lighting_history")
                    .write_image("swapchain", GraphResourceUsage::Present),
            );

        let plan = graph.plan();
        let visible_instances = plan
            .resources
            .get(&GraphResourceHandle::new("visible_instances"))
            .expect("visible_instances");
        assert_eq!(visible_instances.kind, GraphResourceKind::Buffer);
        assert_eq!(visible_instances.producers, vec![0]);
        assert_eq!(visible_instances.consumers, vec![1]);

        let lighting_history = plan
            .resources
            .get(&GraphResourceHandle::new("lighting_history"))
            .expect("lighting_history");
        assert_eq!(lighting_history.kind, GraphResourceKind::Image);
        assert_eq!(lighting_history.producers, vec![1]);
        assert_eq!(lighting_history.consumers, vec![2]);
        assert_eq!(lighting_history.first_use, 1);
        assert_eq!(lighting_history.last_use, 2);
    }
}

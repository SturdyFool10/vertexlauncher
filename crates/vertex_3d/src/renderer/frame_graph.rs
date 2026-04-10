//! Pass graph planning for deferred rendering.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AttachmentLifecycle, GraphAttachment, RenderTargetHandle, RenderTargetScale,
    ShaderGraphDescriptor,
};
use crate::shader::{ReflectionSnapshot, RenderTargetConfig, RenderTargetType};

/// A renderer pass with explicit attachment reads and writes.
#[derive(Debug, Clone)]
pub struct FrameGraphPass {
    pub name: String,
    pub reads: BTreeSet<RenderTargetHandle>,
    pub writes: Vec<FrameGraphUsage>,
}

impl FrameGraphPass {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reads: BTreeSet::new(),
            writes: Vec::new(),
        }
    }

    pub fn reads(mut self, handle: impl Into<RenderTargetHandle>) -> Self {
        self.reads.insert(handle.into());
        self
    }

    pub fn writes(mut self, usage: FrameGraphUsage) -> Self {
        self.writes.push(usage);
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
        }
        FrameGraphPlan {
            passes: self.passes.clone(),
            attachments,
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
            pass = pass.writes(usage);
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
}

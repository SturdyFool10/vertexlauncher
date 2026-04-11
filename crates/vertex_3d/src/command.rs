//! Chainable GPU command recording that lowers into the frame graph.

use std::{borrow::Cow, collections::BTreeMap};

use crate::{
    FrameGraph, FrameGraphPass, FrameGraphPassKind, FrameGraphPlan, GpuImage,
    GraphResourceHandle, GraphResourceUsage, ImageDesc, RenderTargetType,
    renderer::FrameGraphUsage,
};

/// Ordered queue of GPU commands for a frame or offscreen workload.
#[derive(Debug, Clone, Default)]
pub struct CommandQueue {
    commands: Vec<GpuCommand>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(mut self, command: GpuCommand) -> Self {
        self.commands.push(command);
        self
    }

    pub fn raster(
        mut self,
        name: impl Into<String>,
        build: impl FnOnce(RasterCommand) -> RasterCommand,
    ) -> Self {
        self.commands
            .push(GpuCommand::Raster(build(RasterCommand::new(name))));
        self
    }

    pub fn compute(
        mut self,
        name: impl Into<String>,
        build: impl FnOnce(ComputeCommand) -> ComputeCommand,
    ) -> Self {
        self.commands
            .push(GpuCommand::Compute(build(ComputeCommand::new(name))));
        self
    }

    pub fn copy_image(
        mut self,
        name: impl Into<String>,
        source: impl Into<GraphResourceHandle>,
        target: impl Into<GraphResourceHandle>,
    ) -> Self {
        self.commands.push(GpuCommand::CopyImage(CopyImageCommand {
            name: name.into(),
            source: source.into(),
            target: target.into(),
        }));
        self
    }

    pub fn present(
        mut self,
        source: impl Into<GraphResourceHandle>,
        target: impl Into<GraphResourceHandle>,
    ) -> Self {
        self.commands.push(GpuCommand::Present(PresentCommand {
            source: source.into(),
            target: target.into(),
        }));
        self
    }

    pub fn commands(&self) -> &[GpuCommand] {
        &self.commands
    }

    pub fn into_commands(self) -> Vec<GpuCommand> {
        self.commands
    }

    pub fn frame_graph(&self) -> FrameGraph {
        let mut graph = FrameGraph::new();
        for command in &self.commands {
            graph = graph.with_pass(command.to_pass());
        }
        graph
    }

    pub fn plan(&self) -> FrameGraphPlan {
        self.frame_graph().plan()
    }

    pub fn encode<C: CommandExecutionCallbacks>(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        resources: &CommandImageRegistry,
        callbacks: &mut C,
    ) -> Result<(), CommandExecutionError> {
        for command in &self.commands {
            match command {
                GpuCommand::Raster(raster) => callbacks.encode_raster(encoder, resources, raster)?,
                GpuCommand::Compute(compute) => {
                    callbacks.encode_compute(encoder, resources, compute)?
                }
                GpuCommand::CopyImage(copy) => copy.encode(encoder, resources)?,
                GpuCommand::Present(present) => present.encode(encoder, resources)?,
            }
        }
        Ok(())
    }

    pub fn submit<C: CommandExecutionCallbacks>(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &CommandImageRegistry,
        callbacks: &mut C,
    ) -> Result<GpuWorkHandle, CommandExecutionError> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vertex3d-command-queue"),
        });
        self.encode(&mut encoder, resources, callbacks)?;
        let submission = queue.submit(Some(encoder.finish()));
        Ok(GpuWorkHandle { submission })
    }
}

/// Handle to an in-flight GPU submission.
///
/// The GPU executes submitted work asynchronously; you can simply drop this
/// handle and ignore it (fire-and-forget). Call [`GpuWorkHandle::wait`] only
/// when the CPU must block until the GPU finishes this specific submission in
/// the same frame.
pub struct GpuWorkHandle {
    submission: wgpu::SubmissionIndex,
}

impl GpuWorkHandle {
    /// Block the calling thread until the GPU has finished this submission.
    ///
    /// Prefer dropping the handle without calling this. Only use it when you
    /// need the GPU result available to the CPU before the frame advances.
    pub fn wait(self, device: &wgpu::Device) {
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(self.submission),
            timeout: None,
        });
    }
}

/// Named GPU image registry used by the command queue executor.
#[derive(Debug, Default)]
pub struct CommandImageRegistry {
    images: BTreeMap<GraphResourceHandle, CommandImageResource>,
}

impl CommandImageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_image(
        &mut self,
        device: &wgpu::Device,
        handle: impl Into<GraphResourceHandle>,
        label: impl Into<Cow<'static, str>>,
        desc: ImageDesc,
    ) -> &CommandImageResource {
        let handle = handle.into();
        let label = label.into();
        let image = GpuImage::from_desc(device, label.as_ref(), &desc);
        self.images
            .insert(handle.clone(), CommandImageResource { image, desc });
        self.images.get(&handle).expect("image inserted")
    }

    pub fn insert_image(
        &mut self,
        handle: impl Into<GraphResourceHandle>,
        image: GpuImage,
        desc: ImageDesc,
    ) {
        self.images
            .insert(handle.into(), CommandImageResource { image, desc });
    }

    pub fn import_texture(
        &mut self,
        device: &wgpu::Device,
        handle: impl Into<GraphResourceHandle>,
        texture: wgpu::Texture,
        desc: ImageDesc,
    ) {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vertex3d-command-imported-image-sampler"),
            ..Default::default()
        });
        self.insert_image(
            handle,
            GpuImage::new(texture, view, sampler, [desc.size[0], desc.size[1]], desc.format),
            desc,
        );
    }

    pub fn image(&self, handle: impl Into<GraphResourceHandle>) -> Option<&CommandImageResource> {
        self.images.get(&handle.into())
    }
}

/// A named GPU image plus the allocation description it was created/imported with.
#[derive(Debug)]
pub struct CommandImageResource {
    pub image: GpuImage,
    pub desc: ImageDesc,
}

/// Raster/compute encoding hooks used by the generic command executor.
pub trait CommandExecutionCallbacks {
    fn encode_raster(
        &mut self,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &CommandImageRegistry,
        command: &RasterCommand,
    ) -> Result<(), CommandExecutionError> {
        Err(CommandExecutionError::UnhandledRasterCommand {
            name: command.name.clone(),
        })
    }

    fn encode_compute(
        &mut self,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &CommandImageRegistry,
        command: &ComputeCommand,
    ) -> Result<(), CommandExecutionError> {
        Err(CommandExecutionError::UnhandledComputeCommand {
            name: command.name.clone(),
        })
    }
}

impl CommandExecutionCallbacks for () {}

#[derive(Debug, thiserror::Error)]
pub enum CommandExecutionError {
    #[error("command references missing image resource '{handle}'")]
    MissingImageResource { handle: String },

    #[error("image copy '{name}' requires matching 2D sizes, got {source_size:?} -> {target_size:?}")]
    IncompatibleImageSize {
        name: String,
        source_size: [u32; 3],
        target_size: [u32; 3],
    },

    #[error("image copy '{name}' requires matching formats, got {source_format:?} -> {target_format:?}")]
    IncompatibleImageFormat {
        name: String,
        source_format: wgpu::TextureFormat,
        target_format: wgpu::TextureFormat,
    },

    #[error("image copy '{name}' requires single-sampled images, got {source_samples} -> {target_samples}")]
    UnsupportedMultisampledCopy {
        name: String,
        source_samples: u32,
        target_samples: u32,
    },

    #[error("raster command '{name}' was recorded but no raster encoder handled it")]
    UnhandledRasterCommand { name: String },

    #[error("compute command '{name}' was recorded but no compute encoder handled it")]
    UnhandledComputeCommand { name: String },
}

/// One recorded high-level GPU operation.
#[derive(Debug, Clone)]
pub enum GpuCommand {
    Raster(RasterCommand),
    Compute(ComputeCommand),
    CopyImage(CopyImageCommand),
    Present(PresentCommand),
}

impl GpuCommand {
    pub fn name(&self) -> &str {
        match self {
            Self::Raster(command) => command.name.as_str(),
            Self::Compute(command) => command.name.as_str(),
            Self::CopyImage(command) => command.name.as_str(),
            Self::Present(_) => "present",
        }
    }

    fn to_pass(&self) -> FrameGraphPass {
        match self {
            Self::Raster(command) => command.to_pass(),
            Self::Compute(command) => command.to_pass(),
            Self::CopyImage(command) => command.to_pass(),
            Self::Present(command) => command.to_pass(),
        }
    }
}

/// Chainable raster command description.
#[derive(Debug, Clone, Default)]
pub struct RasterCommand {
    pub name: String,
    pub sampled_images: Vec<GraphResourceHandle>,
    pub storage_reads: Vec<GraphResourceHandle>,
    pub storage_writes: Vec<GraphResourceHandle>,
    pub vertex_buffers: Vec<GraphResourceHandle>,
    pub index_buffers: Vec<GraphResourceHandle>,
    pub uniform_buffers: Vec<GraphResourceHandle>,
    pub color_attachments: Vec<RasterAttachment>,
    pub depth_attachment: Option<GraphResourceHandle>,
}

impl RasterCommand {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn sample_image(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.sampled_images.push(handle.into());
        self
    }

    pub fn read_storage_image(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.storage_reads.push(handle.into());
        self
    }

    pub fn write_storage_image(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.storage_writes.push(handle.into());
        self
    }

    pub fn vertex_buffer(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.vertex_buffers.push(handle.into());
        self
    }

    pub fn index_buffer(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.index_buffers.push(handle.into());
        self
    }

    pub fn uniform_buffer(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.uniform_buffers.push(handle.into());
        self
    }

    pub fn color_attachment(
        mut self,
        handle: impl Into<GraphResourceHandle>,
        target_type: RenderTargetType,
    ) -> Self {
        self.color_attachments.push(RasterAttachment {
            handle: handle.into(),
            target_type,
        });
        self
    }

    pub fn depth_attachment(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.depth_attachment = Some(handle.into());
        self
    }

    fn to_pass(&self) -> FrameGraphPass {
        let mut pass = FrameGraphPass::new(self.name.clone()).with_kind(FrameGraphPassKind::Raster);

        for handle in &self.sampled_images {
            pass = pass.sample_image(handle.clone());
        }
        for handle in &self.storage_reads {
            pass = pass.read_image(handle.clone(), GraphResourceUsage::StorageRead);
        }
        for handle in &self.storage_writes {
            pass = pass.write_image(handle.clone(), GraphResourceUsage::StorageWrite);
        }
        for handle in &self.vertex_buffers {
            pass = pass.read_buffer(handle.clone(), GraphResourceUsage::VertexRead);
        }
        for handle in &self.index_buffers {
            pass = pass.read_buffer(handle.clone(), GraphResourceUsage::IndexRead);
        }
        for handle in &self.uniform_buffers {
            pass = pass.read_buffer(handle.clone(), GraphResourceUsage::UniformRead);
        }
        for attachment in &self.color_attachments {
            let target_name = attachment.handle.as_str();
            pass = pass
                .writes(FrameGraphUsage::new(
                    target_name,
                    attachment.target_type.clone(),
                ))
                .write_image(
                    attachment.handle.clone(),
                    GraphResourceUsage::RenderAttachmentWrite,
                );
        }
        if let Some(depth) = &self.depth_attachment {
            pass = pass
                .writes(FrameGraphUsage::new(depth.as_str(), RenderTargetType::Depth))
                .write_image(depth.clone(), GraphResourceUsage::DepthAttachmentWrite);
        }
        pass
    }
}

/// One raster output attachment.
#[derive(Debug, Clone)]
pub struct RasterAttachment {
    pub handle: GraphResourceHandle,
    pub target_type: RenderTargetType,
}

/// Chainable compute command description.
#[derive(Debug, Clone, Default)]
pub struct ComputeCommand {
    pub name: String,
    pub sampled_images: Vec<GraphResourceHandle>,
    pub storage_reads: Vec<GraphResourceHandle>,
    pub storage_writes: Vec<GraphResourceHandle>,
    pub uniform_buffers: Vec<GraphResourceHandle>,
    pub storage_buffers_read: Vec<GraphResourceHandle>,
    pub storage_buffers_write: Vec<GraphResourceHandle>,
    pub indirect_buffers: Vec<GraphResourceHandle>,
}

impl ComputeCommand {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn sample_image(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.sampled_images.push(handle.into());
        self
    }

    pub fn read_storage_image(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.storage_reads.push(handle.into());
        self
    }

    pub fn write_storage_image(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.storage_writes.push(handle.into());
        self
    }

    pub fn uniform_buffer(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.uniform_buffers.push(handle.into());
        self
    }

    pub fn read_storage_buffer(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.storage_buffers_read.push(handle.into());
        self
    }

    pub fn write_storage_buffer(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.storage_buffers_write.push(handle.into());
        self
    }

    pub fn indirect_buffer(mut self, handle: impl Into<GraphResourceHandle>) -> Self {
        self.indirect_buffers.push(handle.into());
        self
    }

    fn to_pass(&self) -> FrameGraphPass {
        let mut pass =
            FrameGraphPass::new(self.name.clone()).with_kind(FrameGraphPassKind::Compute);
        for handle in &self.sampled_images {
            pass = pass.sample_image(handle.clone());
        }
        for handle in &self.storage_reads {
            pass = pass.read_image(handle.clone(), GraphResourceUsage::StorageRead);
        }
        for handle in &self.storage_writes {
            pass = pass.write_image(handle.clone(), GraphResourceUsage::StorageWrite);
        }
        for handle in &self.uniform_buffers {
            pass = pass.read_buffer(handle.clone(), GraphResourceUsage::UniformRead);
        }
        for handle in &self.storage_buffers_read {
            pass = pass.read_buffer(handle.clone(), GraphResourceUsage::StorageRead);
        }
        for handle in &self.storage_buffers_write {
            pass = pass.write_buffer(handle.clone(), GraphResourceUsage::StorageWrite);
        }
        for handle in &self.indirect_buffers {
            pass = pass.read_buffer(handle.clone(), GraphResourceUsage::IndirectRead);
        }
        pass
    }
}

/// Image copy/blit style command.
#[derive(Debug, Clone)]
pub struct CopyImageCommand {
    pub name: String,
    pub source: GraphResourceHandle,
    pub target: GraphResourceHandle,
}

impl CopyImageCommand {
    fn to_pass(&self) -> FrameGraphPass {
        FrameGraphPass::new(self.name.clone())
            .with_kind(FrameGraphPassKind::Copy)
            .read_image(self.source.clone(), GraphResourceUsage::CopySrc)
            .write_image(self.target.clone(), GraphResourceUsage::CopyDst)
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        resources: &CommandImageRegistry,
    ) -> Result<(), CommandExecutionError> {
        encode_image_copy(encoder, resources, &self.name, &self.source, &self.target)
    }
}

/// Presentation operation from an image to a presentation target.
#[derive(Debug, Clone)]
pub struct PresentCommand {
    pub source: GraphResourceHandle,
    pub target: GraphResourceHandle,
}

impl PresentCommand {
    fn to_pass(&self) -> FrameGraphPass {
        FrameGraphPass::new("present")
            .with_kind(FrameGraphPassKind::Present)
            .sample_image(self.source.clone())
            .write_image(self.target.clone(), GraphResourceUsage::Present)
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        resources: &CommandImageRegistry,
    ) -> Result<(), CommandExecutionError> {
        encode_image_copy(encoder, resources, "present", &self.source, &self.target)
    }
}

fn encode_image_copy(
    encoder: &mut wgpu::CommandEncoder,
    resources: &CommandImageRegistry,
    name: &str,
    source: &GraphResourceHandle,
    target: &GraphResourceHandle,
) -> Result<(), CommandExecutionError> {
    let source_image = resources
        .image(source.clone())
        .ok_or_else(|| CommandExecutionError::MissingImageResource {
            handle: source.as_str().to_string(),
        })?;
    let target_image = resources
        .image(target.clone())
        .ok_or_else(|| CommandExecutionError::MissingImageResource {
            handle: target.as_str().to_string(),
        })?;

    if source_image.desc.size != target_image.desc.size {
        return Err(CommandExecutionError::IncompatibleImageSize {
            name: name.to_string(),
            source_size: source_image.desc.size,
            target_size: target_image.desc.size,
        });
    }
    if source_image.desc.format != target_image.desc.format {
        return Err(CommandExecutionError::IncompatibleImageFormat {
            name: name.to_string(),
            source_format: source_image.desc.format,
            target_format: target_image.desc.format,
        });
    }
    if source_image.desc.samples != 1 || target_image.desc.samples != 1 {
        return Err(CommandExecutionError::UnsupportedMultisampledCopy {
            name: name.to_string(),
            source_samples: source_image.desc.samples,
            target_samples: target_image.desc.samples,
        });
    }

    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &source_image.image.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: &target_image.image.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        source_image.desc.extent(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GraphResourceKind;

    #[test]
    fn command_queue_chains_into_graph_plan() {
        let queue = CommandQueue::new()
            .compute("cull", |compute| {
                compute
                    .read_storage_buffer("instance_input")
                    .write_storage_buffer("visible_instances")
            })
            .raster("shade", |raster| {
                raster
                    .vertex_buffer("visible_instances")
                    .sample_image("albedo")
                    .color_attachment("lighting", RenderTargetType::Lighting)
                    .depth_attachment("depth")
            })
            .copy_image("lighting_history_copy", "lighting", "lighting_history")
            .present("lighting_history", "swapchain");

        let plan = queue.plan();
        assert_eq!(plan.passes.len(), 4);
        assert_eq!(plan.passes[0].kind, FrameGraphPassKind::Compute);
        assert_eq!(plan.passes[1].kind, FrameGraphPassKind::Raster);
        assert_eq!(plan.passes[2].kind, FrameGraphPassKind::Copy);
        assert_eq!(plan.passes[3].kind, FrameGraphPassKind::Present);

        let visible_instances = plan
            .resources
            .get(&GraphResourceHandle::new("visible_instances"))
            .expect("visible_instances");
        assert_eq!(visible_instances.kind, GraphResourceKind::Buffer);
        assert_eq!(visible_instances.producers, vec![0]);
        assert_eq!(visible_instances.consumers, vec![1]);

        let swapchain = plan
            .resources
            .get(&GraphResourceHandle::new("swapchain"))
            .expect("swapchain");
        assert_eq!(swapchain.producers, vec![3]);
    }
}

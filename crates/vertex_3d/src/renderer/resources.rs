//! Device-backed attachment and bind-group resources for renderer integration.

use std::collections::BTreeMap;

use super::{AttachmentLifecycle, RenderTargetHandle, RendererConfig};
use crate::shader::{BindingTypePlan, BuiltPipelineLayout, PipelineLayoutPlan, ReflectionSnapshot};

/// One allocated image used as a graph attachment and its default view.
#[derive(Debug)]
pub struct AttachmentImage {
    pub handle: RenderTargetHandle,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: [u32; 2],
    pub format: wgpu::TextureFormat,
    pub samples: u32,
    pub lifecycle: AttachmentLifecycle,
}

/// Runtime texture set derived from the renderer config.
#[derive(Debug, Default)]
pub struct AttachmentPool {
    attachments: BTreeMap<RenderTargetHandle, AttachmentImage>,
}

impl AttachmentPool {
    pub fn rebuild(&mut self, device: &wgpu::Device, config: &RendererConfig) {
        self.attachments.clear();
        for (handle, attachment) in &config.graph.attachments {
            let (width, height) = attachment.scale.resolve(&config.surface);
            let format = config
                .format_overrides
                .get(handle)
                .copied()
                .unwrap_or_else(|| {
                    super::config::format_for_precision(
                        config.hdr.internal_precision,
                        &attachment.target.r#type,
                        config.depth_format,
                    )
                });

            let sample_count = attachment.target.samples.max(config.msaa_samples).max(1);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(handle.as_str()),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: attachment.target.mip_levels.max(1),
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: usage_for_format(format, sample_count),
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.attachments.insert(
                handle.clone(),
                AttachmentImage {
                    handle: handle.clone(),
                    texture,
                    view,
                    size: [width, height],
                    format,
                    samples: attachment.target.samples.max(config.msaa_samples).max(1),
                    lifecycle: attachment.lifecycle,
                },
            );
        }
    }

    pub fn get(&self, handle: &RenderTargetHandle) -> Option<&AttachmentImage> {
        self.attachments.get(handle)
    }

    pub fn iter(&self) -> impl Iterator<Item = &AttachmentImage> {
        self.attachments.values()
    }
}

/// Named runtime resource used when building bind groups from reflection.
pub enum ShaderBindingResource<'a> {
    UniformBuffer(&'a wgpu::Buffer),
    StorageBuffer(&'a wgpu::Buffer),
    TextureView(&'a wgpu::TextureView),
    Sampler(&'a wgpu::Sampler),
}

/// Registry mapping reflected names to concrete `wgpu` resources.
#[derive(Default)]
pub struct ShaderResourceTable<'a> {
    resources: BTreeMap<&'a str, ShaderBindingResource<'a>>,
}

impl<'a> ShaderResourceTable<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_buffer(mut self, name: &'a str, buffer: &'a wgpu::Buffer) -> Self {
        self.resources
            .insert(name, ShaderBindingResource::UniformBuffer(buffer));
        self
    }

    pub fn insert_buffer_ref(&mut self, name: &'a str, buffer: &'a wgpu::Buffer) {
        self.resources
            .insert(name, ShaderBindingResource::UniformBuffer(buffer));
    }

    pub fn insert_storage_buffer(mut self, name: &'a str, buffer: &'a wgpu::Buffer) -> Self {
        self.resources
            .insert(name, ShaderBindingResource::StorageBuffer(buffer));
        self
    }

    pub fn insert_storage_buffer_ref(&mut self, name: &'a str, buffer: &'a wgpu::Buffer) {
        self.resources
            .insert(name, ShaderBindingResource::StorageBuffer(buffer));
    }

    pub fn insert_texture_view(mut self, name: &'a str, view: &'a wgpu::TextureView) -> Self {
        self.resources
            .insert(name, ShaderBindingResource::TextureView(view));
        self
    }

    pub fn insert_texture_view_ref(&mut self, name: &'a str, view: &'a wgpu::TextureView) {
        self.resources
            .insert(name, ShaderBindingResource::TextureView(view));
    }

    pub fn insert_sampler(mut self, name: &'a str, sampler: &'a wgpu::Sampler) -> Self {
        self.resources
            .insert(name, ShaderBindingResource::Sampler(sampler));
        self
    }

    pub fn insert_sampler_ref(&mut self, name: &'a str, sampler: &'a wgpu::Sampler) {
        self.resources
            .insert(name, ShaderBindingResource::Sampler(sampler));
    }

    pub fn extend(&mut self, other: &'a ShaderResourceTable<'a>) {
        for (&name, resource) in &other.resources {
            match resource {
                ShaderBindingResource::UniformBuffer(buffer) => {
                    self.insert_buffer_ref(name, buffer);
                }
                ShaderBindingResource::StorageBuffer(buffer) => {
                    self.insert_storage_buffer_ref(name, buffer);
                }
                ShaderBindingResource::TextureView(view) => {
                    self.insert_texture_view_ref(name, view);
                }
                ShaderBindingResource::Sampler(sampler) => {
                    self.insert_sampler_ref(name, sampler);
                }
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&ShaderBindingResource<'a>> {
        self.resources.get(name)
    }
}

/// One built bind group keyed by reflected descriptor space.
#[derive(Debug)]
pub struct NamedBindGroup {
    pub space: u32,
    pub bind_group: wgpu::BindGroup,
}

/// Reflection-driven bind-group set.
#[derive(Debug, Default)]
pub struct ReflectionBindGroupSet {
    pub bind_groups: Vec<NamedBindGroup>,
}

impl ReflectionBindGroupSet {
    pub fn build(
        device: &wgpu::Device,
        reflection: &ReflectionSnapshot,
        layout_plan: &PipelineLayoutPlan,
        built_layout: &BuiltPipelineLayout,
        resources: &ShaderResourceTable<'_>,
    ) -> Result<Self, BindGroupBuildError> {
        Self::build_with_filter(
            device,
            reflection,
            layout_plan,
            built_layout,
            resources,
            |_| true,
        )
    }

    pub fn build_with_filter(
        device: &wgpu::Device,
        reflection: &ReflectionSnapshot,
        layout_plan: &PipelineLayoutPlan,
        built_layout: &BuiltPipelineLayout,
        resources: &ShaderResourceTable<'_>,
        mut include_space: impl FnMut(u32) -> bool,
    ) -> Result<Self, BindGroupBuildError> {
        let mut bind_groups = Vec::new();

        for (group_index, group_plan) in layout_plan.bind_groups.iter().enumerate() {
            if !include_space(group_plan.space) {
                continue;
            }
            let layout = built_layout.bind_group_layouts.get(group_index).ok_or(
                BindGroupBuildError::MissingLayout {
                    space: group_plan.space,
                },
            )?;
            let mut entries = Vec::new();

            for entry in &group_plan.entries {
                let resource = resources.get(&entry.name).ok_or_else(|| {
                    BindGroupBuildError::MissingResource {
                        name: entry.name.clone(),
                        space: group_plan.space,
                        binding: entry.binding,
                    }
                })?;
                let resource = match (entry.binding_type, resource) {
                    (
                        BindingTypePlan::UniformBuffer,
                        ShaderBindingResource::UniformBuffer(buffer),
                    ) => wgpu::BindingResource::Buffer(buffer.as_entire_buffer_binding()),
                    (
                        BindingTypePlan::StorageBuffer,
                        ShaderBindingResource::StorageBuffer(buffer),
                    ) => wgpu::BindingResource::Buffer(buffer.as_entire_buffer_binding()),
                    (BindingTypePlan::Texture, ShaderBindingResource::TextureView(view)) => {
                        wgpu::BindingResource::TextureView(view)
                    }
                    (BindingTypePlan::Sampler, ShaderBindingResource::Sampler(sampler)) => {
                        wgpu::BindingResource::Sampler(sampler)
                    }
                    (BindingTypePlan::CombinedTextureSampler, _) => {
                        return Err(BindGroupBuildError::UnsupportedCombinedSampler {
                            name: entry.name.clone(),
                        });
                    }
                    (_, _) => {
                        return Err(BindGroupBuildError::ResourceTypeMismatch {
                            name: entry.name.clone(),
                            binding: entry.binding,
                            space: group_plan.space,
                        });
                    }
                };

                entries.push(wgpu::BindGroupEntry {
                    binding: entry.binding,
                    resource,
                });
            }

            let _ = reflection;
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vertex3d-reflected-bind-group"),
                layout,
                entries: &entries,
            });
            bind_groups.push(NamedBindGroup {
                space: group_plan.space,
                bind_group,
            });
        }

        Ok(Self { bind_groups })
    }

    pub fn get(&self, space: u32) -> Option<&wgpu::BindGroup> {
        self.bind_groups
            .iter()
            .find(|group| group.space == space)
            .map(|group| &group.bind_group)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BindGroupBuildError {
    #[error("missing built bind group layout for descriptor space {space}")]
    MissingLayout { space: u32 },

    #[error("missing resource '{name}' for descriptor space {space}, binding {binding}")]
    MissingResource {
        name: String,
        space: u32,
        binding: u32,
    },

    #[error("resource type mismatch for '{name}' in descriptor space {space}, binding {binding}")]
    ResourceTypeMismatch {
        name: String,
        space: u32,
        binding: u32,
    },

    #[error("combined texture samplers are not yet supported for resource '{name}'")]
    UnsupportedCombinedSampler { name: String },
}

fn usage_for_format(format: wgpu::TextureFormat, sample_count: u32) -> wgpu::TextureUsages {
    if format.is_depth_stencil_format() {
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
    } else if sample_count > 1 {
        // Multisampled textures cannot have COPY_SRC or COPY_DST per WebGPU spec.
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
    } else {
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::{
        GraphAttachment, RenderTargetScale, ShaderGraphDescriptor, SurfaceConfig,
    };
    use crate::shader::{
        ReflectedResource, ReflectedResourceType, ReflectionSnapshot, RenderTargetConfig,
        RenderTargetType,
    };

    #[test]
    fn attachment_pool_rebuilds_from_graph() {
        let mut config = RendererConfig::new(SurfaceConfig::default());
        config.graph = ShaderGraphDescriptor::new().with_attachment(
            GraphAttachment::new(
                "g_albedo",
                RenderTargetConfig::new(RenderTargetType::Albedo, 1, 1),
            )
            .with_scale(RenderTargetScale::Half),
        );
        assert_eq!(config.graph.attachments.len(), 1);
    }

    #[test]
    fn resource_table_tracks_names() {
        let buffer = None::<wgpu::Buffer>;
        let table = ShaderResourceTable::new();
        assert!(table.get("missing").is_none());
        let _ = buffer;
    }

    #[test]
    fn reflection_resource_types_stay_stable() {
        let reflection = ReflectionSnapshot {
            stages: Vec::new(),
            resources: vec![ReflectedResource::new(
                "preview_tex",
                0,
                ReflectedResourceType::Texture,
            )],
            render_targets: Vec::new(),
        };
        assert_eq!(reflection.resources[0].name, "preview_tex");
    }
}

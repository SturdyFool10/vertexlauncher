//! Reflection-driven pipeline layout planning.

use std::collections::BTreeMap;

use super::{
    ReflectedResource, ReflectedResourceType, ReflectedTextureDimension, ReflectionSnapshot,
    ResourceType,
};

/// Pipeline layout grouped by descriptor set / bind group space.
#[derive(Debug, Clone, Default)]
pub struct PipelineLayoutPlan {
    pub bind_groups: Vec<BindGroupLayoutPlan>,
}

impl PipelineLayoutPlan {
    pub fn from_reflection(
        reflection: &ReflectionSnapshot,
    ) -> Result<Self, PipelineLayoutPlanError> {
        let mut groups: BTreeMap<u32, Vec<PipelineResourceBindingPlan>> = BTreeMap::new();

        for resource in &reflection.resources {
            let group = groups.entry(resource.space).or_default();
            if group.iter().any(|entry| entry.binding == resource.slot) {
                return Err(PipelineLayoutPlanError::DuplicateBinding {
                    space: resource.space,
                    binding: resource.slot,
                    resource: resource.name.clone(),
                });
            }
            group.push(PipelineResourceBindingPlan::from_reflected_resource(resource));
        }

        let bind_groups = groups
            .into_iter()
            .map(|(space, mut entries)| {
                entries.sort_by_key(|entry| entry.binding);
                BindGroupLayoutPlan { space, entries }
            })
            .collect();

        Ok(Self { bind_groups })
    }

    pub fn bind_group(&self, space: u32) -> Option<&BindGroupLayoutPlan> {
        self.bind_groups.iter().find(|group| group.space == space)
    }

    pub fn create_wgpu_layout(
        &self,
        device: &wgpu::Device,
        label: Option<&str>,
    ) -> Result<BuiltPipelineLayout, PipelineLayoutPlanError> {
        let mut bind_group_layouts = Vec::with_capacity(self.bind_groups.len());

        for group in &self.bind_groups {
            let entries: Result<Vec<_>, _> = group
                .entries
                .iter()
                .map(PipelineResourceBindingPlan::to_wgpu_entry)
                .collect();
            let entries = entries?;
            bind_group_layouts.push(device.create_bind_group_layout(
                &wgpu::BindGroupLayoutDescriptor {
                    label,
                    entries: &entries,
                },
            ));
        }

        let layout_refs: Vec<_> = bind_group_layouts.iter().map(Some).collect();
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label,
            bind_group_layouts: &layout_refs,
            immediate_size: 0,
        });

        Ok(BuiltPipelineLayout {
            bind_group_layouts,
            pipeline_layout,
        })
    }
}

/// Device-backed pipeline layout derived from reflection.
#[derive(Debug)]
pub struct BuiltPipelineLayout {
    pub bind_group_layouts: Vec<wgpu::BindGroupLayout>,
    pub pipeline_layout: wgpu::PipelineLayout,
}

/// One bind group / descriptor set layout.
#[derive(Debug, Clone)]
pub struct BindGroupLayoutPlan {
    pub space: u32,
    pub entries: Vec<PipelineResourceBindingPlan>,
}

/// Planned binding entry derived from shader reflection.
#[derive(Debug, Clone)]
pub struct PipelineResourceBindingPlan {
    pub name: String,
    pub space: u32,
    pub binding: u32,
    pub visibility: wgpu::ShaderStages,
    pub binding_type: BindingTypePlan,
    pub texture_dimension: Option<ReflectedTextureDimension>,
}

impl PipelineResourceBindingPlan {
    pub fn from_reflected_resource(resource: &ReflectedResource) -> Self {
        let visibility = resource.stages.iter().fold(wgpu::ShaderStages::NONE, |acc, stage| {
            acc | wgpu::ShaderStages::from_bits_retain(stage.wgpu_stage_flags())
        });
        Self {
            name: resource.name.clone(),
            space: resource.space,
            binding: resource.slot,
            visibility: if visibility.is_empty() {
                wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE
            } else {
                visibility
            },
            binding_type: BindingTypePlan::from_reflected_type(resource.resource_type),
            texture_dimension: resource.texture_dimension,
        }
    }

    pub fn resource_type(&self) -> ResourceType {
        self.binding_type.resource_type()
    }

    pub fn to_wgpu_entry(&self) -> Result<wgpu::BindGroupLayoutEntry, PipelineLayoutPlanError> {
        Ok(wgpu::BindGroupLayoutEntry {
            binding: self.binding,
            visibility: self.visibility,
            ty: self.binding_type.to_wgpu_binding_type(self.texture_dimension)?,
            count: None,
        })
    }
}

/// Backend-neutral binding kind used to later create `wgpu` layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingTypePlan {
    UniformBuffer,
    StorageBuffer,
    Texture,
    Sampler,
    CombinedTextureSampler,
}

impl BindingTypePlan {
    pub fn from_reflected_type(resource_type: ReflectedResourceType) -> Self {
        match resource_type {
            ReflectedResourceType::UniformBuffer => Self::UniformBuffer,
            ReflectedResourceType::StorageBuffer => Self::StorageBuffer,
            ReflectedResourceType::Texture => Self::Texture,
            ReflectedResourceType::Sampler => Self::Sampler,
            ReflectedResourceType::CombinedTextureSampler => Self::CombinedTextureSampler,
        }
    }

    pub fn resource_type(self) -> ResourceType {
        match self {
            BindingTypePlan::UniformBuffer => ResourceType::UniformBuffer,
            BindingTypePlan::StorageBuffer => ResourceType::StorageBuffer,
            BindingTypePlan::Texture => ResourceType::Texture,
            BindingTypePlan::Sampler => ResourceType::Sampler,
            BindingTypePlan::CombinedTextureSampler => ResourceType::CombinedTextureSampler,
        }
    }

    fn to_wgpu_binding_type(
        self,
        texture_dimension: Option<ReflectedTextureDimension>,
    ) -> Result<wgpu::BindingType, PipelineLayoutPlanError> {
        Ok(match self {
            BindingTypePlan::UniformBuffer => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            BindingTypePlan::StorageBuffer => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            BindingTypePlan::Texture => wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: texture_dimension
                    .unwrap_or(ReflectedTextureDimension::D2)
                    .to_wgpu_view_dimension(),
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
            },
            BindingTypePlan::Sampler => {
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
            }
            BindingTypePlan::CombinedTextureSampler => {
                return Err(PipelineLayoutPlanError::UnsupportedCombinedSampler)
            }
        })
    }
}

impl ReflectedTextureDimension {
    fn to_wgpu_view_dimension(self) -> wgpu::TextureViewDimension {
        match self {
            ReflectedTextureDimension::D1 => wgpu::TextureViewDimension::D1,
            ReflectedTextureDimension::D2 => wgpu::TextureViewDimension::D2,
            ReflectedTextureDimension::D2Array => wgpu::TextureViewDimension::D2Array,
            ReflectedTextureDimension::Cube => wgpu::TextureViewDimension::Cube,
            ReflectedTextureDimension::D3 => wgpu::TextureViewDimension::D3,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineLayoutPlanError {
    #[error("duplicate reflected binding in space {space}, binding {binding}: {resource}")]
    DuplicateBinding {
        space: u32,
        binding: u32,
        resource: String,
    },

    #[error("combined texture samplers require target-specific lowering before wgpu layout creation")]
    UnsupportedCombinedSampler,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::{ReflectedResource, ReflectedResourceType, ShaderKind};

    #[test]
    fn groups_resources_by_space() {
        let mut camera = ReflectedResource::new("camera", 0, ReflectedResourceType::UniformBuffer);
        camera.stages = vec![ShaderKind::Vertex];
        let mut albedo = ReflectedResource::new("albedo", 1, ReflectedResourceType::Texture);
        albedo.space = 1;
        albedo.stages = vec![ShaderKind::Fragment];

        let plan = PipelineLayoutPlan::from_reflection(&ReflectionSnapshot {
            stages: Vec::new(),
            resources: vec![camera, albedo],
            render_targets: Vec::new(),
        })
        .expect("plan");
        assert_eq!(plan.bind_groups.len(), 2);
        assert_eq!(plan.bind_group(0).unwrap().entries.len(), 1);
        assert_eq!(plan.bind_group(1).unwrap().entries.len(), 1);
    }
}

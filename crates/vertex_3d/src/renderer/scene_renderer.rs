//! High-level scene renderer that connects shader reflection, material bindings,
//! and queued scene submissions into actual draw encoding.

use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::{
    DeferredRenderer, DeferredRendererError, GpuInstanceData, GpuResourceRegistry,
    QueuedSceneSubmission, ReflectionBindGroupSet, ShaderResourceTable, SubmissionError,
};
use crate::{
    Material, MaterialHandle, MaterialParameters, MaterialValue, PbrMaterial, PipelineLayoutPlan,
    ReflectedResourceRole, ShaderHandle, TextureHandle, UnlitMaterial, Vertex,
    asset::{RenderAssetLibrary, ShaderAsset},
};

/// Pipeline creation parameters for scene rendering into a specific render pass layout.
#[derive(Debug, Clone)]
pub struct ScenePipelineConfig {
    pub color_formats: Vec<Option<wgpu::TextureFormat>>,
    pub depth_format: Option<wgpu::TextureFormat>,
    pub sample_count: u32,
    pub primitive: wgpu::PrimitiveState,
}

impl ScenePipelineConfig {
    pub fn for_surface(format: wgpu::TextureFormat) -> Self {
        Self {
            color_formats: vec![Some(format)],
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
            sample_count: 1,
            primitive: wgpu::PrimitiveState::default(),
        }
    }

    pub fn with_depth_format(mut self, depth_format: Option<wgpu::TextureFormat>) -> Self {
        self.depth_format = depth_format;
        self
    }

    pub fn with_sample_count(mut self, sample_count: u32) -> Self {
        self.sample_count = sample_count.max(1);
        self
    }
}

/// A caller-provided global bind group that owns one descriptor space.
pub struct ExternalShaderBindGroup<'a> {
    pub space: u32,
    pub bind_group: &'a wgpu::BindGroup,
}

/// Cached shader pipeline and descriptor-space metadata.
pub struct PreparedSceneShader {
    pipeline: wgpu::RenderPipeline,
    material_bind_groups: Vec<u32>,
    space_to_group_index: BTreeMap<u32, u32>,
}

/// Cached GPU material bindings.
pub struct PreparedMaterial {
    _uniform_buffer: wgpu::Buffer,
    bind_groups: ReflectionBindGroupSet,
}

struct DefaultMaterialResources {
    white: super::GpuTexture,
    black: super::GpuTexture,
    normal: super::GpuTexture,
}

/// End-to-end scene renderer for queued submissions and reflected Slang shaders.
pub struct SceneRenderer {
    pipeline: ScenePipelineConfig,
    default_materials: DefaultMaterialResources,
    shaders: BTreeMap<ShaderHandle, PreparedSceneShader>,
    materials: BTreeMap<MaterialHandle, PreparedMaterial>,
}

impl SceneRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, pipeline: ScenePipelineConfig) -> Self {
        Self {
            pipeline,
            default_materials: DefaultMaterialResources::new(device, queue),
            shaders: BTreeMap::new(),
            materials: BTreeMap::new(),
        }
    }

    pub fn prepare_scene(
        &mut self,
        device: &wgpu::Device,
        assets: &RenderAssetLibrary,
        gpu_resources: &GpuResourceRegistry,
        submission: &QueuedSceneSubmission,
    ) -> Result<(), SceneRendererError> {
        for batch in &submission.batches {
            self.ensure_shader(device, assets, batch.shader)?;
            self.ensure_material(device, assets, gpu_resources, batch.material, batch.shader)?;
        }
        Ok(())
    }

    pub fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        gpu_resources: &'pass GpuResourceRegistry,
        submission: &'pass QueuedSceneSubmission,
        external_bind_groups: &'pass [ExternalShaderBindGroup<'pass>],
    ) -> Result<(), SceneRendererError> {
        pass.set_vertex_buffer(
            1,
            submission.instance_upload.buffer.slice(
                submission.instance_upload.offset
                    ..submission.instance_upload.offset + submission.instance_upload.size,
            ),
        );

        for batch in &submission.batches {
            let shader = self.shaders.get(&batch.shader).ok_or(
                SceneRendererError::MissingPreparedShader {
                    handle_id: batch.shader.id(),
                },
            )?;
            let material = self.materials.get(&batch.material).ok_or(
                SceneRendererError::MissingPreparedMaterial {
                    handle_id: batch.material.id(),
                },
            )?;
            let mesh = gpu_resources
                .mesh(batch.mesh)
                .ok_or(SceneRendererError::Submission(
                    SubmissionError::MissingResidentMesh {
                        handle_id: batch.mesh.id(),
                    },
                ))?;

            pass.set_pipeline(&shader.pipeline);
            for external in external_bind_groups {
                if let Some(group_index) = shader.space_to_group_index.get(&external.space) {
                    pass.set_bind_group(*group_index, external.bind_group, &[]);
                }
            }
            for bind_group in &material.bind_groups.bind_groups {
                if let Some(group_index) = shader.space_to_group_index.get(&bind_group.space) {
                    pass.set_bind_group(*group_index, &bind_group.bind_group, &[]);
                }
            }

            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            if let Some(index_buffer) = &mesh.index_buffer {
                pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, batch.instance_range.clone());
            } else {
                pass.draw(0..mesh.vertex_count, batch.instance_range.clone());
            }
        }
        Ok(())
    }

    fn ensure_shader(
        &mut self,
        device: &wgpu::Device,
        assets: &RenderAssetLibrary,
        handle: ShaderHandle,
    ) -> Result<(), SceneRendererError> {
        if self.shaders.contains_key(&handle) {
            return Ok(());
        }

        let shader_asset = assets
            .shader(handle)
            .ok_or(SceneRendererError::MissingShaderAsset {
                handle_id: handle.id(),
            })?;
        let deferred = shader_asset.create_deferred_renderer(
            device,
            crate::SurfaceConfig::new(
                1,
                1,
                self.pipeline.color_formats[0].unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb),
            ),
        )?;
        let layout_plan = PipelineLayoutPlan::from_reflection(&shader_asset.compiled.reflection)?;
        let material_bind_groups =
            classify_material_spaces(&shader_asset.compiled.reflection, &layout_plan)?;
        let pipeline = build_scene_pipeline(device, shader_asset, &deferred, &self.pipeline)?;
        let space_to_group_index = layout_plan
            .bind_groups
            .iter()
            .enumerate()
            .map(|(index, group)| (group.space, index as u32))
            .collect();

        self.shaders.insert(
            handle,
            PreparedSceneShader {
                pipeline,
                material_bind_groups,
                space_to_group_index,
            },
        );
        Ok(())
    }

    fn ensure_material(
        &mut self,
        device: &wgpu::Device,
        assets: &RenderAssetLibrary,
        gpu_resources: &GpuResourceRegistry,
        material_handle: MaterialHandle,
        shader_handle: ShaderHandle,
    ) -> Result<(), SceneRendererError> {
        if self.materials.contains_key(&material_handle) {
            return Ok(());
        }

        let shader_asset =
            assets
                .shader(shader_handle)
                .ok_or(SceneRendererError::MissingShaderAsset {
                    handle_id: shader_handle.id(),
                })?;
        let material =
            assets
                .material(material_handle)
                .ok_or(SceneRendererError::MissingMaterialAsset {
                    handle_id: material_handle.id(),
                })?;
        let prepared_shader =
            self.shaders
                .get(&shader_handle)
                .ok_or(SceneRendererError::MissingPreparedShader {
                    handle_id: shader_handle.id(),
                })?;

        let uniform_data = MaterialUniformData::from_material(material);
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vertex3d-material-uniform"),
            contents: bytemuck::bytes_of(&uniform_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let layout_plan = PipelineLayoutPlan::from_reflection(&shader_asset.compiled.reflection)?;
        let mut resources = ShaderResourceTable::new();
        insert_material_resources(
            &mut resources,
            material,
            &shader_asset.compiled.reflection,
            gpu_resources,
            &self.default_materials,
            &uniform_buffer,
        );

        let bind_groups = ReflectionBindGroupSet::build_with_filter(
            device,
            &shader_asset.compiled.reflection,
            &layout_plan,
            &DeferredRenderer::from_compiled_program(
                device,
                crate::SurfaceConfig::new(
                    1,
                    1,
                    self.pipeline.color_formats[0].unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb),
                ),
                &shader_asset.compiled,
                shader_asset.frame_graph.clone(),
                shader_asset.pass_templates.clone(),
            )?
            .pipeline_layout,
            &resources,
            |space| prepared_shader.material_bind_groups.contains(&space),
        )?;

        self.materials.insert(
            material_handle,
            PreparedMaterial {
                _uniform_buffer: uniform_buffer,
                bind_groups,
            },
        );
        Ok(())
    }
}

fn build_scene_pipeline(
    device: &wgpu::Device,
    shader_asset: &ShaderAsset,
    deferred: &DeferredRenderer,
    pipeline: &ScenePipelineConfig,
) -> Result<wgpu::RenderPipeline, SceneRendererError> {
    let Some(pass) = deferred.passes.first() else {
        return Err(SceneRendererError::MissingShaderPass {
            shader: shader_asset.label.clone(),
        });
    };
    let Some(vertex_module) = pass.vertex_module.as_ref() else {
        return Err(SceneRendererError::MissingVertexStage {
            shader: shader_asset.label.clone(),
        });
    };

    let vertex_entry = shader_asset
        .compiled
        .stages
        .get(&crate::ShaderKind::Vertex)
        .map(|stage| stage.entry_point.as_str())
        .unwrap_or("main");
    let fragment_entry = shader_asset
        .compiled
        .stages
        .get(&crate::ShaderKind::Fragment)
        .map(|stage| stage.entry_point.as_str())
        .unwrap_or("main");

    let color_targets = infer_color_targets(shader_asset, pipeline)?;
    let fragment = pass
        .fragment_module
        .as_ref()
        .map(|module| wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &color_targets,
        });
    let depth_stencil = pipeline.depth_format.map(|format| wgpu::DepthStencilState {
        format,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    });

    Ok(
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("vertex3d-scene-pipeline-{}", shader_asset.label)),
            layout: Some(&deferred.pipeline_layout.pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_module,
                entry_point: Some(vertex_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Vertex::vertex_layout(), GpuInstanceData::vertex_layout()],
            },
            fragment,
            primitive: pipeline.primitive,
            depth_stencil,
            multisample: wgpu::MultisampleState {
                count: pipeline.sample_count,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        }),
    )
}

fn infer_color_targets(
    shader_asset: &ShaderAsset,
    pipeline: &ScenePipelineConfig,
) -> Result<Vec<Option<wgpu::ColorTargetState>>, SceneRendererError> {
    let color_outputs = shader_asset
        .compiled
        .reflection
        .render_targets
        .iter()
        .filter(|target| {
            !matches!(
                target
                    .target_type
                    .as_deref()
                    .map(crate::RenderTargetType::from_reflection_name),
                Some(crate::RenderTargetType::Depth | crate::RenderTargetType::Shadows)
            ) && crate::RenderTargetType::from_reflection_name(target.handle.as_str()).is_color()
        })
        .count();

    let target_count = color_outputs.max(1);
    if pipeline.color_formats.len() < target_count {
        return Err(SceneRendererError::InsufficientColorTargets {
            expected: target_count,
            provided: pipeline.color_formats.len(),
        });
    }

    Ok(pipeline
        .color_formats
        .iter()
        .take(target_count)
        .map(|format| {
            format.map(|format| wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect())
}

fn classify_material_spaces(
    reflection: &crate::ReflectionSnapshot,
    layout_plan: &PipelineLayoutPlan,
) -> Result<Vec<u32>, SceneRendererError> {
    let mut spaces = Vec::new();
    for group in &layout_plan.bind_groups {
        let mut material_count = 0usize;
        for entry in &group.entries {
            if is_material_resource(reflection, &entry.name) {
                material_count += 1;
            }
        }
        if material_count == group.entries.len() {
            spaces.push(group.space);
        } else if material_count > 0 {
            return Err(SceneRendererError::MixedDescriptorSpace { space: group.space });
        }
    }
    Ok(spaces)
}

fn insert_material_resources<'a>(
    resources: &mut ShaderResourceTable<'a>,
    material: &'a Material,
    reflection: &'a crate::ReflectionSnapshot,
    gpu_resources: &'a GpuResourceRegistry,
    defaults: &'a DefaultMaterialResources,
    uniform_buffer: &'a wgpu::Buffer,
) {
    let base_color = resolve_texture(material.textures.base_color, gpu_resources, &defaults.white);
    let metallic_roughness = resolve_texture(
        material.textures.metallic_roughness,
        gpu_resources,
        &defaults.white,
    );
    let normal = resolve_texture(material.textures.normal, gpu_resources, &defaults.normal);
    let emissive = resolve_texture(material.textures.emissive, gpu_resources, &defaults.black);
    let occlusion = resolve_texture(material.textures.occlusion, gpu_resources, &defaults.white);

    for reflected in &reflection.resources {
        let Some(role) = reflected
            .role
            .or_else(|| infer_legacy_material_role(&reflected.name))
        else {
            continue;
        };
        match role {
            ReflectedResourceRole::MaterialUniform => {
                resources.insert_buffer_ref(reflected.name.as_str(), uniform_buffer);
            }
            ReflectedResourceRole::MaterialBaseColorTexture => {
                resources.insert_texture_view_ref(reflected.name.as_str(), &base_color.view);
            }
            ReflectedResourceRole::MaterialMetallicRoughnessTexture => {
                resources
                    .insert_texture_view_ref(reflected.name.as_str(), &metallic_roughness.view);
            }
            ReflectedResourceRole::MaterialNormalTexture => {
                resources.insert_texture_view_ref(reflected.name.as_str(), &normal.view);
            }
            ReflectedResourceRole::MaterialEmissiveTexture => {
                resources.insert_texture_view_ref(reflected.name.as_str(), &emissive.view);
            }
            ReflectedResourceRole::MaterialOcclusionTexture => {
                resources.insert_texture_view_ref(reflected.name.as_str(), &occlusion.view);
            }
            ReflectedResourceRole::MaterialSampler => {
                resources.insert_sampler_ref(reflected.name.as_str(), &base_color.sampler);
            }
        }
    }
}

fn resolve_texture<'a>(
    handle: Option<TextureHandle>,
    gpu_resources: &'a GpuResourceRegistry,
    fallback: &'a super::GpuTexture,
) -> &'a super::GpuTexture {
    handle
        .and_then(|handle| gpu_resources.texture(handle))
        .unwrap_or(fallback)
}

fn is_material_resource(reflection: &crate::ReflectionSnapshot, name: &str) -> bool {
    reflection
        .resources
        .iter()
        .find(|resource| resource.name == name)
        .and_then(|resource| resource.role.or_else(|| infer_legacy_material_role(name)))
        .is_some_and(ReflectedResourceRole::is_material_role)
}

fn infer_legacy_material_role(name: &str) -> Option<ReflectedResourceRole> {
    match name {
        "material" | "material_uniform" | "material_params" | "material_data" => {
            Some(ReflectedResourceRole::MaterialUniform)
        }
        "base_color_texture" | "base_color" | "albedo_texture" | "albedo" => {
            Some(ReflectedResourceRole::MaterialBaseColorTexture)
        }
        "metallic_roughness_texture" | "roughness_metallic_texture" | "metallic_roughness" => {
            Some(ReflectedResourceRole::MaterialMetallicRoughnessTexture)
        }
        "normal_texture" | "normal_map" | "normal" => {
            Some(ReflectedResourceRole::MaterialNormalTexture)
        }
        "emissive_texture" | "emissive_map" | "emissive" => {
            Some(ReflectedResourceRole::MaterialEmissiveTexture)
        }
        "occlusion_texture" | "ao_texture" | "occlusion" => {
            Some(ReflectedResourceRole::MaterialOcclusionTexture)
        }
        "material_sampler" | "base_color_sampler" | "albedo_sampler" | "linear_sampler" => {
            Some(ReflectedResourceRole::MaterialSampler)
        }
        _ => None,
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniformData {
    base_color_factor: [f32; 4],
    emissive_factor: [f32; 4],
    surface_factors: [f32; 4],
    flags: [u32; 4],
}

impl MaterialUniformData {
    fn from_material(material: &Material) -> Self {
        let (
            base_color_factor,
            emissive_factor,
            metallic,
            roughness,
            normal_scale,
            occlusion,
            pbr_flag,
        ) = match &material.parameters {
            MaterialParameters::Unlit(UnlitMaterial { color }) => {
                (*color, [0.0, 0.0, 0.0, 0.0], 0.0, 1.0, 1.0, 1.0, 0)
            }
            MaterialParameters::Pbr(PbrMaterial {
                base_color_factor,
                emissive_factor,
                metallic_factor,
                roughness_factor,
                normal_scale,
                occlusion_strength,
                ..
            }) => (
                *base_color_factor,
                [
                    emissive_factor[0],
                    emissive_factor[1],
                    emissive_factor[2],
                    0.0,
                ],
                *metallic_factor,
                *roughness_factor,
                *normal_scale,
                *occlusion_strength,
                1,
            ),
            MaterialParameters::Custom(values) => (
                custom_vec4(values, "base_color_factor", [1.0, 1.0, 1.0, 1.0]),
                custom_vec4(values, "emissive_factor", [0.0, 0.0, 0.0, 0.0]),
                custom_scalar(values, "metallic_factor", 0.0),
                custom_scalar(values, "roughness_factor", 1.0),
                custom_scalar(values, "normal_scale", 1.0),
                custom_scalar(values, "occlusion_strength", 1.0),
                1,
            ),
        };

        Self {
            base_color_factor,
            emissive_factor,
            surface_factors: [metallic, roughness, normal_scale, occlusion],
            flags: [
                pbr_flag,
                u32::from(material.casts_shadows),
                u32::from(material.receives_shadows),
                match material.alpha_mode {
                    crate::AlphaMode::Opaque => 0,
                    crate::AlphaMode::Mask { .. } => 1,
                    crate::AlphaMode::Blend => 2,
                },
            ],
        }
    }
}

fn custom_scalar(values: &BTreeMap<String, MaterialValue>, key: &str, default: f32) -> f32 {
    match values.get(key) {
        Some(MaterialValue::Scalar(value)) => *value,
        _ => default,
    }
}

fn custom_vec4(values: &BTreeMap<String, MaterialValue>, key: &str, default: [f32; 4]) -> [f32; 4] {
    match values.get(key) {
        Some(MaterialValue::Vec4(value)) => *value,
        Some(MaterialValue::Vec3(value)) => [value[0], value[1], value[2], default[3]],
        _ => default,
    }
}

impl DefaultMaterialResources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            white: create_solid_texture(
                device,
                queue,
                "vertex3d-default-white",
                [255, 255, 255, 255],
            ),
            black: create_solid_texture(device, queue, "vertex3d-default-black", [0, 0, 0, 255]),
            normal: create_solid_texture(
                device,
                queue,
                "vertex3d-default-normal",
                [128, 128, 255, 255],
            ),
        }
    }
}

fn create_solid_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: [u8; 4],
) -> super::GpuTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(&format!("{label}-sampler")),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    super::GpuTexture::new(
        texture,
        view,
        sampler,
        [1, 1],
        wgpu::TextureFormat::Rgba8UnormSrgb,
    )
}

#[derive(Debug, thiserror::Error)]
pub enum SceneRendererError {
    #[error("shader asset {handle_id} was not found")]
    MissingShaderAsset { handle_id: u64 },

    #[error("material asset {handle_id} was not found")]
    MissingMaterialAsset { handle_id: u64 },

    #[error("shader {handle_id} was not prepared before draw")]
    MissingPreparedShader { handle_id: u64 },

    #[error("material {handle_id} was not prepared before draw")]
    MissingPreparedMaterial { handle_id: u64 },

    #[error("shader '{shader}' has no prepared raster pass")]
    MissingShaderPass { shader: String },

    #[error("shader '{shader}' is missing a vertex stage")]
    MissingVertexStage { shader: String },

    #[error("descriptor space {space} mixes material and non-material bindings")]
    MixedDescriptorSpace { space: u32 },

    #[error("scene pipeline expected at least {expected} color target formats but got {provided}")]
    InsufficientColorTargets { expected: usize, provided: usize },

    #[error(transparent)]
    Deferred(#[from] DeferredRendererError),

    #[error(transparent)]
    Layout(#[from] crate::PipelineLayoutPlanError),

    #[error(transparent)]
    BindGroups(#[from] super::BindGroupBuildError),

    #[error(transparent)]
    Submission(#[from] SubmissionError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::{
        ReflectedResource, ReflectedResourceRole, ReflectedResourceType, ReflectionSnapshot,
    };

    #[test]
    fn material_space_detection_accepts_pure_material_groups() {
        let reflection = ReflectionSnapshot {
            stages: Vec::new(),
            resources: vec![
                ReflectedResource::new("material_params", 0, ReflectedResourceType::UniformBuffer)
                    .with_role(ReflectedResourceRole::MaterialUniform),
                {
                    let mut sampler = ReflectedResource::new(
                        "material_sampler",
                        1,
                        ReflectedResourceType::Sampler,
                    );
                    sampler.space = 1;
                    sampler.role = Some(ReflectedResourceRole::MaterialSampler);
                    sampler
                },
            ],
            render_targets: Vec::new(),
        };
        let layout = PipelineLayoutPlan::from_reflection(&reflection).expect("layout");

        let spaces = classify_material_spaces(&reflection, &layout).expect("spaces");
        assert_eq!(spaces, vec![0, 1]);
    }

    #[test]
    fn material_uniform_data_supports_unlit_defaults() {
        let material = Material::unlit("unlit", ShaderHandle::new(1));
        let data = MaterialUniformData::from_material(&material);
        assert_eq!(data.base_color_factor, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(data.flags[0], 0);
    }
}

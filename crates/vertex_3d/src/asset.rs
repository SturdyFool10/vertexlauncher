//! High-level render asset management for consumer-facing renderer workflows.

use std::{collections::BTreeMap, marker::PhantomData};

use crate::{
    image::ImageAsset,
    mesh::Mesh,
    renderer::{
        DeferredRenderPipelineTemplate, DeferredRenderer, DeferredRendererError, FrameGraph,
        SurfaceConfig,
    },
    shader::{
        CompiledShaderProgram, ShaderBackendTarget, ShaderCompileError, ShaderCompileRequest,
        ShaderCompileSource, ShaderCompiler, ShaderKind, ShaderSourceLanguage, StageSource,
    },
};

/// Stable typed handle for a renderer asset.
pub struct AssetHandle<T> {
    id: u64,
    marker: PhantomData<fn() -> T>,
}

impl<T> AssetHandle<T> {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    pub fn id(self) -> u64 {
        self.id
    }
}

impl<T> Copy for AssetHandle<T> {}

impl<T> Clone for AssetHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> std::fmt::Debug for AssetHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AssetHandle").field(&self.id).finish()
    }
}

impl<T> PartialEq for AssetHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for AssetHandle<T> {}

impl<T> PartialOrd for AssetHandle<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for AssetHandle<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl<T> std::hash::Hash for AssetHandle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// CPU-side mesh asset.
#[derive(Clone)]
pub struct MeshAsset {
    pub label: String,
    pub mesh: Mesh,
}

/// A compiled shader asset backed by reflection and frame-graph metadata.
#[derive(Debug, Clone)]
pub struct ShaderAsset {
    pub label: String,
    pub source_language: ShaderSourceLanguage,
    pub compiled: CompiledShaderProgram,
    pub frame_graph: FrameGraph,
    pub pass_templates: Vec<DeferredRenderPipelineTemplate>,
}

impl ShaderAsset {
    pub fn create_deferred_renderer(
        &self,
        device: &wgpu::Device,
        surface: SurfaceConfig,
    ) -> Result<DeferredRenderer, DeferredRendererError> {
        DeferredRenderer::from_compiled_program(
            device,
            surface,
            &self.compiled,
            self.frame_graph.clone(),
            self.pass_templates.clone(),
        )
    }
}

/// Slang-first descriptor used to compile a shader asset for runtime use.
#[derive(Debug, Clone)]
pub struct ShaderAssetDesc {
    pub name: String,
    pub language: ShaderSourceLanguage,
    pub target: ShaderBackendTarget,
    pub stages: Vec<StageSource>,
    pub frame_graph: Option<FrameGraph>,
    pub pass_templates: Vec<DeferredRenderPipelineTemplate>,
}

impl ShaderAssetDesc {
    pub fn slang(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            language: ShaderSourceLanguage::Slang,
            target: ShaderBackendTarget::Wgsl,
            stages: Vec::new(),
            frame_graph: None,
            pass_templates: Vec::new(),
        }
    }

    pub fn with_stage(mut self, stage: StageSource) -> Self {
        self.stages.push(stage);
        self
    }

    pub fn with_stage_file(mut self, kind: ShaderKind, path: impl Into<String>) -> Self {
        self.stages.push(StageSource::file(kind, path));
        self
    }

    pub fn with_stage_inline(mut self, kind: ShaderKind, source: impl Into<String>) -> Self {
        self.stages.push(StageSource::inline(kind, source));
        self
    }

    pub fn with_frame_graph(mut self, frame_graph: FrameGraph) -> Self {
        self.frame_graph = Some(frame_graph);
        self
    }

    pub fn with_pass_template(mut self, template: DeferredRenderPipelineTemplate) -> Self {
        self.pass_templates.push(template);
        self
    }

    pub fn compile<C: ShaderCompiler>(
        &self,
        compiler: &C,
    ) -> Result<ShaderAsset, ShaderAssetBuildError> {
        if self.stages.is_empty() {
            return Err(ShaderAssetBuildError::NoStages {
                shader: self.name.clone(),
            });
        }

        let request = ShaderCompileRequest {
            name: self.name.clone(),
            language: self.language,
            target: self.target,
            stages: self.stages.clone(),
        };
        let compiled = compiler.compile_program(&request)?.apply_reflection();
        let frame_graph = self
            .frame_graph
            .clone()
            .unwrap_or_else(|| FrameGraph::from_reflection(&compiled.reflection));
        let pass_templates = if self.pass_templates.is_empty() {
            default_pass_templates(&compiled)
        } else {
            self.pass_templates.clone()
        };

        Ok(ShaderAsset {
            label: self.name.clone(),
            source_language: self.language,
            compiled,
            frame_graph,
            pass_templates,
        })
    }
}

fn default_pass_templates(compiled: &CompiledShaderProgram) -> Vec<DeferredRenderPipelineTemplate> {
    let has_compute = compiled.stages.contains_key(&ShaderKind::Compute);
    let has_raster = compiled.stages.contains_key(&ShaderKind::Vertex)
        || compiled.stages.contains_key(&ShaderKind::Fragment);

    if has_raster {
        vec![DeferredRenderPipelineTemplate {
            pass_name: compiled.program.name.clone(),
            vertex_stage: compiled
                .stages
                .contains_key(&ShaderKind::Vertex)
                .then_some(ShaderKind::Vertex),
            fragment_stage: compiled
                .stages
                .contains_key(&ShaderKind::Fragment)
                .then_some(ShaderKind::Fragment),
        }]
    } else if has_compute {
        Vec::new()
    } else {
        Vec::new()
    }
}

/// Error emitted while creating a high-level shader asset.
#[derive(Debug, thiserror::Error)]
pub enum ShaderAssetBuildError {
    #[error("shader asset '{shader}' has no stages")]
    NoStages { shader: String },

    #[error(transparent)]
    Compile(#[from] ShaderCompileError),
}

/// Typed asset handles used throughout the public scene/material API.
pub type MeshHandle = AssetHandle<MeshAsset>;
pub type ImageHandle = AssetHandle<ImageAsset>;
pub type ShaderHandle = AssetHandle<ShaderAsset>;

/// In-memory asset registry for meshes, images, shaders, and materials.
#[derive(Default)]
pub struct RenderAssetLibrary {
    next_id: u64,
    meshes: BTreeMap<u64, MeshAsset>,
    images: BTreeMap<u64, ImageAsset>,
    shaders: BTreeMap<u64, ShaderAsset>,
    materials: BTreeMap<u64, crate::material::Material>,
}

impl RenderAssetLibrary {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    pub fn insert_mesh(&mut self, label: impl Into<String>, mesh: Mesh) -> MeshHandle {
        let handle = AssetHandle::new(self.allocate_id());
        self.meshes.insert(
            handle.id(),
            MeshAsset {
                label: label.into(),
                mesh,
            },
        );
        handle
    }

    pub fn insert_image(&mut self, image: ImageAsset) -> ImageHandle {
        let handle = AssetHandle::new(self.allocate_id());
        self.images.insert(handle.id(), image);
        handle
    }

    pub fn insert_shader(&mut self, shader: ShaderAsset) -> ShaderHandle {
        let handle = AssetHandle::new(self.allocate_id());
        self.shaders.insert(handle.id(), shader);
        handle
    }

    pub fn compile_shader<C: ShaderCompiler>(
        &mut self,
        compiler: &C,
        desc: &ShaderAssetDesc,
    ) -> Result<ShaderHandle, ShaderAssetBuildError> {
        let shader = desc.compile(compiler)?;
        Ok(self.insert_shader(shader))
    }

    pub fn insert_material(
        &mut self,
        material: crate::material::Material,
    ) -> crate::material::MaterialHandle {
        let handle = AssetHandle::new(self.allocate_id());
        self.materials.insert(handle.id(), material);
        handle
    }

    pub fn mesh(&self, handle: MeshHandle) -> Option<&MeshAsset> {
        self.meshes.get(&handle.id())
    }

    pub fn image(&self, handle: ImageHandle) -> Option<&ImageAsset> {
        self.images.get(&handle.id())
    }

    pub fn shader(&self, handle: ShaderHandle) -> Option<&ShaderAsset> {
        self.shaders.get(&handle.id())
    }

    pub fn material(
        &self,
        handle: crate::material::MaterialHandle,
    ) -> Option<&crate::material::Material> {
        self.materials.get(&handle.id())
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

impl ShaderAssetDesc {
    pub fn with_stage_source(mut self, kind: ShaderKind, source: ShaderCompileSource) -> Self {
        self.stages.push(StageSource {
            kind,
            source,
            entry_point: None,
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::{
        ReflectedRenderTarget, ReflectedStage, ReflectionPassthroughCompiler, ReflectionSnapshot,
    };

    #[test]
    fn asset_library_allocates_stable_typed_handles() {
        let mut library = RenderAssetLibrary::new();
        let mesh = library.insert_mesh("cube", Mesh::new());
        let image = library.insert_image(ImageAsset::new(
            "albedo",
            1,
            1,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ));

        assert_ne!(mesh.id(), image.id());
        assert_eq!(library.mesh(mesh).unwrap().label, "cube");
        assert_eq!(library.image(image).unwrap().label, "albedo");
    }

    #[test]
    fn shader_asset_uses_reflection_to_build_default_frame_graph() {
        let compiler = ReflectionPassthroughCompiler::new(ReflectionSnapshot {
            stages: vec![
                ReflectedStage::new(ShaderKind::Vertex, "vs_main"),
                ReflectedStage::new(ShaderKind::Fragment, "fs_main"),
            ],
            resources: Vec::new(),
            render_targets: vec![ReflectedRenderTarget::new("g_albedo")],
        });

        let desc = ShaderAssetDesc::slang("basic")
            .with_stage_inline(ShaderKind::Vertex, "@vertex")
            .with_stage_inline(ShaderKind::Fragment, "@fragment");
        let shader = desc.compile(&compiler).expect("shader");

        assert!(
            shader
                .frame_graph
                .plan()
                .attachments
                .contains_key(&"g_albedo".into())
        );
        assert_eq!(shader.pass_templates.len(), 1);
    }
}

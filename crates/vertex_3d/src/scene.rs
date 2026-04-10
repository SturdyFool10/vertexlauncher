//! Scene submission types for non-graphics consumers of the renderer.

use glam::{Mat4, Quat, Vec3};

use crate::{
    asset::{MeshHandle, RenderAssetLibrary},
    material::MaterialHandle,
};

/// Transform for a renderable scene object.
#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    pub fn with_translation(mut self, translation: Vec3) -> Self {
        self.translation = translation;
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }
}

/// One scene object submitted for rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderObject {
    pub mesh: MeshHandle,
    pub material: MaterialHandle,
    pub transform: Transform,
    pub visible: bool,
}

impl RenderObject {
    pub fn new(mesh: MeshHandle, material: MaterialHandle) -> Self {
        Self {
            mesh,
            material,
            transform: Transform::default(),
            visible: true,
        }
    }

    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }
}

/// Simple scene graph for renderer submission.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub objects: Vec<RenderObject>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(mut self, object: RenderObject) -> Self {
        self.objects.push(object);
        self
    }

    pub fn push(&mut self, object: RenderObject) {
        self.objects.push(object);
    }

    /// Generate a compact, GPU-friendly draw list sorted by shader/material/mesh.
    pub fn build_draw_list(&self, assets: &RenderAssetLibrary) -> Vec<DrawPacket> {
        let mut draw_list = self
            .objects
            .iter()
            .filter(|object| object.visible)
            .filter_map(|object| {
                let material = assets.material(object.material)?;
                Some(DrawPacket {
                    shader: material.shader,
                    material: object.material,
                    mesh: object.mesh,
                    transform: object.transform.matrix(),
                })
            })
            .collect::<Vec<_>>();

        draw_list
            .sort_by_key(|packet| (packet.shader.id(), packet.material.id(), packet.mesh.id()));
        draw_list
    }
}

/// Pre-sorted packet that a renderer backend can batch and submit without re-querying scene state.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawPacket {
    pub shader: crate::asset::ShaderHandle,
    pub material: MaterialHandle,
    pub mesh: MeshHandle,
    pub transform: Mat4,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset::{RenderAssetLibrary, ShaderAsset},
        material::Material,
        mesh::Mesh,
        renderer::FrameGraph,
        shader::{
            CompiledShaderProgram, ReflectedStage, ReflectionSnapshot, ShaderBackendTarget,
            ShaderKind, ShaderProgram,
        },
    };

    #[test]
    fn scene_builds_shader_sorted_draw_list() {
        let mut assets = RenderAssetLibrary::new();
        let shader = assets.insert_shader(ShaderAsset {
            label: "basic".to_string(),
            source_language: crate::ShaderSourceLanguage::Slang,
            compiled: CompiledShaderProgram {
                program: ShaderProgram::with_name("basic"),
                reflection: ReflectionSnapshot {
                    stages: vec![ReflectedStage::new(ShaderKind::Vertex, "vs_main")],
                    resources: Vec::new(),
                    render_targets: Vec::new(),
                },
                target: ShaderBackendTarget::Wgsl,
                stages: std::collections::BTreeMap::new(),
            },
            frame_graph: FrameGraph::new(),
            pass_templates: Vec::new(),
        });
        let material = assets.insert_material(Material::unlit("mat", shader));
        let mesh = assets.insert_mesh("mesh", Mesh::new());

        let mut scene = Scene::new();
        scene.push(RenderObject::new(mesh, material));
        let packets = scene.build_draw_list(&assets);

        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].shader, shader);
    }
}

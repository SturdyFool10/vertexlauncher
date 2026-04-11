//! Consumer-facing material system with optional PBR support.

use std::collections::BTreeMap;

use crate::asset::{AssetHandle, ImageHandle, ShaderHandle};

/// Typed handle to a material asset.
pub type MaterialHandle = AssetHandle<Material>;

/// Common material workflows the renderer understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialModel {
    Unlit,
    PbrMetallicRoughness,
    Custom,
}

/// Alpha behavior requested by a material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaMode {
    Opaque,
    Mask { cutoff: f32 },
    Blend,
}

/// Shared material values for custom shader parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum MaterialValue {
    Scalar(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Uint(u32),
    Bool(bool),
}

/// Image slots used by the built-in unlit and PBR material models.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterialImages {
    pub base_color: Option<ImageHandle>,
    pub metallic_roughness: Option<ImageHandle>,
    pub normal: Option<ImageHandle>,
    pub emissive: Option<ImageHandle>,
    pub occlusion: Option<ImageHandle>,
}

impl MaterialImages {
    pub fn with_base_color(mut self, handle: ImageHandle) -> Self {
        self.base_color = Some(handle);
        self
    }

    pub fn with_metallic_roughness(mut self, handle: ImageHandle) -> Self {
        self.metallic_roughness = Some(handle);
        self
    }

    pub fn with_normal(mut self, handle: ImageHandle) -> Self {
        self.normal = Some(handle);
        self
    }

    pub fn with_emissive(mut self, handle: ImageHandle) -> Self {
        self.emissive = Some(handle);
        self
    }

    pub fn with_occlusion(mut self, handle: ImageHandle) -> Self {
        self.occlusion = Some(handle);
        self
    }
}

/// Minimal unlit material parameters for users who do not need full PBR.
#[derive(Debug, Clone, PartialEq)]
pub struct UnlitMaterial {
    pub color: [f32; 4],
}

impl Default for UnlitMaterial {
    fn default() -> Self {
        Self {
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// PBR metallic-roughness material parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct PbrMaterial {
    pub base_color_factor: [f32; 4],
    pub emissive_factor: [f32; 3],
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub normal_scale: f32,
    pub occlusion_strength: f32,
    pub double_sided: bool,
}

impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            emissive_factor: [0.0, 0.0, 0.0],
            metallic_factor: 1.0,
            roughness_factor: 1.0,
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            double_sided: false,
        }
    }
}

/// Material payload used by built-in and custom shader-driven materials.
#[derive(Debug, Clone, PartialEq)]
pub enum MaterialParameters {
    Unlit(UnlitMaterial),
    Pbr(PbrMaterial),
    Custom(BTreeMap<String, MaterialValue>),
}

/// A material asset that can be referenced by scene instances.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    pub shader: ShaderHandle,
    pub model: MaterialModel,
    pub alpha_mode: AlphaMode,
    pub images: MaterialImages,
    pub parameters: MaterialParameters,
    pub casts_shadows: bool,
    pub receives_shadows: bool,
}

impl Material {
    pub fn unlit(name: impl Into<String>, shader: ShaderHandle) -> Self {
        Self {
            name: name.into(),
            shader,
            model: MaterialModel::Unlit,
            alpha_mode: AlphaMode::Opaque,
            images: MaterialImages::default(),
            parameters: MaterialParameters::Unlit(UnlitMaterial::default()),
            casts_shadows: true,
            receives_shadows: true,
        }
    }

    pub fn pbr(name: impl Into<String>, shader: ShaderHandle) -> Self {
        Self {
            name: name.into(),
            shader,
            model: MaterialModel::PbrMetallicRoughness,
            alpha_mode: AlphaMode::Opaque,
            images: MaterialImages::default(),
            parameters: MaterialParameters::Pbr(PbrMaterial::default()),
            casts_shadows: true,
            receives_shadows: true,
        }
    }

    pub fn custom(name: impl Into<String>, shader: ShaderHandle) -> Self {
        Self {
            name: name.into(),
            shader,
            model: MaterialModel::Custom,
            alpha_mode: AlphaMode::Opaque,
            images: MaterialImages::default(),
            parameters: MaterialParameters::Custom(BTreeMap::new()),
            casts_shadows: true,
            receives_shadows: true,
        }
    }

    pub fn with_alpha_mode(mut self, alpha_mode: AlphaMode) -> Self {
        self.alpha_mode = alpha_mode;
        self
    }

    pub fn with_images(mut self, images: MaterialImages) -> Self {
        self.images = images;
        self
    }

    pub fn with_unlit(mut self, unlit: UnlitMaterial) -> Self {
        self.model = MaterialModel::Unlit;
        self.parameters = MaterialParameters::Unlit(unlit);
        self
    }

    pub fn with_pbr(mut self, pbr: PbrMaterial) -> Self {
        self.model = MaterialModel::PbrMetallicRoughness;
        self.parameters = MaterialParameters::Pbr(pbr);
        self
    }

    pub fn with_custom_value(mut self, name: impl Into<String>, value: MaterialValue) -> Self {
        match &mut self.parameters {
            MaterialParameters::Custom(values) => {
                values.insert(name.into(), value);
            }
            _ => {
                let mut values = BTreeMap::new();
                values.insert(name.into(), value);
                self.parameters = MaterialParameters::Custom(values);
                self.model = MaterialModel::Custom;
            }
        }
        self
    }

    pub fn without_shadow_casting(mut self) -> Self {
        self.casts_shadows = false;
        self
    }

    pub fn without_shadow_receiving(mut self) -> Self {
        self.receives_shadows = false;
        self
    }
}

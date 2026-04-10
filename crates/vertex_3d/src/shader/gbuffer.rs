//! GBuffer types for deferred rendering with all components.

use super::hdr::{BufferPrecision, HdrConfig};

/// GBuffer components for deferred shading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GBufferType {
    /// Albedo/color buffer (RGBA).
    Albedo,
    /// Normal vectors in tangent space (RGB).
    NormalsTangent,
    /// Normal vectors in world space (RGB).
    NormalsWorld,
    /// Roughness and metallic factors (RG).
    RoughnessMetallic,
    /// Ambient occlusion (R).
    AmbientOcclusion,
    /// Depth buffer (R).
    Depth,
}

impl GBufferType {
    /// Returns the canonical name for this GBuffer component.
    pub fn name(&self) -> &'static str {
        match self {
            GBufferType::Albedo => "g_albedo",
            GBufferType::NormalsTangent => "g_normals_tangent",
            GBufferType::NormalsWorld => "g_normals_world",
            GBufferType::RoughnessMetallic => "g_roughness_metallic",
            GBufferType::AmbientOcclusion => "g_ao",
            GBufferType::Depth => "g_depth",
        }
    }

    /// Returns the wgpu texture format for this GBuffer component.
    pub fn format(&self, precision: BufferPrecision) -> wgpu::TextureFormat {
        match self {
            GBufferType::Albedo => match precision {
                BufferPrecision::FP16 => wgpu::TextureFormat::Rgba16Float,
                BufferPrecision::FP32 => wgpu::TextureFormat::Rgba32Float,
            },
            GBufferType::NormalsTangent | GBufferType::NormalsWorld => match precision {
                BufferPrecision::FP16 => wgpu::TextureFormat::Rgba16Float,
                BufferPrecision::FP32 => wgpu::TextureFormat::Rgba32Float,
            },
            GBufferType::RoughnessMetallic => match precision {
                BufferPrecision::FP16 => wgpu::TextureFormat::Rg16Float,
                BufferPrecision::FP32 => wgpu::TextureFormat::Rg32Float,
            },
            GBufferType::AmbientOcclusion => match precision {
                BufferPrecision::FP16 => wgpu::TextureFormat::R16Float,
                BufferPrecision::FP32 => wgpu::TextureFormat::R32Float,
            },
            GBufferType::Depth => wgpu::TextureFormat::Depth32Float,
        }
    }

    /// Returns the number of channels for this GBuffer component.
    pub fn channel_count(&self) -> usize {
        match self {
            GBufferType::Albedo => 4,
            GBufferType::NormalsTangent | GBufferType::NormalsWorld => 3,
            GBufferType::RoughnessMetallic => 2,
            GBufferType::AmbientOcclusion | GBufferType::Depth => 1,
        }
    }

    /// Returns true if this is a color buffer (not depth).
    pub fn is_color(&self) -> bool {
        !matches!(self, GBufferType::Depth)
    }

    /// Returns the associated render target type.
    pub fn to_render_target_type(&self) -> RenderTargetType {
        match self {
            GBufferType::Albedo => RenderTargetType::Albedo,
            GBufferType::NormalsTangent | GBufferType::NormalsWorld => RenderTargetType::Normals,
            GBufferType::RoughnessMetallic => {
                RenderTargetType::Custom("roughness_metallic".to_string())
            }
            GBufferType::AmbientOcclusion => RenderTargetType::AmbientOcclusion,
            GBufferType::Depth => RenderTargetType::Depth,
        }
    }

    /// Returns the SLANG semantic name for this GBuffer component.
    pub fn slang_semantic(&self) -> &'static str {
        match self {
            GBufferType::Albedo => "COLOR0",
            GBufferType::NormalsTangent => "COLOR1",
            GBufferType::NormalsWorld => "COLOR2",
            GBufferType::RoughnessMetallic => "COLOR3",
            GBufferType::AmbientOcclusion => "COLOR4",
            GBufferType::Depth => "DEPTH0",
        }
    }
}

/// Render target types for the shader system.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderTargetType {
    /// Main color/albedo buffer.
    Albedo,
    /// Depth buffer.
    Depth,
    /// Normal vectors (tangent space or world space).
    Normals,
    /// Motion vectors for temporal effects.
    MotionVectors,
    /// Ambient occlusion.
    AmbientOcclusion,
    /// Lighting/illumination pass.
    Lighting,
    /// Shadow maps.
    Shadows,
    /// Custom user-defined target (with name).
    Custom(String),
}

impl RenderTargetType {
    /// Returns a canonical name for this render target type.
    pub fn name(&self) -> String {
        match self {
            RenderTargetType::Albedo => "albedo".to_string(),
            RenderTargetType::Depth => "depth".to_string(),
            RenderTargetType::Normals => "normals".to_string(),
            RenderTargetType::MotionVectors => "motion_vectors".to_string(),
            RenderTargetType::AmbientOcclusion => "ao".to_string(),
            RenderTargetType::Lighting => "lighting".to_string(),
            RenderTargetType::Shadows => "shadows".to_string(),
            RenderTargetType::Custom(name) => name.clone(),
        }
    }

    /// Returns true if this is a color target.
    pub fn is_color(&self) -> bool {
        !matches!(self, RenderTargetType::Depth | RenderTargetType::Shadows)
    }

    /// Infer a target type from a reflection-provided name.
    pub fn from_reflection_name(name: &str) -> Self {
        match name {
            "albedo" | "g_albedo" | "color" | "base_color" => Self::Albedo,
            "depth" | "g_depth" => Self::Depth,
            "normals" | "normal" | "g_normals" | "g_normals_tangent" | "g_normals_world" => {
                Self::Normals
            }
            "motion_vectors" | "motion" | "velocity" => Self::MotionVectors,
            "ao" | "ambient_occlusion" | "g_ao" => Self::AmbientOcclusion,
            "lighting" | "light" => Self::Lighting,
            "shadows" | "shadow_map" => Self::Shadows,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl std::fmt::Display for RenderTargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Configuration for a single render target.
#[derive(Debug, Clone)]
pub struct RenderTargetConfig {
    /// Type of this render target.
    pub r#type: RenderTargetType,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Number of mip levels (if applicable).
    pub mip_levels: u32,
    /// Sample count for MSAA.
    pub samples: u32,
}

impl RenderTargetConfig {
    /// Create a new render target configuration.
    pub fn new(r#type: RenderTargetType, width: u32, height: u32) -> Self {
        Self {
            r#type,
            width,
            height,
            mip_levels: 1,
            samples: 1,
        }
    }

    /// Set the number of mip levels.
    pub fn with_mip_levels(mut self, levels: u32) -> Self {
        self.mip_levels = levels;
        self
    }

    /// Set the MSAA sample count.
    pub fn with_samples(mut self, samples: u32) -> Self {
        self.samples = samples;
        self
    }
}

/// GBuffer configuration helper for deferred rendering.
impl HdrConfig {
    /// Create a full GBuffer configuration for deferred rendering with all components.
    pub fn gbuffer(width: u32, height: u32) -> Vec<RenderTargetConfig> {
        vec![
            RenderTargetConfig::new(RenderTargetType::Albedo, width, height),
            RenderTargetConfig::new(RenderTargetType::Normals, width, height),
            RenderTargetConfig::new(
                RenderTargetType::Custom("roughness_metallic".to_string()),
                width,
                height,
            ),
            RenderTargetConfig::new(RenderTargetType::Depth, width, height),
        ]
    }

    /// Create a minimal GBuffer (albedo + normals only).
    pub fn gbuffer_minimal(width: u32, height: u32) -> Vec<RenderTargetConfig> {
        vec![
            RenderTargetConfig::new(RenderTargetType::Albedo, width, height),
            RenderTargetConfig::new(RenderTargetType::Normals, width, height),
        ]
    }

    /// Create an extended GBuffer with AO included.
    pub fn gbuffer_extended(width: u32, height: u32) -> Vec<RenderTargetConfig> {
        vec![
            RenderTargetConfig::new(RenderTargetType::Albedo, width, height),
            RenderTargetConfig::new(RenderTargetType::Normals, width, height),
            RenderTargetConfig::new(
                RenderTargetType::Custom("roughness_metallic".to_string()),
                width,
                height,
            ),
            RenderTargetConfig::new(RenderTargetType::AmbientOcclusion, width, height),
            RenderTargetConfig::new(RenderTargetType::Depth, width, height),
        ]
    }

    /// Create a PBR GBuffer with world-space normals.
    pub fn gbuffer_pbr(width: u32, height: u32) -> Vec<RenderTargetConfig> {
        vec![
            RenderTargetConfig::new(RenderTargetType::Albedo, width, height),
            RenderTargetConfig::new(
                RenderTargetType::Custom("normals_world".to_string()),
                width,
                height,
            ),
            RenderTargetConfig::new(
                RenderTargetType::Custom("roughness_metallic".to_string()),
                width,
                height,
            ),
            RenderTargetConfig::new(RenderTargetType::Depth, width, height),
        ]
    }

    /// Create a motion vector buffer configuration.
    pub fn motion_vector_buffer(width: u32, height: u32) -> RenderTargetConfig {
        RenderTargetConfig::new(RenderTargetType::MotionVectors, width, height)
    }

    /// Create a shadow map buffer configuration.
    pub fn shadow_map_buffer(width: u32, height: u32) -> RenderTargetConfig {
        RenderTargetConfig::new(RenderTargetType::Shadows, width, height)
    }
}

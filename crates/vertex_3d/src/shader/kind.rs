//! Shader kind enumeration with all supported shader stages.

use std::fmt;

/// Enum representing all supported shader stages.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ShaderKind {
    // ===== Core (wgpu / modern baseline) =====
    Vertex,
    Fragment,
    Compute,

    // ===== Optional / Legacy Raster =====
    Geometry,       // discouraged, but exists
    TessControl,    // aka Hull
    TessEvaluation, // aka Domain

    // ===== Modern Pipeline (Mesh Shading) =====
    Task,
    Mesh,

    // ===== Ray Tracing =====
    RayGen,
    RayClosestHit,
    RayAnyHit,
    RayMiss,
    RayIntersection,
    RayCallable,
}

impl ShaderKind {
    /// Returns the SLANG stage name for this shader kind.
    pub fn slang_name(&self) -> &'static str {
        match self {
            ShaderKind::Vertex => "vertex",
            ShaderKind::Fragment => "fragment",
            ShaderKind::Compute => "compute",
            ShaderKind::Geometry => "geometry",
            ShaderKind::TessControl => "hull",
            ShaderKind::TessEvaluation => "domain",
            ShaderKind::Task => "task",
            ShaderKind::Mesh => "mesh",
            ShaderKind::RayGen => "raygeneration",
            ShaderKind::RayClosestHit => "closesthit",
            ShaderKind::RayAnyHit => "anyhit",
            ShaderKind::RayMiss => "miss",
            ShaderKind::RayIntersection => "intersection",
            ShaderKind::RayCallable => "callable",
        }
    }

    /// Returns the wgpu shader stage flags.
    pub fn wgpu_stage_flags(&self) -> u32 {
        use wgpu::ShaderStages;
        match self {
            ShaderKind::Vertex => ShaderStages::VERTEX.bits(),
            ShaderKind::Fragment => ShaderStages::FRAGMENT.bits(),
            ShaderKind::Compute => ShaderStages::COMPUTE.bits(),
            _ => 0, // Other stages handled via extensions or custom pipelines
        }
    }

    /// Returns true if this is a rasterization stage.
    pub fn is_raster(&self) -> bool {
        matches!(
            self,
            ShaderKind::Vertex
                | ShaderKind::Fragment
                | ShaderKind::Geometry
                | ShaderKind::TessControl
                | ShaderKind::TessEvaluation
        )
    }

    /// Returns true if this is a ray tracing stage.
    pub fn is_raytracing(&self) -> bool {
        matches!(
            self,
            ShaderKind::RayGen
                | ShaderKind::RayClosestHit
                | ShaderKind::RayAnyHit
                | ShaderKind::RayMiss
                | ShaderKind::RayIntersection
                | ShaderKind::RayCallable
        )
    }

    /// Returns true if this is a mesh shading stage.
    pub fn is_mesh_shading(&self) -> bool {
        matches!(self, ShaderKind::Task | ShaderKind::Mesh)
    }

    /// Returns true if this is a tessellation stage.
    pub fn is_tessellation(&self) -> bool {
        matches!(self, ShaderKind::TessControl | ShaderKind::TessEvaluation)
    }
}

impl fmt::Display for ShaderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.slang_name())
    }
}

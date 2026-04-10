//! Shader stage wrapper with source and configuration.

use super::config::{PipelineFlags, ResourceBinding, ShaderStageConfig};
use super::kind::ShaderKind;

/// A single shader stage with its source and metadata.
#[derive(Debug, Clone)]
pub struct ShaderStage {
    /// The kind of this shader stage.
    pub kind: ShaderKind,
    /// The raw SLANG source code.
    pub source: String,
    /// Configuration for this stage.
    pub config: ShaderStageConfig,
}

impl ShaderStage {
    // ========================================================================
    // Core Constructors - Rasterization Pipeline
    // ========================================================================

    /// Create a new vertex shader stage.
    pub fn vertex(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::Vertex, source, ShaderStageConfig::default())
    }

    /// Create a new fragment shader stage.
    pub fn fragment(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::Fragment, source, ShaderStageConfig::default())
    }

    /// Create a new compute shader stage.
    pub fn compute(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::Compute, source, ShaderStageConfig::default())
    }

    // ========================================================================
    // Optional / Legacy Rasterization Stages
    // ========================================================================

    /// Create a new geometry shader stage.
    pub fn geometry(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::Geometry, source, ShaderStageConfig::default())
    }

    /// Create a new tessellation control (hull) shader stage.
    pub fn tessellation_control(source: impl Into<String>) -> Self {
        Self::new_with_config(
            ShaderKind::TessControl,
            source,
            ShaderStageConfig::default(),
        )
    }

    /// Create a new tessellation evaluation (domain) shader stage.
    pub fn tessellation_evaluation(source: impl Into<String>) -> Self {
        Self::new_with_config(
            ShaderKind::TessEvaluation,
            source,
            ShaderStageConfig::default(),
        )
    }

    // ========================================================================
    // Modern Pipeline - Mesh Shading Stages
    // ========================================================================

    /// Create a new task shader stage (mesh shading).
    pub fn task(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::Task, source, ShaderStageConfig::default())
    }

    /// Create a new mesh shader stage.
    pub fn mesh(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::Mesh, source, ShaderStageConfig::default())
    }

    // ========================================================================
    // Ray Tracing Stages
    // ========================================================================

    /// Create a new ray generation shader stage.
    pub fn ray_generation(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::RayGen, source, ShaderStageConfig::default())
    }

    /// Create a new intersection shader stage.
    pub fn intersection(source: impl Into<String>) -> Self {
        Self::new_with_config(
            ShaderKind::RayIntersection,
            source,
            ShaderStageConfig::default(),
        )
    }

    /// Create a new any hit shader stage.
    pub fn any_hit(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::RayAnyHit, source, ShaderStageConfig::default())
    }

    /// Create a new closest hit shader stage.
    pub fn closest_hit(source: impl Into<String>) -> Self {
        Self::new_with_config(
            ShaderKind::RayClosestHit,
            source,
            ShaderStageConfig::default(),
        )
    }

    /// Create a new miss shader stage.
    pub fn miss(source: impl Into<String>) -> Self {
        Self::new_with_config(ShaderKind::RayMiss, source, ShaderStageConfig::default())
    }

    /// Create a new callable shader stage.
    pub fn callable(source: impl Into<String>) -> Self {
        Self::new_with_config(
            ShaderKind::RayCallable,
            source,
            ShaderStageConfig::default(),
        )
    }

    // ========================================================================
    // Generic Constructors
    // ========================================================================

    /// Generic constructor for any shader kind with default config.
    pub fn new(kind: ShaderKind, source: impl Into<String>) -> Self {
        Self::new_with_config(kind, source, ShaderStageConfig::default())
    }

    /// Constructor with custom configuration.
    pub fn new_with_config(
        kind: ShaderKind,
        source: impl Into<String>,
        config: ShaderStageConfig,
    ) -> Self {
        Self {
            kind,
            source: source.into(),
            config,
        }
    }

    // ========================================================================
    // Builder Methods - Configuration
    // ========================================================================

    /// Set the entry point function name.
    pub fn with_entry_point(mut self, entry_point: impl Into<String>) -> Self {
        self.config.entry_point = Some(entry_point.into());
        self
    }

    /// Add a specialization constant.
    pub fn with_specialization_constant(mut self, name: impl Into<String>, value: u32) -> Self {
        self.config
            .specialization_constants
            .insert(name.into(), value);
        self
    }

    /// Set that this stage writes to GBuffer (for deferred rendering).
    pub fn writes_gbuffer(mut self) -> Self {
        self.config.writes_gbuffer = true;
        self
    }

    /// Add a resource binding.
    pub fn with_resource_binding(mut self, binding: ResourceBinding) -> Self {
        self.config.resource_bindings.push(binding);
        self
    }

    /// Set pipeline flags for this stage.
    pub fn with_pipeline_flags(mut self, flags: PipelineFlags) -> Self {
        self.config.pipeline_flags = flags;
        self
    }

    // ========================================================================
    // Accessors
    // ========================================================================

    /// Get the entry point if set.
    pub fn entry_point(&self) -> Option<&str> {
        self.config.entry_point.as_deref()
    }

    /// Check if this stage writes to GBuffer.
    pub fn writes_to_gbuffer(&self) -> bool {
        self.config.writes_gbuffer
    }

    /// Get all resource bindings for this stage.
    pub fn resource_bindings(&self) -> &[ResourceBinding] {
        &self.config.resource_bindings
    }

    /// Get the pipeline flags for this stage.
    pub fn pipeline_flags(&self) -> PipelineFlags {
        self.config.pipeline_flags
    }

    // ========================================================================
    // Type Conversion Methods
    // ========================================================================

    /// Convert to vertex shader kind.
    pub fn as_vertex(self) -> Option<Self> {
        if self.kind == ShaderKind::Vertex {
            Some(self)
        } else {
            None
        }
    }

    /// Convert to fragment shader kind.
    pub fn as_fragment(self) -> Option<Self> {
        if self.kind == ShaderKind::Fragment {
            Some(self)
        } else {
            None
        }
    }

    /// Convert to compute shader kind.
    pub fn as_compute(self) -> Option<Self> {
        if self.kind == ShaderKind::Compute {
            Some(self)
        } else {
            None
        }
    }
}

impl From<String> for ShaderStage {
    fn from(source: String) -> Self {
        // Default to fragment shader - caller should use specific constructors
        Self::fragment(source)
    }
}

impl From<&str> for ShaderStage {
    fn from(source: &str) -> Self {
        Self::from(source.to_string())
    }
}

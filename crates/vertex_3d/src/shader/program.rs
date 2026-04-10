//! Shader program combining all stages, HDR config, and render targets.

use std::collections::HashMap;

use super::gbuffer::{RenderTargetConfig, RenderTargetType};
use super::hdr::HdrConfig;
use super::kind::ShaderKind;
use super::reflection::ReflectionSnapshot;
use super::stage::ShaderStage;

/// A complete shader program containing all stages and configuration.
#[derive(Debug, Clone)]
pub struct ShaderProgram {
    /// Name/identifier for this shader program.
    pub name: String,
    /// All shader stages in this program.
    pub stages: HashMap<ShaderKind, ShaderStage>,
    /// HDR rendering configuration.
    pub hdr_config: HdrConfig,
    /// Render target configurations (albedo, depth, normals, etc.).
    pub render_targets: Vec<RenderTargetConfig>,
}

impl ShaderProgram {
    /// Create a new empty shader program with default name.
    pub fn new() -> Self {
        Self::with_name("default")
    }

    /// Create a new shader program with the given name.
    pub fn with_name(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            stages: HashMap::new(),
            hdr_config: HdrConfig::default(),
            render_targets: Vec::new(),
        }
    }

    // ========================================================================
    // Builder Methods - Core Stages (Rasterization)
    // ========================================================================

    /// Add a vertex shader stage.
    pub fn with_vertex(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::Vertex, ShaderStage::vertex(source));
        self
    }

    /// Add a fragment shader stage.
    pub fn with_fragment(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::Fragment, ShaderStage::fragment(source));
        self
    }

    /// Add a compute shader stage.
    pub fn with_compute(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::Compute, ShaderStage::compute(source));
        self
    }

    // ========================================================================
    // Builder Methods - Optional / Legacy Rasterization Stages
    // ========================================================================

    /// Add a geometry shader stage.
    pub fn with_geometry(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::Geometry, ShaderStage::geometry(source));
        self
    }

    /// Add a tessellation control (hull) shader stage.
    pub fn with_tessellation_control(mut self, source: impl Into<String>) -> Self {
        self.stages.insert(
            ShaderKind::TessControl,
            ShaderStage::tessellation_control(source),
        );
        self
    }

    /// Add a tessellation evaluation (domain) shader stage.
    pub fn with_tessellation_evaluation(mut self, source: impl Into<String>) -> Self {
        self.stages.insert(
            ShaderKind::TessEvaluation,
            ShaderStage::tessellation_evaluation(source),
        );
        self
    }

    // ========================================================================
    // Builder Methods - Mesh Shading Stages
    // ========================================================================

    /// Add a task shader stage (mesh shading).
    pub fn with_task(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::Task, ShaderStage::task(source));
        self
    }

    /// Add a mesh shader stage.
    pub fn with_mesh(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::Mesh, ShaderStage::mesh(source));
        self
    }

    // ========================================================================
    // Builder Methods - Ray Tracing Stages
    // ========================================================================

    /// Add a ray generation shader stage.
    pub fn with_ray_generation(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::RayGen, ShaderStage::ray_generation(source));
        self
    }

    /// Add an intersection shader stage.
    pub fn with_intersection(mut self, source: impl Into<String>) -> Self {
        self.stages.insert(
            ShaderKind::RayIntersection,
            ShaderStage::intersection(source),
        );
        self
    }

    /// Add an any hit shader stage.
    pub fn with_any_hit(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::RayAnyHit, ShaderStage::any_hit(source));
        self
    }

    /// Add a closest hit shader stage.
    pub fn with_closest_hit(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::RayClosestHit, ShaderStage::closest_hit(source));
        self
    }

    /// Add a miss shader stage.
    pub fn with_miss(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::RayMiss, ShaderStage::miss(source));
        self
    }

    /// Add a callable shader stage.
    pub fn with_callable(mut self, source: impl Into<String>) -> Self {
        self.stages
            .insert(ShaderKind::RayCallable, ShaderStage::callable(source));
        self
    }

    // ========================================================================
    // Builder Methods - Configuration
    // ========================================================================

    /// Set the HDR configuration for this shader program.
    pub fn with_hdr_config(mut self, config: HdrConfig) -> Self {
        self.hdr_config = config;
        self
    }

    /// Add a render target to this shader program.
    pub fn with_render_target(mut self, config: RenderTargetConfig) -> Self {
        self.render_targets.push(config);
        self
    }

    /// Add multiple render targets at once.
    pub fn with_render_targets(mut self, configs: Vec<RenderTargetConfig>) -> Self {
        self.render_targets.extend(configs);
        self
    }

    /// Merge normalized reflection data into stage metadata and render target declarations.
    pub fn with_reflection(mut self, reflection: &ReflectionSnapshot) -> Self {
        self.apply_reflection(reflection);
        self
    }

    // ========================================================================
    // Builder Methods - Convenience Presets
    // ========================================================================

    /// Configure for full GBuffer deferred rendering.
    pub fn with_gbuffer(mut self, width: u32, height: u32) -> Self {
        self.render_targets = HdrConfig::gbuffer(width, height);
        self
    }

    /// Configure for minimal GBuffer (albedo + normals only).
    pub fn with_gbuffer_minimal(mut self, width: u32, height: u32) -> Self {
        self.render_targets = HdrConfig::gbuffer_minimal(width, height);
        self
    }

    /// Configure for extended GBuffer with AO.
    pub fn with_gbuffer_extended(mut self, width: u32, height: u32) -> Self {
        self.render_targets = HdrConfig::gbuffer_extended(width, height);
        self
    }

    /// Configure for PBR GBuffer with world-space normals.
    pub fn with_gbuffer_pbr(mut self, width: u32, height: u32) -> Self {
        self.render_targets = HdrConfig::gbuffer_pbr(width, height);
        self
    }

    /// Configure for HDR10 output at specified brightness.
    pub fn with_hdr10(mut self, max_nits: u32) -> Self {
        self.hdr_config = HdrConfig::hdr10(max_nits);
        self
    }

    /// Configure for Dolby Vision output.
    pub fn with_dolby_vision(mut self, max_nits: u32) -> Self {
        self.hdr_config = HdrConfig::dolby_vision(max_nits);
        self
    }

    /// Configure for linear HDR (no tone mapping).
    pub fn with_linear_hdr(mut self) -> Self {
        self.hdr_config = HdrConfig::linear_hdr();
        self
    }

    // ========================================================================
    // Accessors - Shader Stages
    // ========================================================================

    /// Get a reference to a specific shader stage by kind.
    pub fn get_stage(&self, kind: ShaderKind) -> Option<&ShaderStage> {
        self.stages.get(&kind)
    }

    /// Get mutable access to a specific shader stage.
    pub fn get_stage_mut(&mut self, kind: ShaderKind) -> Option<&mut ShaderStage> {
        self.stages.get_mut(&kind)
    }

    /// Check if this program has the specified shader stage.
    pub fn has_stage(&self, kind: ShaderKind) -> bool {
        self.stages.contains_key(&kind)
    }

    /// Get all shader stages as a reference to the HashMap.
    pub fn stages(&self) -> &HashMap<ShaderKind, ShaderStage> {
        &self.stages
    }

    // ========================================================================
    // Accessors - Render Targets
    // ========================================================================

    /// Get all render targets of a specific type.
    pub fn get_render_targets_of_type(&self, r#type: RenderTargetType) -> Vec<&RenderTargetConfig> {
        self.render_targets
            .iter()
            .filter(|rt| rt.r#type == r#type)
            .collect()
    }

    /// Get the albedo render target if present.
    pub fn get_albedo_target(&self) -> Option<&RenderTargetConfig> {
        self.render_targets
            .iter()
            .find(|rt| rt.r#type == RenderTargetType::Albedo)
    }

    /// Get the depth render target if present.
    pub fn get_depth_target(&self) -> Option<&RenderTargetConfig> {
        self.render_targets
            .iter()
            .find(|rt| rt.r#type == RenderTargetType::Depth)
    }

    // ========================================================================
    // Validation Methods
    // ========================================================================

    /// Check if this program has all required stages for rasterization.
    pub fn has_raster_stages(&self) -> bool {
        self.stages.contains_key(&ShaderKind::Vertex)
            && self.stages.contains_key(&ShaderKind::Fragment)
    }

    /// Check if this program is a compute-only shader.
    pub fn is_compute_only(&self) -> bool {
        self.stages.len() == 1 && self.stages.contains_key(&ShaderKind::Compute)
    }

    /// Check if this program has all required stages for ray tracing.
    pub fn has_raytracing_stages(&self) -> bool {
        self.stages.contains_key(&ShaderKind::RayGen)
            && (self.stages.contains_key(&ShaderKind::RayClosestHit)
                || self.stages.contains_key(&ShaderKind::RayMiss))
    }

    /// Check if this program has all required stages for mesh shading.
    pub fn has_mesh_shading_stages(&self) -> bool {
        self.stages.contains_key(&ShaderKind::Task) && self.stages.contains_key(&ShaderKind::Mesh)
    }

    /// Validate the shader program configuration.
    pub fn validate(&self) -> Result<(), ShaderProgramError> {
        // Check for required stages based on pipeline type
        if self.is_compute_only() {
            return Ok(());
        }

        if self.has_raster_stages() && !self.is_compute_only() {
            // Rasterization requires vertex + fragment
            if !self.stages.contains_key(&ShaderKind::Vertex) {
                return Err(ShaderProgramError::MissingStage {
                    stage: ShaderKind::Vertex,
                });
            }
            if !self.stages.contains_key(&ShaderKind::Fragment) {
                return Err(ShaderProgramError::MissingStage {
                    stage: ShaderKind::Fragment,
                });
            }
        }

        // Check tessellation requirements
        if self.has_stage(ShaderKind::TessControl) && !self.has_stage(ShaderKind::TessEvaluation) {
            return Err(ShaderProgramError::IncompatibleStages {
                stages: vec![ShaderKind::TessControl, ShaderKind::TessEvaluation],
            });
        }

        // Check mesh shading requirements
        if self.has_stage(ShaderKind::Task) && !self.has_stage(ShaderKind::Mesh) {
            return Err(ShaderProgramError::IncompatibleStages {
                stages: vec![ShaderKind::Task, ShaderKind::Mesh],
            });
        }

        Ok(())
    }

    /// Apply a reflection snapshot to this program in-place.
    pub fn apply_reflection(&mut self, reflection: &ReflectionSnapshot) {
        for stage in &reflection.stages {
            let reflected_stage = self
                .stages
                .entry(stage.kind)
                .or_insert_with(|| ShaderStage::new(stage.kind, ""));
            reflected_stage.config.entry_point = Some(stage.entry_point.clone());
            reflected_stage.config.writes_gbuffer = stage.writes_gbuffer;
            reflected_stage.config.pipeline_flags = stage.flags();
            reflected_stage.config.resource_bindings = reflection
                .resources_for_stage(stage.kind)
                .map(|resource| resource.to_resource_binding())
                .collect();
        }

        let reflected_targets = reflection.inferred_render_targets((1, 1));
        if !reflected_targets.is_empty() {
            self.render_targets = reflected_targets;
        }
    }
}

/// Errors related to shader program validation.
#[derive(Debug, thiserror::Error)]
pub enum ShaderProgramError {
    #[error("Missing required shader stage: {stage:?}")]
    MissingStage { stage: ShaderKind },

    #[error("Incompatible shader stages: {stages:?}")]
    IncompatibleStages { stages: Vec<ShaderKind> },

    #[error("Invalid HDR configuration: {message}")]
    InvalidHdrConfig { message: String },

    #[error("Render target configuration error: {message}")]
    RenderTargetError { message: String },
}

impl Default for ShaderProgram {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raster_pipeline() {
        let program = ShaderProgram::with_name("raster")
            .with_vertex("vertex shader source")
            .with_fragment("fragment shader source");

        assert!(program.has_raster_stages());
        assert!(!program.is_compute_only());
        program.validate().unwrap();
    }

    #[test]
    fn test_compute_pipeline() {
        let program = ShaderProgram::with_name("compute").with_compute("compute shader source");

        assert!(program.is_compute_only());
        assert!(!program.has_raster_stages());
        program.validate().unwrap();
    }

    #[test]
    fn test_gbuffer_configuration() {
        let program = ShaderProgram::with_name("gbuffer")
            .with_vertex("vertex source")
            .with_fragment("fragment source")
            .with_gbuffer(1920, 1080);

        assert_eq!(program.render_targets.len(), 4);
        assert!(program.get_albedo_target().is_some());
    }

    #[test]
    fn test_hdr_configuration() {
        let program = ShaderProgram::with_name("hdr")
            .with_vertex("vertex source")
            .with_fragment("fragment source")
            .with_hdr10(1000);

        assert!(program.hdr_config.is_hdr());
        assert_eq!(program.hdr_config.max_brightness_nits, 1000);
    }
}

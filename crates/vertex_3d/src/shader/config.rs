//! Shader stage configuration with massive flexibility options.

use std::collections::HashMap;

/// Configuration for a shader stage with massive flexibility.
#[derive(Debug, Clone)]
pub struct ShaderStageConfig {
    /// Entry point function name.
    pub entry_point: Option<String>,
    /// Specialization constants (SLANG compile-time parameters).
    pub specialization_constants: HashMap<String, u32>,
    /// Resource bindings for this stage.
    pub resource_bindings: Vec<ResourceBinding>,
    /// Whether this stage writes to GBuffer (for deferred rendering).
    pub writes_gbuffer: bool,
    /// Custom pipeline flags for this stage.
    pub pipeline_flags: PipelineFlags,
}

impl Default for ShaderStageConfig {
    fn default() -> Self {
        Self {
            entry_point: None,
            specialization_constants: HashMap::new(),
            resource_bindings: Vec::new(),
            writes_gbuffer: false,
            pipeline_flags: PipelineFlags::default(),
        }
    }
}

impl ShaderStageConfig {
    /// Create a new default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the entry point function name.
    pub fn with_entry_point(mut self, entry_point: impl Into<String>) -> Self {
        self.entry_point = Some(entry_point.into());
        self
    }

    /// Add a specialization constant.
    pub fn with_specialization_constant(mut self, name: impl Into<String>, value: u32) -> Self {
        self.specialization_constants.insert(name.into(), value);
        self
    }

    /// Set that this stage writes to GBuffer (for deferred rendering).
    pub fn writes_gbuffer(mut self) -> Self {
        self.writes_gbuffer = true;
        self
    }

    /// Add a resource binding.
    pub fn with_resource_binding(mut self, binding: ResourceBinding) -> Self {
        self.resource_bindings.push(binding);
        self
    }

    /// Set pipeline flags for this stage.
    pub fn with_pipeline_flags(mut self, flags: PipelineFlags) -> Self {
        self.pipeline_flags = flags;
        self
    }
}

/// Resource binding for a shader stage.
#[derive(Debug, Clone)]
pub struct ResourceBinding {
    /// Name of the resource in SLANG.
    pub name: String,
    /// Binding slot/index.
    pub slot: u32,
    /// Descriptor set / register space.
    pub space: u32,
    /// Resource type (uniform buffer, texture, sampler).
    pub r#type: ResourceType,
}

impl ResourceBinding {
    /// Create a new resource binding.
    pub fn new(name: impl Into<String>, slot: u32, r#type: ResourceType) -> Self {
        Self {
            name: name.into(),
            slot,
            space: 0,
            r#type,
        }
    }

    /// Set the descriptor set / register space.
    pub fn with_space(mut self, space: u32) -> Self {
        self.space = space;
        self
    }
}

/// Type of shader resource.
#[derive(Debug, Clone, Copy)]
pub enum ResourceType {
    UniformBuffer,
    StorageBuffer,
    Texture,
    Sampler,
    CombinedTextureSampler,
}

impl ResourceType {
    /// Returns the SLANG type name for this resource type.
    pub fn slang_name(&self) -> &'static str {
        match self {
            ResourceType::UniformBuffer => "uniform_buffer",
            ResourceType::StorageBuffer => "storage_buffer",
            ResourceType::Texture => "texture",
            ResourceType::Sampler => "sampler",
            ResourceType::CombinedTextureSampler => "combined_texture_sampler",
        }
    }
}

/// Pipeline flags for fine-grained control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineFlags(u32);

impl PipelineFlags {
    pub const NONE: Self = Self(0);
    pub const WRITE_DEPTH: Self = Self(1 << 0);
    pub const READ_DEPTH: Self = Self(1 << 1);
    pub const SAMPLE_MSAA: Self = Self(1 << 2);
    pub const HDR_OUTPUT: Self = Self(1 << 3);

    pub fn bits(self) -> u32 {
        self.0
    }

    pub fn from_bits(bits: u32) -> Self {
        Self(bits)
    }
}

impl Default for PipelineFlags {
    fn default() -> Self {
        PipelineFlags::WRITE_DEPTH
    }
}

impl std::ops::BitOr for PipelineFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitAnd for PipelineFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl std::fmt::Display for PipelineFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PipelineFlags({})", self.0)
    }
}

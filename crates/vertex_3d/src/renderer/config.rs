//! Renderer configuration and rebuild tracking for a flexible deferred renderer.

use std::collections::BTreeMap;

use super::adapter_selection::AdapterSelector;
use crate::shader::{
    BufferPrecision, HdrConfig, ReflectionSnapshot, RenderTargetConfig, RenderTargetType,
    ShaderProgram,
};

/// High-level surface configuration independent of any specific windowing backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceConfig {
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub present_mode: wgpu::PresentMode,
    pub alpha_mode: wgpu::CompositeAlphaMode,
}

impl SurfaceConfig {
    pub fn new(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
            format,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self::new(1280, 720, wgpu::TextureFormat::Bgra8UnormSrgb)
    }
}

/// Preferred adapter selection policy for renderer initialization or hot-swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdapterPreference {
    #[default]
    Default,
    HighPerformance,
    LowPower,
    DiscreteOnly,
    IntegratedOnly,
}

impl AdapterPreference {
    pub fn power_preference(self) -> wgpu::PowerPreference {
        match self {
            AdapterPreference::Default => wgpu::PowerPreference::default(),
            AdapterPreference::HighPerformance | AdapterPreference::DiscreteOnly => {
                wgpu::PowerPreference::HighPerformance
            }
            AdapterPreference::LowPower | AdapterPreference::IntegratedOnly => {
                wgpu::PowerPreference::LowPower
            }
        }
    }
}

/// Scaling policy for attachments relative to the current output surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RenderTargetScale {
    Full,
    Half,
    Quarter,
    Fixed(u32, u32),
    Dynamic(f32),
}

impl Default for RenderTargetScale {
    fn default() -> Self {
        Self::Full
    }
}

impl RenderTargetScale {
    pub fn resolve(self, surface: &SurfaceConfig) -> (u32, u32) {
        let (width, height) = surface.size();
        match self {
            RenderTargetScale::Full => (width, height),
            RenderTargetScale::Half => ((width / 2).max(1), (height / 2).max(1)),
            RenderTargetScale::Quarter => ((width / 4).max(1), (height / 4).max(1)),
            RenderTargetScale::Fixed(width, height) => (width.max(1), height.max(1)),
            RenderTargetScale::Dynamic(factor) => (
                (width as f32 * factor).round().max(1.0) as u32,
                (height as f32 * factor).round().max(1.0) as u32,
            ),
        }
    }
}

/// Lifecycle hint for attachments the graph may want to keep across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentLifecycle {
    #[default]
    Persistent,
    Transient,
    History,
}

/// Stable identifier for a render target produced or consumed by the graph.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RenderTargetHandle(pub String);

impl RenderTargetHandle {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for RenderTargetHandle {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for RenderTargetHandle {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Shader-graph derived attachment metadata.
#[derive(Debug, Clone, Default)]
pub struct ShaderGraphDescriptor {
    pub attachments: BTreeMap<RenderTargetHandle, GraphAttachment>,
}

impl ShaderGraphDescriptor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_attachment(mut self, attachment: GraphAttachment) -> Self {
        self.attachments
            .insert(attachment.handle.clone(), attachment);
        self
    }

    pub fn inferred_from_program(
        program: &ShaderProgram,
        default_scale: RenderTargetScale,
    ) -> Self {
        let mut graph = Self::new();
        for target in &program.render_targets {
            graph = graph.with_attachment(GraphAttachment::from_render_target_config(
                target.clone(),
                default_scale,
            ));
        }
        graph
    }

    pub fn from_reflection(
        reflection: &ReflectionSnapshot,
        default_scale: RenderTargetScale,
        fallback_size: (u32, u32),
    ) -> Self {
        let mut graph = Self::new();
        for attachment in reflection.inferred_graph_attachments(default_scale, fallback_size) {
            graph = graph.with_attachment(attachment);
        }
        graph
    }
}

/// Attachment metadata that can be filled from Slang reflection.
#[derive(Debug, Clone)]
pub struct GraphAttachment {
    pub handle: RenderTargetHandle,
    pub target: RenderTargetConfig,
    pub scale: RenderTargetScale,
    pub lifecycle: AttachmentLifecycle,
    pub allow_format_override: bool,
}

impl GraphAttachment {
    pub fn new(handle: impl Into<RenderTargetHandle>, target: RenderTargetConfig) -> Self {
        Self {
            handle: handle.into(),
            target,
            scale: RenderTargetScale::Full,
            lifecycle: AttachmentLifecycle::Persistent,
            allow_format_override: true,
        }
    }

    pub fn from_render_target_config(target: RenderTargetConfig, scale: RenderTargetScale) -> Self {
        Self {
            handle: RenderTargetHandle::new(target.r#type.name()),
            target,
            scale,
            lifecycle: AttachmentLifecycle::Persistent,
            allow_format_override: true,
        }
    }

    pub fn with_scale(mut self, scale: RenderTargetScale) -> Self {
        self.scale = scale;
        self
    }

    pub fn with_lifecycle(mut self, lifecycle: AttachmentLifecycle) -> Self {
        self.lifecycle = lifecycle;
        self
    }
}

/// User-controlled renderer settings. Changes are diffed into rebuild flags.
#[derive(Debug, Clone)]
pub struct RendererConfig {
    pub surface: SurfaceConfig,
    pub hdr: HdrConfig,
    pub msaa_samples: u32,
    pub depth_format: wgpu::TextureFormat,
    pub adapter_preference: AdapterPreference,
    pub adapter_selector: AdapterSelector,
    pub fallback_backends: wgpu::Backends,
    pub default_target_scale: RenderTargetScale,
    pub graph: ShaderGraphDescriptor,
    pub format_overrides: BTreeMap<RenderTargetHandle, wgpu::TextureFormat>,
}

impl RendererConfig {
    pub fn new(surface: SurfaceConfig) -> Self {
        Self {
            surface,
            hdr: HdrConfig::default(),
            msaa_samples: 1,
            depth_format: wgpu::TextureFormat::Depth32Float,
            adapter_preference: AdapterPreference::Default,
            adapter_selector: AdapterSelector::Preference(AdapterPreference::Default),
            fallback_backends: wgpu::Backends::all(),
            default_target_scale: RenderTargetScale::Full,
            graph: ShaderGraphDescriptor::default(),
            format_overrides: BTreeMap::new(),
        }
    }

    pub fn for_program(surface: SurfaceConfig, program: &ShaderProgram) -> Self {
        let mut config = Self::new(surface);
        config.hdr = program.hdr_config.clone();
        config.graph =
            ShaderGraphDescriptor::inferred_from_program(program, config.default_target_scale);
        config
    }

    pub fn for_reflection(surface: SurfaceConfig, reflection: &ReflectionSnapshot) -> Self {
        let mut config = Self::new(surface.clone());
        config.graph = ShaderGraphDescriptor::from_reflection(
            reflection,
            config.default_target_scale,
            surface.size(),
        );
        config
    }

    pub fn with_msaa_samples(mut self, samples: u32) -> Self {
        self.msaa_samples = samples.max(1);
        self
    }

    pub fn with_adapter_preference(mut self, preference: AdapterPreference) -> Self {
        self.adapter_preference = preference;
        self.adapter_selector = AdapterSelector::Preference(preference);
        self
    }

    pub fn with_adapter_selector(mut self, selector: AdapterSelector) -> Self {
        if let AdapterSelector::Preference(preference) = selector {
            self.adapter_preference = preference;
        }
        self.adapter_selector = selector;
        self
    }

    pub fn with_default_target_scale(mut self, scale: RenderTargetScale) -> Self {
        self.default_target_scale = scale;
        self
    }

    pub fn with_graph(mut self, graph: ShaderGraphDescriptor) -> Self {
        self.graph = graph;
        self
    }

    pub fn set_format_override(
        &mut self,
        handle: impl Into<RenderTargetHandle>,
        format: wgpu::TextureFormat,
    ) {
        self.format_overrides.insert(handle.into(), format);
    }

    pub fn set_resolution(&mut self, width: u32, height: u32) {
        self.surface.width = width.max(1);
        self.surface.height = height.max(1);
    }
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self::new(SurfaceConfig::default())
    }
}

/// Bitflags describing what must be recreated after a config mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RendererRebuildFlags(u32);

impl RendererRebuildFlags {
    pub const NONE: Self = Self(0);
    pub const SURFACE: Self = Self(1 << 0);
    pub const ATTACHMENTS: Self = Self(1 << 1);
    pub const PIPELINES: Self = Self(1 << 2);
    pub const DEVICE: Self = Self(1 << 3);
    pub const SHADER_GRAPH: Self = Self(1 << 4);

    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for RendererRebuildFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for RendererRebuildFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Concrete attachment plan derived from renderer config and shader graph.
#[derive(Debug, Clone, Default)]
pub struct DerivedRendererState {
    pub attachments: BTreeMap<RenderTargetHandle, ResolvedAttachment>,
}

impl DerivedRendererState {
    pub fn attachment(&self, handle: &RenderTargetHandle) -> Option<&ResolvedAttachment> {
        self.attachments.get(handle)
    }
}

/// Fully resolved attachment info ready for GPU texture creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAttachment {
    pub handle: RenderTargetHandle,
    pub target_type: RenderTargetType,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub samples: u32,
    pub mip_levels: u32,
    pub lifecycle: AttachmentLifecycle,
}

/// Mutable runtime state that tracks dirty regions and rebuild requirements.
#[derive(Debug, Clone)]
pub struct RendererRuntime {
    config: RendererConfig,
    derived: DerivedRendererState,
    pending_rebuild: RendererRebuildFlags,
}

impl RendererRuntime {
    pub fn new(config: RendererConfig) -> Self {
        let derived = Self::derive_state(&config);
        Self {
            config,
            derived,
            pending_rebuild: RendererRebuildFlags::SURFACE
                | RendererRebuildFlags::ATTACHMENTS
                | RendererRebuildFlags::PIPELINES,
        }
    }

    pub fn config(&self) -> &RendererConfig {
        &self.config
    }

    pub fn derived(&self) -> &DerivedRendererState {
        &self.derived
    }

    pub fn pending_rebuild(&self) -> RendererRebuildFlags {
        self.pending_rebuild
    }

    pub fn clear_rebuild_flags(&mut self) {
        self.pending_rebuild = RendererRebuildFlags::NONE;
    }

    pub fn resize(&mut self, width: u32, height: u32) -> RendererRebuildFlags {
        if self.config.surface.width == width.max(1) && self.config.surface.height == height.max(1)
        {
            return RendererRebuildFlags::NONE;
        }
        self.config.set_resolution(width, height);
        self.rederive(RendererRebuildFlags::SURFACE | RendererRebuildFlags::ATTACHMENTS)
    }

    pub fn set_surface_format(&mut self, format: wgpu::TextureFormat) -> RendererRebuildFlags {
        if self.config.surface.format == format {
            return RendererRebuildFlags::NONE;
        }
        self.config.surface.format = format;
        self.rederive(
            RendererRebuildFlags::SURFACE
                | RendererRebuildFlags::ATTACHMENTS
                | RendererRebuildFlags::PIPELINES,
        )
    }

    pub fn set_hdr_config(&mut self, hdr: HdrConfig) -> RendererRebuildFlags {
        if self.config.hdr.internal_precision == hdr.internal_precision
            && self.config.hdr.output_colorspace == hdr.output_colorspace
            && self.config.hdr.max_brightness_nits == hdr.max_brightness_nits
            && self.config.hdr.min_brightness_nits == hdr.min_brightness_nits
        {
            return RendererRebuildFlags::NONE;
        }
        self.config.hdr = hdr;
        self.rederive(RendererRebuildFlags::ATTACHMENTS | RendererRebuildFlags::PIPELINES)
    }

    pub fn set_adapter_preference(
        &mut self,
        preference: AdapterPreference,
    ) -> RendererRebuildFlags {
        if self.config.adapter_preference == preference {
            return RendererRebuildFlags::NONE;
        }
        self.config.adapter_preference = preference;
        self.config.adapter_selector = AdapterSelector::Preference(preference);
        self.pending_rebuild |= RendererRebuildFlags::DEVICE
            | RendererRebuildFlags::ATTACHMENTS
            | RendererRebuildFlags::PIPELINES;
        self.pending_rebuild
    }

    pub fn set_adapter_selector(&mut self, selector: AdapterSelector) -> RendererRebuildFlags {
        if self.config.adapter_selector == selector {
            return RendererRebuildFlags::NONE;
        }
        if let AdapterSelector::Preference(preference) = selector {
            self.config.adapter_preference = preference;
        }
        self.config.adapter_selector = selector;
        self.pending_rebuild |= RendererRebuildFlags::DEVICE
            | RendererRebuildFlags::ATTACHMENTS
            | RendererRebuildFlags::PIPELINES;
        self.pending_rebuild
    }

    pub fn replace_graph(&mut self, graph: ShaderGraphDescriptor) -> RendererRebuildFlags {
        self.config.graph = graph;
        self.rederive(
            RendererRebuildFlags::SHADER_GRAPH
                | RendererRebuildFlags::ATTACHMENTS
                | RendererRebuildFlags::PIPELINES,
        )
    }

    pub fn sync_program(&mut self, program: &ShaderProgram) -> RendererRebuildFlags {
        let mut flags = RendererRebuildFlags::NONE;
        flags |= self.set_hdr_config(program.hdr_config.clone());
        let graph =
            ShaderGraphDescriptor::inferred_from_program(program, self.config.default_target_scale);
        flags |= self.replace_graph(graph);
        flags
    }

    pub fn sync_reflection(&mut self, reflection: &ReflectionSnapshot) -> RendererRebuildFlags {
        let graph = ShaderGraphDescriptor::from_reflection(
            reflection,
            self.config.default_target_scale,
            self.config.surface.size(),
        );
        self.replace_graph(graph)
    }

    fn rederive(&mut self, flags: RendererRebuildFlags) -> RendererRebuildFlags {
        self.derived = Self::derive_state(&self.config);
        self.pending_rebuild |= flags;
        self.pending_rebuild
    }

    fn derive_state(config: &RendererConfig) -> DerivedRendererState {
        let mut attachments = BTreeMap::new();
        for attachment in config.graph.attachments.values() {
            let (width, height) = attachment.scale.resolve(&config.surface);
            let format = config
                .format_overrides
                .get(&attachment.handle)
                .copied()
                .unwrap_or_else(|| {
                    infer_attachment_format(
                        &attachment.target.r#type,
                        &config.hdr,
                        config.depth_format,
                    )
                });

            attachments.insert(
                attachment.handle.clone(),
                ResolvedAttachment {
                    handle: attachment.handle.clone(),
                    target_type: attachment.target.r#type.clone(),
                    width,
                    height,
                    format,
                    samples: attachment.target.samples.max(config.msaa_samples),
                    mip_levels: attachment.target.mip_levels.max(1),
                    lifecycle: attachment.lifecycle,
                },
            );
        }

        DerivedRendererState { attachments }
    }
}

fn infer_attachment_format(
    target_type: &RenderTargetType,
    hdr: &HdrConfig,
    depth_format: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    match target_type {
        RenderTargetType::Depth | RenderTargetType::Shadows => depth_format,
        RenderTargetType::MotionVectors => hdr.rg_format(),
        RenderTargetType::AmbientOcclusion => hdr.r_format(),
        RenderTargetType::Albedo | RenderTargetType::Normals | RenderTargetType::Lighting => {
            hdr.rgba_format()
        }
        RenderTargetType::Custom(name) if name.contains("roughness") => hdr.rg_format(),
        RenderTargetType::Custom(name) if name.contains("ao") => hdr.r_format(),
        RenderTargetType::Custom(_) => hdr.rgba_format(),
    }
}

/// Helper for mapping precision directly when building custom attachments.
pub fn format_for_precision(
    precision: BufferPrecision,
    target_type: &RenderTargetType,
    depth_format: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    match target_type {
        RenderTargetType::Depth | RenderTargetType::Shadows => depth_format,
        RenderTargetType::MotionVectors => precision.rg_format(),
        RenderTargetType::AmbientOcclusion => precision.r_format(),
        RenderTargetType::Custom(name) if name.contains("roughness") => precision.rg_format(),
        _ => precision.rgba_format(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::{
        ReflectedRenderTarget, ReflectedResource, ReflectedResourceType, ReflectedStage,
        ReflectionSnapshot, RenderTargetConfig, RenderTargetType, ShaderKind, ShaderProgram,
    };

    #[test]
    fn resize_marks_surface_and_attachments_dirty() {
        let mut runtime = RendererRuntime::new(RendererConfig::default());
        runtime.clear_rebuild_flags();
        let flags = runtime.resize(1920, 1080);
        assert!(flags.contains(RendererRebuildFlags::SURFACE));
        assert!(flags.contains(RendererRebuildFlags::ATTACHMENTS));
    }

    #[test]
    fn shader_program_infers_graph_attachments() {
        let program = ShaderProgram::with_name("gbuffer").with_render_targets(vec![
            RenderTargetConfig::new(RenderTargetType::Albedo, 1, 1),
            RenderTargetConfig::new(RenderTargetType::Depth, 1, 1),
        ]);
        let config = RendererConfig::for_program(SurfaceConfig::default(), &program);
        let runtime = RendererRuntime::new(config);
        assert!(
            runtime
                .derived()
                .attachment(&RenderTargetHandle::new("albedo"))
                .is_some()
        );
        assert!(
            runtime
                .derived()
                .attachment(&RenderTargetHandle::new("depth"))
                .is_some()
        );
    }

    #[test]
    fn reflection_infers_graph_attachments() {
        let reflection = ReflectionSnapshot {
            stages: vec![ReflectedStage::new(ShaderKind::Fragment, "fs_main")],
            resources: vec![ReflectedResource::new(
                "camera",
                0,
                ReflectedResourceType::UniformBuffer,
            )],
            render_targets: vec![
                ReflectedRenderTarget::new("g_albedo"),
                ReflectedRenderTarget {
                    handle: "g_depth".to_string(),
                    target_type: Some("depth".to_string()),
                    width: None,
                    height: None,
                    mip_levels: None,
                    samples: None,
                    scale: None,
                    lifecycle: Some(AttachmentLifecycle::History),
                },
            ],
        };

        let config = RendererConfig::for_reflection(SurfaceConfig::default(), &reflection);
        let runtime = RendererRuntime::new(config);
        let depth = runtime
            .derived()
            .attachment(&RenderTargetHandle::new("g_depth"))
            .expect("depth attachment");
        assert_eq!(depth.lifecycle, AttachmentLifecycle::History);
        assert_eq!(depth.target_type, RenderTargetType::Depth);
    }
}

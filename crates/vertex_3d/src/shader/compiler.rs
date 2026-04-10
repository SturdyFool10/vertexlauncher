//! Compiler-facing shader abstractions for detached renderer workflows.

use std::collections::BTreeMap;
#[cfg(feature = "slang-api")]
use std::fs;
use std::path::PathBuf;

use super::{ReflectionSnapshot, ShaderKind, ShaderProgram};
#[cfg(feature = "slang-api")]
use slang::Downcast;

/// Source language accepted by a compiler implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderSourceLanguage {
    Slang,
    Wgsl,
    Glsl,
    Hlsl,
}

/// Requested backend target for compiled shader output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderBackendTarget {
    Wgsl,
    SpirV,
    MetalShadingLanguage,
    Dxil,
}

/// Source payload used by the compiler request.
#[derive(Debug, Clone)]
pub enum ShaderCompileSource {
    Inline(String),
    FilePath(String),
}

/// A single stage input to compilation.
#[derive(Debug, Clone)]
pub struct StageSource {
    pub kind: ShaderKind,
    pub source: ShaderCompileSource,
    pub entry_point: Option<String>,
}

impl StageSource {
    pub fn inline(kind: ShaderKind, source: impl Into<String>) -> Self {
        Self {
            kind,
            source: ShaderCompileSource::Inline(source.into()),
            entry_point: None,
        }
    }

    pub fn file(kind: ShaderKind, path: impl Into<String>) -> Self {
        Self {
            kind,
            source: ShaderCompileSource::FilePath(path.into()),
            entry_point: None,
        }
    }

    pub fn with_entry_point(mut self, entry_point: impl Into<String>) -> Self {
        self.entry_point = Some(entry_point.into());
        self
    }
}

/// Request to compile a multi-stage shader program.
#[derive(Debug, Clone)]
pub struct ShaderCompileRequest {
    pub name: String,
    pub language: ShaderSourceLanguage,
    pub target: ShaderBackendTarget,
    pub stages: Vec<StageSource>,
}

impl ShaderCompileRequest {
    pub fn new(
        name: impl Into<String>,
        language: ShaderSourceLanguage,
        target: ShaderBackendTarget,
    ) -> Self {
        Self {
            name: name.into(),
            language,
            target,
            stages: Vec::new(),
        }
    }

    pub fn with_stage(mut self, stage: StageSource) -> Self {
        self.stages.push(stage);
        self
    }

    pub fn from_program(
        program: &ShaderProgram,
        language: ShaderSourceLanguage,
        target: ShaderBackendTarget,
    ) -> Self {
        let mut stages: Vec<_> = program
            .stages
            .values()
            .map(|stage| StageSource {
                kind: stage.kind,
                source: ShaderCompileSource::Inline(stage.source.clone()),
                entry_point: stage.config.entry_point.clone(),
            })
            .collect();
        stages.sort_by_key(|stage| stage.kind.slang_name());
        Self {
            name: program.name.clone(),
            language,
            target,
            stages,
        }
    }
}

/// Compiled payload for a single shader stage.
#[derive(Debug, Clone)]
pub struct CompiledShaderStage {
    pub kind: ShaderKind,
    pub entry_point: String,
    pub target: ShaderBackendTarget,
    pub source: String,
}

/// Compiler result containing both runtime-ready shader code and normalized reflection.
#[derive(Debug, Clone)]
pub struct CompiledShaderProgram {
    pub program: ShaderProgram,
    pub reflection: ReflectionSnapshot,
    pub target: ShaderBackendTarget,
    pub stages: BTreeMap<ShaderKind, CompiledShaderStage>,
}

impl CompiledShaderProgram {
    pub fn apply_reflection(mut self) -> Self {
        self.program.apply_reflection(&self.reflection);
        self
    }

    pub fn create_shader_modules(
        &self,
        device: &wgpu::Device,
    ) -> Result<BTreeMap<ShaderKind, wgpu::ShaderModule>, ShaderCompileError> {
        let mut modules = BTreeMap::new();
        for (kind, stage) in &self.stages {
            modules.insert(*kind, stage.create_shader_module(device)?);
        }
        Ok(modules)
    }
}

impl CompiledShaderStage {
    pub fn create_shader_module(
        &self,
        device: &wgpu::Device,
    ) -> Result<wgpu::ShaderModule, ShaderCompileError> {
        match self.target {
            ShaderBackendTarget::Wgsl => {
                Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(&self.entry_point),
                    source: wgpu::ShaderSource::Wgsl(self.source.clone().into()),
                }))
            }
            _ => Err(ShaderCompileError::BackendUnavailable {
                message: format!(
                    "wgpu shader module creation is only implemented for WGSL outputs, not {:?}",
                    self.target
                ),
            }),
        }
    }
}

/// Compiler abstraction for Slang or alternative frontends.
pub trait ShaderCompiler {
    fn compile_program(
        &self,
        request: &ShaderCompileRequest,
    ) -> Result<CompiledShaderProgram, ShaderCompileError>;
}

/// In-memory compiler that normalizes Slang reflection without shelling out to external tools.
///
/// This compiler intentionally does not invoke `slangc`. It can:
/// - Parse reflection from Slang sources fully in memory.
/// - Pass through WGSL sources as backend-ready stage code.
#[derive(Debug, Clone)]
pub struct SlangCompiler;

impl SlangCompiler {
    pub fn new(_unused: impl Into<PathBuf>) -> Self {
        Self
    }
}

impl ShaderCompiler for SlangCompiler {
    fn compile_program(
        &self,
        request: &ShaderCompileRequest,
    ) -> Result<CompiledShaderProgram, ShaderCompileError> {
        #[cfg(not(feature = "slang-api"))]
        {
            let _ = request;
            return Err(ShaderCompileError::BackendUnavailable {
                message:
                    "Slang API backend is disabled. Enable the `slang-api` feature on vertex_3d."
                        .to_string(),
            });
        }
        #[cfg(feature = "slang-api")]
        {
            compile_with_slang_api(request)
        }
    }
}

#[cfg(feature = "slang-api")]
fn compile_with_slang_api(
    request: &ShaderCompileRequest,
) -> Result<CompiledShaderProgram, ShaderCompileError> {
    if request.language != ShaderSourceLanguage::Slang {
        return Err(ShaderCompileError::BackendUnavailable {
            message: "SlangCompiler only accepts Slang source requests".to_string(),
        });
    }
    if request.target != ShaderBackendTarget::Wgsl {
        return Err(ShaderCompileError::BackendUnavailable {
            message: format!(
                "SlangCompiler currently transpiles only to WGSL, not {:?}",
                request.target
            ),
        });
    }
    let mut program = ShaderProgram::with_name(request.name.clone());
    let mut stages = BTreeMap::new();
    let mut all_slang_sources = String::new();
    let mut file_stages = Vec::new();

    for stage in &request.stages {
        let source = match &stage.source {
            ShaderCompileSource::Inline(source) => source.clone(),
            ShaderCompileSource::FilePath(path) => {
                fs::read_to_string(path).map_err(|error| ShaderCompileError::Io {
                    path: PathBuf::from(path),
                    error,
                })?
            }
        };
        all_slang_sources.push_str(&source);
        all_slang_sources.push('\n');
        let mut program_stage = super::ShaderStage::new(stage.kind, source.clone());
        if let Some(entry_point) = &stage.entry_point {
            program_stage = program_stage.with_entry_point(entry_point.clone());
        }
        program.stages.insert(stage.kind, program_stage);

        let source_path = match &stage.source {
            ShaderCompileSource::FilePath(path) => PathBuf::from(path),
            ShaderCompileSource::Inline(_) => {
                return Err(ShaderCompileError::BackendUnavailable {
                        message: "SlangCompiler currently requires file-backed stage sources for API transpilation".to_string(),
                    });
            }
        };
        file_stages.push((stage.kind, stage.entry_point.clone(), source_path));
    }

    let global_session =
        slang::GlobalSession::new().ok_or_else(|| ShaderCompileError::CompileFailed {
            message: "failed to create Slang global session".to_string(),
        })?;
    let target_desc = slang::TargetDesc::default()
        .format(slang::CompileTarget::Wgsl)
        .profile(global_session.find_profile("sm_6_5"));
    let targets = [target_desc];

    let mut search_paths_owned = Vec::new();
    for (_, _, source_path) in &file_stages {
        if let Some(parent) = source_path.parent() {
            search_paths_owned.push(parent.to_path_buf());
        }
    }
    search_paths_owned.sort();
    search_paths_owned.dedup();
    if search_paths_owned.is_empty() {
        search_paths_owned.push(PathBuf::from("."));
    }
    let search_paths_cstr = search_paths_owned
        .iter()
        .map(|path| {
            std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|_| {
                ShaderCompileError::CompileFailed {
                    message: format!(
                        "search path contains interior null byte: {}",
                        path.display()
                    ),
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let search_paths = search_paths_cstr
        .iter()
        .map(|path| path.as_ptr())
        .collect::<Vec<_>>();

    let session_desc = slang::SessionDesc::default()
        .targets(&targets)
        .search_paths(&search_paths);
    let session = global_session
        .create_session(&session_desc)
        .ok_or_else(|| ShaderCompileError::CompileFailed {
            message: "failed to create Slang session".to_string(),
        })?;

    let reflection = ReflectionSnapshot::from_slang_source(&all_slang_sources);
    for (kind, entry_point_override, source_path) in &file_stages {
        let reflected_entry = reflection
            .stage(*kind)
            .map(|entry| entry.entry_point.clone())
            .or_else(|| entry_point_override.clone())
            .unwrap_or_else(|| "main".to_string());

        let module_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ShaderCompileError::CompileFailed {
                message: format!(
                    "invalid source path for Slang module: {}",
                    source_path.display()
                ),
            })?;
        let module = session.load_module(module_name).map_err(|error| {
            ShaderCompileError::CompileFailed {
                message: format!(
                    "Slang failed to load module '{}' from {}: {}",
                    module_name,
                    source_path.display(),
                    error
                ),
            }
        })?;
        let entry_point = module
            .find_entry_point_by_name(&reflected_entry)
            .ok_or_else(|| ShaderCompileError::CompileFailed {
                message: format!(
                    "entry point '{}' not found in {}",
                    reflected_entry,
                    source_path.display()
                ),
            })?;
        let program_component = session
            .create_composite_component_type(&[
                module.downcast().clone(),
                entry_point.downcast().clone(),
            ])
            .map_err(|error| ShaderCompileError::CompileFailed {
                message: format!(
                    "Slang failed to compose program for {}: {}",
                    source_path.display(),
                    error
                ),
            })?;
        let linked_program =
            program_component
                .link()
                .map_err(|error| ShaderCompileError::CompileFailed {
                    message: format!("Slang link failed for {}: {}", source_path.display(), error),
                })?;
        let wgsl_blob = linked_program.entry_point_code(0, 0).map_err(|error| {
            ShaderCompileError::CompileFailed {
                message: format!(
                    "Slang WGSL transpile failed for {}: {}",
                    source_path.display(),
                    error
                ),
            }
        })?;
        let stage_source = wgsl_blob
            .as_str()
            .map_err(|_| ShaderCompileError::CompileFailed {
                message: format!(
                    "Slang produced non-UTF8 WGSL output for {}",
                    source_path.display()
                ),
            })?;

        stages.insert(
            *kind,
            CompiledShaderStage {
                kind: *kind,
                entry_point: reflected_entry,
                target: request.target,
                source: stage_source.to_string(),
            },
        );
    }

    let mut compiled = CompiledShaderProgram {
        program,
        reflection,
        target: request.target,
        stages,
    };
    compiled.program.apply_reflection(&compiled.reflection);
    Ok(compiled)
}

/// Basic compiler useful for tests and early integration. It forwards sources and reflection
/// without invoking an external compiler.
#[derive(Debug, Clone)]
pub struct ReflectionPassthroughCompiler {
    pub reflection: ReflectionSnapshot,
}

impl ReflectionPassthroughCompiler {
    pub fn new(reflection: ReflectionSnapshot) -> Self {
        Self { reflection }
    }
}

impl ShaderCompiler for ReflectionPassthroughCompiler {
    fn compile_program(
        &self,
        request: &ShaderCompileRequest,
    ) -> Result<CompiledShaderProgram, ShaderCompileError> {
        let mut program = ShaderProgram::with_name(request.name.clone());
        let mut stages = BTreeMap::new();

        for stage in &request.stages {
            let source = match &stage.source {
                ShaderCompileSource::Inline(source) => source.clone(),
                ShaderCompileSource::FilePath(path) => {
                    return Err(ShaderCompileError::UnsupportedSourcePath { path: path.clone() });
                }
            };
            let mut program_stage = super::ShaderStage::new(stage.kind, source.clone());
            if let Some(entry_point) = &stage.entry_point {
                program_stage = program_stage.with_entry_point(entry_point.clone());
            }
            program.stages.insert(stage.kind, program_stage);

            let reflected_entry = self
                .reflection
                .stage(stage.kind)
                .map(|entry| entry.entry_point.clone())
                .or_else(|| stage.entry_point.clone())
                .unwrap_or_else(|| "main".to_string());
            stages.insert(
                stage.kind,
                CompiledShaderStage {
                    kind: stage.kind,
                    entry_point: reflected_entry,
                    target: request.target,
                    source,
                },
            );
        }

        let mut compiled = CompiledShaderProgram {
            program,
            reflection: self.reflection.clone(),
            target: request.target,
            stages,
        };
        compiled.program.apply_reflection(&compiled.reflection);
        Ok(compiled)
    }
}

/// Compiler-level error conditions.
#[derive(Debug, thiserror::Error)]
pub enum ShaderCompileError {
    #[error("compiler backend does not support file-path sources yet: {path}")]
    UnsupportedSourcePath { path: String },

    #[error("compiler backend is not implemented: {message}")]
    BackendUnavailable { message: String },

    #[error("shader compilation failed: {message}")]
    CompileFailed { message: String },

    #[error("i/o error at {path}: {error}")]
    Io {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },

    #[error("failed to parse reflection snapshot at {path}: {error}")]
    Reflection {
        path: PathBuf,
        #[source]
        error: serde_json::Error,
    },
}

impl ShaderBackendTarget {
    pub fn output_extension(self) -> &'static str {
        match self {
            ShaderBackendTarget::Wgsl => "wgsl",
            ShaderBackendTarget::SpirV => "spv",
            ShaderBackendTarget::MetalShadingLanguage => "metal",
            ShaderBackendTarget::Dxil => "dxil",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::{ReflectedRenderTarget, ReflectedStage};
    #[cfg(feature = "slang-api")]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn passthrough_compiler_applies_reflection() {
        let compiler = ReflectionPassthroughCompiler::new(ReflectionSnapshot {
            stages: vec![ReflectedStage::new(ShaderKind::Fragment, "fs_main")],
            resources: Vec::new(),
            render_targets: vec![ReflectedRenderTarget::new("g_albedo")],
        });
        let request = ShaderCompileRequest::new(
            "test",
            ShaderSourceLanguage::Slang,
            ShaderBackendTarget::Wgsl,
        )
        .with_stage(StageSource::inline(ShaderKind::Fragment, "fragment"));
        let compiled = compiler.compile_program(&request).expect("compile");
        assert_eq!(
            compiled
                .program
                .get_stage(ShaderKind::Fragment)
                .unwrap()
                .entry_point(),
            Some("fs_main")
        );
        assert_eq!(compiled.program.render_targets.len(), 1);
    }

    #[test]
    #[cfg(feature = "slang-api")]
    fn slang_compiler_parses_reflection_in_memory() {
        let compiler = SlangCompiler::new("unused");
        let request = ShaderCompileRequest::new(
            "test",
            ShaderSourceLanguage::Slang,
            ShaderBackendTarget::Wgsl,
        )
        .with_stage(StageSource::inline(
            ShaderKind::Fragment,
            r#"
// @vertex3d.target handle=lighting type=lighting lifecycle=transient
[[vk::binding(0, 0)]] Texture2D<float4> source_tex;
[shader("fragment")]
float4 fs_main(float4 pos : SV_Position) : SV_Target { return source_tex.Load(int3(0,0,0)); }
"#,
        ));

        let error = compiler
            .compile_program(&request)
            .expect_err("requires WGSL stage payload");
        assert!(matches!(
            error,
            ShaderCompileError::BackendUnavailable { .. }
        ));
        let reflection = ReflectionSnapshot::from_slang_source(match &request.stages[0].source {
            ShaderCompileSource::Inline(source) => source,
            ShaderCompileSource::FilePath(_) => unreachable!(),
        });
        assert_eq!(reflection.render_targets[0].handle, "lighting");
        assert!(matches!(
            reflection.resources[0].resource_type,
            crate::shader::ReflectedResourceType::Texture
        ));
    }

    #[test]
    #[cfg(feature = "slang-api")]
    fn slang_compiler_transpiles_file_stage_to_wgsl() {
        if std::env::var("SLANG_LIB_DIR").is_err() {
            return;
        }

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("vertex3d-slang-test-{unique}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let source_path = dir.join("test.slang");
        std::fs::write(
            &source_path,
            r#"
[shader("fragment")]
float4 fs_main(float4 pos : SV_Position) : SV_Target { return float4(1.0, 0.0, 0.0, 1.0); }
"#,
        )
        .expect("write slang source");

        let request = ShaderCompileRequest::new(
            "slang-file-test",
            ShaderSourceLanguage::Slang,
            ShaderBackendTarget::Wgsl,
        )
        .with_stage(
            StageSource::file(
                ShaderKind::Fragment,
                source_path.to_string_lossy().to_string(),
            )
            .with_entry_point("fs_main"),
        );
        let compiler = SlangCompiler::new("unused");
        let compiled = compiler
            .compile_program(&request)
            .expect("compile via slang api");
        let stage = compiled
            .stages
            .get(&ShaderKind::Fragment)
            .expect("fragment stage");
        assert!(stage.source.contains("@fragment"));
    }
}

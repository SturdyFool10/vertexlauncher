//! Compiler-facing shader abstractions for detached renderer workflows.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{ReflectionSnapshot, ShaderKind, ShaderProgram};

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
    pub reflection_sidecar: Option<PathBuf>,
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
            reflection_sidecar: None,
        }
    }

    pub fn with_stage(mut self, stage: StageSource) -> Self {
        self.stages.push(stage);
        self
    }

    pub fn with_reflection_sidecar(mut self, path: impl Into<PathBuf>) -> Self {
        self.reflection_sidecar = Some(path.into());
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
            reflection_sidecar: None,
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
            ShaderBackendTarget::Wgsl => Ok(device.create_shader_module(
                wgpu::ShaderModuleDescriptor {
                    label: Some(&self.entry_point),
                    source: wgpu::ShaderSource::Wgsl(self.source.clone().into()),
                },
            )),
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

/// Slang-backed compiler using `slangc` for shader codegen and a JSON reflection sidecar
/// generated by a separate exporter or native bridge.
#[derive(Debug, Clone)]
pub struct SlangCompiler {
    pub slangc_path: PathBuf,
    pub output_directory: PathBuf,
    pub profile: String,
}

impl SlangCompiler {
    pub fn new(output_directory: impl Into<PathBuf>) -> Self {
        Self {
            slangc_path: PathBuf::from("slangc"),
            output_directory: output_directory.into(),
            profile: "sm_6_5".to_string(),
        }
    }

    pub fn with_slangc_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.slangc_path = path.into();
        self
    }

    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = profile.into();
        self
    }

    pub fn compiled_stage_path(
        &self,
        request_name: &str,
        kind: ShaderKind,
        target: ShaderBackendTarget,
    ) -> PathBuf {
        let extension = target.output_extension();
        self.output_directory
            .join(format!("{request_name}.{}.{}", kind.slang_name(), extension))
    }

    pub fn resolve_reflection_sidecar(
        &self,
        request: &ShaderCompileRequest,
    ) -> Result<PathBuf, ShaderCompileError> {
        if let Some(path) = &request.reflection_sidecar {
            return Ok(path.clone());
        }

        if let Some(first_stage_path) = request
            .stages
            .iter()
            .find_map(|stage| match &stage.source {
                ShaderCompileSource::FilePath(path) => Some(PathBuf::from(path)),
                ShaderCompileSource::Inline(_) => None,
            })
        {
            if let Some(sidecar) = adjacent_reflection_sidecar(&first_stage_path) {
                return Ok(sidecar);
            }
        }

        Ok(self
            .output_directory
            .join(format!("{}.reflection.json", request.name)))
    }

    pub fn load_reflection_snapshot(
        &self,
        request: &ShaderCompileRequest,
    ) -> Result<ReflectionSnapshot, ShaderCompileError> {
        let path = self.resolve_reflection_sidecar(request)?;
        if !path.exists() {
            emit_slang_reflection_json(&self.slangc_path, &self.profile, request, &path)?;
        }
        let json = fs::read_to_string(&path).map_err(|error| ShaderCompileError::Io {
            path: path.clone(),
            error,
        })?;
        ReflectionSnapshot::from_json_str(&json).map_err(|error| ShaderCompileError::Reflection {
            path,
            error,
        })
    }
}

impl ShaderCompiler for SlangCompiler {
    fn compile_program(
        &self,
        request: &ShaderCompileRequest,
    ) -> Result<CompiledShaderProgram, ShaderCompileError> {
        if request.language != ShaderSourceLanguage::Slang {
            return Err(ShaderCompileError::BackendUnavailable {
                message: "SlangCompiler only accepts Slang source requests".to_string(),
            });
        }

        fs::create_dir_all(&self.output_directory).map_err(|error| ShaderCompileError::Io {
            path: self.output_directory.clone(),
            error,
        })?;

        let reflection = self.load_reflection_snapshot(request)?;
        let mut program = ShaderProgram::with_name(request.name.clone());
        let mut stages = BTreeMap::new();

        for stage in &request.stages {
            let source_path = match &stage.source {
                ShaderCompileSource::FilePath(path) => PathBuf::from(path),
                ShaderCompileSource::Inline(_) => {
                    return Err(ShaderCompileError::BackendUnavailable {
                        message: "SlangCompiler currently requires file-backed stage sources"
                            .to_string(),
                    });
                }
            };

            let output_path = self.compiled_stage_path(&request.name, stage.kind, request.target);
            let compiled_source = compile_slang_stage(
                &self.slangc_path,
                &self.profile,
                request.target,
                stage,
                &source_path,
                &output_path,
            )?;

            let mut program_stage =
                super::ShaderStage::new(stage.kind, fs::read_to_string(&source_path).map_err(
                    |error| ShaderCompileError::Io {
                        path: source_path.clone(),
                        error,
                    },
                )?);
            if let Some(entry_point) = &stage.entry_point {
                program_stage = program_stage.with_entry_point(entry_point.clone());
            }
            program.stages.insert(stage.kind, program_stage);

            let reflected_entry = reflection
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
                    source: compiled_source,
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
                    return Err(ShaderCompileError::UnsupportedSourcePath {
                        path: path.clone(),
                    });
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

    fn slang_target_name(self) -> &'static str {
        match self {
            ShaderBackendTarget::Wgsl => "wgsl",
            ShaderBackendTarget::SpirV => "spirv",
            ShaderBackendTarget::MetalShadingLanguage => "metal",
            ShaderBackendTarget::Dxil => "dxil",
        }
    }
}

fn adjacent_reflection_sidecar(source_path: &Path) -> Option<PathBuf> {
    let file_name = source_path.file_name()?.to_str()?;
    Some(source_path.with_file_name(format!("{file_name}.reflection.json")))
}

fn compile_slang_stage(
    slangc_path: &Path,
    profile: &str,
    target: ShaderBackendTarget,
    stage: &StageSource,
    source_path: &Path,
    output_path: &Path,
) -> Result<String, ShaderCompileError> {
    let mut command = Command::new(slangc_path);
    command.args(["-target", target.slang_target_name(), "-profile", profile, "-o"]);
    command.arg(output_path);
    if let Some(entry_point) = &stage.entry_point {
        command.args(["-entry", entry_point]);
    }
    command.arg(source_path);

    let output = command.output().map_err(|error| ShaderCompileError::Io {
        path: slangc_path.to_path_buf(),
        error,
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ShaderCompileError::CompileFailed {
            message: format!(
                "slangc failed for {} (stage {}): {}",
                source_path.display(),
                stage.kind,
                stderr.trim()
            ),
        });
    }

    fs::read_to_string(output_path).map_err(|error| ShaderCompileError::Io {
        path: output_path.to_path_buf(),
        error,
    })
}

fn emit_slang_reflection_json(
    slangc_path: &Path,
    profile: &str,
    request: &ShaderCompileRequest,
    reflection_path: &Path,
) -> Result<(), ShaderCompileError> {
    let mut command = Command::new(slangc_path);
    command.args(["-reflection-json"]);
    command.arg(reflection_path);
    command.args(["-profile", profile, "-target", request.target.slang_target_name()]);

    for stage in &request.stages {
        if let Some(entry_point) = &stage.entry_point {
            command.args(["-entry", entry_point]);
        }
    }

    for stage in &request.stages {
        match &stage.source {
            ShaderCompileSource::FilePath(path) => {
                command.arg(path);
            }
            ShaderCompileSource::Inline(_) => {
                return Err(ShaderCompileError::BackendUnavailable {
                    message:
                        "native Slang reflection currently requires file-backed stage sources"
                            .to_string(),
                });
            }
        }
    }

    let output = command.output().map_err(|error| ShaderCompileError::Io {
        path: slangc_path.to_path_buf(),
        error,
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ShaderCompileError::CompileFailed {
            message: format!("slangc reflection failed: {}", stderr.trim()),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::{ReflectedRenderTarget, ReflectedStage};

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
            compiled.program.get_stage(ShaderKind::Fragment).unwrap().entry_point(),
            Some("fs_main")
        );
        assert_eq!(compiled.program.render_targets.len(), 1);
    }

    #[test]
    fn resolves_adjacent_reflection_sidecar() {
        let compiler = SlangCompiler::new("target/slang-tests");
        let request = ShaderCompileRequest::new(
            "test",
            ShaderSourceLanguage::Slang,
            ShaderBackendTarget::Wgsl,
        )
        .with_stage(StageSource::file(ShaderKind::Fragment, "shaders/test.slang"));
        assert_eq!(
            compiler.resolve_reflection_sidecar(&request).unwrap(),
            PathBuf::from("shaders/test.slang.reflection.json")
        );
    }

    #[test]
    fn explicit_reflection_sidecar_wins() {
        let compiler = SlangCompiler::new("target/slang-tests");
        let request = ShaderCompileRequest::new(
            "test",
            ShaderSourceLanguage::Slang,
            ShaderBackendTarget::Wgsl,
        )
        .with_reflection_sidecar("custom/test.reflection.json");
        assert_eq!(
            compiler.resolve_reflection_sidecar(&request).unwrap(),
            PathBuf::from("custom/test.reflection.json")
        );
    }
}

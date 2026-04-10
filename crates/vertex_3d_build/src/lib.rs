use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ReflectionExportError {
    #[error("failed to read shader source {path}: {error}")]
    Read {
        path: String,
        #[source]
        error: std::io::Error,
    },

    #[error("failed to write reflection sidecar {path}: {error}")]
    Write {
        path: String,
        #[source]
        error: std::io::Error,
    },

    #[error("failed to serialize reflection sidecar for {path}: {error}")]
    Serialize {
        path: String,
        #[source]
        error: serde_json::Error,
    },
}

#[derive(Debug, Clone, Serialize, Default)]
struct ReflectionSnapshot {
    stages: Vec<ReflectedStage>,
    resources: Vec<ReflectedResource>,
    render_targets: Vec<ReflectedRenderTarget>,
}

#[derive(Debug, Clone, Serialize)]
struct ReflectedStage {
    kind: String,
    entry_point: String,
    writes_gbuffer: bool,
    pipeline_flags: u32,
}

#[derive(Debug, Clone, Serialize)]
struct ReflectedResource {
    name: String,
    slot: u32,
    space: u32,
    #[serde(rename = "type")]
    resource_type: String,
    stages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    texture_dimension: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ReflectedRenderTarget {
    handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mip_levels: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    samples: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lifecycle: Option<String>,
}

pub fn export_reflection_snapshot_from_slang(
    source_path: &Path,
    output_path: &Path,
) -> Result<(), ReflectionExportError> {
    let source = fs::read_to_string(source_path).map_err(|error| ReflectionExportError::Read {
        path: source_path.display().to_string(),
        error,
    })?;
    let snapshot = parse_slang_reflection(&source);
    let json = serde_json::to_string_pretty(&snapshot).map_err(|error| {
        ReflectionExportError::Serialize {
            path: output_path.display().to_string(),
            error,
        }
    })?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| ReflectionExportError::Write {
            path: parent.display().to_string(),
            error,
        })?;
    }
    fs::write(output_path, json).map_err(|error| ReflectionExportError::Write {
        path: output_path.display().to_string(),
        error,
    })
}

fn parse_slang_reflection(source: &str) -> ReflectionSnapshot {
    let stages = parse_stages(source);
    let stage_bodies = parse_stage_bodies(source, &stages);
    let mut resources = parse_resources(source);
    for resource in &mut resources {
        resource.stages = stage_bodies
            .iter()
            .filter_map(|(kind, body)| body.contains(resource.name.as_str()).then_some(kind.clone()))
            .collect();
    }

    ReflectionSnapshot {
        stages,
        resources,
        render_targets: parse_target_annotations(source),
    }
}

fn parse_stages(source: &str) -> Vec<ReflectedStage> {
    let mut stages = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    for index in 0..lines.len() {
        let line = lines[index].trim();
        if let Some(kind) = parse_shader_attribute(line) {
            for candidate in lines.iter().skip(index + 1) {
                let candidate = candidate.trim();
                if candidate.is_empty() || candidate.starts_with("//") {
                    continue;
                }
                if let Some(entry_point) = parse_function_name(candidate) {
                    stages.push(ReflectedStage {
                        kind,
                        entry_point,
                        writes_gbuffer: false,
                        pipeline_flags: 1,
                    });
                    break;
                }
            }
        }
    }
    stages
}

fn parse_stage_bodies(source: &str, stages: &[ReflectedStage]) -> Vec<(String, String)> {
    stages
        .iter()
        .filter_map(|stage| {
            let function_marker = format!("{}(", stage.entry_point);
            let start = source.find(&function_marker)?;
            let brace_start = source[start..].find('{')? + start;
            let brace_end = find_matching_brace(source, brace_start)?;
            Some((
                stage.kind.clone(),
                source[brace_start..=brace_end].to_string(),
            ))
        })
        .collect()
}

fn parse_resources(source: &str) -> Vec<ReflectedResource> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("[[vk::binding(") {
                return None;
            }
            let binding_end = line.find(")]]")?;
            let binding_spec = &line["[[vk::binding(".len()..binding_end];
            let mut binding_parts = binding_spec.split(',').map(str::trim);
            let slot = binding_parts.next()?.parse().ok()?;
            let space = binding_parts.next()?.parse().ok()?;

            let declaration = line[binding_end + 3..].trim();
            let declaration = declaration.strip_suffix(';').unwrap_or(declaration);
            let mut tokens = declaration.split_whitespace();
            let type_token = tokens.next()?;
            let name = tokens.next()?.trim().to_string();
            let (resource_type, texture_dimension) = classify_resource_type(type_token);

            Some(ReflectedResource {
                name,
                slot,
                space,
                resource_type: resource_type.to_string(),
                stages: Vec::new(),
                texture_dimension: texture_dimension.map(str::to_string),
            })
        })
        .collect()
}

fn parse_target_annotations(source: &str) -> Vec<ReflectedRenderTarget> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let annotation = line.strip_prefix("// @vertex3d.target ")?;
            let mut target = ReflectedRenderTarget {
                handle: String::new(),
                target_type: None,
                width: None,
                height: None,
                mip_levels: None,
                samples: None,
                scale: None,
                lifecycle: None,
            };
            for pair in annotation.split_whitespace() {
                let Some((key, value)) = pair.split_once('=') else {
                    continue;
                };
                match key {
                    "handle" => target.handle = value.to_string(),
                    "type" => target.target_type = Some(value.to_string()),
                    "width" => target.width = value.parse().ok(),
                    "height" => target.height = value.parse().ok(),
                    "mips" => target.mip_levels = value.parse().ok(),
                    "samples" => target.samples = value.parse().ok(),
                    "scale" => target.scale = value.parse().ok(),
                    "lifecycle" => target.lifecycle = Some(value.to_string()),
                    _ => {}
                }
            }
            (!target.handle.is_empty()).then_some(target)
        })
        .collect()
}

fn parse_shader_attribute(line: &str) -> Option<String> {
    let value = line.strip_prefix("[shader(\"")?;
    let value = value.strip_suffix("\")]")?;
    Some(value.to_string())
}

fn parse_function_name(line: &str) -> Option<String> {
    let open = line.find('(')?;
    let prefix = &line[..open];
    prefix.split_whitespace().last().map(str::to_string)
}

fn find_matching_brace(source: &str, brace_start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in source[brace_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(brace_start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn classify_resource_type(type_token: &str) -> (&'static str, Option<&'static str>) {
    if type_token.starts_with("ConstantBuffer<") {
        ("uniform_buffer", None)
    } else if type_token.starts_with("StructuredBuffer<") || type_token.starts_with("RWStructuredBuffer<") {
        ("storage_buffer", None)
    } else if type_token.starts_with("SamplerState") {
        ("sampler", None)
    } else if type_token.starts_with("Texture1D") {
        ("texture", Some("d1"))
    } else if type_token.starts_with("Texture2DArray") {
        ("texture", Some("d2_array"))
    } else if type_token.starts_with("Texture2D") {
        ("texture", Some("d2"))
    } else if type_token.starts_with("TextureCube") {
        ("texture", Some("cube"))
    } else if type_token.starts_with("Texture3D") {
        ("texture", Some("d3"))
    } else {
        ("texture", None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slang_bindings_and_targets() {
        let source = r#"
// @vertex3d.target handle=accumulation type=lighting lifecycle=transient
[[vk::binding(0, 0)]] Texture2D<float4> source_tex;
[[vk::binding(0, 1)]] ConstantBuffer<Scalar> scalar;
[shader("vertex")]
FullscreenOut vs_main(uint vertex_index : SV_VertexID) { return (FullscreenOut)0; }
[shader("fragment")]
float4 fs_main(float4 pos : SV_Position) : SV_Target { return source_tex.Load(int3(0,0,0)) * scalar.value.x; }
"#;
        let snapshot = parse_slang_reflection(source);
        assert_eq!(snapshot.stages.len(), 2);
        assert_eq!(snapshot.resources.len(), 2);
        assert_eq!(snapshot.render_targets[0].handle, "accumulation");
        assert!(snapshot.resources.iter().any(|resource| resource.name == "source_tex"));
    }
}

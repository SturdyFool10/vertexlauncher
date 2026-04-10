use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use vertex_3d_build::export_reflection_snapshot_from_slang;

fn main() {
    let shader_dir = PathBuf::from("src/screens/shaders");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));

    compile_slang_shader(&shader_dir, &out_dir, "skin_preview_post_scene");
    compile_slang_shader(&shader_dir, &out_dir, "skin_preview_accumulate");
    compile_slang_shader(&shader_dir, &out_dir, "skin_preview_fxaa");
    compile_slang_shader(&shader_dir, &out_dir, "skin_preview_smaa");
    compile_slang_shader(&shader_dir, &out_dir, "skin_preview_taa");
    compile_slang_shader(&shader_dir, &out_dir, "skin_preview_present");
}

fn compile_slang_shader(shader_dir: &Path, out_dir: &Path, shader_stem: &str) {
    let source_path = shader_dir.join(format!("{shader_stem}.slang"));
    let output_path = out_dir.join(format!("{shader_stem}.wgsl"));
    let reflection_path = shader_dir.join(format!("{shader_stem}.slang.reflection.json"));
    let out_reflection_path = out_dir.join(format!("{shader_stem}.reflection.json"));

    println!("cargo:rerun-if-changed={}", source_path.display());
    println!("cargo:rerun-if-changed={}", reflection_path.display());
    println!("cargo:rerun-if-changed=build.rs");

    let status = Command::new("slangc")
        .args([
            "-target",
            "wgsl",
            "-profile",
            "sm_6_5",
            "-reflection-json",
            out_reflection_path
                .to_str()
                .expect("reflection output path must be valid UTF-8"),
            "-o",
            output_path
                .to_str()
                .expect("shader output path must be valid UTF-8"),
            source_path
                .to_str()
                .expect("shader source path must be valid UTF-8"),
        ])
        .status()
        .unwrap_or_else(|err| {
            panic!(
                "failed to invoke slangc for {}: {err}",
                source_path.display()
            )
        });

    if !status.success() {
        panic!("slangc failed while compiling {}", source_path.display());
    }

    if reflection_path.exists() {
        fs::copy(&reflection_path, &out_reflection_path).unwrap_or_else(|err| {
            panic!(
                "failed to copy reflection sidecar {} to {}: {err}",
                reflection_path.display(),
                out_reflection_path.display()
            )
        });
    } else if !out_reflection_path.exists() {
        export_reflection_snapshot_from_slang(&source_path, &out_reflection_path).unwrap_or_else(
            |err| {
                panic!(
                    "failed to generate reflection sidecar for {}: {err}",
                    source_path.display()
                )
            },
        );
    }
}

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

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

    println!("cargo:rerun-if-changed={}", source_path.display());
    println!("cargo:rerun-if-changed=build.rs");

    let status = Command::new("slangc")
        .args([
            "-target",
            "wgsl",
            "-profile",
            "sm_6_5",
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
}

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let shader_src = manifest_dir.join("src/screens/shaders");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let out_shaders = out_dir.join("shaders");
    std::fs::create_dir_all(&out_shaders).unwrap();

    let shaders = [
        "skin_preview_post_scene",
        "skin_preview_accumulate",
        "skin_preview_fxaa",
        "skin_preview_smaa",
        "skin_preview_taa",
        "skin_preview_present",
        "skin_preview_ssao",
    ];

    for name in shaders {
        let src = shader_src.join(format!("{name}.slang"));
        let dst = out_shaders.join(format!("{name}.wgsl"));

        println!("cargo:rerun-if-changed={}", src.display());

        let output = Command::new("slangc")
            .arg(&src)
            .arg("-target")
            .arg("wgsl")
            .arg("-o")
            .arg(&dst)
            .output()
            .unwrap_or_else(|e| panic!("failed to run slangc: {e}"));

        if !output.status.success() {
            panic!(
                "slangc failed for {}:\nstdout: {}\nstderr: {}",
                src.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }
}

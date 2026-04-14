use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let shader_src = manifest_dir.join("src/screens/shaders");
    let precompiled_dir = shader_src.join("precompiled");
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

    let has_slangc = Command::new("slangc").arg("-h").output().is_ok();

    for name in shaders {
        let src = shader_src.join(format!("{name}.slang"));
        let dst = out_shaders.join(format!("{name}.wgsl"));
        let precompiled = precompiled_dir.join(format!("{name}.wgsl"));

        println!("cargo:rerun-if-changed={}", src.display());

        let compiled = has_slangc && {
            match Command::new("slangc")
                .arg(&src)
                .arg("-target")
                .arg("wgsl")
                .arg("-o")
                .arg(&dst)
                .output()
            {
                Ok(output) if output.status.success() => true,
                Ok(output) => {
                    println!(
                        "cargo:warning=slangc failed for {name}: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                    false
                }
                Err(e) => {
                    println!("cargo:warning=slangc error for {name}: {e}");
                    false
                }
            }
        };

        if !compiled {
            std::fs::copy(&precompiled, &dst).unwrap_or_else(|e| {
                panic!(
                    "slangc unavailable and no pre-compiled fallback at {}: {e}",
                    precompiled.display()
                );
            });
        }
    }
}

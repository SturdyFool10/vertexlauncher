use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    emit_version_metadata();

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=../launcher_ui/src/assets/vertex.webp");
        if let Err(error) = compile_windows_resources() {
            println!("cargo:warning=failed to configure Windows resources: {error}");
        }
    }
}

fn emit_version_metadata() {
    let package_version =
        env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.3-Alpha".to_owned());
    let display_version = format_display_version(&package_version);
    println!("cargo:rustc-env=VERTEX_APP_VERSION={display_version}");

    if let Some(git_dir) = locate_git_dir() {
        emit_git_rerun_rules(&git_dir);
    }

    let commit_hash = git_commit_hash().unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=VERTEX_GIT_COMMIT_HASH={commit_hash}");
}

fn format_display_version(package_version: &str) -> String {
    let release = package_version.split('-').next().unwrap_or(package_version);
    let mut release_parts = release.split('.');
    let major = release_parts.next().unwrap_or("0");
    let minor = release_parts.next().unwrap_or("0");
    let patch = release_parts.next().unwrap_or("0");

    let prerelease = package_version
        .split_once('-')
        .map(|(_, suffix)| suffix.split('.').next().unwrap_or(suffix))
        .unwrap_or("");

    let channel = match prerelease.to_ascii_lowercase().as_str() {
        "alpha" => " Alpha",
        "beta" => " Beta",
        "rc" => " RC",
        _ => "",
    };

    format!("{major}.{minor}.{patch}{channel}")
}

fn locate_git_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    let repo_root = manifest_dir.parent()?.parent()?;
    let git_path = repo_root.join(".git");

    if git_path.is_dir() {
        return Some(git_path);
    }

    let git_file = fs::read_to_string(&git_path).ok()?;
    let relative = git_file.trim().strip_prefix("gitdir: ")?.trim();
    Some(repo_root.join(relative))
}

fn emit_git_rerun_rules(git_dir: &Path) {
    let head_path = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head_path.display());

    if let Ok(head_contents) = fs::read_to_string(&head_path) {
        if let Some(reference) = head_contents.trim().strip_prefix("ref: ") {
            println!(
                "cargo:rerun-if-changed={}",
                git_dir.join(reference).display()
            );
        }
    }
}

fn git_commit_hash() -> Option<String> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    let output = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .current_dir(manifest_dir)
        .output()
        .ok()?;

    if output.status.success() {
        let hash = String::from_utf8(output.stdout).ok()?;
        Some(hash.trim().to_owned())
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn compile_windows_resources() -> Result<(), String> {
    use image::{
        ExtendedColorType, ImageEncoder, ImageReader, codecs::ico::IcoEncoder, imageops::FilterType,
    };
    use std::{fs::File, io::Cursor, path::PathBuf};

    let out_dir = std::env::var("OUT_DIR")
        .map(PathBuf::from)
        .map_err(|error| format!("OUT_DIR is not set: {error}"))?;
    let icon_path = out_dir.join("vertex.ico");
    let decoded = ImageReader::new(Cursor::new(include_bytes!(
        "../launcher_ui/src/assets/vertex.webp"
    )))
    .with_guessed_format()
    .map_err(|error| format!("failed to detect vertex icon format: {error}"))?
    .decode()
    .map_err(|error| format!("failed to decode vertex icon source image: {error}"))?;
    let resized = decoded.resize(256, 256, FilterType::Lanczos3).to_rgba8();
    let mut icon_file = File::create(&icon_path)
        .map_err(|error| format!("failed to create generated .ico icon: {error}"))?;
    IcoEncoder::new(&mut icon_file)
        .write_image(
            resized.as_raw(),
            resized.width(),
            resized.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("failed to write generated .ico icon: {error}"))?;

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_path.to_string_lossy().as_ref());
    resource
        .compile()
        .map_err(|error| format!("failed to compile Windows resources: {error}"))?;
    Ok(())
}

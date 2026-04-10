use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    emit_version_metadata();

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=../../Vertex.webp");
        if let Err(error) = compile_windows_resources() {
            println!("cargo:warning=failed to configure Windows resources: {error}");
        }
    }
}

fn emit_version_metadata() {
    let package_version =
        env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.15-Alpha".to_owned());
    let display_version = format_display_version(&package_version);
    println!("cargo:rustc-env=VERTEX_APP_VERSION={display_version}");

    let repo_root = locate_repo_root();
    if let Some(repo_root) = repo_root.as_ref() {
        emit_repo_rerun_rules(&repo_root);
    }

    let revision = repo_root
        .as_deref()
        .and_then(git_revision)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=VERTEX_GIT_REVISION={revision}");
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

fn locate_repo_root() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    Some(manifest_dir.parent()?.parent()?.to_path_buf())
}

fn locate_git_dir(repo_root: &Path) -> Option<PathBuf> {
    let git_path = repo_root.join(".git");

    if git_path.is_dir() {
        return Some(git_path);
    }

    let git_file = fs::read_to_string(&git_path).ok()?;
    let relative = git_file.trim().strip_prefix("gitdir: ")?.trim();
    Some(repo_root.join(relative))
}

fn emit_repo_rerun_rules(repo_root: &Path) {
    if let Some(git_dir) = locate_git_dir(repo_root) {
        emit_git_rerun_rules(repo_root, &git_dir);
    }

    for path in git_snapshot_paths(repo_root) {
        println!("cargo:rerun-if-changed={}", repo_root.join(path).display());
    }
}

fn emit_git_rerun_rules(repo_root: &Path, git_dir: &Path) {
    println!(
        "cargo:rerun-if-changed={}",
        repo_root.join(".gitignore").display()
    );
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    println!("cargo:rerun-if-changed={}", git_dir.join("index").display());

    if let Ok(head_contents) = fs::read_to_string(git_dir.join("HEAD")) {
        if let Some(reference) = head_contents.trim().strip_prefix("ref: ") {
            println!(
                "cargo:rerun-if-changed={}",
                git_dir.join(reference).display()
            );
        }
    }
}

fn git_revision(repo_root: &Path) -> Option<String> {
    let revision = git_output(repo_root, &["rev-parse", "--short=8", "HEAD"])
        .or_else(|| git_head_revision(repo_root))?;
    let dirty_suffix = matches!(git_worktree_clean(repo_root), Some(false))
        .then_some("-dirty")
        .unwrap_or_default();
    Some(format!("{revision}{dirty_suffix}"))
}

fn git_worktree_clean(repo_root: &Path) -> Option<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(output.stdout.is_empty())
}

fn git_snapshot_paths(repo_root: &Path) -> Vec<PathBuf> {
    let mut paths = git_path_list(repo_root, &["ls-files", "-z"]).unwrap_or_default();
    for path in git_path_list(
        repo_root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .unwrap_or_default()
    {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }

    paths
}

fn git_path_list(repo_root: &Path, args: &[&str]) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| PathBuf::from(String::from_utf8_lossy(entry).into_owned()))
            .collect(),
    )
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(repo_root);

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn git_head_revision(repo_root: &Path) -> Option<String> {
    let git_dir = locate_git_dir(repo_root)?;
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let full_hash = if let Some(reference) = head.strip_prefix("ref: ") {
        resolve_git_reference(&git_dir, reference.trim())?
    } else {
        head.to_owned()
    };
    Some(shorten_hash(&full_hash).to_owned())
}

fn resolve_git_reference(git_dir: &Path, reference: &str) -> Option<String> {
    let ref_path = git_dir.join(reference);
    if let Ok(contents) = fs::read_to_string(&ref_path) {
        let hash = contents.trim();
        if !hash.is_empty() {
            return Some(hash.to_owned());
        }
    }

    let packed_refs = fs::read_to_string(git_dir.join("packed-refs")).ok()?;
    for line in packed_refs.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        let (hash, packed_reference) = line.split_once(' ')?;
        if packed_reference == reference {
            return Some(hash.to_owned());
        }
    }

    None
}

fn shorten_hash(hash: &str) -> &str {
    let end = hash.len().min(8);
    &hash[..end]
}

#[cfg(target_os = "windows")]
fn compile_windows_resources() -> Result<(), String> {
    use image::{
        ColorType, ImageEncoder, codecs::ico::IcoEncoder, imageops::FilterType,
        io::Reader as ImageReader,
    };
    use std::{fs::File, io::Cursor, path::PathBuf};

    let out_dir = std::env::var("OUT_DIR")
        .map(PathBuf::from)
        .map_err(|error| format!("OUT_DIR is not set: {error}"))?;
    let icon_path = out_dir.join("vertex.ico");
    let decoded = ImageReader::new(Cursor::new(include_bytes!("../../Vertex.webp")))
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
            ColorType::Rgba8,
        )
        .map_err(|error| format!("failed to write generated .ico icon: {error}"))?;

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_path.to_string_lossy().as_ref());
    resource
        .compile()
        .map_err(|error| format!("failed to compile Windows resources: {error}"))?;
    Ok(())
}

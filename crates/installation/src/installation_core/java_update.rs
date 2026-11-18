//! Keeps the launcher-managed OpenJDK runtimes current: finds the newest Adoptium release of a major version, installs it next to the
//! old one, and (once the caller has pointed the configuration at it) removes the old runtime's files.
//!
//! Runtimes live in `<cache>/java/openjdk-<major>-<arch>[-<release>]`. The installed version is never guessed from folder names: the
//! executable is asked (`java --version`, or `-version` for Java 8, which has no long flag).

use super::*;
use sha2::{Digest, Sha256};

/// What an update check found, and what it installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaUpdateReport {
    pub runtime_major: u8,
    /// Adoptium's newest release of this major version, e.g. `jdk-17.0.13+11`.
    pub latest_release: String,
    /// What the current executable reported (its build string), if it could be asked.
    pub installed_release: Option<String>,
    /// The runtime that was installed, when the current one was out of date or missing.
    pub installed: Option<PathBuf>,
}

pub(crate) struct AdoptiumPackage {
    pub link: String,
    pub name: String,
    pub release_name: String,
    pub sha256: Option<String>,
}

pub(crate) fn extract_adoptium_release(metadata: &serde_json::Value) -> Option<AdoptiumPackage> {
    let asset = metadata.as_array()?.first()?;
    let package = asset.get("binary")?.get("package")?;
    Some(AdoptiumPackage {
        link: package.get("link")?.as_str()?.to_owned(),
        name: package.get("name")?.as_str()?.to_owned(),
        release_name: asset.get("release_name")?.as_str()?.to_owned(),
        sha256: package.get("checksum").and_then(|value| value.as_str()).map(str::to_owned),
    })
}

fn managed_java_root() -> PathBuf {
    cache_root_dir().join("java")
}

fn runtime_dir_prefix(runtime_major: u8, arch_cache_key: &str) -> String {
    format!("openjdk-{runtime_major}-{arch_cache_key}")
}

fn release_dir_name(runtime_major: u8, arch_cache_key: &str, release_name: &str) -> String {
    let release: String = release_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    format!("{}-{release}", runtime_dir_prefix(runtime_major, arch_cache_key))
}

/// The Java executable of the newest managed install of `runtime_major` (the unversioned folder older launchers made counts too).
fn find_managed_runtime(runtime_major: u8, arch_cache_key: &str) -> Result<Option<PathBuf>, InstallationError> {
    let root = managed_java_root();
    let prefix = runtime_dir_prefix(runtime_major, arch_cache_key);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(None);
    };
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name != prefix && !name.starts_with(&format!("{prefix}-")) {
            continue;
        }
        let Some(executable) = find_java_executable_under(&entry.path())? else { continue };
        let modified = entry.metadata().and_then(|meta| meta.modified()).unwrap_or(UNIX_EPOCH);
        if best.as_ref().is_none_or(|(newest, _)| modified > *newest) {
            best = Some((modified, executable));
        }
    }
    Ok(best.map(|(_, executable)| canonicalize_existing_path(executable)))
}

/// The version-and-build string of the runtime at `java`, from the executable itself: the `(build ...)` part of its second line
/// (`17.0.13+11`, `1.8.0_432-b06`), else the first version-looking token of the output.
pub fn query_java_release(java: &Path) -> Option<String> {
    for flag in ["--version", "-version"] {
        let mut command = Command::new(java);
        command.arg(flag).stdin(Stdio::null());
        #[cfg(target_os = "windows")]
        command.creation_flags(CREATE_NO_WINDOW);
        let Ok(output) = command.output() else { continue };
        if !output.status.success() {
            continue;
        }
        // `-version` prints to stderr, `--version` to stdout.
        let text = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        if let Some(release) = parse_java_release(&text) {
            return Some(release);
        }
    }
    None
}

fn parse_java_release(output: &str) -> Option<String> {
    if let Some(start) = output.find("(build ") {
        let rest = &output[start + "(build ".len()..];
        let end = rest.find([')', ',', ' ']).unwrap_or(rest.len());
        let build = rest[..end].trim();
        if build.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return Some(build.to_owned());
        }
    }
    // `openjdk 17.0.13 2024-10-15` / `openjdk version "1.8.0_432"`.
    let first = output.lines().map(str::trim).find(|line| !line.is_empty())?;
    first
        .split(|c: char| c.is_whitespace() || c == '"')
        .find(|token| token.chars().next().is_some_and(|c| c.is_ascii_digit()) && token.chars().any(|c| c == '.'))
        .map(str::to_owned)
}

/// A comparable form of a release string: `jdk-17.0.13+11`, `17.0.13+11`, `1.8.0_432-b06` and `jdk8u432-b06` all become number lists
/// (`[17, 0, 13, 11]`, `[8, 0, 432, 6]`).
fn release_key(release: &str) -> Vec<u32> {
    let trimmed = release.trim();
    let trimmed = trimmed.strip_prefix("jdk-").or_else(|| trimmed.strip_prefix("jdk")).unwrap_or(trimmed);
    let mut numbers: Vec<u32> = trimmed
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();
    if trimmed.starts_with("1.") && numbers.len() > 1 {
        numbers.remove(0);
    } else if trimmed.starts_with("8u") && !numbers.is_empty() {
        numbers.insert(1, 0);
    }
    numbers
}

/// Whether `installed` is at least as new as `latest`. A missing build number (`17.0.13` against `17.0.13+11`) counts as equal.
fn is_current(installed: &str, latest: &str) -> bool {
    let (installed, latest) = (release_key(installed), release_key(latest));
    if installed.is_empty() {
        return false;
    }
    let shared = installed.len().min(latest.len());
    installed[..shared] >= latest[..shared]
}

fn download_verified(package: &AdoptiumPackage, archive: &Path) -> Result<(), InstallationError> {
    download_file_simple(&package.link, archive)?;
    let Some(expected) = package.sha256.as_deref() else {
        return Ok(());
    };
    let mut file = fs_file_open(archive)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    if actual.eq_ignore_ascii_case(expected.trim()) {
        return Ok(());
    }
    let _ = std::fs::remove_file(archive);
    Err(InstallationError::Io(std::io::Error::new(
        ErrorKind::InvalidData,
        format!("{} does not match its published SHA-256 checksum", package.name),
    )))
}

/// Downloads and unpacks `package` into its own folder (unpacked beside it first, so a failed run never leaves a half-made runtime
/// that looks installed) and returns its Java executable.
fn install_package(runtime_major: u8, arch_cache_key: &str, package: &AdoptiumPackage) -> Result<PathBuf, InstallationError> {
    let root = managed_java_root();
    let target = root.join(release_dir_name(runtime_major, arch_cache_key, &package.release_name));
    if let Some(existing) = find_java_executable_under(&target)? {
        return Ok(canonicalize_existing_path(existing));
    }
    fs_create_dir_all(&root)?;
    let downloads = cache_root_dir().join("downloads");
    fs_create_dir_all(&downloads)?;
    let archive = downloads.join(&package.name);
    download_verified(package, &archive)?;

    let staging = target.with_file_name(format!("{}.installing", target.file_name().and_then(|n| n.to_str()).unwrap_or("openjdk")));
    extract_archive(&archive, &staging)?;
    if target.exists() {
        fs_remove_dir_all(&target)?;
    }
    fs_rename(&staging, &target)?;
    // The archive is a hundred-odd megabytes the unpacked runtime no longer needs.
    let _ = std::fs::remove_file(&archive);

    let installed = find_java_executable_under(&target)?.ok_or(InstallationError::OpenJdkMetadataMissing { runtime_major })?;
    Ok(canonicalize_existing_path(installed))
}

fn fetch_latest(runtime_major: u8, os: &str, arch: &str) -> Result<AdoptiumPackage, InstallationError> {
    let metadata_url = format!(
        "https://api.adoptium.net/v3/assets/latest/{runtime_major}/hotspot?architecture={arch}&image_type=jdk&jvm_impl=hotspot&os={os}&vendor=eclipse"
    );
    let metadata: serde_json::Value = get_json_with_user_agent(metadata_url.as_str(), OPENJDK_USER_AGENT)?;
    extract_adoptium_release(&metadata).ok_or(InstallationError::OpenJdkMetadataMissing { runtime_major })
}

/// The managed runtime for `runtime_major`, installing the latest release when there is none.
pub fn ensure_openjdk_runtime(runtime_major: u8) -> Result<PathBuf, InstallationError> {
    let (os, arch, arch_cache_key) = platform_for_adoptium()?;
    if let Some(existing) = find_managed_runtime(runtime_major, arch_cache_key)? {
        return Ok(existing);
    }
    let package = fetch_latest(runtime_major, os, arch)?;
    install_package(runtime_major, arch_cache_key, &package)
}

pub async fn ensure_openjdk_runtime_async(runtime_major: u8) -> Result<PathBuf, InstallationError> {
    launcher_runtime::spawn_blocking(move || ensure_openjdk_runtime(runtime_major)).await.map_err(map_join_error)?
}

/// Looks up the newest release of `runtime_major`, compares it with what `current` (the configured executable) reports, and installs
/// the newer one when the current runtime is older, missing or cannot be asked. The caller switches its configuration to
/// [`JavaUpdateReport::installed`] and then calls [`remove_replaced_openjdk`] for the old executable.
pub fn update_openjdk_runtime(runtime_major: u8, current: Option<&Path>) -> Result<JavaUpdateReport, InstallationError> {
    let (os, arch, arch_cache_key) = platform_for_adoptium()?;
    let package = fetch_latest(runtime_major, os, arch)?;
    let installed_release = current.filter(|path| path.exists()).and_then(query_java_release);
    let current_enough = installed_release.as_deref().is_some_and(|installed| is_current(installed, &package.release_name));
    let installed = if current_enough {
        None
    } else {
        Some(install_package(runtime_major, arch_cache_key, &package)?)
    };
    Ok(JavaUpdateReport { runtime_major, latest_release: package.release_name, installed_release, installed })
}

pub async fn update_openjdk_runtime_async(runtime_major: u8, current: Option<PathBuf>) -> Result<JavaUpdateReport, InstallationError> {
    launcher_runtime::spawn_blocking(move || update_openjdk_runtime(runtime_major, current.as_deref()))
        .await
        .map_err(map_join_error)?
}

/// Removes the install folder holding `old` once `keep` has replaced it. Only runtimes the launcher itself installed (under its java
/// cache) are touched: a JDK the user pointed at (`/usr/lib/jvm/...`) is never deleted. Returns whether anything was removed.
pub fn remove_replaced_openjdk(old: &Path, keep: &Path) -> Result<bool, InstallationError> {
    let Ok(root) = fs_canonicalize(managed_java_root().as_path()) else {
        return Ok(false);
    };
    let Ok(old) = fs_canonicalize(old) else {
        return Ok(false);
    };
    let install_dir = |path: &Path| path.strip_prefix(&root).ok().and_then(|rest| rest.components().next()).map(|first| root.join(first));
    let Some(old_dir) = install_dir(&old) else {
        return Ok(false);
    };
    let keep = fs_canonicalize(keep).unwrap_or_else(|_| keep.to_path_buf());
    if install_dir(&keep).as_ref() == Some(&old_dir) {
        return Ok(false);
    }
    fs_remove_dir_all(&old_dir)?;
    Ok(true)
}

pub async fn remove_replaced_openjdk_async(old: PathBuf, keep: PathBuf) -> Result<bool, InstallationError> {
    launcher_runtime::spawn_blocking(move || remove_replaced_openjdk(&old, &keep)).await.map_err(map_join_error)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_build_from_modern_and_java_8_output() {
        let modern = "openjdk 17.0.13 2024-10-15\nOpenJDK Runtime Environment Temurin-17.0.13+11 (build 17.0.13+11)\nOpenJDK 64-Bit Server VM Temurin-17.0.13+11 (build 17.0.13+11, mixed mode, sharing)\n";
        assert_eq!(parse_java_release(modern).as_deref(), Some("17.0.13+11"));
        let java8 = "openjdk version \"1.8.0_432\"\nOpenJDK Runtime Environment (Temurin)(build 1.8.0_432-b06)\nOpenJDK 64-Bit Server VM (Temurin)(build 25.432-b06, mixed mode)\n";
        assert_eq!(parse_java_release(java8).as_deref(), Some("1.8.0_432-b06"));
        assert_eq!(parse_java_release("openjdk 21.0.5 2024-10-15 LTS\n").as_deref(), Some("21.0.5"));
        assert_eq!(parse_java_release("not java"), None);
    }

    #[test]
    fn release_strings_from_different_sources_compare_equal() {
        assert!(is_current("17.0.13+11", "jdk-17.0.13+11"));
        assert!(is_current("1.8.0_432-b06", "jdk8u432-b06"));
        assert!(is_current("21.0.5", "jdk-21.0.5+11"));
        assert!(is_current("17.0.14+7", "jdk-17.0.13+11"));
        assert!(!is_current("17.0.12+7", "jdk-17.0.13+11"));
        assert!(!is_current("17.0.13+9", "jdk-17.0.13+11"));
        assert!(!is_current("1.8.0_422-b05", "jdk8u432-b06"));
        assert!(!is_current("", "jdk-17.0.13+11"));
    }

    #[test]
    fn release_folders_are_filesystem_safe() {
        assert_eq!(release_dir_name(17, "x64", "jdk-17.0.13+11"), "openjdk-17-x64-jdk-17.0.13_11");
    }

    #[test]
    fn only_launcher_installed_runtimes_are_removed() {
        let outside = std::env::temp_dir().join("vertex-java-update-test-outside");
        let _ = std::fs::create_dir_all(outside.join("bin"));
        let executable = outside.join("bin").join("java");
        let _ = std::fs::write(&executable, b"");
        assert_eq!(remove_replaced_openjdk(&executable, &executable).ok(), Some(false));
        assert!(executable.exists());
        let _ = std::fs::remove_dir_all(&outside);
    }
}

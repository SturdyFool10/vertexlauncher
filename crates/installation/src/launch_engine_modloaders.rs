use super::*;

#[cfg(test)]
#[path = "launch_engine_modloaders_tests.rs"]
mod launch_engine_modloaders_tests;

pub(crate) fn install_selected_modloader(
    instance_root: &Path,
    game_version: &str,
    modloader: &str,
    modloader_version: Option<&str>,
    java_executable: Option<&str>,
    policy: &DownloadPolicy,
    downloaded_files: &mut u32,
    progress: Option<&InstallProgressSink>,
) -> Result<Option<String>, InstallationError> {
    let loader_kind = normalized_loader_label(modloader);
    tracing::info!(
        target: "vertexlauncher/installation/modloader",
        requested_modloader = %modloader,
        requested_game_version = %game_version,
        requested_modloader_version = %modloader_version.unwrap_or(""),
        "Selecting modloader installation strategy."
    );
    match loader_kind {
        LoaderKind::Vanilla | LoaderKind::Custom => Ok(None),
        LoaderKind::Fabric | LoaderKind::Quilt => {
            let loader_label = if loader_kind == LoaderKind::Fabric {
                "Fabric"
            } else {
                "Quilt"
            };
            let resolved =
                resolve_loader_version(loader_kind, loader_label, game_version, modloader_version)?;
            if has_fabric_or_quilt_profile(instance_root, game_version, loader_kind, &resolved)? {
                tracing::info!(
                    target: "vertexlauncher/installation/modloader",
                    loader = %loader_label,
                    game_version = %game_version,
                    resolved = %resolved,
                    "Modloader profile already present; skipping profile install."
                );
                return Ok(Some(resolved));
            }
            emit_installing_modloader_progress(
                loader_label,
                &resolved,
                *downloaded_files,
                progress,
            );
            *downloaded_files += install_fabric_or_quilt_profile(
                instance_root,
                game_version,
                loader_kind,
                &resolved,
                policy,
                *downloaded_files,
                progress,
            )?;
            Ok(Some(resolved))
        }
        LoaderKind::Forge => {
            let resolved =
                resolve_loader_version(loader_kind, "Forge", game_version, modloader_version)?;
            if verify_modloader_profile(instance_root, loader_kind, game_version, &resolved)? {
                tracing::info!(
                    target: "vertexlauncher/installation/modloader",
                    loader = "Forge",
                    game_version = %game_version,
                    resolved = %resolved,
                    "Modloader profile already present; skipping installer execution."
                );
                return Ok(Some(resolved));
            }
            emit_installing_modloader_progress("Forge", &resolved, *downloaded_files, progress);
            *downloaded_files += install_forge_installer(
                instance_root,
                game_version,
                &resolved,
                java_executable,
                policy,
                *downloaded_files,
                progress,
            )?;
            Ok(Some(resolved))
        }
        LoaderKind::NeoForge => {
            let resolved =
                resolve_loader_version(loader_kind, "NeoForge", game_version, modloader_version)?;
            if verify_modloader_profile(instance_root, loader_kind, game_version, &resolved)? {
                tracing::info!(
                    target: "vertexlauncher/installation/modloader",
                    loader = "NeoForge",
                    game_version = %game_version,
                    resolved = %resolved,
                    "Modloader profile already present; skipping installer execution."
                );
                return Ok(Some(resolved));
            }
            emit_installing_modloader_progress("NeoForge", &resolved, *downloaded_files, progress);
            *downloaded_files += install_neoforge_installer(
                instance_root,
                game_version,
                &resolved,
                java_executable,
                policy,
                *downloaded_files,
                progress,
            )?;
            Ok(Some(resolved))
        }
    }
}

pub(crate) fn resolve_loader_version(
    _loader_kind: LoaderKind,
    loader_label: &str,
    game_version: &str,
    requested: Option<&str>,
) -> Result<String, InstallationError> {
    let versions = fetch_loader_versions_for_game(loader_label, game_version, false)?;
    if let Some(value) = requested.map(str::trim).filter(|value| !value.is_empty())
        && !is_latest_loader_version_alias(value)
    {
        let supported = versions.iter().any(|candidate| candidate == value);
        if !supported {
            tracing::warn!(
                target: "vertexlauncher/installation/modloader",
                loader = %loader_label,
                game_version = %game_version,
                requested = %value,
                supported_versions = ?versions,
                "Requested modloader version is not compatible with selected Minecraft version."
            );
            return Err(InstallationError::MissingModloaderVersion {
                loader: loader_label.to_owned(),
                game_version: game_version.to_owned(),
            });
        }
        tracing::info!(
            target: "vertexlauncher/installation/modloader",
            loader = %loader_label,
            game_version = %game_version,
            requested = %value,
            "Using explicitly requested compatible modloader version."
        );
        return Ok(value.to_owned());
    }
    let resolved =
        versions
            .first()
            .cloned()
            .ok_or_else(|| InstallationError::MissingModloaderVersion {
                loader: loader_label.to_owned(),
                game_version: game_version.to_owned(),
            })?;
    tracing::info!(
        target: "vertexlauncher/installation/modloader",
        loader = %loader_label,
        game_version = %game_version,
        resolved = %resolved,
        "Resolved latest compatible modloader version."
    );
    Ok(resolved)
}

pub(crate) fn is_latest_loader_version_alias(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "latest" | "latest available" | "use latest version" | "auto" | "default"
    )
}

pub(crate) fn emit_installing_modloader_progress(
    loader_label: &str,
    loader_version: &str,
    downloaded_files: u32,
    progress: Option<&InstallProgressSink>,
) {
    report_install_progress(
        progress,
        InstallProgress {
            stage: InstallStage::InstallingModloader,
            message: format!("Installing {loader_label} {loader_version} artifacts..."),
            downloaded_files,
            total_files: downloaded_files.max(1),
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
        },
    );
}

pub(crate) fn has_fabric_or_quilt_profile(
    instance_root: &Path,
    game_version: &str,
    loader_kind: LoaderKind,
    loader_version: &str,
) -> Result<bool, InstallationError> {
    let id_prefix = match loader_kind {
        LoaderKind::Fabric => "fabric-loader",
        LoaderKind::Quilt => "quilt-loader",
        _ => return Ok(false),
    };
    let version_id = format!("{id_prefix}-{loader_version}-{game_version}");
    let profile_path = instance_root
        .join("versions")
        .join(version_id.as_str())
        .join(format!("{version_id}.json"));
    if !profile_path.exists() {
        return Ok(false);
    }
    let raw = match fs_read_to_string(profile_path.as_path()) {
        Ok(contents) => contents,
        Err(_) => return Ok(false),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let id = parsed
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if id.eq_ignore_ascii_case(version_id.as_str()) {
        return Ok(true);
    }
    let inherits = parsed
        .get("inheritsFrom")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let game_version_lower = game_version.to_ascii_lowercase();
    let loader_version_lower = loader_version.to_ascii_lowercase();
    let id_lower = id.to_ascii_lowercase();
    Ok(id_lower.contains(loader_version_lower.as_str())
        && id_lower.contains(id_prefix)
        && (inherits == game_version_lower || inherits.starts_with(game_version_lower.as_str())))
}

pub(crate) fn install_fabric_or_quilt_profile(
    instance_root: &Path,
    game_version: &str,
    loader_kind: LoaderKind,
    loader_version: &str,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    let profile_url = match loader_kind {
        LoaderKind::Fabric => format!(
            "{}/{}/{}/profile/json",
            FABRIC_VERSION_MATRIX_URL.trim_end_matches('/'),
            url_encode_component(game_version),
            url_encode_component(loader_version),
        ),
        LoaderKind::Quilt => format!(
            "{}/{}/{}/profile/json",
            QUILT_VERSION_MATRIX_URL.trim_end_matches('/'),
            url_encode_component(game_version),
            url_encode_component(loader_version),
        ),
        _ => return Ok(0),
    };

    let id_prefix = match loader_kind {
        LoaderKind::Fabric => "fabric-loader",
        LoaderKind::Quilt => "quilt-loader",
        _ => "loader",
    };
    let version_id = format!("{id_prefix}-{loader_version}-{game_version}");
    let profile_path = instance_root
        .join("versions")
        .join(version_id.as_str())
        .join(format!("{version_id}.json"));
    let task = FileDownloadTask {
        url: profile_url,
        destination: profile_path,
        expected_size: None,
    };
    download_files_concurrent(
        InstallStage::InstallingModloader,
        vec![task],
        policy,
        downloaded_files_offset,
        progress,
    )
}

pub(crate) fn install_forge_installer(
    instance_root: &Path,
    game_version: &str,
    loader_version: &str,
    java_executable: Option<&str>,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    let artifact_version = format!("{game_version}-{loader_version}");
    let installer_file = format!("forge-{artifact_version}-installer.jar");
    let url = format!(
        "https://maven.minecraftforge.net/net/minecraftforge/forge/{artifact_version}/{installer_file}"
    );
    let destination = instance_root
        .join("loaders")
        .join("forge")
        .join(game_version)
        .join(loader_version)
        .join(installer_file);
    let mut tasks = Vec::new();
    if !destination.exists() {
        tasks.push(FileDownloadTask {
            url,
            destination,
            expected_size: None,
        });
    }
    let downloaded = download_files_concurrent(
        InstallStage::InstallingModloader,
        tasks,
        policy,
        downloaded_files_offset,
        progress,
    )?;
    run_modloader_installer_and_verify(
        instance_root,
        LoaderKind::Forge,
        game_version,
        loader_version,
        java_executable,
    )?;
    Ok(downloaded)
}

pub(crate) fn install_neoforge_installer(
    instance_root: &Path,
    game_version: &str,
    loader_version: &str,
    java_executable: Option<&str>,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    let installer_file = format!("neoforge-{loader_version}-installer.jar");
    let url = format!(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/{loader_version}/{installer_file}"
    );
    let destination = instance_root
        .join("loaders")
        .join("neoforge")
        .join(game_version)
        .join(loader_version)
        .join(installer_file);
    let mut tasks = Vec::new();
    if !destination.exists() {
        tasks.push(FileDownloadTask {
            url,
            destination,
            expected_size: None,
        });
    }
    let downloaded = download_files_concurrent(
        InstallStage::InstallingModloader,
        tasks,
        policy,
        downloaded_files_offset,
        progress,
    )?;
    run_modloader_installer_and_verify(
        instance_root,
        LoaderKind::NeoForge,
        game_version,
        loader_version,
        java_executable,
    )?;
    Ok(downloaded)
}

pub(crate) fn run_modloader_installer_and_verify(
    instance_root: &Path,
    loader_kind: LoaderKind,
    game_version: &str,
    loader_version: &str,
    java_executable: Option<&str>,
) -> Result<(), InstallationError> {
    let loader_label = match loader_kind {
        LoaderKind::Forge => "Forge",
        LoaderKind::NeoForge => "NeoForge",
        _ => return Ok(()),
    };
    let configured_java = java_executable
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| InstallationError::MissingJavaRuntime {
            loader: loader_label.to_owned(),
        })?;
    let java = normalize_java_executable(Some(configured_java.as_str()));
    if java == "java" && configured_java != "java" {
        tracing::warn!(
            target: "vertexlauncher/installation/modloader",
            "Configured Java path missing ({}), falling back to `java` from PATH.",
            configured_java
        );
    }
    let installer_path =
        find_installer_jar(instance_root, loader_kind, game_version, loader_version)?.ok_or_else(
            || InstallationError::ModloaderInstallOutputMissing {
                loader: loader_label.to_owned(),
                game_version: game_version.to_owned(),
                loader_version: loader_version.to_owned(),
                versions_dir: instance_root.join("versions"),
            },
        )?;
    let installer_path = match fs_canonicalize(installer_path.as_path()) {
        Ok(path) => path,
        Err(_) => installer_path,
    };
    ensure_launcher_profiles(instance_root)?;
    let installer_target =
        fs_canonicalize(instance_root).unwrap_or_else(|_| instance_root.to_path_buf());
    let installer_path = normalize_child_process_path(installer_path.as_path());
    let installer_target = normalize_child_process_path(installer_target.as_path());
    let installer_path_arg = display_user_path(installer_path.as_path());
    let installer_target_arg = display_user_path(installer_target.as_path());

    // Try both flag variants used by Forge/NeoForge installers.
    let mut last_failure = None;
    for flag in ["--installClient", "--install-client"] {
        let mut cmd = Command::new(java.as_str());
        cmd.arg("-jar")
            .arg(installer_path.as_os_str())
            .arg(flag)
            .arg(installer_target.as_os_str())
            .current_dir(installer_target.as_path());
        let command_line = format!(
            "{} -jar {} {} {}",
            java, installer_path_arg, flag, installer_target_arg
        );
        let output = run_command_output(&mut cmd, java.as_str())?;
        if output.status.success() {
            if verify_modloader_profile(instance_root, loader_kind, game_version, loader_version)? {
                return Ok(());
            }
            return Err(InstallationError::ModloaderInstallOutputMissing {
                loader: loader_label.to_owned(),
                game_version: game_version.to_owned(),
                loader_version: loader_version.to_owned(),
                versions_dir: instance_root.join("versions"),
            });
        }
        last_failure = Some((command_line, output.status.code(), output.stderr));
    }

    let (command, status_code, stderr_bytes) = last_failure.unwrap_or_default();
    Err(InstallationError::ModloaderInstallerFailed {
        loader: loader_label.to_owned(),
        game_version: game_version.to_owned(),
        loader_version: loader_version.to_owned(),
        command,
        status: status_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated by signal".to_owned()),
        stderr: String::from_utf8_lossy(&stderr_bytes).trim().to_owned(),
    })
}

pub(crate) fn ensure_launcher_profiles(instance_root: &Path) -> Result<(), InstallationError> {
    let profile_path = instance_root.join("launcher_profiles.json");
    if profile_path.exists() {
        return Ok(());
    }
    let profile = serde_json::json!({
        "profiles": {},
        "selectedProfile": null,
        "clientToken": "vertexlauncher",
        "authenticationDatabase": {},
        "launcherVersion": {
            "name": "Vertex Launcher",
            "format": 21
        },
        "settings": {}
    });
    fs_write(profile_path, serde_json::to_string_pretty(&profile)?)?;
    Ok(())
}

pub(crate) fn find_installer_jar(
    instance_root: &Path,
    loader_kind: LoaderKind,
    game_version: &str,
    loader_version: &str,
) -> Result<Option<PathBuf>, InstallationError> {
    let file_name = match loader_kind {
        LoaderKind::Forge => format!("forge-{game_version}-{loader_version}-installer.jar"),
        LoaderKind::NeoForge => format!("neoforge-{loader_version}-installer.jar"),
        _ => return Ok(None),
    };
    let loader_dir = match loader_kind {
        LoaderKind::Forge => "forge",
        LoaderKind::NeoForge => "neoforge",
        _ => "",
    };
    let path = instance_root
        .join("loaders")
        .join(loader_dir)
        .join(game_version)
        .join(loader_version)
        .join(file_name);
    Ok(path.exists().then_some(path))
}

pub(crate) fn verify_modloader_profile(
    instance_root: &Path,
    loader_kind: LoaderKind,
    game_version: &str,
    loader_version: &str,
) -> Result<bool, InstallationError> {
    let versions_dir = instance_root.join("versions");
    if !versions_dir.exists() {
        return Ok(false);
    }
    let loader_hint = match loader_kind {
        LoaderKind::Forge => "forge",
        LoaderKind::NeoForge => "neoforge",
        _ => return Ok(true),
    };
    for entry in fs_read_dir(&versions_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir_name = entry.file_name();
        let dir_name = dir_name.to_string_lossy();
        let profile_path = entry.path().join(format!("{dir_name}.json"));
        if !profile_path.exists() {
            continue;
        }
        let raw = fs_read_to_string(&profile_path)?;
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let id = parsed
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let inherits = parsed
            .get("inheritsFrom")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let game_version_lower = game_version.to_ascii_lowercase();
        let loader_version_lower = loader_version.to_ascii_lowercase();
        let matches_loader = id.contains(loader_hint)
            || (loader_kind == LoaderKind::NeoForge && id.contains("forge"));
        let matches_version = id.contains(loader_version_lower.as_str());
        let matches_game = id.contains(game_version_lower.as_str())
            || inherits == game_version_lower
            || inherits.starts_with(game_version_lower.as_str());
        if matches_loader && matches_version && matches_game {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn cache_root_dir() -> PathBuf {
    app_paths::cache_root()
}

pub(crate) fn canonicalize_existing_path(path: PathBuf) -> PathBuf {
    fs_canonicalize(path.as_path()).unwrap_or(path)
}

pub(crate) fn platform_for_adoptium()
-> Result<(&'static str, &'static str, &'static str), InstallationError> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        return Err(InstallationError::UnsupportedPlatform(
            std::env::consts::OS.to_owned(),
        ));
    };
    let arch = current_runtime_architecture().ok_or_else(|| {
        InstallationError::UnsupportedPlatform(
            detected_runtime_architecture().unwrap_or_else(|| std::env::consts::ARCH.to_owned()),
        )
    })?;
    Ok((os, arch.adoptium_value(), arch.cache_key()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeArchitecture {
    X86,
    X64,
    Arm,
    Aarch64,
}

impl RuntimeArchitecture {
    fn adoptium_value(self) -> &'static str {
        match self {
            RuntimeArchitecture::X86 => "x32",
            RuntimeArchitecture::X64 => "x64",
            RuntimeArchitecture::Arm => "arm",
            RuntimeArchitecture::Aarch64 => "aarch64",
        }
    }

    fn cache_key(self) -> &'static str {
        match self {
            RuntimeArchitecture::X86 => "x86",
            RuntimeArchitecture::X64 => "x64",
            RuntimeArchitecture::Arm => "arm",
            RuntimeArchitecture::Aarch64 => "aarch64",
        }
    }
}

pub(crate) fn current_runtime_architecture() -> Option<RuntimeArchitecture> {
    normalize_runtime_architecture(
        detected_runtime_architecture()
            .unwrap_or_else(|| std::env::consts::ARCH.to_owned())
            .as_str(),
    )
}

pub(crate) fn normalize_runtime_architecture(raw: &str) -> Option<RuntimeArchitecture> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "x86" | "i386" | "i486" | "i586" | "i686" => Some(RuntimeArchitecture::X86),
        "x86_64" | "amd64" => Some(RuntimeArchitecture::X64),
        "arm" | "armv7" | "armv7l" => Some(RuntimeArchitecture::Arm),
        "arm64" | "aarch64" => Some(RuntimeArchitecture::Aarch64),
        _ => None,
    }
}

pub(crate) fn detected_runtime_architecture() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        return std::env::var("PROCESSOR_ARCHITEW6432")
            .ok()
            .or_else(|| std::env::var("PROCESSOR_ARCHITECTURE").ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
    }

    #[cfg(target_os = "macos")]
    {
        let uname = command_stdout_trimmed("uname", ["-m"])?;
        if normalize_runtime_architecture(uname.as_str()) == Some(RuntimeArchitecture::X64)
            && command_stdout_trimmed("sysctl", ["-in", "hw.optional.arm64"]).as_deref()
                == Some("1")
        {
            return Some("arm64".to_owned());
        }
        return Some(uname);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return command_stdout_trimmed("uname", ["-m"]);
    }

    #[allow(unreachable_code)]
    None
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn command_stdout_trimmed<const N: usize>(
    program: &str,
    args: [&str; N],
) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod runtime_architecture_tests {
    use super::{RuntimeArchitecture, normalize_runtime_architecture};

    #[test]
    fn normalizes_common_x64_aliases() {
        assert_eq!(
            normalize_runtime_architecture("x86_64"),
            Some(RuntimeArchitecture::X64)
        );
        assert_eq!(
            normalize_runtime_architecture("AMD64"),
            Some(RuntimeArchitecture::X64)
        );
    }

    #[test]
    fn normalizes_common_arm64_aliases() {
        assert_eq!(
            normalize_runtime_architecture("arm64"),
            Some(RuntimeArchitecture::Aarch64)
        );
        assert_eq!(
            normalize_runtime_architecture("aarch64"),
            Some(RuntimeArchitecture::Aarch64)
        );
    }

    #[test]
    fn normalizes_x86_aliases() {
        assert_eq!(
            normalize_runtime_architecture("i686"),
            Some(RuntimeArchitecture::X86)
        );
    }

    #[test]
    fn rejects_unknown_architecture_strings() {
        assert_eq!(normalize_runtime_architecture("sparc64"), None);
    }
}

pub(crate) fn extract_adoptium_package(metadata: &serde_json::Value) -> Option<(String, String)> {
    let package = metadata
        .as_array()?
        .first()?
        .get("binary")?
        .get("package")?;
    let link = package.get("link")?.as_str()?.to_owned();
    let name = package.get("name")?.as_str()?.to_owned();
    Some((link, name))
}

pub(crate) fn download_file_simple(url: &str, destination: &Path) -> Result<(), InstallationError> {
    if destination.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs_create_dir_all(parent)?;
    }
    let response = ureq::get(url)
        .header("User-Agent", OPENJDK_USER_AGENT)
        .call()
        .map_err(map_ureq_error)?;
    let (_, body) = response.into_parts();
    let mut reader = body.into_reader();
    let temp = temporary_download_path(destination);
    let mut file = fs_file_create(&temp)?;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url,
                temp_path = %temp.display(),
                destination = %destination.display(),
                error = %err,
                "Failed while reading OpenJDK download response body."
            );
            InstallationError::Io(err)
        })?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url,
                temp_path = %temp.display(),
                destination = %destination.display(),
                error = %err,
                "Failed while writing OpenJDK download chunk to temporary file."
            );
            InstallationError::Io(err)
        })?;
    }
    file.flush().map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url,
            temp_path = %temp.display(),
            destination = %destination.display(),
            error = %err,
            "Failed while flushing OpenJDK temporary download file."
        );
        InstallationError::Io(err)
    })?;
    fs_rename(temp.as_path(), destination).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url,
            temp_path = %temp.display(),
            destination = %destination.display(),
            error = %err,
            "Failed while promoting OpenJDK temporary download file into place."
        );
        err
    })?;
    Ok(())
}

pub(crate) fn temporary_download_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.downloading"))
        .unwrap_or_else(|| "download.downloading".to_owned());
    destination.with_file_name(file_name)
}

pub(crate) fn extract_archive(
    archive_path: &Path,
    destination: &Path,
) -> Result<(), InstallationError> {
    if destination.exists() {
        fs_remove_dir_all(destination)?;
    }
    fs_create_dir_all(destination)?;
    let file_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if file_name.ends_with(".zip") {
        let file = fs_file_open(archive_path)?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|err| InstallationError::Io(std::io::Error::other(err.to_string())))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|err| InstallationError::Io(std::io::Error::other(err.to_string())))?;
            let Some(enclosed) = entry.enclosed_name() else {
                continue;
            };
            let out_path = destination.join(enclosed);
            if entry.is_dir() {
                fs_create_dir_all(&out_path)?;
                continue;
            }
            if let Some(parent) = out_path.parent() {
                fs_create_dir_all(parent)?;
            }
            let mut out = fs_file_create(&out_path)?;
            std::io::copy(&mut entry, &mut out)?;
        }
        return Ok(());
    }

    if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
        let tar_gz = fs_file_open(archive_path)?;
        let decoder = flate2::read::GzDecoder::new(tar_gz);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(destination)?;
        return Ok(());
    }

    Err(InstallationError::Io(std::io::Error::new(
        ErrorKind::InvalidInput,
        format!("unsupported archive format: {}", archive_path.display()),
    )))
}

pub(crate) fn find_java_executable_under(
    root: &Path,
) -> Result<Option<PathBuf>, InstallationError> {
    if !root.exists() {
        return Ok(None);
    }
    let binary = if cfg!(target_os = "windows") {
        "java.exe"
    } else {
        "java"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs_read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.eq_ignore_ascii_case(binary)
                && path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|n| n.to_str())
                    .is_some_and(|part| part.eq_ignore_ascii_case("bin"))
            {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

pub(crate) fn cache_file_path(include_snapshots_and_betas: bool) -> PathBuf {
    let file_name = if include_snapshots_and_betas {
        CACHE_VERSION_CATALOG_ALL_FILE
    } else {
        CACHE_VERSION_CATALOG_RELEASES_FILE
    };
    cache_root_dir().join(file_name)
}

pub(crate) fn read_cached_version_catalog(
    include_snapshots_and_betas: bool,
) -> Result<CachedVersionCatalog, InstallationError> {
    let raw = fs_read_to_string(cache_file_path(include_snapshots_and_betas))?;
    Ok(serde_json::from_str(&raw)?)
}

pub(crate) fn write_cached_version_catalog(
    include_snapshots_and_betas: bool,
    catalog: &VersionCatalog,
) -> Result<(), InstallationError> {
    let path = cache_file_path(include_snapshots_and_betas);
    if let Some(parent) = path.parent() {
        fs_create_dir_all(parent)?;
    }

    let payload = CachedVersionCatalog {
        fetched_at_unix_secs: now_unix_secs(),
        include_snapshots_and_betas,
        catalog: catalog.clone(),
    };
    let file = fs_file_create(path)?;
    serde_json::to_writer_pretty(file, &payload)?;
    Ok(())
}

pub(crate) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub(crate) fn is_cache_expired(fetched_at_unix_secs: u64) -> bool {
    let now = now_unix_secs();
    now.saturating_sub(fetched_at_unix_secs) > VERSION_CATALOG_CACHE_TTL.as_secs()
}

pub(crate) fn catalog_has_loader_version_data(catalog: &VersionCatalog) -> bool {
    let loader_versions = &catalog.loader_versions;
    [
        &loader_versions.fabric,
        &loader_versions.forge,
        &loader_versions.neoforge,
        &loader_versions.quilt,
    ]
    .into_iter()
    .any(|versions_by_game_version| {
        versions_by_game_version
            .values()
            .any(|versions| !versions.is_empty())
    })
}

pub(crate) fn normalize_version_catalog_ordering(catalog: &mut VersionCatalog) {
    catalog
        .game_versions
        .sort_by(|left, right| compare_version_like_desc(left.id.as_str(), right.id.as_str()));
    catalog.loader_versions.sort_desc();
}

pub(crate) fn fetch_fabric_versions() -> Result<HashSet<String>, InstallationError> {
    let versions: Vec<FabricGameVersion> = get_json(FABRIC_GAME_VERSIONS_URL)?;
    Ok(versions
        .into_iter()
        .map(|version| version.version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .collect())
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LoaderVersionCatalog {
    pub(crate) supported_game_versions: HashSet<String>,
    pub(crate) versions_by_game_version: BTreeMap<String, Vec<String>>,
}

impl LoaderVersionCatalog {
    fn finalize(mut self) -> Self {
        self.supported_game_versions = self.versions_by_game_version.keys().cloned().collect();
        sort_loader_version_map_desc(&mut self.versions_by_game_version);
        self
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LoaderVersionFetchResult {
    pub(crate) selected_versions: Vec<String>,
    pub(crate) versions_by_game_version: BTreeMap<String, Vec<String>>,
}

pub(crate) fn fetch_fabric_loader_catalog() -> Result<LoaderVersionCatalog, InstallationError> {
    let matrix: serde_json::Value = get_json(FABRIC_VERSION_MATRIX_URL)?;
    Ok(parse_loader_version_matrix(&matrix))
}

pub(crate) fn fetch_quilt_versions() -> Result<HashSet<String>, InstallationError> {
    let versions: Vec<QuiltGameVersion> = get_json(QUILT_GAME_VERSIONS_URL)?;
    Ok(versions
        .into_iter()
        .filter_map(|version| {
            let id = version.version.or(version.id)?;
            let trimmed = id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        })
        .collect())
}

pub(crate) fn fetch_quilt_loader_catalog() -> Result<LoaderVersionCatalog, InstallationError> {
    let matrix: serde_json::Value = get_json(QUILT_VERSION_MATRIX_URL)?;
    Ok(parse_loader_version_matrix(&matrix))
}

pub(crate) fn fetch_forge_versions() -> Result<HashSet<String>, InstallationError> {
    let metadata = get_text(FORGE_MAVEN_METADATA_URL)?;
    Ok(parse_minecraft_versions_from_maven_metadata(
        &metadata, true,
    ))
}

pub(crate) fn fetch_forge_loader_catalog() -> Result<LoaderVersionCatalog, InstallationError> {
    let metadata = get_text(FORGE_MAVEN_METADATA_URL)?;
    Ok(parse_forge_loader_catalog_from_metadata(&metadata))
}

pub(crate) fn fetch_neoforge_versions() -> Result<HashSet<String>, InstallationError> {
    let primary = get_text(NEOFORGE_MAVEN_METADATA_URL)?;
    let mut versions = parse_neoforge_versions_from_metadata(&primary);

    if let Ok(legacy) = get_text(NEOFORGE_LEGACY_FORGE_METADATA_URL) {
        versions.extend(parse_minecraft_versions_from_maven_metadata(&legacy, true));
    }

    Ok(versions)
}

pub(crate) fn fetch_neoforge_loader_catalog() -> Result<LoaderVersionCatalog, InstallationError> {
    let primary = get_text(NEOFORGE_MAVEN_METADATA_URL)?;
    let mut catalog = parse_neoforge_loader_catalog_from_metadata(&primary);

    if let Ok(legacy) = get_text(NEOFORGE_LEGACY_FORGE_METADATA_URL) {
        let legacy_neoforge = parse_neoforge_loader_catalog_from_metadata(&legacy);
        merge_loader_catalog(&mut catalog, legacy_neoforge);
        let legacy_forge_style = parse_forge_loader_catalog_from_metadata(&legacy);
        merge_loader_catalog(&mut catalog, legacy_forge_style);
    }

    Ok(catalog)
}

pub(crate) fn fetch_fabric_loader_catalog_with_fallback() -> LoaderVersionCatalog {
    match fetch_fabric_loader_catalog() {
        Ok(catalog) if !catalog.supported_game_versions.is_empty() => catalog,
        _ => LoaderVersionCatalog {
            supported_game_versions: fetch_fabric_versions().unwrap_or_default(),
            ..LoaderVersionCatalog::default()
        },
    }
}

pub(crate) fn fetch_quilt_loader_catalog_with_fallback() -> LoaderVersionCatalog {
    match fetch_quilt_loader_catalog() {
        Ok(catalog) if !catalog.supported_game_versions.is_empty() => catalog,
        _ => LoaderVersionCatalog {
            supported_game_versions: fetch_quilt_versions().unwrap_or_default(),
            ..LoaderVersionCatalog::default()
        },
    }
}

pub(crate) fn fetch_forge_loader_catalog_with_fallback() -> LoaderVersionCatalog {
    match fetch_forge_loader_catalog() {
        Ok(catalog) if !catalog.supported_game_versions.is_empty() => catalog,
        _ => LoaderVersionCatalog {
            supported_game_versions: fetch_forge_versions().unwrap_or_default(),
            ..LoaderVersionCatalog::default()
        },
    }
}

pub(crate) fn fetch_neoforge_loader_catalog_with_fallback() -> LoaderVersionCatalog {
    match fetch_neoforge_loader_catalog() {
        Ok(catalog) if !catalog.supported_game_versions.is_empty() => catalog,
        _ => LoaderVersionCatalog {
            supported_game_versions: fetch_neoforge_versions().unwrap_or_default(),
            ..LoaderVersionCatalog::default()
        },
    }
}

pub(crate) fn parse_minecraft_versions_from_maven_metadata(
    metadata_xml: &str,
    read_prefix_before_dash: bool,
) -> HashSet<String> {
    parse_xml_versions(metadata_xml)
        .into_iter()
        .filter_map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }

            let candidate = if read_prefix_before_dash {
                trimmed.split('-').next().unwrap_or(trimmed)
            } else {
                trimmed
            };

            if is_probable_minecraft_version(candidate) {
                Some(candidate.to_owned())
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn parse_loader_version_matrix(matrix: &serde_json::Value) -> LoaderVersionCatalog {
    let mut catalog = LoaderVersionCatalog::default();

    match matrix {
        serde_json::Value::Array(entries) => {
            collect_loader_versions_from_entries(entries, &mut catalog);
        }
        serde_json::Value::Object(object) => {
            // Support alternate wrappers some APIs use.
            for key in ["loader", "versions", "data"] {
                if let Some(entries) = object.get(key).and_then(serde_json::Value::as_array) {
                    collect_loader_versions_from_entries(entries, &mut catalog);
                }
            }
        }
        _ => {}
    }

    catalog.finalize()
}

pub(crate) fn collect_loader_versions_from_entries(
    entries: &[serde_json::Value],
    catalog: &mut LoaderVersionCatalog,
) {
    for entry in entries {
        let Some(entry) = entry.as_object() else {
            continue;
        };

        let Some(game_version) = extract_game_version_from_loader_entry(entry) else {
            continue;
        };
        let Some(loader_version) = extract_loader_version_from_loader_entry(entry) else {
            continue;
        };

        push_unique_loader_version(
            &mut catalog.versions_by_game_version,
            game_version.as_str(),
            loader_version,
        );
    }
}

pub(crate) fn parse_global_loader_versions(matrix: &serde_json::Value) -> Vec<String> {
    let mut versions = Vec::new();
    let mut seen = HashSet::new();
    let mut push_unique = |candidate: String| {
        if seen.insert(candidate.clone()) {
            versions.push(candidate);
        }
    };

    match matrix {
        serde_json::Value::Array(entries) => {
            collect_global_loader_versions_from_entries(entries, &mut push_unique);
        }
        serde_json::Value::Object(object) => {
            let mut found_wrapped_entries = false;
            for key in ["loader", "versions", "data"] {
                if let Some(entries) = object.get(key).and_then(serde_json::Value::as_array) {
                    found_wrapped_entries = true;
                    collect_global_loader_versions_from_entries(entries, &mut push_unique);
                }
            }
            if !found_wrapped_entries {
                if let Some(version) = extract_loader_version_from_loader_entry(object) {
                    push_unique(version);
                } else if let Some(version) = object
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                {
                    push_unique(version);
                }
            }
        }
        _ => {}
    }

    sort_loader_versions_desc(versions)
}

pub(crate) fn collect_global_loader_versions_from_entries<F>(
    entries: &[serde_json::Value],
    push_unique: &mut F,
) where
    F: FnMut(String),
{
    for entry in entries {
        let Some(object) = entry.as_object() else {
            continue;
        };
        if let Some(version) = extract_loader_version_from_loader_entry(object) {
            push_unique(version);
            continue;
        }
        if let Some(version) = object
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
        {
            push_unique(version);
        }
    }
}

pub(crate) fn url_encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        let is_unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if is_unreserved {
            out.push(byte as char);
        } else {
            use std::fmt::Write as _;

            out.push('%');
            let _ = write!(out, "{byte:02X}");
        }
    }
    out
}

pub(crate) fn fetch_loader_versions_for_game_uncached(
    loader_kind: LoaderKind,
    game_version: &str,
) -> Result<LoaderVersionFetchResult, InstallationError> {
    match loader_kind {
        LoaderKind::Fabric => {
            let url = format!(
                "{}/{}",
                FABRIC_VERSION_MATRIX_URL.trim_end_matches('/'),
                url_encode_component(game_version)
            );
            let payload: serde_json::Value = get_json(&url)?;
            let selected_versions = parse_global_loader_versions(&payload);
            let mut versions_by_game_version = BTreeMap::new();
            versions_by_game_version.insert(game_version.to_owned(), selected_versions.clone());
            Ok(LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version,
            })
        }
        LoaderKind::Quilt => {
            let url = format!(
                "{}/{}",
                QUILT_VERSION_MATRIX_URL.trim_end_matches('/'),
                url_encode_component(game_version)
            );
            let payload: serde_json::Value = get_json(&url)?;
            let selected_versions = parse_global_loader_versions(&payload);
            let mut versions_by_game_version = BTreeMap::new();
            versions_by_game_version.insert(game_version.to_owned(), selected_versions.clone());
            Ok(LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version,
            })
        }
        LoaderKind::Forge => {
            let metadata = get_text(FORGE_MAVEN_METADATA_URL)?;
            let catalog = parse_forge_loader_catalog_from_metadata(&metadata);
            let selected_versions = catalog
                .versions_by_game_version
                .get(game_version)
                .cloned()
                .unwrap_or_default();
            Ok(LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version: catalog.versions_by_game_version,
            })
        }
        LoaderKind::NeoForge => {
            let catalog = fetch_neoforge_loader_catalog()?;
            let selected_versions = catalog
                .versions_by_game_version
                .get(game_version)
                .cloned()
                .unwrap_or_default();
            Ok(LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version: catalog.versions_by_game_version,
            })
        }
        LoaderKind::Vanilla | LoaderKind::Custom => Ok(LoaderVersionFetchResult::default()),
    }
}

pub(crate) fn loader_versions_cache_file_path(loader_kind: LoaderKind) -> Option<PathBuf> {
    let file_name = match loader_kind {
        LoaderKind::Fabric => "fabric_loader_versions.json",
        LoaderKind::Forge => "forge_loader_versions.json",
        LoaderKind::NeoForge => "neoforge_loader_versions.json",
        LoaderKind::Quilt => "quilt_loader_versions.json",
        LoaderKind::Vanilla | LoaderKind::Custom => return None,
    };
    Some(
        cache_root_dir()
            .join(CACHE_LOADER_VERSIONS_DIR_NAME)
            .join(file_name),
    )
}

pub(crate) fn read_cached_loader_versions(
    loader_kind: LoaderKind,
) -> Result<CachedLoaderVersions, InstallationError> {
    let Some(path) = loader_versions_cache_file_path(loader_kind) else {
        return Ok(CachedLoaderVersions::default());
    };
    let raw = fs_read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub(crate) fn write_cached_loader_versions(
    loader_kind: LoaderKind,
    cached: &CachedLoaderVersions,
) -> Result<(), InstallationError> {
    let Some(path) = loader_versions_cache_file_path(loader_kind) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs_create_dir_all(parent)?;
    }
    let file = fs_file_create(path)?;
    serde_json::to_writer_pretty(file, cached)?;
    Ok(())
}

pub(crate) fn extract_game_version_from_loader_entry(
    entry: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    // Fabric/Quilt loader endpoints commonly encode Minecraft version in "intermediary.version".
    for key in [
        "game",
        "minecraft",
        "minecraft_version",
        "mcversion",
        "intermediary",
    ] {
        if let Some(version) = entry.get(key).and_then(extract_version_from_json_value)
            && is_probable_minecraft_version(version.as_str())
        {
            return Some(version);
        }
    }

    // Fallback: check all object fields for a probable MC version string.
    entry
        .values()
        .find_map(extract_version_from_json_value)
        .filter(|version| is_probable_minecraft_version(version.as_str()))
}

pub(crate) fn extract_loader_version_from_loader_entry(
    entry: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    for key in ["loader", "loader_version", "version"] {
        if let Some(version) = entry.get(key).and_then(extract_version_from_json_value) {
            return Some(version);
        }
    }
    None
}

pub(crate) fn parse_forge_loader_catalog_from_metadata(metadata_xml: &str) -> LoaderVersionCatalog {
    let mut catalog = LoaderVersionCatalog::default();
    for raw in parse_xml_versions(metadata_xml) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((game_version, loader_version)) = trimmed.split_once('-') else {
            continue;
        };
        let game_version = game_version.trim();
        let loader_version = loader_version.trim();
        if game_version.is_empty()
            || loader_version.is_empty()
            || !is_probable_minecraft_version(game_version)
        {
            continue;
        }
        push_unique_loader_version(
            &mut catalog.versions_by_game_version,
            game_version,
            loader_version.to_owned(),
        );
    }
    catalog.finalize()
}

pub(crate) fn parse_neoforge_loader_catalog_from_metadata(
    metadata_xml: &str,
) -> LoaderVersionCatalog {
    let mut catalog = LoaderVersionCatalog::default();
    for raw in parse_xml_versions(metadata_xml) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(game_version) = infer_neoforge_game_version(trimmed) else {
            continue;
        };
        push_unique_loader_version(
            &mut catalog.versions_by_game_version,
            game_version.as_str(),
            trimmed.to_owned(),
        );
    }
    catalog.finalize()
}

pub(crate) fn parse_neoforge_versions_from_metadata(metadata_xml: &str) -> HashSet<String> {
    parse_xml_versions(metadata_xml)
        .into_iter()
        .filter_map(|version| {
            let prefix = version.split('-').next().unwrap_or(version.as_str());
            let mut segments = prefix.split('.');
            let major = segments.next()?.parse::<u32>().ok()?;
            let minor = segments.next()?.parse::<u32>().ok()?;
            Some(format!("1.{major}.{minor}"))
        })
        .collect()
}

pub(crate) fn infer_neoforge_game_version(raw: &str) -> Option<String> {
    let prefix = raw.split('-').next().unwrap_or(raw).trim();
    if prefix.is_empty() {
        return None;
    }
    if is_probable_minecraft_version(prefix) {
        return Some(prefix.to_owned());
    }

    let mut segments = prefix.split('.');
    let major = segments.next()?.parse::<u32>().ok()?;
    let minor = segments.next()?.parse::<u32>().ok()?;
    Some(format!("1.{major}.{minor}"))
}

pub(crate) fn extract_version_from_json_value(value: &serde_json::Value) -> Option<String> {
    if let Some(raw) = value.as_str() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_owned());
    }

    let object = value.as_object()?;
    for key in ["version", "id", "name"] {
        let Some(raw) = object.get(key).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    None
}

pub(crate) fn push_unique_loader_version(
    versions_by_game_version: &mut BTreeMap<String, Vec<String>>,
    game_version: &str,
    loader_version: String,
) {
    let versions = versions_by_game_version
        .entry(game_version.to_owned())
        .or_default();
    if !versions.iter().any(|existing| existing == &loader_version) {
        versions.push(loader_version);
    }
}

pub(crate) fn sort_loader_version_map_desc(
    versions_by_game_version: &mut BTreeMap<String, Vec<String>>,
) {
    for versions in versions_by_game_version.values_mut() {
        sort_loader_versions_desc_in_place(versions);
    }
}

pub(crate) fn sort_loader_versions_desc(mut versions: Vec<String>) -> Vec<String> {
    sort_loader_versions_desc_in_place(&mut versions);
    versions
}

pub(crate) fn sort_loader_versions_desc_in_place(versions: &mut [String]) {
    versions.sort_by(|left, right| compare_version_like_desc(left.as_str(), right.as_str()));
}

pub(crate) fn compare_version_like_desc(left: &str, right: &str) -> std::cmp::Ordering {
    compare_version_like(left, right).reverse()
}

pub(crate) fn compare_version_like(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let left_tokens = version_like_tokens(left);
    let right_tokens = version_like_tokens(right);

    for (left_token, right_token) in left_tokens.iter().zip(right_tokens.iter()) {
        let ordering = match (left_token, right_token) {
            (VersionToken::Number(left), VersionToken::Number(right)) => left.cmp(right),
            (VersionToken::Text(left), VersionToken::Text(right)) => {
                left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
            }
            (VersionToken::Number(_), VersionToken::Text(_)) => Ordering::Greater,
            (VersionToken::Text(_), VersionToken::Number(_)) => Ordering::Less,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    left_tokens
        .len()
        .cmp(&right_tokens.len())
        .then_with(|| left.cmp(right))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VersionToken {
    Number(u64),
    Text(String),
}

pub(crate) fn version_like_tokens(raw: &str) -> Vec<VersionToken> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_is_digit = None;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            let is_digit = ch.is_ascii_digit();
            match current_is_digit {
                Some(previous) if previous != is_digit => {
                    push_version_token(&mut tokens, &mut current, previous);
                    current_is_digit = Some(is_digit);
                }
                None => current_is_digit = Some(is_digit),
                _ => {}
            }
            current.push(ch);
        } else if let Some(previous) = current_is_digit.take() {
            push_version_token(&mut tokens, &mut current, previous);
        }
    }

    if let Some(previous) = current_is_digit {
        push_version_token(&mut tokens, &mut current, previous);
    }

    tokens
}

pub(crate) fn push_version_token(
    tokens: &mut Vec<VersionToken>,
    current: &mut String,
    was_digit: bool,
) {
    if current.is_empty() {
        return;
    }
    let token = if was_digit {
        VersionToken::Number(current.parse::<u64>().unwrap_or(0))
    } else {
        VersionToken::Text(current.clone())
    };
    tokens.push(token);
    current.clear();
}

pub(crate) fn merge_loader_catalog(
    target: &mut LoaderVersionCatalog,
    source: LoaderVersionCatalog,
) {
    for game_version in source.supported_game_versions {
        target.supported_game_versions.insert(game_version);
    }
    for (game_version, versions) in source.versions_by_game_version {
        for version in versions {
            push_unique_loader_version(
                &mut target.versions_by_game_version,
                &game_version,
                version,
            );
        }
    }
}

pub(crate) fn parse_xml_versions(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    const START: &str = "<version>";
    const END: &str = "</version>";

    while let Some(start_offset) = xml[cursor..].find(START) {
        let start_index = cursor + start_offset + START.len();
        let Some(end_offset) = xml[start_index..].find(END) else {
            break;
        };
        let end_index = start_index + end_offset;
        out.push(xml[start_index..end_index].to_owned());
        cursor = end_index + END.len();
    }

    out
}

pub(crate) fn map_version_type(raw: &str) -> MinecraftVersionType {
    match raw {
        "release" => MinecraftVersionType::Release,
        "snapshot" => MinecraftVersionType::Snapshot,
        "old_beta" => MinecraftVersionType::OldBeta,
        "old_alpha" => MinecraftVersionType::OldAlpha,
        _ => MinecraftVersionType::Unknown,
    }
}

pub(crate) fn is_probable_minecraft_version(value: &str) -> bool {
    let mut segments = value.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    let Some(second) = segments.next() else {
        return false;
    };
    if !first.chars().all(|ch| ch.is_ascii_digit()) || !second.chars().all(|ch| ch.is_ascii_digit())
    {
        return false;
    }

    if !first.starts_with('1') {
        return false;
    }

    segments.all(|segment| !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_digit()))
}

pub(crate) fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, InstallationError> {
    let raw = get_text(url)?;
    Ok(serde_json::from_str(&raw)?)
}

pub(crate) fn get_json_with_user_agent<T: DeserializeOwned>(
    url: &str,
    user_agent: &str,
) -> Result<T, InstallationError> {
    let raw = call_get_with_retry(url, user_agent)?;
    Ok(serde_json::from_str(&raw)?)
}

pub(crate) fn http_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT_GLOBAL))
            .timeout_connect(Some(HTTP_TIMEOUT_CONNECT))
            .timeout_recv_response(Some(HTTP_TIMEOUT_RECV_RESPONSE))
            .timeout_recv_body(Some(HTTP_TIMEOUT_RECV_BODY))
            .build();
        ureq::Agent::new_with_config(config)
    })
}

pub(crate) fn get_text(url: &str) -> Result<String, InstallationError> {
    call_get_with_retry(url, DEFAULT_USER_AGENT)
}

pub(crate) fn call_get_response_with_retry(
    url: &str,
    user_agent: &str,
) -> Result<ureq::http::Response<ureq::Body>, InstallationError> {
    let mut last_err = None;
    for attempt in 1..=HTTP_RETRY_ATTEMPTS {
        tracing::trace!(
            target: "vertexlauncher/installation/http",
            url,
            user_agent,
            attempt,
            max_attempts = HTTP_RETRY_ATTEMPTS,
            "Sending HTTP GET request."
        );
        match http_agent()
            .get(url)
            .header("User-Agent", user_agent)
            .config()
            .http_status_as_error(false)
            .build()
            .call()
        {
            Ok(mut response) => {
                let status = response.status().as_u16();
                tracing::trace!(
                    target: "vertexlauncher/installation/http",
                    url,
                    attempt,
                    status,
                    "HTTP GET request completed."
                );
                if status < 400 {
                    return Ok(response);
                }
                let mut body = String::new();
                let _ = response.body_mut().as_reader().read_to_string(&mut body);
                let err = InstallationError::HttpStatus {
                    url: url.to_owned(),
                    status,
                    body,
                };
                let retryable = should_retry_http_status(status);
                if !retryable || attempt >= HTTP_RETRY_ATTEMPTS {
                    return Err(err);
                }
                last_err = Some(err);
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/installation/http",
                    url,
                    attempt,
                    error = %err,
                    "HTTP GET request failed before a valid response was received."
                );
                let mapped = InstallationError::Transport {
                    url: url.to_owned(),
                    message: err.to_string(),
                };
                if attempt >= HTTP_RETRY_ATTEMPTS {
                    return Err(mapped);
                }
                last_err = Some(mapped);
            }
        }

        let delay = retry_delay_for_attempt(attempt);
        tracing::warn!(
            target: "vertexlauncher/installation/downloads",
            "Request retry {}/{} for {} after {:?}: {}",
            attempt,
            HTTP_RETRY_ATTEMPTS,
            url,
            delay,
            last_err
                .as_ref()
                .map_or_else(|| "request failed".to_owned(), ToString::to_string)
        );
        thread::sleep(delay);
    }

    Err(last_err.unwrap_or_else(|| InstallationError::Transport {
        url: url.to_owned(),
        message: "request failed without detailed error".to_owned(),
    }))
}

pub(crate) fn call_get_with_retry(
    url: &str,
    user_agent: &str,
) -> Result<String, InstallationError> {
    let mut response = call_get_response_with_retry(url, user_agent)?;
    let mut raw = String::new();
    response
        .body_mut()
        .as_reader()
        .read_to_string(&mut raw)
        .map_err(InstallationError::Io)?;
    Ok(raw)
}

pub(crate) fn should_retry_http_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || (500..=599).contains(&status)
}

pub(crate) fn retry_delay_for_attempt(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    let multiplier = 1u64 << exponent;
    let millis = HTTP_RETRY_BASE_DELAY_MS
        .saturating_mul(multiplier)
        .min(5_000);
    Duration::from_millis(millis)
}

pub(crate) fn map_ureq_error(error: ureq::Error) -> InstallationError {
    match error {
        ureq::Error::StatusCode(status) => InstallationError::HttpStatus {
            url: "<unknown>".to_owned(),
            status,
            body: String::new(),
        },
        other => InstallationError::Transport {
            url: "<transport>".to_owned(),
            message: other.to_string(),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoaderKind {
    Vanilla,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
    Custom,
}

pub(crate) fn normalized_loader_label(loader_label: &str) -> LoaderKind {
    match loader_label.trim().to_ascii_lowercase().as_str() {
        "vanilla" => LoaderKind::Vanilla,
        "fabric" => LoaderKind::Fabric,
        "forge" => LoaderKind::Forge,
        "neoforge" => LoaderKind::NeoForge,
        "quilt" => LoaderKind::Quilt,
        _ => LoaderKind::Custom,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MojangVersionManifest {
    pub(crate) versions: Vec<MojangVersionEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MojangVersionEntry {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) version_type: String,
    pub(crate) release_time: String,
    pub(crate) url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FabricGameVersion {
    version: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QuiltGameVersion {
    version: Option<String>,
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MojangVersionMeta {
    pub(crate) downloads: Option<MojangDownloads>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MojangDownloads {
    pub(crate) client: Option<MojangDownloadArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MojangDownloadArtifact {
    pub(crate) url: String,
    pub(crate) size: Option<u64>,
}

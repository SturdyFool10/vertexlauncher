use super::*;

#[path = "launch_engine_downloads.rs"]
mod launch_engine_downloads;
#[path = "launch_engine_modloaders.rs"]
mod launch_engine_modloaders;

pub(crate) use launch_engine_downloads::*;
pub(crate) use launch_engine_modloaders::*;

pub(crate) struct LaunchContext {
    substitutions: HashMap<String, String>,
    features: HashMap<String, bool>,
}

pub(crate) fn normalize_java_executable(configured: Option<&str>) -> String {
    let mut java = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("java")
        .to_owned();
    let path_like = java.contains('/') || java.contains('\\');
    if path_like {
        let java_path = Path::new(java.as_str());
        if !java_path.exists() {
            java = "java".to_owned();
        } else if java_path.is_relative() {
            if let Ok(canonical) = fs_canonicalize(java_path) {
                java = display_user_path(canonical.as_path());
            } else if let Ok(cwd) = std::env::current_dir() {
                java = display_user_path(cwd.join(java_path).as_path());
            }
        } else {
            java = display_user_path(java_path);
        }
    }
    java
}

pub(crate) fn run_command_output(
    cmd: &mut Command,
    executable: &str,
) -> Result<Output, InstallationError> {
    cmd.output().map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            InstallationError::JavaExecutableNotFound {
                executable: executable.to_owned(),
            }
        } else {
            InstallationError::Io(err)
        }
    })
}

pub(crate) fn spawn_command_child(
    cmd: &mut Command,
    executable: &str,
) -> Result<Child, InstallationError> {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            InstallationError::JavaExecutableNotFound {
                executable: executable.to_owned(),
            }
        } else {
            InstallationError::Io(err)
        }
    })
}

pub(crate) fn prepare_launch_log_file(
    instance_root: &Path,
) -> Result<(std::fs::File, PathBuf), InstallationError> {
    let logs_dir = instance_root.join("logs");
    fs_create_dir_all(&logs_dir)?;
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let log_path = logs_dir.join(format!("launch_{timestamp_ms}.log"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    Ok((file, log_path))
}

pub(crate) fn resolve_launch_profile_path(
    instance_root: &Path,
    game_version: &str,
    modloader: &str,
    modloader_version: Option<&str>,
) -> Result<(String, PathBuf), InstallationError> {
    let versions_dir = instance_root.join("versions");
    let requested_loader = normalized_loader_label(modloader);
    let allow_vanilla_fallback =
        matches!(requested_loader, LoaderKind::Vanilla | LoaderKind::Custom);
    tracing::info!(
        target: "vertexlauncher/installation/launch_profile",
        requested_modloader = %modloader,
        requested_game_version = %game_version,
        requested_modloader_version = %modloader_version.unwrap_or(""),
        allow_vanilla_fallback,
        "Resolving launch profile."
    );
    let mut candidates = Vec::<(String, PathBuf)>::new();

    if allow_vanilla_fallback {
        let game_path = versions_dir
            .join(game_version)
            .join(format!("{game_version}.json"));
        if game_path.exists() {
            candidates.push((game_version.to_owned(), game_path));
        }
    }

    if matches!(requested_loader, LoaderKind::Fabric | LoaderKind::Quilt)
        && let Some(loader_version) = modloader_version.map(str::trim).filter(|v| !v.is_empty())
    {
        let prefix = if requested_loader == LoaderKind::Fabric {
            "fabric-loader"
        } else {
            "quilt-loader"
        };
        let id = format!("{prefix}-{loader_version}-{game_version}");
        let path = versions_dir.join(id.as_str()).join(format!("{id}.json"));
        if path.exists() {
            candidates.insert(0, (id, path));
        }
    }

    let loader_hint = match requested_loader {
        LoaderKind::Forge => Some("forge"),
        LoaderKind::NeoForge => Some("neoforge"),
        LoaderKind::Fabric => Some("fabric-loader"),
        LoaderKind::Quilt => Some("quilt-loader"),
        LoaderKind::Vanilla | LoaderKind::Custom => None,
    };
    if let Some(loader_hint) = loader_hint
        && versions_dir.exists()
    {
        for entry in fs_read_dir(&versions_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let lower = dir_name.to_ascii_lowercase();
            if !lower.contains(loader_hint) {
                continue;
            }
            let game_lower = game_version.to_ascii_lowercase();
            if !lower.contains(game_lower.as_str()) {
                let profile_path = entry.path().join(format!("{dir_name}.json"));
                if !profile_path.exists() {
                    continue;
                }
                let raw = fs_read_to_string(&profile_path)?;
                let parsed: serde_json::Value = serde_json::from_str(&raw)?;
                let inherits = parsed
                    .get("inheritsFrom")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !inherits.starts_with(game_lower.as_str()) && inherits != game_lower {
                    continue;
                }
            }
            if let Some(loader_version) = modloader_version.map(str::trim).filter(|v| !v.is_empty())
            {
                let lv = loader_version.to_ascii_lowercase();
                if !lower.contains(lv.as_str()) {
                    continue;
                }
            }
            let profile_path = entry.path().join(format!("{dir_name}.json"));
            if profile_path.exists() {
                candidates.insert(0, (dir_name, profile_path));
            }
        }
    }

    let resolved = candidates
        .into_iter()
        .find(|(_, path)| path.exists())
        .ok_or_else(|| InstallationError::LaunchProfileMissing {
            modloader: modloader.to_owned(),
            game_version: game_version.to_owned(),
        })?;
    tracing::info!(
        target: "vertexlauncher/installation/launch_profile",
        profile_id = %resolved.0,
        profile_path = %resolved.1.display(),
        "Resolved launch profile."
    );
    Ok(resolved)
}

pub(crate) fn load_profile_chain(
    instance_root: &Path,
    profile_path: &Path,
) -> Result<Vec<serde_json::Value>, InstallationError> {
    let mut chain = Vec::new();
    let mut cursor = profile_path.to_path_buf();
    let mut guard = 0usize;
    while guard < 16 {
        let raw = fs_read_to_string(cursor.as_path())?;
        let parsed: serde_json::Value = serde_json::from_str(&raw)?;
        let inherits = parsed
            .get("inheritsFrom")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        chain.push(parsed);
        let Some(parent) = inherits else {
            break;
        };
        cursor = instance_root
            .join("versions")
            .join(parent.as_str())
            .join(format!("{parent}.json"));
        guard = guard.saturating_add(1);
    }
    chain.reverse();
    Ok(chain)
}

pub(crate) fn resolve_main_class(chain: &[serde_json::Value]) -> Option<String> {
    for profile in chain.iter().rev() {
        if let Some(main_class) = profile.get("mainClass").and_then(serde_json::Value::as_str) {
            let trimmed = main_class.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

pub(crate) fn build_classpath_entries(
    instance_root: &Path,
    profile_id: &str,
    game_version: &str,
    main_class: &str,
    chain: &[serde_json::Value],
) -> Result<Vec<PathBuf>, InstallationError> {
    let mut classpath = Vec::<PathBuf>::new();
    let mut library_indices = HashMap::<String, usize>::new();

    for profile in chain {
        let Some(libraries) = profile
            .get("libraries")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for lib in libraries {
            if !library_rules_allow(lib) {
                continue;
            }
            let artifact_path = lib
                .get("downloads")
                .and_then(|v| v.get("artifact"))
                .and_then(|v| v.get("path"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| resolve_library_maven_download(lib).map(|(_, path)| path));
            let Some(artifact_path) = artifact_path else {
                continue;
            };
            let full = instance_root.join("libraries").join(artifact_path.as_str());
            if full.exists() {
                let dedupe_key = library_classpath_dedupe_key(lib, artifact_path.as_str());
                if let Some(existing_index) = library_indices.get(dedupe_key.as_str()).copied() {
                    classpath[existing_index] = full;
                } else {
                    library_indices.insert(dedupe_key, classpath.len());
                    classpath.push(full);
                }
            }
        }
    }

    let launch_jar = instance_root
        .join("versions")
        .join(profile_id)
        .join(format!("{profile_id}.jar"));
    if launch_jar.exists() {
        classpath.push(launch_jar);
    } else {
        // Forge 1.17+ (BootstrapLauncher) and NeoForge use the JPMS module system.
        // They load game classes via JarJar/FML — adding the vanilla jar to the
        // classpath would create a duplicate module (_1._20._1 vs minecraft) and
        // crash at startup. Skip the game jar entirely for these loaders.
        let uses_bootstrap_launcher = main_class.contains("BootstrapLauncher");
        if !uses_bootstrap_launcher {
            let fallback_jar = instance_root
                .join("versions")
                .join(game_version)
                .join(format!("{game_version}.jar"));
            if fallback_jar.exists() {
                classpath.push(fallback_jar);
            } else {
                return Err(InstallationError::LaunchFileMissing {
                    profile_id: profile_id.to_owned(),
                    path: launch_jar,
                });
            }
        }
    }
    Ok(classpath)
}

pub(crate) fn join_classpath(entries: &[PathBuf]) -> String {
    entries
        .iter()
        .map(|entry| display_user_path(entry.as_path()))
        .collect::<Vec<_>>()
        .join(classpath_separator())
}

pub(crate) struct LaunchClasspath {
    pub(crate) resolved: String,
    pub(crate) argfile: Option<PathBuf>,
}

pub(crate) fn prepare_launch_classpath(
    instance_root: &Path,
    profile_id: &str,
    entries: &[PathBuf],
) -> Result<LaunchClasspath, InstallationError> {
    let joined = join_classpath(entries);
    if joined.len() <= 7000 {
        return Ok(LaunchClasspath {
            resolved: joined,
            argfile: None,
        });
    }

    let argfile = write_classpath_argfile(instance_root, profile_id, joined.as_str())?;
    Ok(LaunchClasspath {
        resolved: joined,
        argfile: Some(argfile),
    })
}

pub(crate) fn write_classpath_argfile(
    instance_root: &Path,
    profile_id: &str,
    classpath: &str,
) -> Result<PathBuf, InstallationError> {
    let cache_dir = instance_root.join(".vertexlauncher");
    fs_create_dir_all(&cache_dir)?;
    let argfile_path = cache_dir.join(format!("classpath-{profile_id}.args"));
    let argfile_contents = format!("-cp\n{}\n", quote_java_argfile_value(classpath));
    fs_write(argfile_path.as_path(), argfile_contents.as_bytes())?;
    Ok(argfile_path)
}

pub(crate) fn quote_command_arg(arg: &str) -> String {
    if arg.is_empty()
        || arg
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\''))
    {
        format!("{arg:?}")
    } else {
        arg.to_owned()
    }
}

pub(crate) fn quote_java_argfile_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub(crate) fn strip_explicit_classpath_args(args: &mut Vec<String>) {
    let mut filtered = Vec::with_capacity(args.len());
    let mut index = 0usize;
    while index < args.len() {
        let current = args[index].as_str();
        if current == "-cp" || current == "-classpath" {
            index += 1;
            if index < args.len() {
                index += 1;
            }
            continue;
        }
        filtered.push(args[index].clone());
        index += 1;
    }
    *args = filtered;
}

pub(crate) fn has_explicit_classpath_args(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "-cp" | "-classpath"))
}

pub(crate) fn library_classpath_dedupe_key(lib: &serde_json::Value, artifact_path: &str) -> String {
    if let Some(name) = lib.get("name").and_then(serde_json::Value::as_str) {
        let mut parts = name.split(':');
        if let (Some(group), Some(artifact)) = (parts.next(), parts.next()) {
            let group = group.trim();
            let artifact = artifact.trim();
            if !group.is_empty() && !artifact.is_empty() {
                let classifier = parts
                    .nth(1)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("");
                return format!("{group}:{artifact}:{classifier}");
            }
        }
    }
    artifact_path.to_owned()
}

pub(crate) fn prepare_natives_dir(
    instance_root: &Path,
    profile_id: &str,
    chain: &[serde_json::Value],
) -> Result<PathBuf, InstallationError> {
    let natives_root = instance_root.join("natives").join(profile_id);
    if natives_root.exists() {
        fs_remove_dir_all(&natives_root)?;
    }
    fs_create_dir_all(&natives_root)?;

    for profile in chain {
        let Some(libraries) = profile
            .get("libraries")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for lib in libraries {
            if !library_rules_allow(lib) {
                continue;
            }
            let Some(natives) = lib.get("natives").and_then(serde_json::Value::as_object) else {
                continue;
            };
            let os_key = current_os_natives_key();
            let Some(classifier_template) = natives.get(os_key).and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let classifier = classifier_template.replace("${arch}", current_arch_natives_value());
            let Some(path) = lib
                .get("downloads")
                .and_then(|v| v.get("classifiers"))
                .and_then(|v| v.get(classifier.as_str()))
                .and_then(|v| v.get("path"))
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let archive = instance_root.join("libraries").join(path);
            if !archive.exists() {
                continue;
            }
            let excludes = lib
                .get("extract")
                .and_then(|v| v.get("exclude"))
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            extract_natives_archive(archive.as_path(), natives_root.as_path(), &excludes)?;
        }
    }
    Ok(natives_root)
}

pub(crate) fn extract_natives_archive(
    archive_path: &Path,
    destination: &Path,
    excludes: &[String],
) -> Result<(), InstallationError> {
    let file = fs_file_open(archive_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err.to_string()))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        if name.starts_with("META-INF/") || excludes.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        let out = destination.join(name.as_str());
        if let Some(parent) = out.parent() {
            fs_create_dir_all(parent)?;
        }
        let mut writer = fs_file_create(out)?;
        std::io::copy(&mut entry, &mut writer)?;
    }
    Ok(())
}

pub(crate) fn build_launch_context(
    instance_root: &Path,
    _game_version: &str,
    profile_id: &str,
    assets_index_name: &str,
    classpath: &str,
    natives_dir: &Path,
    player_name: Option<&str>,
    player_uuid: Option<&str>,
    auth_access_token: Option<&str>,
    auth_xuid: Option<&str>,
    auth_user_type: Option<&str>,
    quick_play_singleplayer: Option<&str>,
    quick_play_multiplayer: Option<&str>,
) -> LaunchContext {
    let username = player_name
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("Player");
    let uuid = player_uuid
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("00000000000000000000000000000000");
    let access_token = auth_access_token
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("0");
    let xuid = auth_xuid
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("0");
    let user_type = auth_user_type
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("legacy");
    let mut substitutions = HashMap::new();
    substitutions.insert("auth_player_name".to_owned(), username.to_owned());
    substitutions.insert("version_name".to_owned(), profile_id.to_owned());
    substitutions.insert(
        "game_directory".to_owned(),
        display_user_path(instance_root),
    );
    substitutions.insert(
        "assets_root".to_owned(),
        display_user_path(instance_root.join("assets").as_path()),
    );
    substitutions.insert("assets_index_name".to_owned(), assets_index_name.to_owned());
    // Legacy token used by pre-1.13 minecraftArguments as --assetsDir value.
    substitutions.insert(
        "game_assets".to_owned(),
        display_user_path(
            instance_root
                .join("assets")
                .join("virtual")
                .join(assets_index_name)
                .as_path(),
        ),
    );
    substitutions.insert("auth_uuid".to_owned(), uuid.to_owned());
    substitutions.insert("auth_access_token".to_owned(), access_token.to_owned());
    substitutions.insert("clientid".to_owned(), "0".to_owned());
    substitutions.insert("auth_xuid".to_owned(), xuid.to_owned());
    substitutions.insert("user_type".to_owned(), user_type.to_owned());
    substitutions.insert("version_type".to_owned(), "release".to_owned());
    substitutions.insert("user_properties".to_owned(), "{}".to_owned());
    substitutions.insert("classpath".to_owned(), classpath.to_owned());
    substitutions.insert(
        "classpath_separator".to_owned(),
        classpath_separator().to_owned(),
    );
    substitutions.insert(
        "library_directory".to_owned(),
        display_user_path(instance_root.join("libraries").as_path()),
    );
    substitutions.insert(
        "natives_directory".to_owned(),
        display_user_path(natives_dir),
    );
    substitutions.insert("launcher_name".to_owned(), "vertexlauncher".to_owned());
    substitutions.insert("launcher_version".to_owned(), "0.1".to_owned());
    substitutions.insert(
        "quickPlayPath".to_owned(),
        display_user_path(instance_root.join("quickPlay").as_path()),
    );
    if let Some(world) = quick_play_singleplayer
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        substitutions.insert("quickPlaySingleplayer".to_owned(), world.to_owned());
    }
    if let Some(server) = quick_play_multiplayer
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        substitutions.insert("quickPlayMultiplayer".to_owned(), server.to_owned());
    }
    let mut features = HashMap::new();
    features.insert("is_demo_user".to_owned(), false);
    features.insert("has_custom_resolution".to_owned(), false);
    let has_quick_play_singleplayer = quick_play_singleplayer
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let has_quick_play_multiplayer = quick_play_multiplayer
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    features.insert(
        "has_quick_plays_support".to_owned(),
        has_quick_play_singleplayer || has_quick_play_multiplayer,
    );
    features.insert(
        "is_quick_play_singleplayer".to_owned(),
        has_quick_play_singleplayer,
    );
    features.insert(
        "is_quick_play_multiplayer".to_owned(),
        has_quick_play_multiplayer,
    );
    features.insert("is_quick_play_realms".to_owned(), false);
    LaunchContext {
        substitutions,
        features,
    }
}

pub(crate) fn resolve_assets_index_name<'a>(
    chain: &'a [serde_json::Value],
    fallback: &'a str,
) -> &'a str {
    for profile in chain.iter().rev() {
        if let Some(id) = profile
            .get("assetIndex")
            .and_then(|value| value.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return id;
        }
    }
    fallback
}

pub(crate) fn collect_jvm_arguments(
    chain: &[serde_json::Value],
    context: &LaunchContext,
) -> Vec<String> {
    let mut args = Vec::new();
    for profile in chain {
        if let Some(values) = profile
            .get("arguments")
            .and_then(|v| v.get("jvm"))
            .and_then(serde_json::Value::as_array)
        {
            args.extend(collect_argument_array(values, context));
        }
    }
    if args.is_empty() {
        args.push("-Djava.library.path=${natives_directory}".to_owned());
        args.push("-cp".to_owned());
        args.push("${classpath}".to_owned());
    }
    args.into_iter()
        .map(|entry| substitute_tokens(entry.as_str(), context))
        .collect()
}

pub(crate) fn collect_game_arguments(
    chain: &[serde_json::Value],
    context: &LaunchContext,
) -> Vec<String> {
    let mut args = Vec::new();
    for profile in chain {
        if let Some(values) = profile
            .get("arguments")
            .and_then(|v| v.get("game"))
            .and_then(serde_json::Value::as_array)
        {
            args.extend(collect_argument_array(values, context));
        }
    }
    if args.is_empty() {
        for profile in chain.iter().rev() {
            if let Some(raw) = profile
                .get("minecraftArguments")
                .and_then(serde_json::Value::as_str)
            {
                args.extend(raw.split_whitespace().map(str::to_owned));
                break;
            }
        }
    }
    let resolved: Vec<String> = args
        .into_iter()
        .map(|entry| substitute_tokens(entry.as_str(), context))
        .collect();
    normalize_quick_play_arguments(resolved, context)
}

pub(crate) fn normalize_quick_play_arguments(
    args: Vec<String>,
    context: &LaunchContext,
) -> Vec<String> {
    let quick_play_path = context
        .substitutions
        .get("quickPlayPath")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let quick_play_singleplayer = context
        .substitutions
        .get("quickPlaySingleplayer")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let quick_play_multiplayer = context
        .substitutions
        .get("quickPlayMultiplayer")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let requested_quick_play_mode = quick_play_singleplayer
        .map(|world| ("--quickPlaySingleplayer", world))
        .or_else(|| quick_play_multiplayer.map(|server| ("--quickPlayMultiplayer", server)));

    let mut out = Vec::new();
    let mut cursor = 0usize;
    let mut has_quick_play_path = false;
    let mut quick_play_mode_selected = false;

    while cursor < args.len() {
        let current = args[cursor].as_str();
        let is_quick_play_flag = matches!(
            current,
            "--quickPlayPath"
                | "--quickPlaySingleplayer"
                | "--quickPlayMultiplayer"
                | "--quickPlayRealms"
        );
        if !is_quick_play_flag {
            out.push(args[cursor].clone());
            cursor += 1;
            continue;
        }

        let value = args.get(cursor + 1).map(String::as_str).unwrap_or_default();
        let unresolved_placeholder =
            value.starts_with("${quickPlay") && value.ends_with('}') && value.len() > 2;
        if value.trim().is_empty() || unresolved_placeholder {
            cursor = cursor.saturating_add(2);
            continue;
        }

        if current == "--quickPlayPath" {
            if has_quick_play_path {
                cursor = cursor.saturating_add(2);
                continue;
            }
            has_quick_play_path = true;
            out.push(args[cursor].clone());
            out.push(args[cursor + 1].clone());
            cursor += 2;
            continue;
        }

        if quick_play_mode_selected {
            cursor = cursor.saturating_add(2);
            continue;
        }

        out.push(args[cursor].clone());
        out.push(args[cursor + 1].clone());
        quick_play_mode_selected = true;
        cursor += 2;
    }

    if let Some((flag, value)) = requested_quick_play_mode
        && !quick_play_mode_selected
    {
        if !has_quick_play_path && let Some(path) = quick_play_path {
            out.push("--quickPlayPath".to_owned());
            out.push(path.to_owned());
        }
        out.push(flag.to_owned());
        out.push(value.to_owned());
    }

    out
}

pub(crate) fn collect_argument_array(
    values: &[serde_json::Value],
    context: &LaunchContext,
) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if let Some(raw) = value.as_str() {
            out.push(substitute_tokens(raw, context));
            continue;
        }
        let Some(object) = value.as_object() else {
            continue;
        };
        if !rules_allow_for_launch(object.get("rules"), context) {
            continue;
        }
        let Some(arg_value) = object.get("value") else {
            continue;
        };
        if let Some(single) = arg_value.as_str() {
            out.push(substitute_tokens(single, context));
        } else if let Some(array) = arg_value.as_array() {
            for entry in array {
                if let Some(single) = entry.as_str() {
                    out.push(substitute_tokens(single, context));
                }
            }
        }
    }
    out
}

pub(crate) fn library_rules_allow(library: &serde_json::Value) -> bool {
    rules_allow_os_only(library.get("rules"))
}

pub(crate) fn rules_allow_for_launch(
    rules_value: Option<&serde_json::Value>,
    context: &LaunchContext,
) -> bool {
    let Some(rules) = rules_value.and_then(serde_json::Value::as_array) else {
        return true;
    };
    let mut allowed = false;
    for rule in rules {
        let Some(object) = rule.as_object() else {
            continue;
        };
        let action = object
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("allow");
        let applies = rule_applies_to_current_os(object.get("os"))
            && rule_features_match(object.get("features"), context);
        if applies {
            allowed = action == "allow";
        }
    }
    allowed
}

pub(crate) fn rules_allow_os_only(rules_value: Option<&serde_json::Value>) -> bool {
    let Some(rules) = rules_value.and_then(serde_json::Value::as_array) else {
        return true;
    };
    let mut allowed = false;
    for rule in rules {
        let Some(object) = rule.as_object() else {
            continue;
        };
        let action = object
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("allow");
        let applies = rule_applies_to_current_os(object.get("os"));
        if applies {
            allowed = action == "allow";
        }
    }
    allowed
}

pub(crate) fn rule_features_match(
    features_value: Option<&serde_json::Value>,
    context: &LaunchContext,
) -> bool {
    let Some(features) = features_value.and_then(serde_json::Value::as_object) else {
        return true;
    };
    for (feature, expected) in features {
        let Some(expected) = expected.as_bool() else {
            continue;
        };
        let actual = context.features.get(feature).copied().unwrap_or(false);
        if actual != expected {
            return false;
        }
    }
    true
}

pub(crate) fn rule_applies_to_current_os(os_value: Option<&serde_json::Value>) -> bool {
    let Some(os_object) = os_value.and_then(serde_json::Value::as_object) else {
        return true;
    };
    if let Some(name) = os_object.get("name").and_then(serde_json::Value::as_str)
        && name != current_os_natives_key()
    {
        return false;
    }
    if let Some(arch) = os_object.get("arch").and_then(serde_json::Value::as_str)
        && !arch_matches_current_target(arch)
    {
        return false;
    }
    true
}

pub(crate) fn substitute_tokens(raw: &str, context: &LaunchContext) -> String {
    let mut result = raw.to_owned();
    for (key, value) in &context.substitutions {
        let token = format!("${{{key}}}");
        result = result.replace(token.as_str(), value.as_str());
    }
    result
}

pub(crate) fn parse_user_args(raw: Option<&str>) -> Vec<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

pub(crate) fn classpath_separator() -> &'static str {
    if cfg!(target_os = "windows") {
        ";"
    } else {
        ":"
    }
}

pub(crate) fn should_use_environment_classpath() -> bool {
    false
}

pub(crate) fn current_os_natives_key() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

pub(crate) fn current_arch_natives_value() -> &'static str {
    if cfg!(target_pointer_width = "64") {
        "64"
    } else {
        "32"
    }
}

pub(crate) fn arch_matches_current_target(expected: &str) -> bool {
    let normalized = expected.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "x86" | "i386" | "i686" | "32" => cfg!(target_arch = "x86"),
        "x86_64" | "amd64" | "64" => cfg!(target_arch = "x86_64"),
        "arm64" | "aarch64" => cfg!(target_arch = "aarch64"),
        "arm" => cfg!(target_arch = "arm"),
        other => std::env::consts::ARCH.eq_ignore_ascii_case(other),
    }
}

pub(crate) fn report_install_progress(
    progress: Option<&InstallProgressSink>,
    event: InstallProgress,
) {
    if let Some(callback) = progress {
        callback(event);
    }
}

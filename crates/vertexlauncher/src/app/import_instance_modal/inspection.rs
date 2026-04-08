use super::*;

pub(super) fn inspect_package(path: &Path) -> Result<ImportPreview, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "vtmpack" => inspect_vtmpack(path),
        "mrpack" => inspect_mrpack(path),
        "zip" => inspect_curseforge_pack(path),
        _ => Err(format!(
            "Unsupported import file {}. Expected .vtmpack, .mrpack, or a CurseForge modpack .zip.",
            path.display()
        )),
    }
}

pub(super) fn inspect_vtmpack(path: &Path) -> Result<ImportPreview, String> {
    let manifest = read_vtmpack_manifest(path)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Manifest(ImportPackageKind::VertexPack),
        detected_name: manifest.instance.name.clone(),
        game_version: manifest.instance.game_version.clone(),
        modloader: manifest.instance.modloader.clone(),
        modloader_version: manifest.instance.modloader_version.clone(),
        summary: format!(
            "{} for Minecraft {} ({}) with {} downloadable items, {} bundled mods, {} config files.",
            manifest.instance.name,
            manifest.instance.game_version,
            format_loader_label(
                manifest.instance.modloader.as_str(),
                manifest.instance.modloader_version.as_str()
            ),
            manifest.downloadable_content.len(),
            manifest.bundled_mods.len(),
            manifest.configs.len()
        ),
    })
}

pub(super) fn inspect_mrpack(path: &Path) -> Result<ImportPreview, String> {
    let manifest = read_mrpack_manifest(path)?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Manifest(ImportPackageKind::ModrinthPack),
        detected_name: non_empty(manifest.name.as_str())
            .unwrap_or_else(|| "Imported Modrinth Pack".to_owned()),
        game_version: dependency_info.game_version.clone(),
        modloader: dependency_info.modloader.clone(),
        modloader_version: dependency_info.modloader_version.clone(),
        summary: format!(
            "{} {} for Minecraft {} ({}) with {} packaged files.",
            non_empty(manifest.name.as_str()).unwrap_or_else(|| "Modrinth pack".to_owned()),
            non_empty(manifest.version_id.as_str()).unwrap_or_default(),
            dependency_info.game_version,
            format_loader_label(
                dependency_info.modloader.as_str(),
                dependency_info.modloader_version.as_str()
            ),
            manifest.files.len()
        )
        .trim()
        .to_owned(),
    })
}

pub(super) fn inspect_curseforge_pack(path: &Path) -> Result<ImportPreview, String> {
    let manifest = read_curseforge_pack_manifest(path)?;
    let dependency_info = resolve_curseforge_pack_dependencies(&manifest.minecraft)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Manifest(ImportPackageKind::CurseForgePack),
        detected_name: non_empty(manifest.name.as_str())
            .unwrap_or_else(|| "Imported CurseForge Pack".to_owned()),
        game_version: dependency_info.game_version.clone(),
        modloader: dependency_info.modloader.clone(),
        modloader_version: dependency_info.modloader_version.clone(),
        summary: format!(
            "{} {} for Minecraft {} ({}) with {} packaged files.",
            non_empty(manifest.name.as_str()).unwrap_or_else(|| "CurseForge pack".to_owned()),
            non_empty(manifest.version.as_str()).unwrap_or_default(),
            dependency_info.game_version,
            format_loader_label(
                dependency_info.modloader.as_str(),
                dependency_info.modloader_version.as_str()
            ),
            manifest.files.len()
        )
        .trim()
        .to_owned(),
    })
}

#[derive(Clone, Debug)]
pub(super) struct LauncherInspection {
    pub(super) launcher: LauncherKind,
    pub(super) name: String,
    pub(super) description: Option<String>,
    pub(super) game_version: String,
    pub(super) modloader: String,
    pub(super) modloader_version: String,
    pub(super) summary: String,
    pub(super) source_root: PathBuf,
    pub(super) managed_manifest: ContentInstallManifest,
}

pub(super) fn inspect_launcher_instance(
    path: &Path,
    launcher_hint: Option<LauncherKind>,
) -> Result<ImportPreview, String> {
    let inspection = inspect_launcher_details(path, launcher_hint)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Launcher(inspection.launcher),
        detected_name: inspection.name,
        game_version: inspection.game_version,
        modloader: inspection.modloader,
        modloader_version: inspection.modloader_version,
        summary: inspection.summary,
    })
}

pub(super) fn inspect_launcher_details(
    path: &Path,
    launcher_hint: Option<LauncherKind>,
) -> Result<LauncherInspection, String> {
    if !path.exists() {
        return Err(format!(
            "Instance folder {} does not exist.",
            path.display()
        ));
    }
    if !path.is_dir() {
        return Err(format!(
            "Import source {} is not a directory.",
            path.display()
        ));
    }

    let launcher = launcher_hint.unwrap_or_else(|| detect_launcher_kind(path));
    match launcher {
        LauncherKind::Modrinth => inspect_modrinth_launcher_instance(path),
        LauncherKind::CurseForge => inspect_curseforge_launcher_instance(path),
        LauncherKind::Prism => inspect_prism_launcher_instance(path),
        LauncherKind::ATLauncher => inspect_atlauncher_instance(path),
        LauncherKind::Unknown => inspect_generic_launcher_instance(path),
    }
}

pub(super) fn import_launcher_instance(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
) -> Result<InstanceRecord, String> {
    let ImportSource::LauncherDirectory { path, launcher } = &request.source else {
        return Err("Launcher import requires an instance directory source.".to_owned());
    };

    let inspection = inspect_launcher_details(path.as_path(), *launcher)?;
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: inspection.description.clone(),
            thumbnail_path: None,
            modloader: default_if_blank(inspection.modloader.as_str(), "Vanilla".to_owned()),
            game_version: default_if_blank(inspection.game_version.as_str(), "latest".to_owned()),
            modloader_version: inspection.modloader_version.clone(),
        },
    )
    .map_err(|err| format!("failed to create imported profile: {err}"))?;
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = copy_launcher_instance_content(
        inspection.source_root.as_path(),
        instance_root.as_path(),
        &inspection.managed_manifest,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    Ok(instance)
}

pub(super) fn detect_launcher_kind(path: &Path) -> LauncherKind {
    if path.join(CONTENT_MANIFEST_FILE_NAME).is_file() {
        LauncherKind::Unknown
    } else if path.join("profile.json").is_file() || looks_like_modrinth_profile_path(path) {
        LauncherKind::Modrinth
    } else if path.join("minecraftinstance.json").is_file() {
        LauncherKind::CurseForge
    } else if path.join("instance.cfg").is_file() || path.join("mmc-pack.json").is_file() {
        LauncherKind::Prism
    } else if path.join("instance.json").is_file() {
        LauncherKind::ATLauncher
    } else {
        LauncherKind::Unknown
    }
}

pub(super) fn inspect_modrinth_launcher_instance(
    path: &Path,
) -> Result<LauncherInspection, String> {
    if !path.join("profile.json").is_file() {
        let mut inspection =
            inspect_generic_launcher_instance_with_launcher(path, LauncherKind::Modrinth)?;
        let inferred = infer_modrinth_profile_metadata(path);
        if let Some(game_version) = inferred.game_version {
            inspection.game_version = game_version;
        }
        if let Some(modloader) = inferred.modloader {
            inspection.modloader = modloader;
        }
        if let Some(modloader_version) = inferred.modloader_version {
            inspection.modloader_version = modloader_version;
        }
        inspection.description = Some(
            "Imported from a Modrinth instance folder without profile.json metadata.".to_owned(),
        );
        inspection.summary = format!(
            "Detected {} by location. No profile.json was present, so Minecraft and loader metadata were inferred from profile contents where possible; files will still be copied from the instance root.",
            inspection.launcher.label()
        );
        return Ok(inspection);
    }

    let profile = read_json_file(path.join("profile.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        json_string_at_path(&profile, &["metadata", "name"]),
        json_string_at_path(&profile, &["name"]),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported Modrinth Instance".to_owned());
    let game_version = first_non_empty([
        json_string_at_path(&profile, &["metadata", "game_version"]),
        json_string_at_path(&profile, &["game_version"]),
        json_string_at_path(&profile, &["metadata", "minecraft_version"]),
        json_string_at_path(&profile, &["minecraft_version"]),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let (modloader, modloader_version) = infer_loader_pair(
        first_non_empty([
            json_string_at_path(&profile, &["metadata", "loader"]),
            json_string_at_path(&profile, &["loader"]),
            json_string_at_path(&profile, &["loader_type"]),
        ]),
        first_non_empty([
            json_string_at_path(&profile, &["metadata", "loader_version"]),
            json_string_at_path(&profile, &["loader_version"]),
            json_string_at_path(&profile, &["loaderVersion"]),
        ]),
    );
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ContentInstallManifest::default());
    if managed_manifest.projects.is_empty() {
        managed_manifest = extract_managed_manifest_from_json(
            &profile,
            source_root.as_path(),
            ManagedContentSourceHint::Modrinth,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::Modrinth,
        name,
        Some("Imported from an existing Modrinth launcher instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

pub(super) fn looks_like_modrinth_profile_path(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(parent_name) = parent.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if parent_name != "profiles" {
        return false;
    }
    path.ancestors().any(|ancestor| {
        ancestor
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name == "ModrinthApp")
    })
}

#[derive(Default)]
pub(super) struct ModrinthProfileMetadata {
    pub(super) game_version: Option<String>,
    pub(super) modloader: Option<String>,
    pub(super) modloader_version: Option<String>,
}

pub(super) fn infer_modrinth_profile_metadata(path: &Path) -> ModrinthProfileMetadata {
    let mut metadata = ModrinthProfileMetadata::default();
    metadata.game_version = infer_modrinth_game_version_from_telemetry(path);

    let (modloader, modloader_version) = infer_modrinth_loader_from_profile(path);
    metadata.modloader = modloader;
    metadata.modloader_version = modloader_version;

    if let Some(app_root) = modrinth_app_root(path) {
        refine_modrinth_metadata_from_meta_cache(app_root.as_path(), &mut metadata);
    }

    if metadata.game_version.is_none() {
        metadata.game_version = infer_modrinth_game_version_from_filenames(path);
    }

    metadata
}

pub(super) fn infer_modrinth_game_version_from_telemetry(path: &Path) -> Option<String> {
    let telemetry_dir = path.join("logs").join("telemetry");
    let mut files = fs::read_dir(telemetry_dir)
        .ok()?
        .flatten()
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| entry.file_name());
    files.reverse();

    for entry in files {
        let raw = fs::read_to_string(entry.path()).ok()?;
        for line in raw.lines().rev() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && let Some(game_version) = value.get("game_version").and_then(Value::as_str)
            {
                if let Some(normalized) = normalize_minecraft_game_version(game_version) {
                    return Some(normalized);
                }
            }
        }
    }

    None
}

pub(super) fn infer_modrinth_loader_from_profile(path: &Path) -> (Option<String>, Option<String>) {
    if let Some((loader, version)) = infer_modrinth_loader_from_dependencies_file(
        path.join("config/fabric_loader_dependencies.json")
            .as_path(),
    ) {
        return (Some(loader), Some(version));
    }

    if let Some((loader, version)) = infer_modrinth_loader_from_mod_filenames(path) {
        return (Some(loader), version);
    }

    (None, None)
}

pub(super) fn infer_modrinth_loader_from_dependencies_file(
    path: &Path,
) -> Option<(String, String)> {
    let value = read_json_file_optional(path).ok()??;
    let fabric_requirement = value
        .get("overrides")
        .and_then(|value| value.get("fabricloader"))
        .and_then(|value| value.get("+depends"))
        .and_then(|value| value.get("fabricloader"))
        .and_then(Value::as_str)
        .and_then(clean_version_requirement)?;
    Some(("Fabric".to_owned(), fabric_requirement))
}

pub(super) fn clean_version_requirement(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut out = String::new();
    let mut started = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            started = true;
            out.push(ch);
            continue;
        }
        if started && ch == '.' {
            out.push(ch);
            continue;
        }
        if started {
            break;
        }
    }

    if out.is_empty() { None } else { Some(out) }
}

pub(super) fn infer_modrinth_loader_from_mod_filenames(
    path: &Path,
) -> Option<(String, Option<String>)> {
    let mods_dir = path.join("mods");
    let entries = fs::read_dir(mods_dir).ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if file_name.contains("fabric") {
            return Some(("Fabric".to_owned(), None));
        }
        if file_name.contains("quilt") {
            return Some(("Quilt".to_owned(), None));
        }
        if file_name.contains("neoforge") {
            return Some(("NeoForge".to_owned(), None));
        }
        if file_name.contains("forge") {
            return Some(("Forge".to_owned(), None));
        }
    }
    None
}

pub(super) fn infer_modrinth_game_version_from_filenames(path: &Path) -> Option<String> {
    let mods_dir = path.join("mods");
    let entries = fs::read_dir(mods_dir).ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if let Some(version) = find_minecraft_version_in_text(file_name.as_ref()) {
            return Some(version);
        }
    }
    None
}

pub(super) fn find_minecraft_version_in_text(text: &str) -> Option<String> {
    let chars = text.chars().collect::<Vec<_>>();
    for start in 0..chars.len() {
        if !chars[start].is_ascii_digit() {
            continue;
        }
        let mut end = start;
        let mut dot_count = 0usize;
        while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == '.') {
            if chars[end] == '.' {
                dot_count += 1;
            }
            end += 1;
        }
        if dot_count >= 2 {
            let candidate = chars[start..end].iter().collect::<String>();
            if candidate.split('.').all(|segment| !segment.is_empty())
                && let Some(normalized) = normalize_minecraft_game_version(&candidate)
            {
                return Some(normalized);
            }
        }
    }
    None
}

pub(super) fn modrinth_app_root(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        ancestor
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name == "ModrinthApp")
            .then(|| ancestor.to_path_buf())
    })
}

pub(super) fn refine_modrinth_metadata_from_meta_cache(
    app_root: &Path,
    metadata: &mut ModrinthProfileMetadata,
) {
    let versions_dir = app_root.join("meta").join("versions");
    let Ok(entries) = fs::read_dir(versions_dir) else {
        return;
    };

    let game_version = metadata.game_version.clone();
    for entry in entries.flatten() {
        let version_name = entry.file_name().to_string_lossy().to_string();
        let Some(version_json) =
            read_meta_version_file(entry.path().as_path(), version_name.as_str())
        else {
            continue;
        };

        if let Some(expected_game_version) = game_version.as_deref()
            && !version_name.starts_with(expected_game_version)
        {
            continue;
        }

        if metadata.modloader.is_none() || metadata.modloader_version.is_none() {
            if let Some((loader, loader_version)) = infer_loader_from_meta_version(&version_json) {
                metadata.modloader.get_or_insert(loader);
                metadata.modloader_version.get_or_insert(loader_version);
            }
        }

        if metadata.game_version.is_none() {
            if let Some(version) = normalize_minecraft_game_version(&version_name)
                .or_else(|| {
                    version_json
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(normalize_minecraft_game_version)
                })
                .or_else(|| {
                    version_json
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(find_minecraft_version_in_text)
                })
            {
                metadata.game_version = Some(version);
            }
        }

        if metadata.game_version.is_some()
            && metadata.modloader.is_some()
            && metadata.modloader_version.is_some()
        {
            break;
        }
    }
}

pub(super) fn read_meta_version_file(dir: &Path, dir_name: &str) -> Option<Value> {
    let path = dir.join(format!("{dir_name}.json"));
    read_json_file_optional(path.as_path()).ok().flatten()
}

pub(super) fn infer_loader_from_meta_version(value: &Value) -> Option<(String, String)> {
    let libraries = value.get("libraries")?.as_array()?;
    for library in libraries {
        let name = library
            .get("name")
            .and_then(Value::as_str)?
            .to_ascii_lowercase();
        if let Some(version) = name.strip_prefix("net.fabricmc:fabric-loader:") {
            return Some(("Fabric".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("org.quiltmc:quilt-loader:") {
            return Some(("Quilt".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("net.neoforged:neoforge:") {
            return Some(("NeoForge".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("net.minecraftforge:forge:") {
            return Some(("Forge".to_owned(), version.to_owned()));
        }
    }
    None
}

pub(super) fn inspect_curseforge_launcher_instance(
    path: &Path,
) -> Result<LauncherInspection, String> {
    let manifest = read_json_file(path.join("minecraftinstance.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        json_string_at_path(&manifest, &["name"]),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported CurseForge Instance".to_owned());
    let game_version = first_non_empty([
        json_string_at_path(&manifest, &["gameVersion"]),
        json_string_at_path(&manifest, &["minecraftVersion"]),
        json_string_at_path(&manifest, &["baseModLoader", "minecraftVersion"]),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let loader_hint = first_non_empty([
        json_string_at_path(&manifest, &["baseModLoader", "name"]),
        json_string_at_path(&manifest, &["baseModLoader", "modLoader"]),
        json_string_at_path(&manifest, &["modLoader"]),
    ]);
    let loader_version_hint = first_non_empty([
        json_string_at_path(&manifest, &["baseModLoader", "forgeVersion"]),
        json_string_at_path(&manifest, &["baseModLoader", "version"]),
        json_string_at_path(&manifest, &["modLoaderVersion"]),
    ]);
    let (modloader, modloader_version) = infer_loader_pair(loader_hint, loader_version_hint);
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ContentInstallManifest::default());
    if managed_manifest.projects.is_empty() {
        managed_manifest = extract_managed_manifest_from_json(
            &manifest,
            source_root.as_path(),
            ManagedContentSourceHint::CurseForge,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::CurseForge,
        name,
        Some("Imported from an existing CurseForge instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

pub(super) fn inspect_prism_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    let source_root = if path.join(".minecraft").is_dir() {
        path.join(".minecraft")
    } else {
        path.to_path_buf()
    };
    let cfg = read_key_value_file(path.join("instance.cfg").as_path()).unwrap_or_default();
    let pack_json = read_json_file_optional(path.join("mmc-pack.json").as_path())?;
    let name = first_non_empty([
        cfg.get("name").cloned(),
        pack_json
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["name"])),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported Prism Instance".to_owned());
    let (game_version, modloader, modloader_version) =
        parse_prism_versions(pack_json.as_ref(), cfg.get("MCVersion").cloned());
    let managed_manifest =
        load_existing_managed_manifest(source_root.as_path()).unwrap_or_default();
    Ok(build_launcher_inspection(
        LauncherKind::Prism,
        name,
        Some("Imported from a Prism / MultiMC style instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

pub(super) fn inspect_atlauncher_instance(path: &Path) -> Result<LauncherInspection, String> {
    let manifest = read_json_file_optional(path.join("instance.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["name"])),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported ATLauncher Instance".to_owned());
    let game_version = first_non_empty([
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["minecraft", "version"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["minecraftVersion"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["version"])),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let (modloader, modloader_version) = infer_loader_pair(
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["loader"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["loaderVersion"])),
    );
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ContentInstallManifest::default());
    if managed_manifest.projects.is_empty()
        && let Some(value) = manifest.as_ref()
    {
        managed_manifest = extract_managed_manifest_from_json(
            value,
            source_root.as_path(),
            ManagedContentSourceHint::Auto,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::ATLauncher,
        name,
        Some("Imported from an existing ATLauncher instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

pub(super) fn inspect_generic_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    inspect_generic_launcher_instance_with_launcher(path, LauncherKind::Unknown)
}

pub(super) fn inspect_generic_launcher_instance_with_launcher(
    path: &Path,
    launcher: LauncherKind,
) -> Result<LauncherInspection, String> {
    if !path.is_dir() {
        return Err(format!("{} is not a directory.", path.display()));
    }
    let managed_manifest = load_existing_managed_manifest(path).unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| "Imported Instance".to_owned());
    Ok(build_launcher_inspection(
        launcher,
        name,
        Some(format!(
            "Imported by copying files from {}.",
            launcher.label()
        )),
        "latest".to_owned(),
        "Vanilla".to_owned(),
        String::new(),
        path.to_path_buf(),
        managed_manifest,
    ))
}

pub(super) fn build_launcher_inspection(
    launcher: LauncherKind,
    name: String,
    description: Option<String>,
    game_version: String,
    modloader: String,
    modloader_version: String,
    source_root: PathBuf,
    managed_manifest: ContentInstallManifest,
) -> LauncherInspection {
    let mods_count = count_regular_files(source_root.join("mods").as_path());
    let config_count = count_regular_files(source_root.join("config").as_path());
    let managed_count = managed_manifest.projects.len();
    LauncherInspection {
        launcher,
        name,
        description,
        game_version: default_if_blank(game_version.as_str(), "latest".to_owned()),
        modloader: default_if_blank(modloader.as_str(), "Vanilla".to_owned()),
        modloader_version,
        summary: format!(
            "Detected {} with {} managed projects, {} mods, and {} config files.",
            launcher.label(),
            managed_count,
            mods_count,
            config_count
        ),
        source_root,
        managed_manifest,
    }
}

pub(super) fn copy_launcher_instance_content(
    source_root: &Path,
    destination_root: &Path,
    managed_manifest: &ContentInstallManifest,
) -> Result<(), String> {
    copy_dir_recursive(source_root, source_root, destination_root)?;
    if !managed_manifest.projects.is_empty() {
        let raw = toml::to_string_pretty(managed_manifest)
            .map_err(|err| format!("failed to serialize managed import manifest: {err}"))?;
        fs_write_logged(
            destination_root.join(CONTENT_MANIFEST_FILE_NAME).as_path(),
            raw,
        )
        .map_err(|err| {
            format!(
                "failed to write managed import manifest into {}: {err}",
                destination_root.display()
            )
        })?;
    }
    Ok(())
}

pub(super) fn copy_dir_recursive(
    root: &Path,
    current: &Path,
    destination_root: &Path,
) -> Result<(), String> {
    let entries = fs_read_dir_logged(current)
        .map_err(|err| format!("failed to read {}: {err}", current.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|err| format!("failed to normalize {}: {err}", path.display()))?;
        if should_skip_import_path(relative) {
            continue;
        }
        let destination = destination_root.join(relative);
        if path.is_dir() {
            fs_create_dir_all_logged(destination.as_path())
                .map_err(|err| format!("failed to create {}: {err}", destination.display()))?;
            copy_dir_recursive(root, path.as_path(), destination_root)?;
        } else if path.is_file() {
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent)
                    .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
            }
            fs_copy_logged(path.as_path(), destination.as_path()).map_err(|err| {
                format!(
                    "failed to copy {} to {}: {err}",
                    path.display(),
                    destination.display()
                )
            })?;
        }
    }
    Ok(())
}

pub(super) fn should_skip_import_path(relative: &Path) -> bool {
    let normalized = relative.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        return false;
    }
    let skip_exact = [
        "instance.cfg",
        "mmc-pack.json",
        "profile.json",
        "minecraftinstance.json",
        "instance.json",
        CONTENT_MANIFEST_FILE_NAME,
    ];
    if skip_exact
        .iter()
        .any(|candidate| normalized.eq_ignore_ascii_case(candidate))
    {
        return true;
    }
    let skip_prefixes = [
        "logs/",
        "crash-reports/",
        "versions/",
        "libraries/",
        "natives/",
        ".cache/",
        "cache/",
        "downloads/",
    ];
    skip_prefixes
        .iter()
        .any(|prefix| normalized.to_ascii_lowercase().starts_with(prefix))
}

pub(super) fn read_json_file(path: &Path) -> Result<Value, String> {
    let raw = fs_read_to_string_logged(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

pub(super) fn read_json_file_optional(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    read_json_file(path).map(Some)
}

pub(super) fn read_key_value_file(path: &Path) -> Result<HashMap<String, String>, String> {
    let raw = fs_read_to_string_logged(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut values = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            values.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    Ok(values)
}

pub(super) fn parse_prism_versions(
    pack_json: Option<&Value>,
    cfg_game_version: Option<String>,
) -> (String, String, String) {
    let mut game_version = cfg_game_version.unwrap_or_else(|| "latest".to_owned());
    let mut loader = "Vanilla".to_owned();
    let mut loader_version = String::new();

    if let Some(Value::Array(components)) =
        pack_json.and_then(|value| value.get("components")).cloned()
    {
        for component in components {
            let uid = component
                .get("uid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let version = component
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if uid.contains("minecraft") && game_version == "latest" && !version.trim().is_empty() {
                game_version = version.clone();
            }
            if uid.contains("fabric") {
                loader = "Fabric".to_owned();
                loader_version = version;
            } else if uid.contains("neoforge") {
                loader = "NeoForge".to_owned();
                loader_version = version;
            } else if uid.contains("forge") {
                loader = "Forge".to_owned();
                loader_version = version;
            } else if uid.contains("quilt") {
                loader = "Quilt".to_owned();
                loader_version = version;
            }
        }
    }

    (game_version, loader, loader_version)
}

pub(super) fn json_string_at_path(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn first_non_empty<const N: usize>(values: [Option<String>; N]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

pub(super) fn infer_loader_pair(
    loader_hint: Option<String>,
    version_hint: Option<String>,
) -> (String, String) {
    let loader_hint = loader_hint.unwrap_or_else(|| "Vanilla".to_owned());
    let loader_hint_trimmed = loader_hint.trim().to_owned();
    let loader_hint_lower = loader_hint_trimmed.to_ascii_lowercase();
    let version_hint = version_hint.unwrap_or_default();
    if loader_hint_lower.contains("neoforge") {
        return (
            "NeoForge".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("fabric") {
        return (
            "Fabric".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("quilt") {
        return (
            "Quilt".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("forge") {
        return (
            "Forge".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    (
        default_if_blank(loader_hint_trimmed.as_str(), "Vanilla".to_owned()),
        version_hint,
    )
}

pub(super) fn trailing_loader_version(loader_hint: &str, explicit_version: &str) -> String {
    let explicit = explicit_version.trim();
    if !explicit.is_empty() {
        return explicit.to_owned();
    }
    loader_hint
        .split_once('-')
        .map(|(_, version)| version.trim().to_owned())
        .unwrap_or_default()
}

pub(super) fn load_existing_managed_manifest(
    path: &Path,
) -> Result<ContentInstallManifest, String> {
    let manifest_path = path.join(CONTENT_MANIFEST_FILE_NAME);
    if !manifest_path.exists() {
        return Ok(ContentInstallManifest::default());
    }
    let raw = fs_read_to_string_logged(manifest_path.as_path())
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
    toml::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", manifest_path.display()))
}

#[derive(Clone, Copy)]
pub(super) enum ManagedContentSourceHint {
    Auto,
    Modrinth,
    CurseForge,
}

pub(super) fn extract_managed_manifest_from_json(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
) -> ContentInstallManifest {
    let mut manifest = ContentInstallManifest::default();
    walk_json_for_projects(value, source_root, source_hint, &mut manifest);
    manifest
}

pub(super) fn walk_json_for_projects(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
    manifest: &mut ContentInstallManifest,
) {
    maybe_add_project_from_json(value, source_root, source_hint, manifest);
    match value {
        Value::Object(map) => {
            for child in map.values() {
                walk_json_for_projects(child, source_root, source_hint, manifest);
            }
        }
        Value::Array(values) => {
            for child in values {
                walk_json_for_projects(child, source_root, source_hint, manifest);
            }
        }
        _ => {}
    }
}

pub(super) fn maybe_add_project_from_json(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
    manifest: &mut ContentInstallManifest,
) {
    let Value::Object(map) = value else {
        return;
    };

    let modrinth_project_id = json_object_string(
        map,
        &[
            "project_id",
            "projectId",
            "modrinth_project_id",
            "modrinthProjectId",
        ],
    );
    let curseforge_project_id = json_object_u64(
        map,
        &[
            "addonID",
            "addonId",
            "projectID",
            "projectId",
            "curseforge_project_id",
            "curseforgeProjectId",
        ],
    );
    let source = match source_hint {
        ManagedContentSourceHint::Modrinth if modrinth_project_id.is_some() => Some("modrinth"),
        ManagedContentSourceHint::CurseForge if curseforge_project_id.is_some() => {
            Some("curseforge")
        }
        ManagedContentSourceHint::Auto => {
            if modrinth_project_id.is_some() {
                Some("modrinth")
            } else if curseforge_project_id.is_some() {
                Some("curseforge")
            } else {
                None
            }
        }
        _ => None,
    };
    if source.is_none() {
        return;
    }

    let version_id = first_non_empty([
        json_object_string(
            map,
            &[
                "version_id",
                "versionId",
                "fileId",
                "fileID",
                "gameVersionFileID",
            ],
        ),
        map.get("installedFile")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_u64)
            .map(|value| value.to_string()),
    ])
    .unwrap_or_default();
    let metadata_file_name = json_object_string(map, &["fileName", "filename", "file_name"])
        .or_else(|| {
            map.get("installedFile")
                .and_then(|value| value.get("fileName"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let version_name = json_object_string(map, &["version_name", "versionName"])
        .or_else(|| metadata_file_name.clone())
        .unwrap_or_default();
    let name = json_object_string(map, &["name", "title"])
        .or_else(|| {
            map.get("installedFile")
                .and_then(|value| value.get("displayName"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| version_name.clone());

    let Some(metadata_file_name) = metadata_file_name.as_deref() else {
        return;
    };

    let file_path = first_non_empty([
        json_object_string(map, &["path", "file_path", "filePath"]).and_then(|value| {
            resolve_existing_relative_file_path(source_root, value.as_str(), metadata_file_name)
        }),
        Some(metadata_file_name.to_owned()).and_then(|value| {
            resolve_existing_relative_file_path(source_root, value.as_str(), metadata_file_name)
        }),
    ]);
    let Some(file_path) = file_path else {
        return;
    };

    let project_key = if let Some(id) = modrinth_project_id.as_ref() {
        format!("modrinth:{id}")
    } else if let Some(id) = curseforge_project_id {
        format!("curseforge:{id}")
    } else {
        normalize_project_key(file_path.as_str())
    };
    manifest.projects.insert(
        project_key.clone(),
        InstalledContentProject {
            project_key,
            name,
            file_path: PathBuf::from(file_path),
            modrinth_project_id,
            curseforge_project_id,
            selected_source: match source {
                Some("modrinth") => Some(managed_content::ManagedContentSource::Modrinth),
                Some("curseforge") => Some(managed_content::ManagedContentSource::CurseForge),
                _ => None,
            },
            selected_version_id: non_empty(version_id.as_str()),
            selected_version_name: non_empty(version_name.as_str()),
            ..InstalledContentProject::default()
        },
    );
}

pub(super) fn json_object_string(
    map: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

pub(super) fn json_object_u64(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            Value::Number(number) => number.as_u64(),
            Value::String(raw) => raw.trim().parse::<u64>().ok(),
            _ => None,
        })
    })
}

pub(super) fn resolve_existing_relative_file_path(
    source_root: &Path,
    raw: &str,
    expected_file_name: &str,
) -> Option<String> {
    let normalized = normalize_project_key(raw);
    if normalized.is_empty() {
        return None;
    }

    let direct = source_root.join(normalized.as_str());
    if direct.is_file() && file_name_matches(direct.as_path(), expected_file_name) {
        return Some(normalized);
    }

    let known_dirs = ["mods", "resourcepacks", "shaderpacks", "datapacks"];
    for dir in known_dirs {
        let candidate = source_root.join(dir).join(raw);
        if candidate.is_file() && file_name_matches(candidate.as_path(), expected_file_name) {
            return Some(format!("{dir}/{}", raw.trim()));
        }
    }

    None
}

pub(super) fn file_name_matches(path: &Path, expected_file_name: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == expected_file_name.trim())
}

pub(super) fn normalize_project_key(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase()
}

pub(super) fn count_regular_files(path: &Path) -> usize {
    if !path.exists() {
        return 0;
    }
    count_regular_files_recursive(path).unwrap_or(0)
}

pub(super) fn count_regular_files_recursive(path: &Path) -> Result<usize, String> {
    let mut count = 0usize;
    let entries = fs_read_dir_logged(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            count += count_regular_files_recursive(entry_path.as_path())?;
        } else if entry_path.is_file() {
            count += 1;
        }
    }
    Ok(count)
}

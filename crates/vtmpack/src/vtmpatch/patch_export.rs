use vertex_i18n::tr;
use super::*;

pub fn export_instance_as_vtmpatch_with_progress<F>(
    instance_name: &str,
    instance_root: &Path,
    base_vtmpack_path: &Path,
    output_path: &Path,
    options: &VtmpackExportOptions,
    mut progress: F,
) -> Result<VtmpackExportStats, String>
where
    F: FnMut(VtmpackExportProgress),
{
    tracing::info!(
        target: LOG,
        instance = %instance_name,
        base = %base_vtmpack_path.display(),
        output = %output_path.display(),
        "vtmpatch export started"
    );

    progress(progress_update(vertex_i18n::src!("Reading base .vtmpack..."), 0, 5));
    let (base_manifest, base_entries) = read_base_snapshot(base_vtmpack_path)?;
    tracing::debug!(
        target: LOG,
        instance = %instance_name,
        base_name = %base_manifest.instance.name,
        base_entry_count = base_entries.len(),
        "base vtmpack snapshot loaded"
    );

    progress(progress_update(vertex_i18n::src!("Scanning current instance files..."), 1, 5));
    let current_entries = build_current_snapshot(instance_root, options)?;
    tracing::debug!(
        target: LOG,
        instance = %instance_name,
        current_entry_count = current_entries.len(),
        "current instance snapshot built"
    );

    // Diff: collect everything that changed or is new.
    let mut all_changed: Vec<PathBuf> = Vec::new();
    for (path, entry) in &current_entries {
        let base = base_entries.get(path);
        let changed = base.is_none_or(|b| b.differs_from(entry));
        if changed {
            tracing::trace!(
                target: LOG,
                instance = %instance_name,
                path = %path.display(),
                is_new = base.is_none(),
                "diff: entry changed or added"
            );
            all_changed.push(path.clone());
        }
    }

    let current_paths: BTreeSet<PathBuf> = current_entries.keys().cloned().collect();
    let mut removed_paths: Vec<PathBuf> = base_entries
        .keys()
        .filter(|path| !current_paths.contains(*path))
        .cloned()
        .collect();

    all_changed.sort();
    removed_paths.sort();

    for path in &removed_paths {
        tracing::trace!(
            target: LOG,
            instance = %instance_name,
            path = %path.display(),
            "diff: entry removed"
        );
    }
    tracing::info!(
        target: LOG,
        instance = %instance_name,
        changed_or_added = all_changed.len(),
        removed = removed_paths.len(),
        "diff complete"
    );

    // Separate mod files (candidates for Modrinth rediscovery) from everything else.
    let (mod_changed, other_changed): (Vec<PathBuf>, Vec<PathBuf>) = all_changed
        .into_iter()
        .partition(|p| path_to_string(p).starts_with("bundled_mods/"));

    tracing::debug!(
        target: LOG,
        instance = %instance_name,
        mod_changed = mod_changed.len(),
        other_changed = other_changed.len(),
        "partitioned changed entries into mods vs other"
    );

    // Attempt Modrinth rediscovery for changed mod files.
    progress(progress_update(
        vertex_i18n::src!("Looking up changed mods on Modrinth..."),
        2,
        5,
    ));
    let mut downloadable_entries = Vec::new();
    let mut downloadable_fallback_paths = Vec::new();
    let mut unresolved_mod_changed = Vec::new();
    for path in mod_changed {
        let Some(entry) = current_entries.get(&path) else {
            continue;
        };
        if let Some(downloadable) = entry.downloadable_entry.clone() {
            if downloadable_needs_bundle_fallback(&downloadable) && entry.source_path.is_some() {
                tracing::debug!(
                    target: LOG,
                    instance = %instance_name,
                    path = %path.display(),
                    name = %downloadable.name,
                    "managed mod needs bundle fallback (missing version or hash)"
                );
                downloadable_fallback_paths.push(path);
            } else {
                tracing::debug!(
                    target: LOG,
                    instance = %instance_name,
                    path = %path.display(),
                    name = %downloadable.name,
                    version_id = ?downloadable.selected_version_id,
                    source = ?downloadable.selected_source,
                    "managed mod resolved as downloadable entry"
                );
            }
            downloadable_entries.push(downloadable);
        } else {
            tracing::debug!(
                target: LOG,
                instance = %instance_name,
                path = %path.display(),
                "mod not in managed manifest — queuing for Modrinth hash rediscovery"
            );
            unresolved_mod_changed.push(path);
        }
    }

    tracing::info!(
        target: LOG,
        instance = %instance_name,
        already_resolved = downloadable_entries.len(),
        needs_rediscovery = unresolved_mod_changed.len(),
        "managed mods accounted for; starting Modrinth rediscovery for the rest"
    );

    let (rediscovered_downloadable_entries, bundled_mod_paths) =
        rediscover_changed_mods(&unresolved_mod_changed, &current_entries);

    tracing::info!(
        target: LOG,
        instance = %instance_name,
        rediscovered = rediscovered_downloadable_entries.len(),
        still_bundled = bundled_mod_paths.len(),
        "Modrinth rediscovery complete"
    );
    for entry in &rediscovered_downloadable_entries {
        tracing::debug!(
            target: LOG,
            instance = %instance_name,
            name = %entry.name,
            modrinth_project = ?entry.modrinth_project_id,
            version_id = ?entry.selected_version_id,
            "rediscovered mod will be downloaded on apply"
        );
    }
    for path in &bundled_mod_paths {
        tracing::debug!(
            target: LOG,
            instance = %instance_name,
            path = %path.display(),
            "mod not on Modrinth — embedding raw bytes in patch archive"
        );
    }

    downloadable_entries.extend(rediscovered_downloadable_entries);

    let mut bundled_paths: Vec<PathBuf> = other_changed;
    bundled_paths.extend(downloadable_fallback_paths);
    bundled_paths.extend(bundled_mod_paths);

    // Local mods (those not resolved to any downloadable provider) must always
    // be bundled in full, even if they are identical to the base.  Downloadable
    // mods can be re-fetched on apply; local-only files have no external source,
    // so omitting an unchanged one from the patch would silently leave it
    // missing on any instance that has not already had the base vtmpack applied.
    let downloadable_archive_paths: BTreeSet<PathBuf> = downloadable_entries
        .iter()
        .map(|e| instance_relative_to_archive_path(e.file_path.as_path()))
        .collect();
    for (path, entry) in &current_entries {
        if !path_to_string(path).starts_with("bundled_mods/") {
            continue;
        }
        // Skip anything already tracked as a downloadable entry.
        if entry.downloadable_entry.is_some() || downloadable_archive_paths.contains(path) {
            continue;
        }
        // Must have a source file on disk to be bundleable.
        if entry.source_path.is_none() {
            continue;
        }
        if !bundled_paths.contains(path) {
            tracing::debug!(
                target: LOG,
                instance = %instance_name,
                path = %path.display(),
                "including unchanged local mod in patch (no external source available)"
            );
            bundled_paths.push(path.clone());
        }
    }

    bundled_paths.sort();
    bundled_paths.dedup();

    tracing::info!(
        target: LOG,
        instance = %instance_name,
        downloadable = downloadable_entries.len(),
        bundled = bundled_paths.len(),
        removed = removed_paths.len(),
        "patch contents determined; writing archive"
    );

    // Build output archive.
    progress(progress_update(vertex_i18n::src!("Writing patch archive..."), 3, 5));

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| {
            tr!("failed to create patch export directory {parent}: {err}", parent = parent.display(), err = err).into_owned()
        })?;
    }

    let output_file = fs::File::create(output_path)
        .map_err(|err| tr!("failed to create {output_path}: {err}", output_path = output_path.display(), err = err).into_owned())?;
    let encoder = xz2::write::XzEncoder::new(
        output_file,
        xz_preset_for_compression_mode(options.compression_mode),
    );
    let mut archive = tar::Builder::new(encoder);

    let patch_manifest = VtmpatchManifest {
        format: "vtmpatch".to_owned(),
        version: VTMPACK_MANIFEST_VERSION,
        generated_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        base_instance_name: base_manifest.instance.name,
        target_instance_name: instance_name.to_owned(),
        downloadable_entries: downloadable_entries.clone(),
        bundled_paths: bundled_paths.clone(),
        removed_paths: removed_paths.clone(),
    };

    let manifest_toml = toml::to_string_pretty(&patch_manifest)
        .map_err(|err| tr!("failed to serialize vtmpatch manifest: {err}", err = err).into_owned())?
        .into_bytes();
    append_bytes_to_archive(&mut archive, PATCH_MANIFEST_PATH, &manifest_toml)?;

    // Embed only files that couldn't be resolved via Modrinth.
    progress(progress_update(vertex_i18n::src!("Embedding bundled files..."), 4, 5));
    for path in &bundled_paths {
        let Some(entry) = current_entries.get(path) else {
            tracing::warn!(
                target: LOG,
                instance = %instance_name,
                path = %path.display(),
                "bundled path not found in current snapshot — skipping"
            );
            continue;
        };
        if let Some(source_path) = entry.source_path.as_ref() {
            tracing::trace!(
                target: LOG,
                instance = %instance_name,
                path = %path.display(),
                source = %source_path.display(),
                "appending bundled file from disk"
            );
            archive
                .append_path_with_name(source_path.as_path(), path.as_path())
                .map_err(|err| tr!("failed to append {source_path}: {err}", source_path = source_path.display(), err = err).into_owned())?;
        } else {
            tracing::trace!(
                target: LOG,
                instance = %instance_name,
                path = %path.display(),
                bytes = entry.bytes.len(),
                "appending bundled file from memory"
            );
            append_bytes_to_archive(
                &mut archive,
                path_to_archive_name(path)?.as_str(),
                &entry.bytes,
            )?;
        }
    }

    archive
        .finish()
        .map_err(|err| tr!("failed to finalize patch archive: {err}", err = err).into_owned())?;
    let encoder = archive
        .into_inner()
        .map_err(|err| tr!("failed to flush patch archive: {err}", err = err).into_owned())?;
    encoder
        .finish()
        .map_err(|err| tr!("failed to finalize patch xz stream: {err}", err = err).into_owned())?;

    let bundled_mod_count = bundled_paths
        .iter()
        .filter(|p| path_to_string(p).starts_with("bundled_mods/"))
        .count();
    let config_count = bundled_paths
        .iter()
        .filter(|p| path_to_string(p).starts_with("configs/"))
        .count();
    let additional_count = bundled_paths
        .iter()
        .filter(|p| path_to_string(p).starts_with("root_entries/"))
        .count();

    tracing::info!(
        target: LOG,
        instance = %instance_name,
        output = %output_path.display(),
        downloadable_mods = downloadable_entries.len(),
        bundled_mods = bundled_mod_count,
        config_files = config_count,
        additional_files = additional_count,
        removed = removed_paths.len(),
        "vtmpatch export complete"
    );

    progress(progress_update(vertex_i18n::src!("Patch export complete."), 5, 5));

    Ok(VtmpackExportStats {
        bundled_mod_files: bundled_mod_count,
        downloadable_mod_files: downloadable_entries.len(),
        config_files: config_count,
        additional_files: additional_count,
    })
}

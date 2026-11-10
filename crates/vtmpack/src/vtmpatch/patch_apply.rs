use super::*;

pub fn apply_vtmpatch_to_instance(
    patch_path: &Path,
    instance_root: &Path,
) -> Result<String, String> {
    let mut archive = open_patch_archive(patch_path)?;
    let mut patch_manifest: Option<VtmpatchManifest> = None;
    let mut bundled_entries: Vec<(PathBuf, Vec<u8>)> = Vec::new();

    for entry in archive
        .entries()
        .map_err(|err| format!("failed to read {}: {err}", patch_path.display()))?
    {
        let mut entry = entry.map_err(|err| format!("failed to read patch entry: {err}"))?;
        let archive_path = entry
            .path()
            .map_err(|err| format!("failed to decode patch entry path: {err}"))?
            .to_path_buf();
        let archive_name = path_to_string(&archive_path);
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|err| format!("failed to read {archive_name}: {err}"))?;

        if archive_name == PATCH_MANIFEST_PATH {
            patch_manifest = Some(
                toml::from_str::<VtmpatchManifest>(
                    std::str::from_utf8(&bytes)
                        .map_err(|err| format!("patch.toml is not valid UTF-8: {err}"))?,
                )
                .map_err(|err| format!("failed to parse patch.toml: {err}"))?,
            );
        } else {
            bundled_entries.push((archive_path, bytes));
        }
    }

    let patch_manifest = patch_manifest.ok_or_else(|| {
        format!(
            "No {PATCH_MANIFEST_PATH} found in Vertex patch {}",
            patch_path.display()
        )
    })?;

    tracing::info!(
        target: LOG,
        patch = %patch_path.display(),
        instance = %instance_root.display(),
        base = %patch_manifest.base_instance_name,
        target = %patch_manifest.target_instance_name,
        downloadable = patch_manifest.downloadable_entries.len(),
        bundled = patch_manifest.bundled_paths.len(),
        removed = patch_manifest.removed_paths.len(),
        "vtmpatch apply started"
    );

    // Remove deleted files.
    let mut removed = 0usize;
    for archive_path in &patch_manifest.removed_paths {
        let destination = archive_path_to_instance_path(instance_root, archive_path)?;
        if destination.is_file() {
            tracing::debug!(
                target: LOG,
                path = %destination.display(),
                "removing file per patch removed_paths"
            );
            fs::remove_file(&destination)
                .map_err(|err| format!("failed to remove {}: {err}", destination.display()))?;
            removed += 1;
        } else if destination.is_dir() {
            tracing::debug!(
                target: LOG,
                path = %destination.display(),
                "removing directory per patch removed_paths"
            );
            fs::remove_dir_all(&destination)
                .map_err(|err| format!("failed to remove {}: {err}", destination.display()))?;
            removed += 1;
        } else {
            tracing::debug!(
                target: LOG,
                path = %destination.display(),
                archive_path = %archive_path.display(),
                "removal target not found on disk — already absent, skipping"
            );
        }
    }
    tracing::info!(
        target: LOG,
        instance = %instance_root.display(),
        removed,
        "removal phase complete"
    );

    // Extract embedded (bundled) files.
    let mut changed = 0usize;
    let mut bundled_applied_paths = BTreeSet::<PathBuf>::new();
    for (archive_path, bytes) in bundled_entries {
        let destination = archive_path_to_instance_path(instance_root, archive_path.as_path())?;
        tracing::debug!(
            target: LOG,
            archive_path = %archive_path.display(),
            destination = %destination.display(),
            bytes = bytes.len(),
            "writing bundled file"
        );
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        fs::write(&destination, &bytes)
            .map_err(|err| format!("failed to write {}: {err}", destination.display()))?;
        bundled_applied_paths.insert(archive_path);
        changed += 1;
    }
    tracing::info!(
        target: LOG,
        instance = %instance_root.display(),
        changed,
        "bundled-file extraction complete"
    );

    // Download provider-resolvable files.
    let mut downloaded = 0usize;
    let mut download_failures = Vec::<String>::new();
    if !patch_manifest.downloadable_entries.is_empty() {
        tracing::info!(
            target: LOG,
            instance = %instance_root.display(),
            count = patch_manifest.downloadable_entries.len(),
            "downloading provider-resolved files"
        );
        let entries: Vec<&VtmpackDownloadableEntry> = patch_manifest.downloadable_entries.iter().collect();
        let results = download_downloadable_entries(&entries, instance_root);
        for (entry, result) in entries.into_iter().zip(results) {
            match result {
                Ok(()) => {
                    tracing::debug!(
                        target: LOG,
                        name = %entry.name,
                        file_path = %entry.file_path.display(),
                        "download succeeded"
                    );
                    downloaded += 1;
                }
                Err(err) => {
                    let archive_path = instance_relative_to_archive_path(entry.file_path.as_path());
                    if bundled_applied_paths.contains(&archive_path) {
                        tracing::warn!(
                            target: LOG,
                            name = %entry.name,
                            file_path = %entry.file_path.display(),
                            error = %err,
                            "provider download failed — using bundled fallback that was already written"
                        );
                    } else {
                        tracing::warn!(
                            target: LOG,
                            name = %entry.name,
                            file_path = %entry.file_path.display(),
                            error = %err,
                            "provider download failed and no bundled fallback is available"
                        );
                        download_failures.push(format!("{}: {err}", entry.file_path.display()));
                    }
                }
            }
        }
        tracing::info!(
            target: LOG,
            instance = %instance_root.display(),
            downloaded,
            failed = download_failures.len(),
            "explicit downloadable_entries phase complete"
        );
    }

    // Fill any gaps left by the delta-patch approach: read the content manifest
    // that was just written and download any managed Modrinth mods that the
    // diff omitted because they were "unchanged" — they may still be absent on
    // a fresh instance that never had the base vtmpack applied.
    let manifest_path = content_manifest_path(instance_root);
    if let Ok(raw) = fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = toml::from_str::<ContentInstallManifest>(&raw) {
            let mut gap_fills = 0usize;
            let mut gap_entries = Vec::new();
            for project in manifest.projects.values() {
                if project.selected_source != Some(ManagedContentSource::Modrinth) {
                    continue;
                }
                match project.selected_version_id.as_deref() {
                    Some(v) if !v.trim().is_empty() => {}
                    _ => continue,
                }
                let dest = instance_root.join(project.file_path.as_path());
                if dest.is_file() {
                    continue; // Already present — nothing to do.
                }
                tracing::debug!(
                    target: LOG,
                    name = %project.name,
                    file_path = %project.file_path.display(),
                    "manifest lists a managed mod absent from disk — downloading"
                );
                gap_entries.push(VtmpackDownloadableEntry {
                    project_key: project.project_key.clone(),
                    name: project.name.clone(),
                    file_path: normalize_pack_path(project.file_path.as_path()),
                    modrinth_project_id: project.modrinth_project_id.clone(),
                    curseforge_project_id: None,
                    selected_source: Some("Modrinth".to_owned()),
                    selected_version_id: project.selected_version_id.clone(),
                    selected_version_name: project.selected_version_name.clone(),
                    selected_file_sha1: project.selected_file_sha1.clone(),
                    selected_file_sha512: project.selected_file_sha512.clone(),
                });
            }
            let gap_refs: Vec<&VtmpackDownloadableEntry> = gap_entries.iter().collect();
            for (entry, result) in gap_refs.iter().zip(download_downloadable_entries(&gap_refs, instance_root)) {
                match result {
                    Ok(()) => {
                        tracing::debug!(target: LOG, name = %entry.name, "gap-fill download succeeded");
                        downloaded += 1;
                        gap_fills += 1;
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: LOG,
                            name = %entry.name,
                            file_path = %entry.file_path.display(),
                            error = %err,
                            "gap-fill download failed"
                        );
                        download_failures
                            .push(format!("{} (gap fill): {err}", entry.file_path.display()));
                    }
                }
            }
            if gap_fills > 0 {
                tracing::info!(
                    target: LOG,
                    instance = %instance_root.display(),
                    gap_fills,
                    "gap-fill phase downloaded missing managed mods"
                );
            }
        }
    }

    let planned_mod_operations = patch_manifest
        .bundled_paths
        .iter()
        .filter(|path| path_to_string(path).starts_with("bundled_mods/"))
        .count()
        + patch_manifest
            .downloadable_entries
            .iter()
            .filter(|entry| path_to_string(entry.file_path.as_path()).starts_with("mods/"))
            .count()
        + patch_manifest
            .removed_paths
            .iter()
            .filter(|path| path_to_string(path).starts_with("bundled_mods/"))
            .count();

    if !download_failures.is_empty() {
        let err_msg = format!(
            "Patch apply incomplete: {changed} bundled file{}, {downloaded} downloaded file{}, {removed} removed file{}. {} file{} failed to download: {}",
            plural(changed),
            plural(downloaded),
            plural(removed),
            download_failures.len(),
            plural(download_failures.len()),
            download_failures.join("; ")
        );
        tracing::error!(
            target: LOG,
            instance = %instance_root.display(),
            changed,
            downloaded,
            removed,
            failures = download_failures.len(),
            "vtmpatch apply finished with download errors: {err_msg}"
        );
        return Err(err_msg);
    }

    let message = format!(
        "Applied patch: {changed} file{} bundled, {downloaded} file{} downloaded, {removed} file{} removed ({planned_mod_operations} planned mod operation{}).",
        plural(changed),
        plural(downloaded),
        plural(removed),
        plural(planned_mod_operations),
    );
    tracing::info!(
        target: LOG,
        instance = %instance_root.display(),
        changed,
        downloaded,
        removed,
        planned_mod_operations,
        "vtmpatch apply succeeded: {message}"
    );

    if changed == 0 && downloaded == 0 && removed == 0 {
        let planned_changes = patch_manifest.bundled_paths.len()
            + patch_manifest.downloadable_entries.len()
            + patch_manifest.removed_paths.len();
        if planned_changes == 0 {
            tracing::info!(
                target: LOG,
                instance = %instance_root.display(),
                patch = %patch_path.display(),
                "patch contained no changes — instance already matches the target"
            );
            return Ok("Applied patch: no changes were present in the patch.".to_owned());
        } else {
            let err_msg = format!(
                "Patch apply made no filesystem changes despite {planned_changes} planned operation{}. \
                 The target instance may already match the patch, or all removal targets were already absent.",
                plural(planned_changes)
            );
            tracing::warn!(
                target: LOG,
                instance = %instance_root.display(),
                patch = %patch_path.display(),
                planned_changes,
                "vtmpatch apply made no filesystem changes: {err_msg}"
            );
            return Err(err_msg);
        }
    }

    Ok(message)
}

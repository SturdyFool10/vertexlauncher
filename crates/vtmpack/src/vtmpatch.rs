use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use managed_content::{
    CONTENT_MANIFEST_FILE_NAME, ContentInstallManifest, ManagedContentSource, content_manifest_path,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha512};

use crate::{
    VTMPACK_MANIFEST_VERSION, VtmpackCompressionMode, VtmpackDownloadableEntry,
    VtmpackExportOptions, VtmpackExportProgress, VtmpackExportStats, VtmpackManifest,
    VtmpackProviderMode,
};

const XZ_PRESET_STANDARD: u32 = 6;
const XZ_PRESET_EXTREME_FLAG: u32 = 1 << 31;
const XZ_PRESET_EXTREME: u32 = 9 | XZ_PRESET_EXTREME_FLAG;
const PATCH_MANIFEST_PATH: &str = "patch.toml";
const LOG: &str = "vertexlauncher/vtmpatch";
/// Archive path used for the managed-content manifest inside both `.vtmpack`
/// and `.vtmpatch` archives.  Must match the hard-coded string in export.rs.
/// Note: the on-disk filename is `.vertex-content-manifest.toml` (leading dot),
/// but the archive path intentionally omits the dot so it isn't hidden on
/// archive inspection tools.
const CONTENT_MANIFEST_ARCHIVE_PATH: &str = "metadata/vertex-content-manifest.toml";

/// Patch manifest stored inside a `.vtmpatch` archive.
///
/// Files in the patch fall into three categories:
///
/// * **`downloadable_entries`** — mods (and other content) that were resolved
///   to a Modrinth version.  Only their metadata is stored here; the apply
///   step downloads the actual bytes from the CDN.  This is the main reason
///   patches are small: a 20 MB JAR becomes ~300 bytes of TOML.
///
/// * **`bundled_paths`** — files that could *not* be resolved on Modrinth
///   (custom mods, datapacks, local configs, etc.) and are therefore embedded
///   verbatim in the archive, compressed with XZ.
///
/// * **`removed_paths`** — paths that existed in the base pack but are absent
///   from the target; they are deleted during apply.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VtmpatchManifest {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub generated_at_ms: u64,
    #[serde(default)]
    pub base_instance_name: String,
    #[serde(default)]
    pub target_instance_name: String,
    /// Content resolved to Modrinth versions — downloaded, not embedded.
    #[serde(default)]
    pub downloadable_entries: Vec<VtmpackDownloadableEntry>,
    /// Files embedded in the archive (Modrinth lookup failed or not a mod).
    /// Legacy field name `added_or_changed_paths` is accepted for old patches.
    #[serde(default, alias = "added_or_changed_paths")]
    pub bundled_paths: Vec<PathBuf>,
    #[serde(default)]
    pub removed_paths: Vec<PathBuf>,
}

// ── Snapshot types ────────────────────────────────────────────────────────────

/// One entry in the in-memory snapshot used for diffing.
///
/// For files that are just compared by content we store `bytes`.
/// For mod files (which can be large and are better compared by hash) we
/// store `sha512` instead.  The two are mutually exclusive in practice:
/// a file with `sha512` set will have `bytes` empty, and vice-versa.
#[derive(Debug, Clone, Default)]
struct SnapshotEntry {
    /// Raw content (configs, manifests, non-mod root-entries).
    bytes: Vec<u8>,
    /// SHA-512 hex digest (mod jars — avoids loading MB of bytes for comparison).
    sha512: Option<String>,
    /// Original on-disk path (only set when we still need to read the file
    /// for archiving; absent for downloadable-entry sentinels).
    source_path: Option<PathBuf>,
    /// Download metadata for files that `.vtmpack` would shrink by storing as
    /// downloadable content instead of bundling bytes.
    downloadable_entry: Option<VtmpackDownloadableEntry>,
}

impl SnapshotEntry {
    fn differs_from(&self, other: &SnapshotEntry) -> bool {
        match (&self.sha512, &other.sha512) {
            (Some(a), Some(b)) => return !a.eq_ignore_ascii_case(b),
            (Some(a), None) => {
                if !other.bytes.is_empty() {
                    return !a.eq_ignore_ascii_case(hash_bytes_sha512_hex(&other.bytes).as_str());
                }
                return true;
            }
            (None, Some(b)) => {
                if !self.bytes.is_empty() {
                    return !hash_bytes_sha512_hex(&self.bytes).eq_ignore_ascii_case(b);
                }
                return true;
            }
            (None, None) => {}
        }

        match (&self.downloadable_entry, &other.downloadable_entry) {
            (Some(a), Some(b)) => return downloadable_entries_differ(a, b),
            // One side is a downloadable sentinel and the other is a plain file
            // (or absent) — always treat as changed.
            (Some(_), None) | (None, Some(_)) => return true,
            (None, None) => {}
        }

        // Plain byte-content comparison for configs and other non-mod files.
        // Two entries with no sha512, no downloadable_entry, and no bytes are
        // considered equal (both absent / empty).
        self.bytes != other.bytes
    }
}

fn downloadable_entries_differ(a: &VtmpackDownloadableEntry, b: &VtmpackDownloadableEntry) -> bool {
    a.file_path != b.file_path
        || a.modrinth_project_id != b.modrinth_project_id
        || a.curseforge_project_id != b.curseforge_project_id
        || a.selected_source != b.selected_source
        || a.selected_version_id != b.selected_version_id
        || a.selected_version_name != b.selected_version_name
        || a.selected_file_sha1 != b.selected_file_sha1
        || a.selected_file_sha512 != b.selected_file_sha512
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn default_vtmpatch_file_name(instance_name: &str) -> String {
    let base = crate::default_vtmpack_file_name(instance_name);
    base.strip_suffix(".vtmpack")
        .map(|name| format!("{name}.vtmpatch"))
        .unwrap_or_else(|| "instance.vtmpatch".to_owned())
}

#[must_use]
pub fn enforce_vtmpatch_extension(mut path: PathBuf) -> PathBuf {
    let has_extension = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(crate::VTMPATCH_EXTENSION));
    if !has_extension {
        path.set_extension(crate::VTMPATCH_EXTENSION);
    }
    path
}

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

    progress(progress_update("Reading base .vtmpack...", 0, 5));
    let (base_manifest, base_entries) = read_base_snapshot(base_vtmpack_path)?;
    tracing::debug!(
        target: LOG,
        instance = %instance_name,
        base_name = %base_manifest.instance.name,
        base_entry_count = base_entries.len(),
        "base vtmpack snapshot loaded"
    );

    progress(progress_update("Scanning current instance files...", 1, 5));
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
        "Looking up changed mods on Modrinth...",
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
    progress(progress_update("Writing patch archive...", 3, 5));

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create patch export directory {}: {err}",
                parent.display()
            )
        })?;
    }

    let output_file = fs::File::create(output_path)
        .map_err(|err| format!("failed to create {}: {err}", output_path.display()))?;
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
        .map_err(|err| format!("failed to serialize vtmpatch manifest: {err}"))?
        .into_bytes();
    append_bytes_to_archive(&mut archive, PATCH_MANIFEST_PATH, &manifest_toml)?;

    // Embed only files that couldn't be resolved via Modrinth.
    progress(progress_update("Embedding bundled files...", 4, 5));
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
                .map_err(|err| format!("failed to append {}: {err}", source_path.display()))?;
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
        .map_err(|err| format!("failed to finalize patch archive: {err}"))?;
    let encoder = archive
        .into_inner()
        .map_err(|err| format!("failed to flush patch archive: {err}"))?;
    encoder
        .finish()
        .map_err(|err| format!("failed to finalize patch xz stream: {err}"))?;

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

    progress(progress_update("Patch export complete.", 5, 5));

    Ok(VtmpackExportStats {
        bundled_mod_files: bundled_mod_count,
        downloadable_mod_files: downloadable_entries.len(),
        config_files: config_count,
        additional_files: additional_count,
    })
}

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
        for entry in &patch_manifest.downloadable_entries {
            tracing::debug!(
                target: LOG,
                name = %entry.name,
                file_path = %entry.file_path.display(),
                source = ?entry.selected_source,
                version_id = ?entry.selected_version_id,
                "downloading entry"
            );
            match download_downloadable_entry(entry, instance_root) {
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
            for project in manifest.projects.values() {
                if project.selected_source != Some(ManagedContentSource::Modrinth) {
                    continue;
                }
                let version_id = match project.selected_version_id.as_deref() {
                    Some(v) if !v.trim().is_empty() => v,
                    _ => continue,
                };
                let dest = instance_root.join(project.file_path.as_path());
                if dest.is_file() {
                    continue; // Already present — nothing to do.
                }
                tracing::debug!(
                    target: LOG,
                    name = %project.name,
                    file_path = %project.file_path.display(),
                    version_id,
                    "manifest lists a managed mod absent from disk — downloading"
                );
                let entry = VtmpackDownloadableEntry {
                    project_key: project.project_key.clone(),
                    name: project.name.clone(),
                    file_path: crate::export::normalize_pack_path(project.file_path.as_path()),
                    modrinth_project_id: project.modrinth_project_id.clone(),
                    curseforge_project_id: None,
                    selected_source: Some("Modrinth".to_owned()),
                    selected_version_id: project.selected_version_id.clone(),
                    selected_version_name: project.selected_version_name.clone(),
                    selected_file_sha1: project.selected_file_sha1.clone(),
                    selected_file_sha512: project.selected_file_sha512.clone(),
                };
                match download_downloadable_entry(&entry, instance_root) {
                    Ok(()) => {
                        tracing::debug!(
                            target: LOG,
                            name = %project.name,
                            "gap-fill download succeeded"
                        );
                        downloaded += 1;
                        gap_fills += 1;
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: LOG,
                            name = %project.name,
                            file_path = %project.file_path.display(),
                            error = %err,
                            "gap-fill download failed"
                        );
                        download_failures
                            .push(format!("{} (gap fill): {err}", project.file_path.display()));
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

// ── Snapshot building ─────────────────────────────────────────────────────────

/// Read the base vtmpack into a snapshot map.
///
/// Bundled files are stored with their raw bytes for exact comparison.
/// Downloadable entries are stored as hash-only sentinels — comparing them
/// against the current instance by hash avoids always treating every managed
/// mod as "changed" (the bug that made old patches huge).
fn read_base_snapshot(
    path: &Path,
) -> Result<(VtmpackManifest, BTreeMap<PathBuf, SnapshotEntry>), String> {
    let mut archive = crate::open_vtmpack_tar_archive(path)?;
    let mut manifest: Option<VtmpackManifest> = None;
    let mut entries: BTreeMap<PathBuf, SnapshotEntry> = BTreeMap::new();

    for entry in archive
        .entries()
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?
    {
        let mut entry = entry.map_err(|err| format!("failed to read archive entry: {err}"))?;
        let archive_path = entry
            .path()
            .map_err(|err| format!("failed to decode archive path: {err}"))?
            .to_path_buf();
        let archive_name = path_to_string(&archive_path);
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|err| format!("failed to read {archive_name}: {err}"))?;

        if archive_name == "manifest.toml" {
            manifest = Some(
                toml::from_str::<VtmpackManifest>(
                    std::str::from_utf8(&bytes)
                        .map_err(|err| format!("base manifest.toml is not valid UTF-8: {err}"))?,
                )
                .map_err(|err| format!("failed to parse base manifest.toml: {err}"))?,
            );
        } else {
            let sha512 = archive_name
                .starts_with("bundled_mods/")
                .then(|| hash_bytes_sha512_hex(&bytes));
            entries.insert(
                archive_path,
                SnapshotEntry {
                    bytes,
                    sha512,
                    source_path: None,
                    downloadable_entry: None,
                },
            );
        }
    }

    let manifest = manifest
        .ok_or_else(|| format!("No manifest.toml found in Vertex pack {}", path.display()))?;

    // Insert hash-only sentinels for downloadable entries so they compare
    // correctly against mod files that are present in the current instance.
    for dl in &manifest.downloadable_content {
        if dl.file_path.as_os_str().is_empty() {
            continue;
        }
        let archive_path = instance_relative_to_archive_path(dl.file_path.as_path());
        entries
            .entry(archive_path)
            .or_insert_with(|| SnapshotEntry {
                bytes: Vec::new(),
                sha512: dl.selected_file_sha512.clone(),
                source_path: None,
                downloadable_entry: Some(dl.clone()),
            });
    }

    Ok((manifest, entries))
}

/// Build a snapshot of the current instance, matching the archive-path layout
/// used by the vtmpack format.
///
/// Mod files are stored as hash-only entries (avoids loading large JARs into
/// memory just for comparison); everything else is stored as raw bytes.
fn build_current_snapshot(
    instance_root: &Path,
    options: &VtmpackExportOptions,
) -> Result<BTreeMap<PathBuf, SnapshotEntry>, String> {
    let mut entries: BTreeMap<PathBuf, SnapshotEntry> = BTreeMap::new();

    let mut selected_root_entries: HashSet<&str> = options
        .included_root_entries
        .iter()
        .filter_map(|(entry, included)| included.then_some(entry.as_str()))
        .collect();
    // Patches must always diff mods. The include checkboxes are useful for
    // optional root content, but omitting `mods` can produce a patch that
    // appears to apply successfully while never changing the mod set.
    selected_root_entries.insert("mods");

    let managed_manifest = {
        let path = content_manifest_path(instance_root);
        let manifest = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str::<ContentInstallManifest>(&raw).ok())
            .unwrap_or_default();
        crate::export::manifest_with_disabled_mod_paths(instance_root, &manifest)
    };
    let rediscovered_manifest = crate::export::rediscover_modrinth_mods(
        instance_root,
        &managed_manifest,
        &selected_root_entries,
    );
    let sanitized_manifest = sanitize_patch_manifest_for_export(&rediscovered_manifest, options);
    let downloadable_entries = downloadable_entries_from_manifest(&sanitized_manifest, options);
    let downloadable_archive_paths = downloadable_entries
        .iter()
        .map(|entry| instance_relative_to_archive_path(entry.file_path.as_path()))
        .collect::<BTreeSet<_>>();

    if !sanitized_manifest.projects.is_empty() {
        let bytes = toml::to_string_pretty(&sanitized_manifest)
            .map_err(|err| format!("failed to serialize patch content manifest: {err}"))?
            .into_bytes();
        entries.insert(
            PathBuf::from(CONTENT_MANIFEST_ARCHIVE_PATH),
            SnapshotEntry {
                bytes,
                sha512: None,
                source_path: None,
                downloadable_entry: None,
            },
        );
    }

    for downloadable in downloadable_entries {
        if downloadable.file_path.as_os_str().is_empty() {
            continue;
        }
        let source_path = instance_root.join(downloadable.file_path.as_path());
        if !source_path.is_file() {
            // File absent on the pack author's disk.  Omitting the entry entirely
            // is the correct move: if this mod was in the base vtmpack the diff
            // will see it as "not in current" and add it to `removed_paths`.  If
            // it is brand-new (not in the base at all) we conservatively exclude
            // it rather than risk emitting a download that can never be verified.
            // Pack authors should have their working mod set installed locally
            // before generating a patch.
            continue;
        }
        let actual_sha512 = modrinth::hash_file_sha512_hex(source_path.as_path())
            .map_err(|err| format!("failed to hash {}: {err}", source_path.display()))?;
        let archive_path = instance_relative_to_archive_path(downloadable.file_path.as_path());
        entries.insert(
            archive_path,
            SnapshotEntry {
                bytes: Vec::new(),
                sha512: Some(actual_sha512),
                source_path: Some(source_path),
                downloadable_entry: Some(downloadable),
            },
        );
    }

    for entry_name in &selected_root_entries {
        let root_path = instance_root.join(entry_name);
        if !root_path.exists() || *entry_name == CONTENT_MANIFEST_FILE_NAME {
            continue;
        }

        let mut files = Vec::new();
        if root_path.is_file() {
            files.push(root_path);
        } else if root_path.is_dir() {
            collect_regular_files_recursive(root_path.as_path(), &mut files).map_err(|err| {
                format!(
                    "failed to collect files under {}: {err}",
                    root_path.display()
                )
            })?;
        }

        let is_mods_dir = *entry_name == "mods";

        for file in files {
            let relative = file.strip_prefix(instance_root).unwrap_or(file.as_path());
            let archive_path = instance_relative_to_archive_path(relative);
            if downloadable_archive_paths.contains(&archive_path) {
                continue;
            }

            let snapshot_entry = if is_mods_dir {
                // Hash-only for unresolved local mod jars — avoids keeping all JARs in memory.
                let sha512 = modrinth::hash_file_sha512_hex(file.as_path())
                    .map_err(|err| format!("failed to hash {}: {err}", file.display()))?;
                SnapshotEntry {
                    bytes: Vec::new(),
                    sha512: Some(sha512),
                    source_path: Some(file),
                    downloadable_entry: None,
                }
            } else {
                let bytes = fs::read(file.as_path())
                    .map_err(|err| format!("failed to read {}: {err}", file.display()))?;
                SnapshotEntry {
                    bytes,
                    sha512: None,
                    source_path: Some(file),
                    downloadable_entry: None,
                }
            };

            entries.insert(archive_path, snapshot_entry);
        }
    }

    Ok(entries)
}

fn sanitize_patch_manifest_for_export(
    manifest: &ContentInstallManifest,
    options: &VtmpackExportOptions,
) -> ContentInstallManifest {
    let mut sanitized = manifest.clone();
    sanitized.projects.retain(|_, project| {
        if project.selected_source == Some(ManagedContentSource::CurseForge)
            && options.provider_mode == VtmpackProviderMode::ExcludeCurseForge
        {
            // CurseForge entries have already had a Modrinth rediscovery attempt
            // by this point. If they are still CurseForge and the export excludes
            // CurseForge, do not keep provider metadata for them; the filesystem
            // scan will treat their files as unknown local mods and bundle changed
            // or added ones instead.
            return false;
        }
        true
    });

    for project in sanitized.projects.values_mut() {
        if options.provider_mode == VtmpackProviderMode::ExcludeCurseForge {
            project.curseforge_project_id = None;
        }
    }
    sanitized
}

fn downloadable_needs_bundle_fallback(entry: &VtmpackDownloadableEntry) -> bool {
    let selected_source = entry.selected_source.as_deref().unwrap_or_default();
    if selected_source.eq_ignore_ascii_case("CurseForge") {
        // If CurseForge metadata is allowed, unresolved CurseForge entries should
        // remain CurseForge entries instead of being duplicated as bundled files.
        return false;
    }

    entry
        .selected_version_id
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
        || entry
            .selected_file_sha512
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
}

fn downloadable_entries_from_manifest(
    manifest: &ContentInstallManifest,
    options: &VtmpackExportOptions,
) -> Vec<VtmpackDownloadableEntry> {
    manifest
        .projects
        .iter()
        .filter_map(|(project_key, project)| {
            let source = project.selected_source?;
            if source == ManagedContentSource::CurseForge
                && options.provider_mode == VtmpackProviderMode::ExcludeCurseForge
            {
                return None;
            }
            if !matches!(
                source,
                ManagedContentSource::Modrinth | ManagedContentSource::CurseForge
            ) {
                return None;
            }
            let normalized_path = crate::export::normalize_pack_path(project.file_path.as_path());
            if normalized_path.as_os_str().is_empty() {
                return None;
            }
            Some(VtmpackDownloadableEntry {
                project_key: if project.project_key.trim().is_empty() {
                    project_key.clone()
                } else {
                    project.project_key.clone()
                },
                name: project.name.clone(),
                file_path: normalized_path,
                modrinth_project_id: project.modrinth_project_id.clone(),
                curseforge_project_id: project.curseforge_project_id,
                selected_source: Some(source.label().to_owned()),
                selected_version_id: project.selected_version_id.clone(),
                selected_version_name: project.selected_version_name.clone(),
                selected_file_sha1: project.selected_file_sha1.clone(),
                selected_file_sha512: project.selected_file_sha512.clone(),
            })
        })
        .collect()
}

// ── Modrinth rediscovery ──────────────────────────────────────────────────────

/// Given the set of mod-file archive paths that differ between base and current,
/// batch-query Modrinth by SHA-512.  Returns:
///
/// * `downloadable_entries` — mods successfully resolved; store metadata only.
/// * `bundled_paths`        — mods that couldn't be resolved; must be embedded.
fn rediscover_changed_mods(
    changed_mod_paths: &[PathBuf],
    current_entries: &BTreeMap<PathBuf, SnapshotEntry>,
) -> (Vec<VtmpackDownloadableEntry>, Vec<PathBuf>) {
    if changed_mod_paths.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // Collect sha512 → (archive_path, source_path) for all changed mod files.
    let mut hash_to_path: HashMap<String, PathBuf> = HashMap::new();
    let mut path_to_sha512: HashMap<PathBuf, String> = HashMap::new();
    let mut path_to_sha1: HashMap<PathBuf, String> = HashMap::new();

    for archive_path in changed_mod_paths {
        let Some(entry) = current_entries.get(archive_path) else {
            continue;
        };
        // We need both sha512 (for Modrinth lookup) and sha1.  The snapshot
        // already computed sha512; we may need to re-hash for sha1.
        let source = match entry.source_path.as_ref() {
            Some(p) => p,
            None => continue,
        };

        let (sha1, sha512) = match modrinth::hash_file_sha1_and_sha512_hex(source.as_path()) {
            Ok(hashes) => hashes,
            Err(err) => {
                tracing::warn!(
                    target: LOG,
                    path = %source.display(),
                    error = %err,
                    "failed to hash mod file for Modrinth rediscovery — will be bundled instead"
                );
                continue;
            }
        };
        tracing::trace!(
            target: LOG,
            archive_path = %archive_path.display(),
            sha512 = &sha512[..12],
            "hashed mod for Modrinth lookup"
        );

        hash_to_path.insert(sha512.clone(), archive_path.clone());
        path_to_sha512.insert(archive_path.clone(), sha512);
        path_to_sha1.insert(archive_path.clone(), sha1);
    }

    if hash_to_path.is_empty() {
        tracing::debug!(
            target: LOG,
            "no hashable unmanaged mods to rediscover — all will be bundled"
        );
        return (Vec::new(), changed_mod_paths.to_vec());
    }

    // Batch query Modrinth.
    tracing::info!(
        target: LOG,
        count = hash_to_path.len(),
        "querying Modrinth /version_files for unmanaged changed mods"
    );
    let client = modrinth::Client::default();
    let sha512_hashes: Vec<String> = hash_to_path.keys().cloned().collect();
    let version_matches = match client.get_versions_from_hashes(&sha512_hashes, "sha512") {
        Ok(m) => {
            tracing::info!(
                target: LOG,
                queried = sha512_hashes.len(),
                matched = m.len(),
                "Modrinth hash lookup returned matches"
            );
            m
        }
        Err(err) => {
            tracing::warn!(
                target: LOG,
                error = %err,
                "Modrinth hash lookup failed — all unmanaged changed mods will be bundled"
            );
            return (Vec::new(), changed_mod_paths.to_vec());
        }
    };

    // Fetch project metadata so we can give each entry a display name.
    let project_ids: Vec<String> = version_matches
        .values()
        .map(|v| v.project_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let projects_by_id: HashMap<String, modrinth::Project> = client
        .get_projects(&project_ids)
        .map(|projects| {
            projects
                .into_iter()
                .map(|p| (p.project_id.clone(), p))
                .collect()
        })
        .unwrap_or_else(|err| {
            tracing::warn!(
                target: LOG,
                error = %err,
                "failed to fetch Modrinth project metadata during rediscovery — names may be missing"
            );
            HashMap::new()
        });

    let mut downloadable: Vec<VtmpackDownloadableEntry> = Vec::new();
    let mut resolved_paths: BTreeSet<PathBuf> = BTreeSet::new();

    for (sha512, archive_path) in &hash_to_path {
        let Some(version) = version_matches.get(sha512.as_str()) else {
            continue;
        };

        // Verify the returned version actually contains our exact file.
        if !version.files.iter().any(|f| {
            f.hashes
                .get("sha512")
                .is_some_and(|h| h.eq_ignore_ascii_case(sha512))
        }) {
            tracing::warn!(
                target: LOG,
                archive_path = %archive_path.display(),
                version_id = %version.id,
                "Modrinth returned a version without the matched file hash; bundling"
            );
            continue;
        }

        // Map archive path (bundled_mods/foo.jar) back to instance path (mods/foo.jar).
        let instance_file_path = archive_path_to_mod_instance_path(archive_path);

        let name = projects_by_id
            .get(version.project_id.as_str())
            .map(|p| p.title.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| {
                archive_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Unknown mod".to_owned())
            });

        downloadable.push(VtmpackDownloadableEntry {
            project_key: format!("modrinth:{}", version.project_id),
            name,
            file_path: instance_file_path,
            modrinth_project_id: Some(version.project_id.clone()),
            curseforge_project_id: None,
            selected_source: Some("Modrinth".to_owned()),
            selected_version_id: Some(version.id.clone()),
            selected_version_name: non_empty(version.version_number.as_str()),
            selected_file_sha1: path_to_sha1.get(archive_path).cloned(),
            selected_file_sha512: Some(sha512.clone()),
        });

        resolved_paths.insert(archive_path.clone());
    }

    let bundled: Vec<PathBuf> = changed_mod_paths
        .iter()
        .filter(|p| !resolved_paths.contains(*p))
        .cloned()
        .collect();

    (downloadable, bundled)
}

/// Convert an archive mod path (`bundled_mods/sodium.jar`) to the
/// instance-relative path the mod lives at (`mods/sodium.jar`).
fn archive_path_to_mod_instance_path(archive_path: &Path) -> PathBuf {
    let s = path_to_string(archive_path);
    if let Some(rel) = s.strip_prefix("bundled_mods/") {
        PathBuf::from(format!("mods/{rel}"))
    } else {
        archive_path.to_path_buf()
    }
}

// ── Apply helpers ─────────────────────────────────────────────────────────────

fn download_downloadable_entry(
    entry: &VtmpackDownloadableEntry,
    instance_root: &Path,
) -> Result<(), String> {
    if entry
        .selected_source
        .as_deref()
        .is_some_and(|source| source.eq_ignore_ascii_case("CurseForge"))
        || entry.curseforge_project_id.is_some() && entry.modrinth_project_id.is_none()
    {
        return download_curseforge_entry(entry, instance_root);
    }
    download_modrinth_entry(entry, instance_root)
}

/// Download a single Modrinth-tracked file and write it to the instance.
fn download_modrinth_entry(
    entry: &VtmpackDownloadableEntry,
    instance_root: &Path,
) -> Result<(), String> {
    let client = modrinth::Client::default();
    let version_id = entry
        .selected_version_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "downloadable entry {} has no version_id",
                entry.file_path.display()
            )
        })?;

    let version = client
        .get_version(version_id)
        .map_err(|err| format!("failed to fetch Modrinth version {version_id}: {err}"))?;

    // Prefer the file whose sha512 matches the manifest; fall back to primary.
    let file = version
        .files
        .iter()
        .find(|f| {
            entry
                .selected_file_sha512
                .as_deref()
                .is_some_and(|expected| {
                    f.hashes
                        .get("sha512")
                        .is_some_and(|h| h.eq_ignore_ascii_case(expected))
                })
        })
        .or_else(|| version.files.iter().find(|f| f.primary))
        .or_else(|| version.files.first())
        .ok_or_else(|| format!("no downloadable files for Modrinth version {version_id}"))?;

    let mut response = ureq::get(&file.url)
        .call()
        .map_err(|err| format!("HTTP request to {} failed: {err}", file.url))?;
    let mut reader = response.body_mut().as_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read response body from {}: {err}", file.url))?;

    let destination = instance_root.join(entry.file_path.as_path());
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(&destination, &bytes)
        .map_err(|err| format!("failed to write {}: {err}", destination.display()))?;

    // Verify the hash if we have it.
    if let Some(expected_sha512) = &entry.selected_file_sha512 {
        let actual = modrinth::hash_file_sha512_hex(destination.as_path()).map_err(|err| {
            format!(
                "failed to hash downloaded file {}: {err}",
                destination.display()
            )
        })?;
        if !actual.eq_ignore_ascii_case(expected_sha512) {
            let _ = fs::remove_file(destination.as_path());
            return Err(format!(
                "SHA-512 mismatch for {} (expected {expected_sha512}, got {actual})",
                file.filename
            ));
        }
    }

    Ok(())
}

fn download_curseforge_entry(
    entry: &VtmpackDownloadableEntry,
    instance_root: &Path,
) -> Result<(), String> {
    let project_id = entry.curseforge_project_id.ok_or_else(|| {
        format!(
            "downloadable entry {} has no CurseForge project id",
            entry.file_path.display()
        )
    })?;
    let file_id = entry
        .selected_version_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "downloadable entry {} has no CurseForge file id",
                entry.file_path.display()
            )
        })?
        .parse::<u64>()
        .map_err(|err| format!("invalid CurseForge file id for {}: {err}", entry.name))?;
    let client = curseforge::Client::from_env().ok_or_else(|| {
        "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to apply this patch."
            .to_owned()
    })?;
    let download_url = client
        .get_mod_file_download_url(project_id, file_id)
        .map_err(|err| {
            format!("failed to resolve CurseForge download URL for {project_id}/{file_id}: {err}")
        })?
        .ok_or_else(|| {
            format!("CurseForge file {file_id} for project {project_id} has no download URL")
        })?;
    let mut response = ureq::get(download_url.as_str())
        .call()
        .map_err(|err| format!("HTTP request to {download_url} failed: {err}"))?;
    let mut reader = response.body_mut().as_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read response body from {download_url}: {err}"))?;
    let destination = instance_root.join(entry.file_path.as_path());
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(&destination, &bytes)
        .map_err(|err| format!("failed to write {}: {err}", destination.display()))
}

// ── Archive helpers ───────────────────────────────────────────────────────────

fn open_patch_archive(path: &Path) -> Result<tar::Archive<Box<dyn Read>>, String> {
    let bytes =
        fs::read(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let decoder = xz2::read::XzDecoder::new(Cursor::new(bytes));
    Ok(tar::Archive::new(Box::new(decoder)))
}

fn archive_path_to_instance_path(
    instance_root: &Path,
    archive_path: &Path,
) -> Result<PathBuf, String> {
    let archive_name = path_to_string(archive_path);
    if archive_name == PATCH_MANIFEST_PATH || archive_name == "manifest.toml" {
        return Err(format!(
            "patch entry {archive_name} is not an instance file"
        ));
    }
    if archive_name == CONTENT_MANIFEST_ARCHIVE_PATH {
        return Ok(instance_root.join(CONTENT_MANIFEST_FILE_NAME));
    }
    if let Some(relative) = archive_name.strip_prefix("bundled_mods/") {
        return join_safe(&instance_root.join("mods"), Path::new(relative));
    }
    if let Some(relative) = archive_name.strip_prefix("configs/") {
        return join_safe(&instance_root.join("config"), Path::new(relative));
    }
    if let Some(relative) = archive_name.strip_prefix("root_entries/") {
        return join_safe(instance_root, Path::new(relative));
    }
    Err(format!("unsupported patch entry path: {archive_name}"))
}

fn join_safe(base: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut out = base.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "unsafe patch path component in {}",
                    relative.display()
                ));
            }
        }
    }
    Ok(out)
}

fn instance_relative_to_archive_path(path: &Path) -> PathBuf {
    let normalized = normalize_pack_path(path);
    let text = path_to_string(&normalized);
    if let Some(relative) = text.strip_prefix("mods/") {
        Path::new("bundled_mods").join(relative)
    } else if let Some(relative) = text.strip_prefix("config/") {
        Path::new("configs").join(relative)
    } else {
        Path::new("root_entries").join(normalized)
    }
}

fn collect_regular_files_recursive(root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_regular_files_recursive(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn append_bytes_to_archive<W: Write>(
    archive: &mut tar::Builder<W>,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, path, Cursor::new(bytes))
        .map_err(|err| format!("failed to append {path} to archive: {err}"))
}

fn xz_preset_for_compression_mode(mode: VtmpackCompressionMode) -> u32 {
    match mode {
        VtmpackCompressionMode::Standard => XZ_PRESET_STANDARD,
        VtmpackCompressionMode::Extreme => XZ_PRESET_EXTREME,
    }
}

// ── Misc helpers ──────────────────────────────────────────────────────────────

fn progress_update(
    message: &str,
    completed_steps: usize,
    total_steps: usize,
) -> VtmpackExportProgress {
    VtmpackExportProgress {
        message: message.to_owned(),
        completed_steps,
        total_steps,
    }
}

fn path_to_archive_name(path: &Path) -> Result<String, String> {
    let value = path_to_string(path);
    if value.trim().is_empty() {
        Err("empty archive path".to_owned())
    } else {
        Ok(value)
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn hash_bytes_sha512_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha512::new();
    Sha2Digest::update(&mut hasher, bytes);
    bytes_to_lower_hex(Sha2Digest::finalize(hasher).as_slice())
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn normalize_pack_path(path: &Path) -> PathBuf {
    let normalized = path
        .to_string_lossy()
        .trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/");
    if normalized.is_empty() {
        PathBuf::new()
    } else {
        PathBuf::from(normalized)
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_owned())
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

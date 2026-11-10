use vertex_i18n::tr;
use super::*;

// ── Snapshot types ────────────────────────────────────────────────────────────

/// One entry in the in-memory snapshot used for diffing.
///
/// For files that are just compared by content we store `bytes`.
/// For mod files (which can be large and are better compared by hash) we
/// store `sha512` instead.  The two are mutually exclusive in practice:
/// a file with `sha512` set will have `bytes` empty, and vice-versa.
#[derive(Debug, Clone, Default)]
pub(super) struct SnapshotEntry {
    /// Raw content (configs, manifests, non-mod root-entries).
    pub(super) bytes: Vec<u8>,
    /// SHA-512 hex digest (mod jars — avoids loading MB of bytes for comparison).
    pub(super) sha512: Option<String>,
    /// Original on-disk path (only set when we still need to read the file
    /// for archiving; absent for downloadable-entry sentinels).
    pub(super) source_path: Option<PathBuf>,
    /// Download metadata for files that `.vtmpack` would shrink by storing as
    /// downloadable content instead of bundling bytes.
    pub(super) downloadable_entry: Option<VtmpackDownloadableEntry>,
}

impl SnapshotEntry {
    pub(super) fn differs_from(&self, other: &SnapshotEntry) -> bool {
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

pub(super) fn downloadable_entries_differ(
    a: &VtmpackDownloadableEntry,
    b: &VtmpackDownloadableEntry,
) -> bool {
    a.file_path != b.file_path
        || a.modrinth_project_id != b.modrinth_project_id
        || a.curseforge_project_id != b.curseforge_project_id
        || a.selected_source != b.selected_source
        || a.selected_version_id != b.selected_version_id
        || a.selected_version_name != b.selected_version_name
        || a.selected_file_sha1 != b.selected_file_sha1
        || a.selected_file_sha512 != b.selected_file_sha512
}

// ── Snapshot building ─────────────────────────────────────────────────────────

/// Read the base vtmpack into a snapshot map.
///
/// Bundled files are stored with their raw bytes for exact comparison.
/// Downloadable entries are stored as hash-only sentinels — comparing them
/// against the current instance by hash avoids always treating every managed
/// mod as "changed" (the bug that made old patches huge).
pub(super) fn read_base_snapshot(
    path: &Path,
) -> Result<(VtmpackManifest, BTreeMap<PathBuf, SnapshotEntry>), String> {
    let mut archive = crate::open_vtmpack_tar_archive(path)?;
    let mut manifest: Option<VtmpackManifest> = None;
    let mut entries: BTreeMap<PathBuf, SnapshotEntry> = BTreeMap::new();

    for entry in archive
        .entries()
        .map_err(|err| tr!("failed to read {path}: {err}", path = path.display(), err = err).into_owned())?
    {
        let mut entry = entry.map_err(|err| tr!("failed to read archive entry: {err}", err = err).into_owned())?;
        let archive_path = entry
            .path()
            .map_err(|err| tr!("failed to decode archive path: {err}", err = err).into_owned())?
            .to_path_buf();
        let archive_name = path_to_string(&archive_path);
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|err| tr!("failed to read {archive_name}: {err}", archive_name = archive_name, err = err).into_owned())?;

        if archive_name == "manifest.toml" {
            manifest = Some(
                toml::from_str::<VtmpackManifest>(
                    std::str::from_utf8(&bytes)
                        .map_err(|err| tr!("base manifest.toml is not valid UTF-8: {err}", err = err).into_owned())?,
                )
                .map_err(|err| tr!("failed to parse base manifest.toml: {err}", err = err).into_owned())?,
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
        .ok_or_else(|| tr!("No manifest.toml found in Vertex pack {path}", path = path.display()).into_owned())?;

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
pub(super) fn build_current_snapshot(
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
            .map_err(|err| tr!("failed to serialize patch content manifest: {err}", err = err).into_owned())?
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
            .map_err(|err| tr!("failed to hash {source_path}: {err}", source_path = source_path.display(), err = err).into_owned())?;
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
                tr!("failed to collect files under {root_path}: {err}", root_path = root_path.display(), err = err).into_owned()
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
                    .map_err(|err| tr!("failed to hash {file}: {err}", file = file.display(), err = err).into_owned())?;
                SnapshotEntry {
                    bytes: Vec::new(),
                    sha512: Some(sha512),
                    source_path: Some(file),
                    downloadable_entry: None,
                }
            } else {
                let bytes = fs::read(file.as_path())
                    .map_err(|err| tr!("failed to read {file}: {err}", file = file.display(), err = err).into_owned())?;
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

pub(super) fn sanitize_patch_manifest_for_export(
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

pub(super) fn downloadable_needs_bundle_fallback(entry: &VtmpackDownloadableEntry) -> bool {
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

pub(super) fn downloadable_entries_from_manifest(
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
            let normalized_path = normalize_pack_path(project.file_path.as_path());
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
pub(super) fn rediscover_changed_mods(
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
                    .unwrap_or_else(|| tr!("Unknown mod").into_owned())
            });

        downloadable.push(VtmpackDownloadableEntry {
            project_key: format!("modrinth:{}", version.project_id),
            name,
            file_path: instance_file_path,
            modrinth_project_id: Some(version.project_id.clone()),
            curseforge_project_id: None,
            selected_source: Some(tr!("Modrinth").into_owned()),
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
pub(super) fn archive_path_to_mod_instance_path(archive_path: &Path) -> PathBuf {
    let s = path_to_string(archive_path);
    if let Some(rel) = s.strip_prefix("bundled_mods/") {
        PathBuf::from(format!("mods/{rel}"))
    } else {
        archive_path.to_path_buf()
    }
}

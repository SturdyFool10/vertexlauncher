use vertex_i18n::tr;
use super::*;

// ── Apply helpers ─────────────────────────────────────────────────────────────

/// Where one entry's file comes from and goes to, worked out before anything is downloaded.
struct ResolvedFile {
    url: String,
    destination: PathBuf,
    /// Checked after the download when the pack recorded it.
    expected_sha512: Option<String>,
    display_name: String,
}

fn is_curseforge_entry(entry: &VtmpackDownloadableEntry) -> bool {
    entry
        .selected_source
        .as_deref()
        .is_some_and(|source| source.eq_ignore_ascii_case("CurseForge"))
        || entry.curseforge_project_id.is_some() && entry.modrinth_project_id.is_none()
}

/// Downloads every entry together: the download URLs are resolved first (Modrinth versions in one batched request,
/// CurseForge ones in parallel), then all files are fetched concurrently under the user's download limits. Returns one
/// result per entry, in the order given, so a file that fails does not stop the others.
pub(super) fn download_downloadable_entries(
    entries: &[&VtmpackDownloadableEntry],
    instance_root: &Path,
) -> Vec<Result<(), String>> {
    let policy = installation::shared_download_policy();
    let workers = policy.max_concurrent_downloads.clamp(1, 64) as usize;
    let mut resolved: Vec<Option<Result<ResolvedFile, String>>> = entries.iter().map(|_| None).collect();

    // Modrinth: one request for all the versions instead of one each.
    let modrinth_indices: Vec<usize> = (0..entries.len()).filter(|&i| !is_curseforge_entry(entries[i])).collect();
    let version_ids: Vec<String> = modrinth_indices
        .iter()
        .filter_map(|&i| entries[i].selected_version_id.as_deref())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    let versions: HashMap<String, modrinth::ProjectVersion> = modrinth::Client::default()
        .get_versions(&version_ids)
        .map(|versions| versions.into_iter().map(|version| (version.id.clone(), version)).collect())
        .unwrap_or_default();
    for &index in &modrinth_indices {
        resolved[index] = Some(resolve_modrinth_entry(entries[index], &versions, instance_root));
    }

    // CurseForge: each URL is its own request, so they are resolved by several workers at once.
    let curseforge_indices: Vec<usize> = (0..entries.len()).filter(|&i| is_curseforge_entry(entries[i])).collect();
    if !curseforge_indices.is_empty() {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..workers.min(curseforge_indices.len()) {
                scope.spawn(|| {
                    loop {
                        let slot = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&index) = curseforge_indices.get(slot) else { return };
                        let result = resolve_curseforge_entry(entries[index], instance_root);
                        results.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push((index, result));
                    }
                });
            }
        });
        for (index, result) in results.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner()) {
            resolved[index] = Some(result);
        }
    }

    // Everything that resolved goes in one batch.
    let mut results: Vec<Result<(), String>> = Vec::with_capacity(entries.len());
    let mut to_download: Vec<(usize, &ResolvedFile)> = Vec::new();
    for (index, slot) in resolved.iter().enumerate() {
        match slot {
            Some(Ok(file)) => {
                if let Some(parent) = file.destination.parent()
                    && let Err(err) = fs::create_dir_all(parent)
                {
                    results.push(Err(tr!("failed to create {parent}: {err}", parent = parent.display(), err = err).into_owned()));
                    continue;
                }
                to_download.push((index, file));
                results.push(Ok(()));
            }
            Some(Err(err)) => results.push(Err(err.clone())),
            None => results.push(Err(tr!("entry was not resolved").into_owned())),
        }
    }
    let task_for = |file: &ResolvedFile| installation::DownloadBatchTask {
        url: file.url.clone(),
        destination: file.destination.clone(),
        expected_size: None,
    };
    if !to_download.is_empty() {
        let batch: Vec<_> = to_download.iter().map(|(_, file)| task_for(file)).collect();
        if let Err(batch_err) = installation::download_batch(batch, &policy) {
            // One bad file must not sink the rest: retry each on its own (several at a time) so only the files that
            // really cannot be fetched are reported.
            tracing::warn!(
                target: "vertexlauncher/vtmpatch",
                error = %batch_err,
                "downloading the patch files together failed; retrying them one by one"
            );
            let next = std::sync::atomic::AtomicUsize::new(0);
            let failures = std::sync::Mutex::new(Vec::new());
            std::thread::scope(|scope| {
                for _ in 0..workers.min(to_download.len()) {
                    scope.spawn(|| {
                        loop {
                            let slot = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some((index, file)) = to_download.get(slot) else { return };
                            if let Err(err) = installation::download_batch(vec![task_for(file)], &policy) {
                                failures
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .push((*index, tr!("download failed for {url}: {err}", url = file.url, err = err).into_owned()));
                            }
                        }
                    });
                }
            });
            for (index, err) in failures.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner()) {
                results[index] = Err(err);
            }
        }
    }

    // Hash check for the files that arrived.
    for (index, file) in &to_download {
        if results[*index].is_err() {
            continue;
        }
        let Some(expected) = file.expected_sha512.as_deref() else { continue };
        match modrinth::hash_file_sha512_hex(file.destination.as_path()) {
            Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
            Ok(actual) => {
                let _ = fs::remove_file(file.destination.as_path());
                results[*index] = Err(tr!("SHA-512 mismatch for {display_name} (expected {expected}, got {actual})", display_name = file.display_name, expected = expected, actual = actual).into_owned());
            }
            Err(err) => {
                results[*index] = Err(tr!("failed to hash downloaded file {destination}: {err}", destination = file.destination.display(), err = err).into_owned());
            }
        }
    }
    results
}

fn resolve_modrinth_entry(
    entry: &VtmpackDownloadableEntry,
    versions: &HashMap<String, modrinth::ProjectVersion>,
    instance_root: &Path,
) -> Result<ResolvedFile, String> {
    let version_id = entry
        .selected_version_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            tr!("downloadable entry {file_path} has no version_id", file_path = entry.file_path.display()).into_owned()
        })?;
    let version = versions
        .get(version_id)
        .ok_or_else(|| tr!("failed to fetch Modrinth version {version_id}", version_id = version_id).into_owned())?;

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
        .ok_or_else(|| tr!("no downloadable files for Modrinth version {version_id}", version_id = version_id).into_owned())?;
    Ok(ResolvedFile {
        url: file.url.clone(),
        destination: instance_root.join(entry.file_path.as_path()),
        expected_sha512: entry.selected_file_sha512.clone(),
        display_name: file.filename.clone(),
    })
}

fn resolve_curseforge_entry(
    entry: &VtmpackDownloadableEntry,
    instance_root: &Path,
) -> Result<ResolvedFile, String> {
    let project_id = entry.curseforge_project_id.ok_or_else(|| {
        tr!("downloadable entry {file_path} has no CurseForge project id", file_path = entry.file_path.display()).into_owned()
    })?;
    let file_id = entry
        .selected_version_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            tr!("downloadable entry {file_path} has no CurseForge file id", file_path = entry.file_path.display()).into_owned()
        })?
        .parse::<u64>()
        .map_err(|err| tr!("invalid CurseForge file id for {name}: {err}", name = entry.name, err = err).into_owned())?;
    let client = curseforge::Client::from_env().ok_or_else(|| {
        "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to apply this patch."
            .to_owned()
    })?;
    let download_url = client
        .get_mod_file_download_url(project_id, file_id)
        .map_err(|err| {
            tr!("failed to resolve CurseForge download URL for {project_id}/{file_id}: {err}", project_id = project_id, file_id = file_id, err = err).into_owned()
        })?
        .ok_or_else(|| {
            tr!("CurseForge file {file_id} for project {project_id} has no download URL", file_id = file_id, project_id = project_id).into_owned()
        })?;
    Ok(ResolvedFile {
        url: download_url,
        destination: instance_root.join(entry.file_path.as_path()),
        expected_sha512: None,
        display_name: entry.name.clone(),
    })
}

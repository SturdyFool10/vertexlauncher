use vertex_i18n::tr;
use super::*;

fn should_preflight_curseforge_manual_downloads(
    request: &import_instance_modal::ImportRequest,
) -> bool {
    let import_instance_modal::ImportSource::ManifestFile(path) = &request.source else {
        return false;
    };
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

pub(super) fn cleanup_import_request_manual_staging(
    request: Option<&import_instance_modal::ImportRequest>,
) {
    let Some(request) = request else {
        return;
    };
    if let Some(staging_dir) = request.manual_curseforge_staging_dir.clone() {
        remove_staging_dir_in_background(staging_dir, "cleanup import manual CurseForge staging");
    }
}

#[derive(Debug)]
pub(super) enum ManualDownloadContinuation {
    Import(import_instance_modal::ImportRequest),
    DiscoverInstall(screens::DiscoverInstallRequest),
}

#[derive(Debug)]
pub(super) struct PendingCurseForgeManualDownloadState {
    pub(super) continuation: ManualDownloadContinuation,
    pub(super) downloads_dir: PathBuf,
    pub(super) staging_dir: PathBuf,
    pub(super) pending_files: Vec<import_instance_modal::CurseForgeManualDownloadRequirement>,
    pub(super) staged_files: HashMap<u64, PathBuf>,
    pub(super) last_scan_at: Instant,
    pub(super) error: Option<String>,
    /// Background scans report here so the downloads folder is never read on the UI thread.
    pub(super) scan_results: launcher_runtime::WorkerChannel<Result<Vec<(u64, PathBuf)>, String>>,
    pub(super) scan_in_flight: bool,
}

impl PendingCurseForgeManualDownloadState {
    pub(super) fn new(
        continuation: ManualDownloadContinuation,
        pending_files: Vec<import_instance_modal::CurseForgeManualDownloadRequirement>,
        initial_staged_files: HashMap<u64, PathBuf>,
        installations_root: &Path,
    ) -> Result<Self, String> {
        let downloads_dir = default_downloads_dir(installations_root);
        let staging_dir = std::env::temp_dir().join(format!(
            "vertexlauncher-cf-manual-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        fs::create_dir_all(staging_dir.as_path()).map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %staging_dir.display(), error = %err, context = "create manual CurseForge staging");
            tr!("failed to create staging directory: {err}", err = err).into_owned()
        })?;
        let mut staged_files = HashMap::new();
        for (file_id, source_path) in initial_staged_files {
            let file_name = source_path
                .file_name()
                .ok_or_else(|| {
                    tr!("staged CurseForge retry file {source_path} had no file name", source_path = source_path.display()).into_owned()
                })?
                .to_owned();
            let destination = staging_dir.join(file_name);
            if source_path != destination {
                fs::copy(source_path.as_path(), destination.as_path()).map_err(|err| {
                    tracing::warn!(target: "vertexlauncher/io", op = "copy", from = %source_path.display(), to = %destination.display(), error = %err, context = "stage CurseForge retry file");
                    tr!("failed to copy staged CurseForge retry file {source_path} into {destination}: {err}", source_path = source_path.display(), destination = destination.display(), err = err).into_owned()
                })?;
            }
            staged_files.insert(file_id, destination);
        }
        Ok(Self {
            continuation,
            downloads_dir,
            staging_dir,
            pending_files,
            staged_files,
            last_scan_at: Instant::now() - Duration::from_secs(1),
            error: None,
            scan_results: Default::default(),
            scan_in_flight: false,
        })
    }
}

/// Collects finished downloads-folder scans and starts the next one. Returns true once every required file is
/// staged and the install/import can continue.
pub(super) fn step_pending_curseforge_manual_download(
    state: &mut PendingCurseForgeManualDownloadState,
) -> bool {
    let mut should_resume = false;
    let drained = state.scan_results.drain();
    if drained.disconnected {
        state.scan_results.reset();
        state.scan_in_flight = false;
    }
    for result in drained.items {
        state.scan_in_flight = false;
        match result {
            Ok(found) => {
                apply_found_manual_downloads(state, found);
                if state.pending_files.is_empty() {
                    should_resume = true;
                }
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/app/import",
                    error = %err,
                    remaining_files = state.pending_files.len(),
                    "Pending manual CurseForge download scan failed."
                );
                state.error = Some(err);
            }
        }
    }
    if !should_resume
        && !state.scan_in_flight
        && state.last_scan_at.elapsed() >= Duration::from_millis(400)
    {
        // Reading the downloads folder and copying matches can be slow; do it off-thread.
        state.last_scan_at = Instant::now();
        state.scan_in_flight = true;
        let tx = state.scan_results.sender();
        let downloads_dir = state.downloads_dir.clone();
        let staging_dir = state.staging_dir.clone();
        let pending_files = state.pending_files.clone();
        tokio_runtime::spawn_blocking_detached(move || {
            let _ = tx.send(find_and_stage_manual_downloads(
                downloads_dir.as_path(),
                staging_dir.as_path(),
                &pending_files,
            ));
        });
    }
    should_resume
}

/// One-shot synchronous scan, used while a request is being set up.
pub(super) fn scan_curseforge_manual_downloads(
    state: &mut PendingCurseForgeManualDownloadState,
) -> Result<(), String> {
    let found = find_and_stage_manual_downloads(
        state.downloads_dir.as_path(),
        state.staging_dir.as_path(),
        &state.pending_files,
    )?;
    apply_found_manual_downloads(state, found);
    Ok(())
}

/// Looks in `downloads_dir` for the requested files and copies matches into `staging_dir`.
/// Pure filesystem work with no shared state, so it can run on a blocking thread.
pub(super) fn find_and_stage_manual_downloads(
    downloads_dir: &Path,
    staging_dir: &Path,
    pending_files: &[import_instance_modal::CurseForgeManualDownloadRequirement],
) -> Result<Vec<(u64, PathBuf)>, String> {
    let entries = fs::read_dir(downloads_dir)
        .map_err(|err| tr!("failed to read downloads folder: {err}", err = err).into_owned())?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| tr!("failed to inspect downloads folder: {err}", err = err).into_owned())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        candidates.push((file_name.to_owned(), path));
    }

    let mut found = Vec::new();
    for requirement in pending_files {
        let Some((_, source_path)) = candidates.iter().find(|(candidate_name, _)| {
            downloaded_filename_matches(candidate_name.as_str(), requirement.file_name.as_str())
        }) else {
            continue;
        };
        let staged_path = staging_dir.join(requirement.file_name.as_str());
        fs::copy(source_path, staged_path.as_path()).map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "copy", from = %source_path.display(), to = %staged_path.display(), error = %err, context = "stage manual CurseForge file");
            tr!("failed to stage {file_name} from downloads folder: {err}", file_name = requirement.file_name, err = err).into_owned()
        })?;
        found.push((requirement.file_id, staged_path));
    }
    Ok(found)
}

pub(super) fn apply_found_manual_downloads(
    state: &mut PendingCurseForgeManualDownloadState,
    found: Vec<(u64, PathBuf)>,
) {
    if found.is_empty() {
        return;
    }
    state.pending_files.retain(|requirement| {
        !found
            .iter()
            .any(|(file_id, _)| *file_id == requirement.file_id)
    });
    for (file_id, staged_path) in found {
        state.staged_files.insert(file_id, staged_path);
    }
    state.error = None;
}

pub(super) fn downloaded_filename_matches(candidate_name: &str, expected_name: &str) -> bool {
    if candidate_name == expected_name {
        return true;
    }
    let expected_path = Path::new(expected_name);
    let candidate_path = Path::new(candidate_name);
    let Some(expected_stem) = expected_path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    let Some(candidate_stem) = candidate_path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    if expected_path.extension() != candidate_path.extension() {
        return false;
    }
    let Some(suffix) = candidate_stem.strip_prefix(expected_stem) else {
        return false;
    };
    suffix.starts_with(" (")
        && suffix.ends_with(')')
        && suffix[2..suffix.len() - 1]
            .chars()
            .all(|ch| ch.is_ascii_digit())
}

pub(super) fn cleanup_pending_curseforge_manual_download(
    pending: Option<PendingCurseForgeManualDownloadState>,
) {
    let Some(pending) = pending else {
        return;
    };
    remove_staging_dir_in_background(pending.staging_dir, "cleanup manual CurseForge staging");
}

/// Deleting staged mod files can take a while; keep it off the UI thread.
pub(super) fn remove_staging_dir_in_background(staging_dir: PathBuf, context: &'static str) {
    tokio_runtime::spawn_blocking_detached(move || {
        if let Err(err) = fs::remove_dir_all(staging_dir.as_path()) {
            tracing::warn!(target: "vertexlauncher/io", op = "remove_dir_all", path = %staging_dir.display(), error = %err, context = context);
        }
    });
}

pub(super) fn default_downloads_dir(installations_root: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(profile_dir) = std::env::var_os("USERPROFILE") {
            let candidate = PathBuf::from(profile_dir).join("Downloads");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home_dir) = std::env::var_os("HOME") {
            let candidate = PathBuf::from(home_dir).join("Downloads");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    installations_root.to_path_buf()
}

pub(super) fn import_package_in_background(
    mut store: InstanceStore,
    installations_root: PathBuf,
    request: import_instance_modal::ImportRequest,
    progress_tx: mpsc::Sender<import_instance_modal::ImportProgress>,
) -> import_instance_modal::ImportTaskResult {
    let instance = import_instance_modal::import_package_with_progress(
        &mut store,
        installations_root.as_path(),
        request,
        |progress| {
            if let Err(err) = progress_tx.send(progress) {
                tracing::error!(
                    target: "vertexlauncher/app/import",
                    error = %err,
                    "Failed to deliver import progress update."
                );
            }
        },
    )?;
    Ok((store, instance))
}

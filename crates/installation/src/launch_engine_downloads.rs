use super::*;

pub(crate) fn download_version_dependencies(
    instance_root: &Path,
    version_json_path: &Path,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    if !version_json_path.exists() {
        return Ok(0);
    }
    tracing::info!(
        target: "vertexlauncher/installation/dependencies",
        instance_root = %instance_root.display(),
        version_json_path = %version_json_path.display(),
        "Starting version dependency resolution."
    );
    let raw = fs_read_to_string(version_json_path).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/dependencies",
            instance_root = %instance_root.display(),
            version_json_path = %version_json_path.display(),
            error = %err,
            "Failed to read version metadata JSON."
        );
        InstallationError::Io(err)
    })?;
    let version_meta: serde_json::Value = serde_json::from_str(&raw)?;
    let mut downloaded = 0u32;

    let mut library_tasks = Vec::new();
    collect_library_download_tasks(instance_root, &version_meta, &mut library_tasks);
    tracing::info!(
        target: "vertexlauncher/installation/dependencies",
        library_task_count = library_tasks.len(),
        "Collected library download tasks."
    );
    downloaded += download_files_concurrent(
        InstallStage::DownloadingCore,
        library_tasks,
        policy,
        downloaded_files_offset.saturating_add(downloaded),
        progress,
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/dependencies",
            instance_root = %instance_root.display(),
            version_json_path = %version_json_path.display(),
            error = %err,
            "Library dependency download batch failed."
        );
        err
    })?;

    let mut asset_index_task = Vec::new();
    let asset_index_path =
        collect_asset_index_download_task(instance_root, &version_meta, &mut asset_index_task);
    tracing::info!(
        target: "vertexlauncher/installation/dependencies",
        asset_index_task_count = asset_index_task.len(),
        asset_index_path = %asset_index_path.as_ref().map(|path| path.display().to_string()).unwrap_or_default(),
        "Collected asset index download task."
    );
    downloaded += download_files_concurrent(
        InstallStage::DownloadingCore,
        asset_index_task,
        policy,
        downloaded_files_offset.saturating_add(downloaded),
        progress,
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/dependencies",
            instance_root = %instance_root.display(),
            version_json_path = %version_json_path.display(),
            asset_index_path = %asset_index_path.as_ref().map(|path| path.display().to_string()).unwrap_or_default(),
            error = %err,
            "Asset index download batch failed."
        );
        err
    })?;

    if let Some(asset_index_path) = asset_index_path {
        let mut object_tasks = Vec::new();
        collect_asset_object_download_tasks(
            instance_root,
            asset_index_path.as_path(),
            &mut object_tasks,
        )
        .map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/dependencies",
                instance_root = %instance_root.display(),
                asset_index_path = %asset_index_path.display(),
                error = %err,
                "Failed while collecting asset object download tasks from asset index."
            );
            err
        })?;
        tracing::info!(
            target: "vertexlauncher/installation/dependencies",
            asset_index_path = %asset_index_path.display(),
            asset_object_task_count = object_tasks.len(),
            "Collected asset object download tasks."
        );
        downloaded += download_files_concurrent(
            InstallStage::DownloadingCore,
            object_tasks,
            policy,
            downloaded_files_offset.saturating_add(downloaded),
            progress,
        )
        .map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/dependencies",
                instance_root = %instance_root.display(),
                asset_index_path = %asset_index_path.display(),
                error = %err,
                "Asset object download batch failed."
            );
            err
        })?;
    }

    tracing::info!(
        target: "vertexlauncher/installation/dependencies",
        instance_root = %instance_root.display(),
        version_json_path = %version_json_path.display(),
        downloaded,
        "Completed version dependency resolution."
    );
    Ok(downloaded)
}

pub(crate) fn collect_library_download_tasks(
    instance_root: &Path,
    version_meta: &serde_json::Value,
    tasks: &mut Vec<FileDownloadTask>,
) {
    let Some(libraries) = version_meta
        .get("libraries")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for library in libraries {
        if let Some(downloads) = library.get("downloads") {
            if let Some(artifact) = downloads.get("artifact") {
                push_download_task_from_download_entry(
                    instance_root.join("libraries").as_path(),
                    artifact,
                    tasks,
                );
            }
            if let Some(classifiers) = downloads
                .get("classifiers")
                .and_then(serde_json::Value::as_object)
            {
                for entry in classifiers.values() {
                    push_download_task_from_download_entry(
                        instance_root.join("libraries").as_path(),
                        entry,
                        tasks,
                    );
                }
            }
        } else if let Some((url, relative_path)) = resolve_library_maven_download(library) {
            let destination = instance_root.join("libraries").join(relative_path.as_str());
            if !destination.exists() {
                tasks.push(FileDownloadTask {
                    url,
                    destination,
                    expected_size: library
                        .get("size")
                        .and_then(serde_json::Value::as_u64)
                        .filter(|size| *size > 0),
                });
            }
        }
    }
}

pub(crate) fn resolve_library_maven_download(
    library: &serde_json::Value,
) -> Option<(String, String)> {
    let name = library.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let mut parts = name.split(':');
    let group = parts.next()?.trim();
    let artifact = parts.next()?.trim();
    let version_and_ext = parts.next()?.trim();
    let classifier = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if group.is_empty() || artifact.is_empty() || version_and_ext.is_empty() {
        return None;
    }

    let (version, extension) = if let Some((version, ext)) = version_and_ext.split_once('@') {
        (version.trim(), ext.trim())
    } else {
        (version_and_ext, "jar")
    };
    if version.is_empty() || extension.is_empty() {
        return None;
    }

    let base_url = library
        .get("url")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("https://libraries.minecraft.net/");
    let group_path = group.replace('.', "/");
    let file_name = if let Some(classifier) = classifier {
        format!("{artifact}-{version}-{classifier}.{extension}")
    } else {
        format!("{artifact}-{version}.{extension}")
    };
    let relative_path = format!("{group_path}/{artifact}/{version}/{file_name}");
    let url = format!(
        "{}{relative_path}",
        base_url.trim_end_matches('/').to_owned() + "/"
    );
    Some((url, relative_path))
}

pub(crate) fn collect_asset_index_download_task(
    instance_root: &Path,
    version_meta: &serde_json::Value,
    tasks: &mut Vec<FileDownloadTask>,
) -> Option<PathBuf> {
    let asset_index = version_meta.get("assetIndex")?;
    let url = asset_index.get("url")?.as_str()?.trim();
    if url.is_empty() {
        return None;
    }
    let id = asset_index
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");
    let destination = instance_root
        .join("assets")
        .join("indexes")
        .join(format!("{id}.json"));
    if !destination.exists() {
        tasks.push(FileDownloadTask {
            url: url.to_owned(),
            destination: destination.clone(),
            expected_size: asset_index
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .filter(|size| *size > 0),
        });
    }
    Some(destination)
}

pub(crate) fn collect_asset_object_download_tasks(
    instance_root: &Path,
    asset_index_path: &Path,
    tasks: &mut Vec<FileDownloadTask>,
) -> Result<(), InstallationError> {
    if !asset_index_path.exists() {
        return Ok(());
    }
    let raw = fs_read_to_string(asset_index_path).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/dependencies",
            instance_root = %instance_root.display(),
            asset_index_path = %asset_index_path.display(),
            error = %err,
            "Failed to read asset index JSON."
        );
        InstallationError::Io(err)
    })?;
    let index: serde_json::Value = serde_json::from_str(&raw)?;
    let Some(objects) = index.get("objects").and_then(serde_json::Value::as_object) else {
        return Ok(());
    };
    let mut seen_destinations = HashSet::new();
    let mut duplicate_count = 0usize;
    for entry in objects.values() {
        let Some(hash) = entry.get("hash").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let hash = hash.trim();
        if hash.len() < 2 {
            continue;
        }
        let prefix = &hash[..2];
        let destination = instance_root
            .join("assets")
            .join("objects")
            .join(prefix)
            .join(hash);
        if destination.exists() {
            continue;
        }
        if !seen_destinations.insert(destination.clone()) {
            duplicate_count += 1;
            continue;
        }
        tasks.push(FileDownloadTask {
            url: format!("https://resources.download.minecraft.net/{prefix}/{hash}"),
            destination,
            expected_size: entry
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .filter(|size| *size > 0),
        });
    }
    if duplicate_count > 0 {
        tracing::info!(
            target: "vertexlauncher/installation/dependencies",
            asset_index_path = %asset_index_path.display(),
            duplicate_count,
            deduped_task_count = tasks.len(),
            "Deduplicated repeated asset object downloads that pointed to the same destination."
        );
    }
    Ok(())
}

pub(crate) fn push_download_task_from_download_entry(
    root: &Path,
    download_entry: &serde_json::Value,
    tasks: &mut Vec<FileDownloadTask>,
) {
    let Some(url) = download_entry
        .get("url")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let Some(path) = download_entry
        .get("path")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let url = url.trim();
    let path = path.trim();
    if url.is_empty() || path.is_empty() {
        return;
    }
    let destination = root.join(path);
    if destination.exists() {
        return;
    }
    tasks.push(FileDownloadTask {
        url: url.to_owned(),
        destination,
        expected_size: download_entry
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .filter(|size| *size > 0),
    });
}

#[derive(Clone, Debug)]
pub(crate) struct FileDownloadTask {
    pub(crate) url: String,
    pub(crate) destination: PathBuf,
    pub(crate) expected_size: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct BandwidthLimiter {
    bits_per_second: u64,
    state: Mutex<BandwidthState>,
}

#[derive(Debug)]
pub(crate) struct BandwidthState {
    window_start: Instant,
    bits_sent: u128,
}

impl BandwidthLimiter {
    fn new(bits_per_second: u64) -> Self {
        Self {
            bits_per_second: bits_per_second.max(1),
            state: Mutex::new(BandwidthState {
                window_start: Instant::now(),
                bits_sent: 0,
            }),
        }
    }

    fn consume(&self, bytes: usize) {
        let requested_bits = (bytes as u128).saturating_mul(8);
        loop {
            let wait_duration = {
                let Ok(mut state) = self.state.lock() else {
                    return;
                };
                let elapsed = state.window_start.elapsed();
                if elapsed >= Duration::from_secs(1) {
                    state.window_start = Instant::now();
                    state.bits_sent = 0;
                }
                let max_bits = self.bits_per_second as u128;
                if state.bits_sent.saturating_add(requested_bits) <= max_bits {
                    state.bits_sent = state.bits_sent.saturating_add(requested_bits);
                    None
                } else {
                    Some(Duration::from_secs(1).saturating_sub(elapsed))
                }
            };
            if let Some(wait) = wait_duration {
                thread::sleep(wait.max(Duration::from_millis(1)));
                continue;
            }
            return;
        }
    }
}

#[derive(Debug)]
pub(crate) struct DownloadTelemetry {
    started_at: Instant,
    total_files: u32,
    completed_files: AtomicU32,
    downloaded_bytes: AtomicU64,
    known_total_bytes: AtomicU64,
    last_emit_millis: AtomicU64,
    eta_state: Mutex<ProgressEtaState>,
}

impl DownloadTelemetry {
    fn new(total_files: u32, known_total_bytes: u64) -> Self {
        Self {
            started_at: Instant::now(),
            total_files,
            completed_files: AtomicU32::new(0),
            downloaded_bytes: AtomicU64::new(0),
            known_total_bytes: AtomicU64::new(known_total_bytes),
            last_emit_millis: AtomicU64::new(0),
            eta_state: Mutex::new(ProgressEtaState::default()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProgressEtaPoint {
    fraction: f64,
    at_millis: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProgressEtaState {
    last_point: Option<ProgressEtaPoint>,
    last_eta_seconds: Option<u64>,
}

impl ProgressEtaState {
    fn observe(&mut self, point: ProgressEtaPoint) -> Option<u64> {
        let fraction = point.fraction.clamp(0.0, 1.0);
        if fraction >= 1.0 {
            self.last_point = Some(point);
            self.last_eta_seconds = Some(0);
            return Some(0);
        }

        let Some(previous) = self.last_point else {
            self.last_point = Some(point);
            self.last_eta_seconds = None;
            return None;
        };

        if fraction < previous.fraction || point.at_millis <= previous.at_millis {
            self.last_point = Some(point);
            self.last_eta_seconds = None;
            return None;
        }

        let delta_fraction = fraction - previous.fraction;
        if delta_fraction <= f64::EPSILON {
            return self.last_eta_seconds;
        }

        let delta_seconds = (point.at_millis - previous.at_millis) as f64 / 1000.0;
        if delta_seconds <= 0.0 {
            return self.last_eta_seconds;
        }

        let fraction_per_second = delta_fraction / delta_seconds;
        let eta_seconds = if fraction_per_second > 0.0 {
            Some(((1.0 - fraction) / fraction_per_second).ceil().max(0.0) as u64)
        } else {
            None
        };

        self.last_point = Some(point);
        self.last_eta_seconds = eta_seconds;
        eta_seconds
    }
}

pub(crate) fn install_progress_fraction(
    downloaded_files: u32,
    total_files: u32,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> f64 {
    if let Some(total_bytes) = total_bytes
        && total_bytes > 0
    {
        return (downloaded_bytes as f64 / total_bytes as f64).clamp(0.0, 1.0);
    }
    if total_files > 0 {
        return (downloaded_files as f64 / total_files as f64).clamp(0.0, 1.0);
    }
    0.0
}

pub(crate) fn emit_download_progress(
    progress: Option<&InstallProgressSink>,
    telemetry: &DownloadTelemetry,
    stage: InstallStage,
    downloaded_files_offset: u32,
) {
    let Some(progress) = progress else {
        return;
    };

    let now_millis = telemetry.started_at.elapsed().as_millis() as u64;
    let last_millis = telemetry.last_emit_millis.load(Ordering::Relaxed);
    if now_millis > 0 && now_millis.saturating_sub(last_millis) < 200 {
        return;
    }
    telemetry
        .last_emit_millis
        .store(now_millis, Ordering::Relaxed);

    let completed_files = telemetry.completed_files.load(Ordering::Relaxed);
    let downloaded_bytes = telemetry.downloaded_bytes.load(Ordering::Relaxed);
    let known_total_bytes = telemetry.known_total_bytes.load(Ordering::Relaxed);
    let elapsed = telemetry.started_at.elapsed().as_secs_f64().max(0.001);
    let bytes_per_second = downloaded_bytes as f64 / elapsed;
    let total_bytes = (known_total_bytes > 0).then_some(known_total_bytes);
    let downloaded_files = downloaded_files_offset.saturating_add(completed_files);
    let total_files = downloaded_files_offset.saturating_add(telemetry.total_files);
    let fraction =
        install_progress_fraction(downloaded_files, total_files, downloaded_bytes, total_bytes);
    let eta_seconds = telemetry.eta_state.lock().ok().and_then(|mut state| {
        state.observe(ProgressEtaPoint {
            fraction,
            at_millis: now_millis,
        })
    });

    progress(InstallProgress {
        stage,
        message: format!(
            "Downloading files ({}/{})...",
            downloaded_files, total_files
        ),
        downloaded_files,
        total_files,
        downloaded_bytes,
        total_bytes,
        bytes_per_second,
        eta_seconds,
    });
}

pub(crate) fn prefetch_batch_total_bytes(
    tasks: &mut [FileDownloadTask],
    probe_workers: usize,
    max_probe_count: usize,
) -> Result<u64, InstallationError> {
    let mut total_known_bytes = 0u64;
    let mut unknown = std::collections::VecDeque::new();
    let mut skipped_probe_count = 0usize;
    for (index, task) in tasks.iter().enumerate() {
        if let Some(size) = task.expected_size {
            total_known_bytes = total_known_bytes.saturating_add(size);
        } else {
            if unknown.len() < max_probe_count {
                unknown.push_back((index, task.url.clone()));
            } else {
                skipped_probe_count += 1;
            }
        }
    }
    if unknown.is_empty() {
        return Ok(total_known_bytes);
    }
    if skipped_probe_count > 0 {
        tracing::debug!(
            target: "vertexlauncher/installation/downloads",
            probed = unknown.len(),
            skipped = skipped_probe_count,
            "Skipping some HEAD size probes to reduce batch startup latency"
        );
    }

    let queue = Arc::new(Mutex::new(unknown));
    let discovered = Arc::new(Mutex::new(Vec::<(usize, u64)>::new()));
    thread::scope(|scope| -> Result<(), InstallationError> {
        let mut workers = Vec::new();
        for _ in 0..probe_workers.max(1) {
            let queue = Arc::clone(&queue);
            let discovered = Arc::clone(&discovered);
            workers.push(scope.spawn(move || {
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some((index, url)) = next else {
                        break;
                    };
                    if let Some(size) = probe_content_length(url.as_str())
                        && let Ok(mut guard) = discovered.lock()
                    {
                        guard.push((index, size));
                    }
                }
            }));
        }
        for worker in workers {
            worker.join().map_err(|_| {
                InstallationError::Io(std::io::Error::other(
                    "content-length probe worker panicked",
                ))
            })?;
        }
        Ok(())
    })?;

    if let Ok(discovered) = discovered.lock() {
        for (index, size) in discovered.iter().copied() {
            if let Some(task) = tasks.get_mut(index)
                && task.expected_size.is_none()
            {
                task.expected_size = Some(size);
                total_known_bytes = total_known_bytes.saturating_add(size);
            }
        }
    }

    Ok(total_known_bytes)
}

pub(crate) fn probe_content_length(url: &str) -> Option<u64> {
    let response = match http_agent()
        .head(url)
        .header("User-Agent", DEFAULT_USER_AGENT)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
    {
        Ok(response) => response,
        Err(err) => {
            tracing::debug!(
                target: "vertexlauncher/installation/downloads",
                "Size prefetch HEAD transport error for {}: {}",
                url,
                err
            );
            return None;
        }
    };
    if response.status().as_u16() >= 400 {
        tracing::debug!(
            target: "vertexlauncher/installation/downloads",
            "Size prefetch HEAD failed for {} with status {}",
            url,
            response.status().as_u16()
        );
        return None;
    }
    response
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|size| *size > 0)
}

pub(crate) fn download_files_concurrent(
    stage: InstallStage,
    tasks: Vec<FileDownloadTask>,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    if tasks.is_empty() {
        return Ok(0);
    }

    let total_files = tasks.len() as u32;
    let mut tasks = tasks;
    let batch_started_at = Instant::now();
    let worker_count = policy.max_concurrent_downloads.clamp(1, 64) as usize;
    let size_probe_workers = worker_count.min(8).max(1);
    let max_size_probes = MAX_CONTENT_LENGTH_PROBES_PER_BATCH.max(size_probe_workers);
    let prefetched_total_bytes =
        prefetch_batch_total_bytes(&mut tasks, size_probe_workers, max_size_probes)?;
    // Prioritize larger files so long-running transfers start earlier.
    tasks.sort_by_key(|task| std::cmp::Reverse(task.expected_size.unwrap_or(0)));
    tracing::info!(
        target: "vertexlauncher/installation/downloads",
        "Starting {:?} batch: {} file(s), prefetched_total_bytes={}, max_concurrent_downloads={}, speed_limit_bps={:?}.",
        stage,
        total_files,
        prefetched_total_bytes,
        policy.max_concurrent_downloads,
        policy.max_download_bps
    );
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(tasks)));
    let bandwidth_limiter = policy
        .max_download_bps
        .map(BandwidthLimiter::new)
        .map(Arc::new);
    let telemetry = Arc::new(DownloadTelemetry::new(total_files, prefetched_total_bytes));

    emit_download_progress(progress, &telemetry, stage, downloaded_files_offset);

    let downloaded_files = thread::scope(|scope| -> Result<u32, InstallationError> {
        let mut workers = Vec::new();
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let bandwidth_limiter = bandwidth_limiter.as_ref().map(Arc::clone);
            let telemetry = Arc::clone(&telemetry);
            workers.push(scope.spawn(move || -> Result<u32, InstallationError> {
                let mut completed = 0u32;
                loop {
                    let next_task = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some(task) = next_task else {
                        break;
                    };
                    download_to_file(
                        task,
                        bandwidth_limiter.as_deref(),
                        &telemetry,
                        downloaded_files_offset,
                        stage,
                        progress,
                    )?;
                    completed += 1;
                }
                Ok(completed)
            }));
        }

        let mut total = 0u32;
        for worker in workers {
            match worker.join() {
                Ok(Ok(count)) => total += count,
                Ok(Err(err)) => return Err(err),
                Err(_) => {
                    return Err(InstallationError::Io(std::io::Error::other(
                        "download worker panicked",
                    )));
                }
            }
        }
        Ok(total)
    })?;

    emit_download_progress(progress, &telemetry, stage, downloaded_files_offset);
    tracing::info!(
        target: "vertexlauncher/installation/downloads",
        "Completed {:?} batch: {} file(s) in {:.2}s.",
        stage,
        downloaded_files,
        batch_started_at.elapsed().as_secs_f64()
    );
    Ok(downloaded_files)
}

pub(crate) fn download_to_file(
    task: FileDownloadTask,
    bandwidth_limiter: Option<&BandwidthLimiter>,
    telemetry: &DownloadTelemetry,
    downloaded_files_offset: u32,
    stage: InstallStage,
    progress: Option<&InstallProgressSink>,
) -> Result<(), InstallationError> {
    if let Some(parent) = task.destination.parent() {
        fs_create_dir_all(parent)?;
    }
    let started_at = Instant::now();
    tracing::debug!(
        target: "vertexlauncher/installation/downloads",
        "Download start: {} -> {}",
        task.url,
        task.destination.display()
    );

    let mut response = call_get_response_with_retry(task.url.as_str(), DEFAULT_USER_AGENT)?;
    if task.expected_size.is_none()
        && let Some(content_length) = response
            .headers()
            .get("Content-Length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|size| *size > 0)
    {
        telemetry
            .known_total_bytes
            .fetch_add(content_length, Ordering::Relaxed);
    }

    let temp_path = temporary_download_path(task.destination.as_path());
    let mut reader = response.body_mut().as_reader();
    let mut file = fs_file_create(&temp_path)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url = %task.url,
                temp_path = %temp_path.display(),
                destination = %task.destination.display(),
                error = %err,
                "Failed while reading HTTP response body for download."
            );
            InstallationError::Io(err)
        })?;
        if read == 0 {
            break;
        }
        if let Some(limiter) = bandwidth_limiter {
            limiter.consume(read);
        }
        telemetry
            .downloaded_bytes
            .fetch_add(read as u64, Ordering::Relaxed);
        emit_download_progress(progress, telemetry, stage, downloaded_files_offset);
        file.write_all(&buffer[..read]).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url = %task.url,
                temp_path = %temp_path.display(),
                destination = %task.destination.display(),
                error = %err,
                "Failed while writing download chunk to temporary file."
            );
            InstallationError::Io(err)
        })?;
    }
    file.flush().map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url = %task.url,
            temp_path = %temp_path.display(),
            destination = %task.destination.display(),
            error = %err,
            "Failed while flushing temporary download file."
        );
        InstallationError::Io(err)
    })?;
    fs_rename(temp_path.as_path(), task.destination.as_path()).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url = %task.url,
            temp_path = %temp_path.display(),
            destination = %task.destination.display(),
            error = %err,
            "Failed while promoting temporary download file into place."
        );
        err
    })?;
    telemetry.completed_files.fetch_add(1, Ordering::Relaxed);
    emit_download_progress(progress, telemetry, stage, downloaded_files_offset);
    tracing::debug!(
        target: "vertexlauncher/installation/downloads",
        "Download complete: {} ({:.2}s)",
        task.destination.display(),
        started_at.elapsed().as_secs_f64()
    );
    Ok(())
}

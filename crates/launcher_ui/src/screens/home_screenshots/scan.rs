use super::*;

static SCREENSHOT_RESULTS: OnceLock<Mutex<ScreenshotResultChannel>> = OnceLock::new();

pub(super) fn screenshot_results() -> &'static Mutex<ScreenshotResultChannel> {
    SCREENSHOT_RESULTS.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<ScreenshotScanMessage>();
        Mutex::new(ScreenshotResultChannel {
            rx: result_rx,
            tx: result_tx,
        })
    })
}

pub(in crate::screens::home) fn build_screenshot_scan_request(
    instances: &InstanceStore,
    config: &Config,
) -> ScreenshotScanRequest {
    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let screenshot_instances = instances
        .instances
        .iter()
        .map(|instance| ScreenshotScanInstance {
            instance_name: instance.name.clone(),
            screenshots_dir: instance_root_path(installations_root.as_path(), instance)
                .join("screenshots"),
        })
        .collect();

    ScreenshotScanRequest {
        scanned_instance_count: instances.instances.len(),
        instances: screenshot_instances,
    }
}

pub(in crate::screens::home) fn poll_screenshot_results(state: &mut HomeState) {
    let Ok(channel) = screenshot_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            request_id = state.latest_requested_screenshot_scan_id,
            "Home screenshot results receiver mutex was poisoned while polling screenshot entries."
        );
        return;
    };

    let mut messages = Vec::new();
    while let Ok(msg) = channel.rx.try_recv() {
        messages.push(msg);
    }
    drop(channel);

    let mut any_entries = false;
    for msg in messages {
        match msg {
            ScreenshotScanMessage::EntryLoaded { request_id, entry } => {
                if request_id != state.latest_requested_screenshot_scan_id {
                    continue;
                }
                state.screenshots.push(entry);
                any_entries = true;
            }
            ScreenshotScanMessage::TaskDone { request_id } => {
                if request_id != state.latest_requested_screenshot_scan_id {
                    continue;
                }
                state.screenshot_tasks_done += 1;
                let all_pages_spawned =
                    state.screenshot_loaded_count >= state.screenshot_candidates.len();
                if state.screenshot_scan_ready
                    && all_pages_spawned
                    && state.screenshot_tasks_done >= state.screenshot_tasks_total
                {
                    state.screenshot_scan_pending = false;
                    state.last_screenshot_scan_at = Some(Instant::now());
                }
            }
        }
    }

    if any_entries {
        state.screenshots.sort_by(|a, b| {
            b.modified_at_ms
                .unwrap_or(0)
                .cmp(&a.modified_at_ms.unwrap_or(0))
                .then_with(|| a.file_name.cmp(&b.file_name))
        });
        state.mark_screenshot_layout_dirty();
    }
}

pub(in crate::screens::home) fn spawn_screenshot_load_page(state: &mut HomeState, request_id: u64, page_size: usize) {
    let start = state.screenshot_loaded_count;
    let end = (start + page_size).min(state.screenshot_candidates.len());
    if start >= end {
        return;
    }

    let Ok(channel) = screenshot_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            request_id,
            start = start,
            end = end,
            page_size,
            "Home screenshot results channel mutex was poisoned while spawning screenshot page load."
        );
        return;
    };
    let result_tx = channel.tx.clone();
    drop(channel);

    state.screenshot_loaded_count = end;
    state.screenshot_tasks_total += end - start;

    for candidate in state.screenshot_candidates[start..end].iter().cloned() {
        let tx = result_tx.clone();
        tokio_runtime::spawn_detached(async move {
            let entry = tokio_runtime::spawn_blocking(move || {
                let Ok((width, height)) = image::image_dimensions(&candidate.path) else {
                    return None;
                };
                if width == 0 || height == 0 {
                    return None;
                }
                Some(ScreenshotEntry {
                    instance_name: candidate.instance_name,
                    path: candidate.path,
                    file_name: candidate.file_name,
                    width,
                    height,
                    modified_at_ms: candidate.modified_at_ms,
                })
            })
            .await
            .ok()
            .flatten();
            if let Some(entry) = entry {
                if let Err(err) = tx.send(ScreenshotScanMessage::EntryLoaded { request_id, entry })
                {
                    tracing::error!(
                        target: "vertexlauncher/home",
                        request_id,
                        error = %err,
                        "Failed to deliver home screenshot entry."
                    );
                }
            }
            if let Err(err) = tx.send(ScreenshotScanMessage::TaskDone { request_id }) {
                tracing::error!(
                    target: "vertexlauncher/home",
                    request_id,
                    error = %err,
                    "Failed to deliver home screenshot task completion."
                );
            }
        });
    }
}

/// Looks for screenshot files on a worker (listing and statting thousands of files must not happen on the UI thread).
/// [`poll_screenshot_candidates`] applies the result.
pub(in crate::screens::home) fn refresh_screenshot_state(
    state: &mut HomeState,
    instances: &InstanceStore,
    config: &Config,
    force: bool,
) {
    if (state.screenshot_scan_pending || state.candidate_scan_in_flight) && !force {
        return;
    }
    let request = build_screenshot_scan_request(instances, config);
    let serial = state.candidate_scan_serial.saturating_add(1);
    state.candidate_scan_serial = serial;
    state.candidate_scan_in_flight = true;
    let count = request.scanned_instance_count;
    let tx = state.candidate_scans.sender();
    tokio_runtime::spawn_blocking_detached(move || {
        let candidates = collect_screenshot_candidates(&request);
        let _ = tx.send((serial, count, force, candidates));
    });
}

/// Takes a finished file listing: unchanged ones just restart the rescan timer; changed ones replace the gallery and
/// start loading it, a page at a time.
pub(in crate::screens::home) fn poll_screenshot_candidates(state: &mut HomeState) {
    let drained = state.candidate_scans.drain();
    if drained.disconnected {
        state.candidate_scan_in_flight = false;
    }
    for (serial, count, force, candidates) in drained.items {
        if serial != state.candidate_scan_serial {
            continue;
        }
        state.candidate_scan_in_flight = false;
        if !force && count == state.scanned_screenshot_instance_count && candidates == state.screenshot_candidates {
            state.last_screenshot_scan_at = Some(Instant::now());
            state.screenshot_scan_pending = false;
            continue;
        }
        let request_id = state.latest_requested_screenshot_scan_id.saturating_add(1);
        state.latest_requested_screenshot_scan_id = request_id;
        state.screenshot_scan_pending = true;
        state.screenshot_scan_ready = true;
        state.screenshot_tasks_total = 0;
        state.screenshot_tasks_done = 0;
        state.screenshots.clear();
        state.mark_screenshot_layout_dirty();
        state.scanned_screenshot_instance_count = count;
        state.screenshot_candidates = candidates;
        state.screenshot_loaded_count = 0;
        if state.screenshot_candidates.is_empty() {
            state.screenshot_scan_pending = false;
            state.last_screenshot_scan_at = Some(Instant::now());
        } else {
            spawn_screenshot_load_page(state, request_id, SCREENSHOT_PAGE_SIZE);
        }
    }
}

/// Starts the next page of screenshots once the previous page has been read, until every file has been.
pub(in crate::screens::home) fn load_next_screenshot_page(state: &mut HomeState) {
    if state.screenshot_scan_ready
        && state.screenshot_tasks_done >= state.screenshot_tasks_total
        && state.screenshot_loaded_count < state.screenshot_candidates.len()
    {
        let request_id = state.latest_requested_screenshot_scan_id;
        spawn_screenshot_load_page(state, request_id, SCREENSHOT_PAGE_SIZE * 4);
    }
}

pub(super) fn collect_screenshot_candidates(
    request: &ScreenshotScanRequest,
) -> Vec<ScreenshotCandidate> {
    let mut candidates = Vec::new();
    for instance in &request.instances {
        let Ok(entries) = fs::read_dir(&instance.screenshots_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !is_supported_screenshot_path(path.as_path()) {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            candidates.push(ScreenshotCandidate {
                instance_name: instance.instance_name.clone(),
                file_name,
                modified_at_ms: modified_millis(path.as_path()),
                path,
            });
        }
    }
    candidates.sort_by(|a, b| {
        b.modified_at_ms
            .unwrap_or(0)
            .cmp(&a.modified_at_ms.unwrap_or(0))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    candidates
}

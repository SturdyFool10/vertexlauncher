use super::*;

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

/// Takes a finished file listing: unchanged ones just restart the rescan timer; changed ones replace the gallery.
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
        // Nothing is read from the files here: a picture's size comes from the engine's asset manager when the gallery loads
        // it, so this is just the listing, already newest first.
        state.screenshots = candidates
            .iter()
            .map(|candidate| ScreenshotEntry {
                instance_name: candidate.instance_name.clone(),
                path: candidate.path.clone(),
                file_name: candidate.file_name.clone(),
                modified_at_ms: candidate.modified_at_ms,
            })
            .collect();
        state.screenshot_candidates = candidates;
        state.scanned_screenshot_instance_count = count;
        state.screenshot_scan_pending = false;
        state.last_screenshot_scan_at = Some(Instant::now());
        state.mark_screenshot_layout_dirty();
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
    candidates.sort_by_cached_key(|candidate| {
        (
            std::cmp::Reverse(config::screenshot_sort_key(&candidate.path, candidate.modified_at_ms)),
            std::cmp::Reverse(candidate.file_name.clone()),
        )
    });
    candidates
}

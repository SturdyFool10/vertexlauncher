use vertex_i18n::tr;
use super::*;







pub(super) fn poll_instance_log_scan_results(state: &mut InstanceScreenState) {
    let drained = state.log_scan_results.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/instance",
            "Instance log-scan worker disconnected unexpectedly."
        );
        state.log_scan_in_flight = false;
    }
    for (request_id, logs) in drained.items {
        if request_id != state.log_scan_request_serial {
            continue;
        }
        state.logs = logs;
        if state
            .selected_log_path
            .as_ref()
            .is_some_and(|selected| !state.logs.iter().any(|entry| entry.path == *selected))
        {
            state.selected_log_path = None;
        }
        state.last_log_scan_at = Some(Instant::now());
        state.log_scan_in_flight = false;
    }
}

pub(super) fn refresh_instance_logs(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    force: bool,
) {
    if state.log_scan_in_flight && !force {
        return;
    }

    let tx = state.log_scan_results.sender();
    state.log_scan_request_serial = state.log_scan_request_serial.saturating_add(1);
    let request_id = state.log_scan_request_serial;
    state.log_scan_in_flight = true;
    let instance_root = instance_root.to_path_buf();
    let _ = tokio_runtime::spawn_detached(async move {
        let logs = collect_instance_logs(instance_root.as_path());
        if let Err(err) = tx.send((request_id, logs)) {
            tracing::error!(
                target: "vertexlauncher/instance",
                request_id,
                error = %err,
                "Failed to deliver instance log scan result."
            );
        }
    });
}

fn collect_instance_logs(instance_root: &Path) -> Vec<InstanceLogEntry> {
    let logs_dir = instance_root.join("logs");
    let Ok(entries) = fs::read_dir(logs_dir) else {
        return Vec::new();
    };
    let mut logs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        logs.push(InstanceLogEntry {
            file_name: entry.file_name().to_string_lossy().to_string(),
            modified_at_ms: modified_millis(path.as_path()),
            path,
            size_bytes: metadata.len(),
        });
    }
    logs.sort_by(|a, b| {
        b.modified_at_ms
            .unwrap_or(0)
            .cmp(&a.modified_at_ms.unwrap_or(0))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    logs
}

pub(super) fn sync_selected_instance_log(state: &mut InstanceScreenState) {
    if state.selected_log_path.is_none() {
        state.selected_log_path = state.logs.first().map(|entry| entry.path.clone());
    }
    let Some(selected_log_path) = state.selected_log_path.as_ref() else {
        state.loaded_log_path = None;
        state.loaded_log_lines.clear();
        state.loaded_log_error = None;
        state.loaded_log_modified_at_ms = None;
        state.loaded_log_truncated = false;
        state.log_load_in_flight = false;
        state.requested_log_load_path = None;
        state.requested_log_load_modified_at_ms = None;
        return;
    };
    let current_modified = state
        .logs
        .iter()
        .find(|entry| &entry.path == selected_log_path)
        .and_then(|entry| entry.modified_at_ms);
    if state.log_load_in_flight
        && state.requested_log_load_path.as_ref() == Some(selected_log_path)
        && state.requested_log_load_modified_at_ms == current_modified
    {
        return;
    }
    if state.loaded_log_path.as_ref() != Some(selected_log_path)
        || state.loaded_log_modified_at_ms != current_modified
    {
        load_selected_instance_log(state);
    }
}

pub(super) fn poll_instance_log_load_results(state: &mut InstanceScreenState) {
    let drained = state.log_load_results.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/instance",
            "Instance log-load worker disconnected unexpectedly."
        );
        state.log_load_in_flight = false;
        state.requested_log_load_path = None;
        state.requested_log_load_modified_at_ms = None;
        state.loaded_log_error = Some(tr!("Log load worker stopped unexpectedly.").into_owned());
        state.loaded_log_truncated = false;
    }
    for (request_id, path, modified_at_ms, result) in drained.items {
        if request_id != state.log_load_request_serial {
            continue;
        }
        state.log_load_in_flight = false;
        state.requested_log_load_path = None;
        state.requested_log_load_modified_at_ms = None;
        state.loaded_log_path = Some(path.clone());
        state.loaded_log_modified_at_ms = modified_at_ms;
        match result {
            Ok((lines, truncated)) => {
                state.loaded_log_lines = lines;
                state.loaded_log_error = None;
                state.loaded_log_truncated = truncated;
            }
            Err(err) => {
                state.loaded_log_lines.clear();
                state.loaded_log_error = Some(err);
                state.loaded_log_truncated = false;
            }
        }
    }
}

pub(super) fn load_selected_instance_log(state: &mut InstanceScreenState) {
    let Some(selected_log_path) = state.selected_log_path.clone() else {
        state.loaded_log_path = None;
        state.loaded_log_lines.clear();
        state.loaded_log_error = None;
        state.loaded_log_modified_at_ms = None;
        state.loaded_log_truncated = false;
        state.log_load_in_flight = false;
        state.requested_log_load_path = None;
        state.requested_log_load_modified_at_ms = None;
        return;
    };

    let tx = state.log_load_results.sender();
    let modified_at_ms = modified_millis(selected_log_path.as_path());
    state.log_load_request_serial = state.log_load_request_serial.saturating_add(1);
    let request_id = state.log_load_request_serial;
    state.log_load_in_flight = true;
    state.requested_log_load_path = Some(selected_log_path.clone());
    state.requested_log_load_modified_at_ms = modified_at_ms;
    let path_for_worker = selected_log_path.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let result = read_instance_log_lines(path_for_worker.as_path());
        if let Err(err) = tx.send((
            request_id,
            selected_log_path.clone(),
            modified_at_ms,
            result,
        )) {
            tracing::error!(
                target: "vertexlauncher/instance",
                request_id,
                path = %selected_log_path.display(),
                error = %err,
                "Failed to deliver instance log load result."
            );
        }
    });
}

fn read_instance_log_lines(path: &Path) -> Result<(Vec<String>, bool), String> {
    let bytes =
        fs::read(path).map_err(|err| tr!("Failed to read '{path}': {err}", path = path.display(), err = err).into_owned())?;
    let decoded = if path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"))
    {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut output = Vec::new();
        decoder
            .read_to_end(&mut output)
            .map_err(|err| tr!("Failed to decompress '{path}': {err}", path = path.display(), err = err).into_owned())?;
        output
    } else {
        bytes
    };
    let text = String::from_utf8_lossy(&decoded);
    let total_lines = text.lines().count();
    let truncated = total_lines > MAX_INSTANCE_LOG_LINES;
    let lines = text
        .lines()
        .skip(total_lines.saturating_sub(MAX_INSTANCE_LOG_LINES))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    Ok((lines, truncated))
}

pub(super) fn format_log_file_size(size_bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let size = size_bytes as f64;
    if size >= MIB {
        tr!("{size:.1} MiB", size = size / MIB).into_owned()
    } else if size >= KIB {
        tr!("{size:.1} KiB", size = size / KIB).into_owned()
    } else {
        tr!("{size_bytes} B", size_bytes = size_bytes).into_owned()
    }
}

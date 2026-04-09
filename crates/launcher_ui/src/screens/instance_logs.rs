use super::*;

pub(super) fn render_instance_logs_tab(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
) {
    let title_style = style::body_strong(ui);
    let body_style = style::muted(ui);
    let _ = text_ui.label(ui, "instance_logs_title", "Logs", &title_style);
    let _ = text_ui.label(
        ui,
        "instance_logs_summary",
        "Select a log file to view it with the same highlighting rules as the live console.",
        &body_style,
    );
    ui.add_space(8.0);

    if state.logs.is_empty() {
        let _ = text_ui.label(
            ui,
            "instance_logs_empty",
            "No log files found under this instance's logs folder.",
            &body_style,
        );
        return;
    }

    let full_size = ui.available_size().max(egui::vec2(1.0, 1.0));
    let compact = is_compact_width(full_size.x, 760.0);
    let sidebar_width = (full_size.x * 0.28).clamp(220.0, 320.0);
    let logs_snapshot = state.logs.clone();
    if compact {
        let list_height = (full_size.y * 0.32).clamp(140.0, 240.0);
        ui.allocate_ui_with_layout(
            egui::vec2(full_size.x, list_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| render_instance_log_list(ui, state, &logs_snapshot),
        );
        ui.add_space(12.0);
        ui.allocate_ui_with_layout(
            egui::vec2(full_size.x, (full_size.y - list_height - 12.0).max(1.0)),
            egui::Layout::top_down(egui::Align::Min),
            |ui| render_instance_log_viewer(ui, text_ui, state, &title_style, &body_style),
        );
    } else {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(sidebar_width, full_size.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| render_instance_log_list(ui, state, &logs_snapshot),
            );
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(
                egui::vec2((full_size.x - sidebar_width - 12.0).max(1.0), full_size.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| render_instance_log_viewer(ui, text_ui, state, &title_style, &body_style),
            );
        });
    }
}

fn render_instance_log_list(
    ui: &mut Ui,
    state: &mut InstanceScreenState,
    logs_snapshot: &[InstanceLogEntry],
) {
    egui::ScrollArea::vertical()
        .id_salt("instance_logs_file_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for log in logs_snapshot {
                let selected = state.selected_log_path.as_ref() == Some(&log.path);
                let mut label = log.file_name.clone();
                if log.size_bytes > 0 {
                    label.push_str(&format!(
                        "\n{} | {}",
                        format_log_file_size(log.size_bytes),
                        format_time_ago(log.modified_at_ms, current_time_millis())
                    ));
                }
                let response = selectable_row_button(
                    ui,
                    egui::RichText::new(label).color(if selected {
                        ui.visuals().selection.stroke.color
                    } else {
                        ui.visuals().text_color()
                    }),
                    selected,
                    egui::vec2(ui.available_width(), 44.0),
                );
                if response.clicked() {
                    state.selected_log_path = Some(log.path.clone());
                    load_selected_instance_log(state);
                }
                ui.add_space(6.0);
            }
        });
}

fn render_instance_log_viewer(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
    title_style: &LabelOptions,
    body_style: &LabelOptions,
) {
    if let Some(selected_log_path) = state.selected_log_path.as_ref() {
        let log_name = selected_log_path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("Log");
        let _ = text_ui.label(ui, "instance_logs_selected_name", log_name, title_style);
        let mut details = selected_log_path.display().to_string();
        if state.loaded_log_truncated {
            details.push_str(&format!(" | showing last {} lines", MAX_INSTANCE_LOG_LINES));
        }
        let _ = text_ui.label(
            ui,
            "instance_logs_selected_path",
            details.as_str(),
            body_style,
        );
        ui.add_space(8.0);
        if state.log_load_in_flight {
            let _ = text_ui.label(
                ui,
                "instance_logs_loading",
                "Loading log contents...",
                body_style,
            );
            ui.add_space(8.0);
        }
        if let Some(error) = state.loaded_log_error.as_deref() {
            let _ = text_ui.label(ui, "instance_logs_error", error, &style::error_text(ui));
            return;
        }
        console_screen::render_log_buffer(
            ui,
            text_ui,
            ("instance_log_viewer", selected_log_path),
            &state.loaded_log_lines,
            "Log is empty.",
            false,
            crate::console::text_redraw_generation(),
        );
    } else {
        let _ = text_ui.label(
            ui,
            "instance_logs_no_selection",
            "Select a log file from the left to view it.",
            body_style,
        );
    }
}

fn ensure_instance_log_scan_channel(state: &mut InstanceScreenState) {
    if state.log_scan_results_tx.is_some() && state.log_scan_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(u64, Vec<InstanceLogEntry>)>();
    state.log_scan_results_tx = Some(tx);
    state.log_scan_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn poll_instance_log_scan_results(state: &mut InstanceScreenState) {
    let Some(rx) = state.log_scan_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/instance",
            "Instance log-scan receiver mutex was poisoned."
        );
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok((request_id, logs)) => {
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
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/instance",
                    "Instance log-scan worker disconnected unexpectedly."
                );
                state.log_scan_in_flight = false;
                break;
            }
        }
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

    ensure_instance_log_scan_channel(state);
    let Some(tx) = state.log_scan_results_tx.as_ref().cloned() else {
        return;
    };
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

fn ensure_instance_log_load_channel(state: &mut InstanceScreenState) {
    if state.log_load_results_tx.is_some() && state.log_load_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(
        u64,
        PathBuf,
        Option<u64>,
        Result<(Vec<String>, bool), String>,
    )>();
    state.log_load_results_tx = Some(tx);
    state.log_load_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn poll_instance_log_load_results(state: &mut InstanceScreenState) {
    let Some(rx) = state.log_load_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/instance",
            "Instance log-load receiver mutex was poisoned."
        );
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok((request_id, path, modified_at_ms, result)) => {
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
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/instance",
                    "Instance log-load worker disconnected unexpectedly."
                );
                state.log_load_in_flight = false;
                state.requested_log_load_path = None;
                state.requested_log_load_modified_at_ms = None;
                state.loaded_log_error = Some("Log load worker stopped unexpectedly.".to_owned());
                state.loaded_log_truncated = false;
                break;
            }
        }
    }
}

fn load_selected_instance_log(state: &mut InstanceScreenState) {
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

    ensure_instance_log_load_channel(state);
    let Some(tx) = state.log_load_results_tx.as_ref().cloned() else {
        return;
    };
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
        fs::read(path).map_err(|err| format!("Failed to read '{}': {err}", path.display()))?;
    let decoded = if path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"))
    {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut output = Vec::new();
        decoder
            .read_to_end(&mut output)
            .map_err(|err| format!("Failed to decompress '{}': {err}", path.display()))?;
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

fn format_log_file_size(size_bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let size = size_bytes as f64;
    if size >= MIB {
        format!("{:.1} MiB", size / MIB)
    } else if size >= KIB {
        format!("{:.1} KiB", size / KIB)
    } else {
        format!("{size_bytes} B")
    }
}

use vertex_i18n::tr;
use super::*;
use crate::screens::home::screenshot_common::is_supported_screenshot_path;

#[path = "instance_screenshots/instance_screenshot_overlay_action.rs"]
mod instance_screenshot_overlay_action;
#[path = "instance_screenshots/instance_screenshot_overlay_button_result.rs"]
mod instance_screenshot_overlay_button_result;
#[path = "instance_screenshots/instance_screenshot_overlay_result.rs"]
mod instance_screenshot_overlay_result;
#[path = "instance_screenshots/instance_screenshot_tile_action.rs"]
mod instance_screenshot_tile_action;





















pub(super) fn poll_instance_screenshot_scan_results(state: &mut InstanceScreenState) {
    let drained = state.screenshot_scan_results.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/instance",
            "Instance screenshot-scan worker disconnected unexpectedly."
        );
        state.screenshot_scan_in_flight = false;
    }
    for (request_id, screenshots) in drained.items {
        if request_id != state.screenshot_scan_request_serial {
            continue;
        }
        if screenshots != state.screenshots {
            state.screenshots = screenshots;
            state.mark_screenshot_layout_dirty();
        }
        state.last_screenshot_scan_at = Some(Instant::now());
        state.screenshot_scan_in_flight = false;
    }
}

pub(super) fn request_instance_screenshot_delete(
    state: &mut InstanceScreenState,
    screenshot_key: String,
    path: PathBuf,
    file_name: String,
) {
    if state.delete_screenshot_in_flight {
        return;
    }

    let tx = state.delete_screenshot_results.sender();

    state.delete_screenshot_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = launcher_runtime::fs::remove_file(path.as_path()).await.map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %path.display(), error = %err, context = "delete instance screenshot");
            tr!("failed to remove {path}: {err}", path = path.display(), err = err).into_owned()
        });
        if let Err(err) = tx.send((screenshot_key.clone(), file_name.clone(), result)) {
            tracing::error!(
                target: "vertexlauncher/instance",
                screenshot_key = %screenshot_key,
                file_name = %file_name,
                error = %err,
                "Failed to deliver instance screenshot-delete result."
            );
        }
    });
}

pub(super) fn poll_instance_screenshot_delete_results(
    state: &mut InstanceScreenState,
    instance_root: &Path,
) {
    let drained = state.delete_screenshot_results.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/instance",
            "Instance screenshot-delete worker disconnected unexpectedly."
        );
        state.delete_screenshot_in_flight = false;
        state.pending_delete_screenshot_key = None;
    }
    for (screenshot_key, file_name, result) in drained.items {
        state.delete_screenshot_in_flight = false;
        match result {
            Ok(()) => {
                if state
                    .screenshot_viewer
                    .as_ref()
                    .is_some_and(|viewer| viewer.screenshot_key == screenshot_key)
                {
                    tracing::info!(
                        target: "vertexlauncher/screenshots",
                        screenshot_key = screenshot_key.as_str(),
                        "Instance screenshot viewer closed because the screenshot was deleted."
                    );
                    state.screenshot_viewer = None;
                }
                state.pending_delete_screenshot_key = None;
                refresh_instance_screenshots(state, instance_root, true);
                notification::info!("instance/screenshots", "Deleted '{file_name}' from disk.", file_name = file_name);
            }
            Err(err) => {
                state.pending_delete_screenshot_key = None;
                notification::error!("instance/screenshots", "Failed to delete '{file_name}': {err}", file_name = file_name, err = err);
            }
        }
    }
}

pub(super) fn refresh_instance_screenshots(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    force: bool,
) {
    if state.screenshot_scan_in_flight && !force {
        return;
    }

    let tx = state.screenshot_scan_results.sender();
    state.screenshot_scan_request_serial = state.screenshot_scan_request_serial.saturating_add(1);
    let request_id = state.screenshot_scan_request_serial;
    state.screenshot_scan_in_flight = true;
    let instance_root = instance_root.to_path_buf();
    tokio_runtime::spawn_detached(async move {
        let screenshots = tokio_runtime::spawn_blocking(move || {
            collect_instance_screenshots(instance_root.as_path())
        })
        .await;
        let Ok(screenshots) = screenshots else {
            tracing::error!(
                target: "vertexlauncher/instance",
                request_id,
                "Failed to complete instance screenshot scan task."
            );
            return;
        };
        if let Err(err) = tx.send((request_id, screenshots)) {
            tracing::error!(
                target: "vertexlauncher/instance",
                request_id,
                error = %err,
                "Failed to deliver instance screenshot scan result."
            );
        }
    });
}

fn collect_instance_screenshots(instance_root: &Path) -> Vec<InstanceScreenshotEntry> {
    let screenshots_dir = instance_root.join("screenshots");
    let Ok(entries) = fs::read_dir(screenshots_dir) else {
        return Vec::new();
    };
    let mut screenshots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_supported_screenshot_path(path.as_path()) {
            continue;
        }
        let Ok((width, height)) = image::image_dimensions(path.as_path()) else {
            continue;
        };
        if width == 0 || height == 0 {
            continue;
        }
        screenshots.push(InstanceScreenshotEntry {
            file_name: entry.file_name().to_string_lossy().to_string(),
            modified_at_ms: modified_millis(path.as_path()),
            path,
            width,
            height,
        });
    }
    screenshots.sort_by(|a, b| {
        b.modified_at_ms
            .unwrap_or(0)
            .cmp(&a.modified_at_ms.unwrap_or(0))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    screenshots
}



pub(super) fn screenshot_key(path: &Path) -> String {
    path.to_string_lossy().to_string()
}



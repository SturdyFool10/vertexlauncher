use vertex_i18n::tr;
use super::*;

pub(in crate::screens::home) fn request_screenshot_delete(
    state: &mut HomeState,
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
            tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %path.display(), error = %err, context = "delete home screenshot");
            tr!("failed to remove {path}: {err}", path = path.display(), err = err).into_owned()
        });
        if let Err(err) = tx.send((screenshot_key.clone(), file_name.clone(), result)) {
            tracing::error!(
                target: "vertexlauncher/home",
                screenshot_key = %screenshot_key,
                file_name = %file_name,
                error = %err,
                "Failed to deliver home screenshot-delete result."
            );
        }
    });
}

pub(in crate::screens::home) fn poll_delete_screenshot_results(
    state: &mut HomeState,
    instances: &InstanceStore,
    config: &Config,
) {
    let drained = state.delete_screenshot_results.drain();
    let updates = drained.items;

    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/home", "delete_screenshot_results worker channel stopped unexpectedly.");
        state.delete_screenshot_in_flight = false;
        notification::error!("home/screenshots", "Screenshot delete worker stopped unexpectedly.");
    }

    for (screenshot_key, file_name, result) in updates {
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
                        "Home screenshot viewer closed because the screenshot was deleted."
                    );
                    state.screenshot_viewer = None;
                }
                state.pending_delete_screenshot_key = None;
                refresh_screenshot_state(state, instances, config, true);
                notification::info!("home/screenshots", "Deleted '{file_name}' from disk.", file_name = file_name);
            }
            Err(err) => {
                state.pending_delete_screenshot_key = None;
                notification::error!("home/screenshots", "Failed to delete '{file_name}': {err}", file_name = file_name, err = err);
            }
        }
    }
}



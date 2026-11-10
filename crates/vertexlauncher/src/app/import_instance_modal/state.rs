use vertex_i18n::tr;
use super::*;

#[path = "state/import_instance_state.rs"]
mod import_instance_state;
#[path = "state/import_mode.rs"]
mod import_mode;
#[path = "state/import_package_error.rs"]
mod import_package_error;
#[path = "state/import_package_kind.rs"]
mod import_package_kind;
#[path = "state/import_preview.rs"]
mod import_preview;
#[path = "state/import_preview_kind.rs"]
mod import_preview_kind;
#[path = "state/import_progress.rs"]
mod import_progress;
#[path = "state/import_request.rs"]
mod import_request;
#[path = "state/import_source.rs"]
mod import_source;
#[path = "state/import_task_result.rs"]
mod import_task_result;
#[path = "state/launcher_kind.rs"]
mod launcher_kind;
#[path = "state/modal_action.rs"]
mod modal_action;

pub use self::import_instance_state::ImportInstanceState;
pub(crate) use self::import_mode::ImportMode;
pub use self::import_package_error::ImportPackageError;
pub(crate) use self::import_package_kind::ImportPackageKind;
pub(crate) use self::import_preview::ImportPreview;
pub(crate) use self::import_preview_kind::ImportPreviewKind;
pub use self::import_progress::ImportProgress;
pub use self::import_request::ImportRequest;
pub use self::import_source::ImportSource;
pub use self::import_task_result::ImportTaskResult;
pub(crate) use self::launcher_kind::LauncherKind;

pub(crate) fn poll_preview_results(state: &mut ImportInstanceState) {
    for (request_serial, message) in state.preview_progress.drain().items {
        if request_serial == state.preview_request_serial {
            state.preview_status_message = Some(message);
        }
    }

    let drained = state.preview_results.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/import_instance",
            request_serial = state.preview_request_serial,
            "Import preview worker channel disconnected unexpectedly."
        );
        state.preview_in_flight = false;
        state.error = Some(tr!("Import preview worker stopped unexpectedly.").into_owned());
        state.preview_results.reset();
    }
    for (request_serial, result) in drained.items {
        if request_serial != state.preview_request_serial {
            tracing::debug!(
                target: "vertexlauncher/import_instance",
                request_serial,
                active_request_serial = state.preview_request_serial,
                "Ignoring stale import preview result."
            );
            continue;
        }
        state.preview_in_flight = false;
        state.preview_status_message = None;
        match result {
            Ok(preview) => {
                tracing::info!(
                    target: "vertexlauncher/import_instance",
                    request_serial,
                    preview_kind = %preview.kind.label(),
                    detected_name = %preview.detected_name,
                    game_version = %preview.game_version,
                    modloader = %preview.modloader,
                    modloader_version = %preview.modloader_version,
                    "Import preview completed."
                );
                if state.instance_name.trim().is_empty() {
                    state.instance_name = preview.detected_name.clone();
                }
                state.preview = Some(preview);
                state.error = None;
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/import_instance",
                    request_serial,
                    error = %err,
                    "Import preview failed."
                );
                state.preview = None;
                state.error = Some(err);
            }
        }
    }
}

pub(crate) const LAUNCHER_KIND_OPTIONS: [&str; 5] = [
    vertex_i18n::src!("Auto-detect"),
    vertex_i18n::src!("Modrinth"),
    vertex_i18n::src!("CurseForge"),
    vertex_i18n::src!("Prism / MultiMC"),
    vertex_i18n::src!("ATLauncher"),
];

pub(crate) fn selected_import_mode(state: &ImportInstanceState) -> ImportMode {
    ImportMode::from_index(state.source_mode_index)
}

pub(crate) fn selected_launcher_hint(state: &ImportInstanceState) -> Option<LauncherKind> {
    match state.launcher_kind_index {
        1 => Some(LauncherKind::Modrinth),
        2 => Some(LauncherKind::CurseForge),
        3 => Some(LauncherKind::Prism),
        4 => Some(LauncherKind::ATLauncher),
        _ => None,
    }
}

pub(crate) fn path_input_string(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

pub(crate) fn update_path_from_input(path: &mut PathBuf, input: &str) -> bool {
    let trimmed = input.trim();
    let next = if trimmed.is_empty() {
        PathBuf::new()
    } else {
        PathBuf::from(trimmed)
    };
    if *path == next {
        return false;
    }
    *path = next;
    true
}

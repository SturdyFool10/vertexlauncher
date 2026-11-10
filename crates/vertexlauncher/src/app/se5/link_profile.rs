//! "Link existing Vertex profile": pick a folder, detect its Minecraft and loader versions on a worker, and register it
//! as an instance that keeps living where it is.

use vertex_i18n::tr;
use std::path::PathBuf;

use installation::display_user_path;
use instances::{LinkedInstanceSpec, instance_root_path, link_instance_root};
use crate::app::se5::file_dialog::{Dialog as PickerDialog, Pick};
use launcher_ui::notification;
use launcher_ui::screens::{DetectedInstanceVersions, detect_instance_versions};

use super::{AppScreen, AppState};

const SLOT: &str = "link-profile::folder";

/// The detection result for a picked folder.
pub type Detected = (PathBuf, Result<DetectedInstanceVersions, String>);

/// Asks for the profile folder.
pub fn start(state: &mut AppState) {
    state.file_dialogs.open(
        SLOT,
        Pick::Folder,
        PickerDialog::new().title(tr!("Link Existing Vertex Profile")),
    );
}

/// Advances linking: picked folder, then version detection off the UI thread, then registration. Call every frame.
pub fn poll(state: &mut AppState) {
    if let Some(path) = state
        .file_dialogs
        .take(SLOT)
        .and_then(|paths| paths.into_iter().next())
    {
        let installations_root = state
            .config
            .minecraft_installations_root_path()
            .to_path_buf();
        let registered = state.instances.instances.iter().any(|instance| {
            instance.instance_root_override.as_deref() == Some(path.as_path())
                || instance_root_path(&installations_root, instance) == path
        });
        if registered {
            notification::warn!("instance_link", "That profile folder is already registered in Vertex.");
        } else {
            // Reading the profile's version files is filesystem work; keep it off the UI thread.
            let tx = state.link_results.sender();
            launcher_runtime::spawn_blocking_detached(move || {
                let detected =
                    detect_instance_versions(path.as_path()).map_err(|err| err.to_string());
                let _ = tx.send((path, detected));
            });
        }
    }
    for (path, detected) in state.link_results.drain().items {
        finish(state, path, detected);
    }
}

fn finish(state: &mut AppState, path: PathBuf, detected: Result<DetectedInstanceVersions, String>) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Linked Profile")
        .to_owned();
    let detected = match detected {
        Ok(detected) => Some(detected),
        Err(err) => {
            tracing::warn!(target: "vertexlauncher/instances", root = %path.display(), error = %err, "Failed to auto-detect linked instance versions; using fallback metadata.");
            notification::warn!("instance_link", "Linked profile version detection failed; using fallback version metadata. You can run auto-detect from Instance Settings.");
            None
        }
    };
    let summary = detected.as_ref().map(|detected| {
        if detected.modloader_version.trim().is_empty() {
            tr!("{modloader} for Minecraft {game_version}", modloader = detected.modloader, game_version = detected.game_version).into_owned()
        } else {
            tr!("{modloader} {modloader_version} for Minecraft {game_version}", modloader = detected.modloader, modloader_version = detected.modloader_version, game_version = detected.game_version).into_owned()
        }
    });
    let (modloader, game_version, modloader_version) = detected
        .map(|detected| {
            (
                detected.modloader,
                detected.game_version,
                detected.modloader_version,
            )
        })
        .unwrap_or_else(|| ("Vanilla".to_owned(), "latest".to_owned(), String::new()));
    let installations_root = state
        .config
        .minecraft_installations_root_path()
        .to_path_buf();
    match link_instance_root(
        &mut state.instances,
        &installations_root,
        LinkedInstanceSpec {
            name,
            root_path: path.clone(),
            description: Some(tr!("Linked existing profile at {path}.", path = display_user_path(path.as_path())).into_owned()),
            thumbnail_path: None,
            modloader,
            game_version,
            modloader_version,
        },
    ) {
        Ok(instance) => {
            state.selected_instance_id = Some(instance.id.clone());
            state.screen = AppScreen::Instance;
            match summary {
                Some(summary) => notification::info!("instance_link", "Linked existing profile '{name}' and detected {summary}.", name = instance.name, summary = summary),
                None => notification::info!("instance_link", "Linked existing profile '{name}'. You can adjust Minecraft/version settings from the profile screen.", name = instance.name),
            }
        }
        Err(err) => notification::error!("instance_link", "Failed to link existing profile folder: {err}", err = err),
    }
}

//! Toolkit-free view of the instance settings dialog: metadata, Minecraft version and modloader selection, runtime
//! overrides and maintenance actions, using the same validation and saving code as the egui dialog.

use vertex_i18n::tr;
use super::*;


/// One modloader choice.
#[derive(Clone, Debug)]
pub struct LoaderChoice {
    pub label: &'static str,
    /// Why it can't be picked for the selected Minecraft version.
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug)]
pub struct VersionControls {
    pub game_versions: Vec<String>,
    pub game_version: usize,
    pub catalog_loading: bool,
    pub catalog_error: Option<String>,
    pub loaders: Vec<LoaderChoice>,
    pub loader: usize,
    pub custom_loader: bool,
    pub loader_versions: Vec<String>,
    pub loader_version: usize,
    pub loader_versions_loading: bool,
    /// (text, is_error)
    pub loader_status: Option<(String, bool)>,
    pub validation_error: Option<String>,
}

/// Brings the selection state up to date (supported loader, loader versions, remote fetches) and describes it.
pub fn version_controls(state: &mut InstanceScreenState) -> VersionControls {
    let game_version = selected_game_version(state).to_owned();
    ensure_selected_modloader_is_supported(state, game_version.as_str());

    let game_versions: Vec<String> = state.available_game_versions.iter().map(MinecraftVersionEntry::display_label).collect();
    let game_version_index = state.selected_game_version_index.min(game_versions.len().saturating_sub(1));

    let loaders = MODLOADER_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| LoaderChoice {
            label: option,
            unavailable: if index == CUSTOM_MODLOADER_INDEX {
                None
            } else {
                state.loader_support.unavailable_reason(option, game_version.as_str())
            },
        })
        .collect();

    let mut loader_versions = Vec::new();
    let mut loader_version = 0;
    let mut loader_versions_loading = false;
    let mut loader_status = None;
    if state.selected_modloader == 0 {
        state.modloader_version_input.clear();
    } else {
        let label = selected_modloader_value(state);
        let key = modloader_versions_cache_key(label.as_str(), game_version.as_str());
        let mut options = selected_modloader_versions(state, game_version.as_str()).to_vec();
        if state.selected_modloader != CUSTOM_MODLOADER_INDEX && options.is_empty() {
            if let Some(cached) = state.modloader_versions_cache.get(&key) {
                options = cached.clone();
            } else {
                request_modloader_versions(state, label.as_str(), game_version.as_str(), false);
            }
        }
        loader_versions_loading = state.modloader_versions_in_flight.contains(&key);
        if state.modloader_versions_status_key.as_deref() == Some(key.as_str())
            && let Some(status) = state.modloader_versions_status.as_deref()
        {
            loader_status = Some((status.to_owned(), status.starts_with("Failed")));
        }
        if state.modloader_version_input.trim().is_empty()
            && let Some(first) = options.first()
        {
            state.modloader_version_input = first.clone();
        }
        let current = state.modloader_version_input.trim().to_owned();
        loader_version = options.iter().position(|entry| *entry == current).unwrap_or(0);
        loader_versions = options;
    }

    VersionControls {
        game_versions,
        game_version: game_version_index,
        catalog_loading: state.version_catalog_in_flight,
        catalog_error: state.version_catalog_error.clone(),
        loaders,
        loader: state.selected_modloader,
        custom_loader: state.selected_modloader == CUSTOM_MODLOADER_INDEX,
        loader_versions,
        loader_version,
        loader_versions_loading,
        loader_status,
        validation_error: metadata_validation_error(state),
    }
}

pub fn set_game_version(state: &mut InstanceScreenState, index: usize) {
    state.selected_game_version_index = index;
    if let Some(version) = state.available_game_versions.get(index) {
        state.game_version_input = version.id.clone();
    }
}

pub fn set_loader(state: &mut InstanceScreenState, index: usize) {
    if state.selected_modloader != index {
        state.selected_modloader = index;
        state.modloader_version_input.clear();
        state.incompatible_modloader_version_warning_key = None;
    }
}

pub fn set_loader_version(state: &mut InstanceScreenState, controls: &VersionControls, index: usize) {
    if let Some(version) = controls.loader_versions.get(index) {
        state.modloader_version_input = version.clone();
    }
}

/// Fetches the Minecraft version catalog again (and forgets the cached loader versions).
pub fn refresh_versions(state: &mut InstanceScreenState, config: &Config) {
    sync_version_catalog(
        state,
        installation::VersionCatalogFilter {
            include_snapshots_and_betas: config.include_snapshots_and_betas(),
            include_alpha: config.include_alpha_versions(),
            include_experimental: config.include_experimental_versions(),
        },
        true,
    );
    state.modloader_versions_cache.clear();
    state.modloader_versions_status = None;
    state.modloader_versions_status_key = None;
}

pub fn refresh_loader_versions(state: &mut InstanceScreenState) {
    let game_version = selected_game_version(state).to_owned();
    let label = selected_modloader_value(state);
    request_modloader_versions(state, label.as_str(), game_version.as_str(), true);
}

/// Reads the versions the instance folder was actually installed with and saves them. Returns whether the store
/// changed.
pub fn detect_versions(state: &mut InstanceScreenState, instances: &mut InstanceStore, instance_id: &str, instance_root: &Path) -> bool {
    match detect_instance_versions(instance_root) {
        Ok(detected) => {
            apply_detected_instance_versions(state, &detected);
            match set_instance_versions(
                instances,
                instance_id,
                detected.modloader.clone(),
                detected.game_version.clone(),
                detected.modloader_version.clone(),
            ) {
                Ok(()) => {
                    state.status_message = Some(tr!("Detected and saved {summary} from profile '{source_profile_id}'.", summary = detected_versions_summary(&detected), source_profile_id = detected.source_profile_id).into_owned());
                    true
                }
                Err(err) => {
                    state.status_message = Some(tr!("Detected versions, but failed to save them: {err}", err = err).into_owned());
                    false
                }
            }
        }
        Err(err) => {
            state.status_message = Some(tr!("Could not auto-detect installed versions: {err}", err = err).into_owned());
            false
        }
    }
}

pub fn save_versions(state: &mut InstanceScreenState, instance_id: &str, instances: &mut InstanceStore) -> bool {
    let mut changed = false;
    handle_save_versions_click(state, instance_id, instances, &mut changed);
    changed
}

pub fn reinstall(state: &mut InstanceScreenState, instance_id: &str, instances: &mut InstanceStore, config: &Config) -> bool {
    let mut changed = false;
    handle_reinstall_click(state, instance_id, instances, config, &mut changed);
    changed
}

/// What the runtime-override controls need besides the state.
#[derive(Clone, Debug)]
pub struct OverrideLimits {
    pub memory_max_mib: u128,
    pub memory_pending: bool,
    pub java_options: Vec<(u8, String)>,
}

pub fn override_limits(config: &Config) -> OverrideLimits {
    let (memory_max_mib, memory_pending) = memory_slider_max_mib();
    OverrideLimits { memory_max_mib, memory_pending, java_options: configured_java_path_options(config) }
}

pub fn save_overrides(state: &mut InstanceScreenState, instance_id: &str, instances: &mut InstanceStore, limits: &OverrideLimits) -> bool {
    match save_runtime_overrides(state, instance_id, instances, &limits.java_options, limits.memory_max_mib) {
        Ok(()) => {
            state.status_message = Some(tr!("Saved instance settings.").into_owned());
            true
        }
        Err(err) => {
            state.status_message = Some(err);
            false
        }
    }
}

/// Writes override edits once they have been idle for a moment. Returns whether the store changed and when to look
/// again.
pub fn autosave_overrides(
    state: &mut InstanceScreenState,
    instance_id: &str,
    instances: &mut InstanceStore,
    limits: &OverrideLimits,
) -> (bool, Option<Duration>) {
    autosave_runtime_overrides_step(state, instance_id, instances, &limits.java_options, limits.memory_max_mib)
}

/// Writes pending override edits now (the dialog is closing).
pub fn flush_overrides(state: &mut InstanceScreenState, instance_id: &str, instances: &mut InstanceStore, config: &Config) -> bool {
    flush_runtime_overrides_autosave(state, instance_id, instances, config)
}

pub fn open_folder_action(state: &mut InstanceScreenState, instances: &InstanceStore, config: &Config, instance_id: &str) {
    open_instance_folder(state, instances, config, instance_id);
}

pub fn prepare_move(state: &mut InstanceScreenState, instances: &InstanceStore, config: &Config, instance_id: &str) {
    prepare_move_instance_modal(state, instances, config, instance_id);
}

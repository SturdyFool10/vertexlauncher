//! Toolkit-free view of the instance screen: one [`InstanceSession`] per open instance, polled every frame, with
//! plain-data views and calls for the settings dialog, the content/screenshot/log tabs, exports and moving. The egui
//! screen drives the same code through its own widgets.

use vertex_i18n::tr;
use super::*;
use instances::InstanceRecord;

pub use super::content::api as content;
pub use super::instance_settings_modal::api as settings;
pub use super::instance_screen_state::{
    InstanceLogEntry, InstanceScreenState, InstanceScreenTab, InstanceScreenshotEntry, MoveInstanceProgress,
};

/// Everything about one open instance screen.
pub struct InstanceSession {
    pub id: String,
    pub state: InstanceScreenState,
}

pub struct Tick {
    /// Something is running in the background; the UI should look again soon.
    pub busy: bool,
    /// The instance store changed (a move finished, runtime prep updated a version...).
    pub instances_changed: bool,
}

impl InstanceSession {
    pub fn new(instance: &InstanceRecord, config: &Config) -> Self {
        Self { id: instance.id.clone(), state: InstanceScreenState::from_instance(instance, config) }
    }

    /// Polls every background task of the screen; call once per frame while it is visible.
    pub fn tick(&mut self, config: &mut Config, instances: &mut InstanceStore) -> Tick {
        let state = &mut self.state;
        let instance_id = self.id.as_str();
        let mut instances_changed = false;
        poll_background_tasks(state, config, instances, instance_id);
        poll_vtmpack_export_progress(state);
        poll_vtmpack_export_results(state);
        poll_server_export_progress(state);
        poll_server_export_results(state);
        poll_move_instance_progress(state);
        if let Some(result) = poll_move_instance_results(state) {
            match result {
                MoveInstanceResult::Complete { dest_path } => {
                    if let Some(instance) = instances.find_mut(instance_id) {
                        instance.instance_root_override = Some(dest_path.clone());
                        instances_changed = true;
                    }
                    state.move_instance_completion_message = Some(tr!("Instance moved to {dest_path}.", dest_path = dest_path.display()).into_owned());
                    state.move_instance_completion_failed = false;
                    state.status_message = Some(tr!("Instance moved to {dest_path}.", dest_path = dest_path.display()).into_owned());
                }
                MoveInstanceResult::Failed { reason } => {
                    state.move_instance_completion_message = Some(tr!("Instance move failed: {reason}", reason = reason).into_owned());
                    state.move_instance_completion_failed = true;
                    state.status_message = Some(tr!("Instance move failed: {reason}", reason = reason).into_owned());
                }
            }
        }
        poll_instance_screenshot_scan_results(state);
        poll_instance_log_scan_results(state);
        poll_instance_log_load_results(state);
        sync_version_catalog(
            state,
            installation::VersionCatalogFilter {
                include_snapshots_and_betas: config.include_snapshots_and_betas(),
                include_alpha: config.include_alpha_versions(),
                include_experimental: config.include_experimental_versions(),
            },
            false,
        );
        let game_version = selected_game_version(state).to_owned();
        ensure_selected_modloader_is_supported(state, game_version.as_str());
        let root = instances
            .find(instance_id)
            .map(|instance| instances::instance_root_path(config.minecraft_installations_root_path(), instance));
        if let Some(root) = root.as_deref() {
            poll_instance_screenshot_delete_results(state, root);
        }
        let busy = state.version_catalog_in_flight
            || !state.modloader_versions_in_flight.is_empty()
            || state.runtime_prepare_in_flight
            || state.content_apply_in_flight
            || state.screenshot_scan_in_flight
            || state.delete_screenshot_in_flight
            || state.log_scan_in_flight
            || state.log_load_in_flight
            || state.export_vtmpack_in_flight
            || state.export_server_in_flight
            || state.move_instance_in_flight;
        Tick { busy, instances_changed }
    }

    pub fn root(&self, instances: &InstanceStore, config: &Config) -> Option<PathBuf> {
        instances
            .find(self.id.as_str())
            .map(|instance| instances::instance_root_path(config.minecraft_installations_root_path(), instance))
    }

    /// Whether installing/preparing keeps the Content Browser locked.
    pub fn install_active(&self) -> bool {
        let external = install_activity::snapshot().filter(|activity| {
            activity.instance_id == self.state.name_input
                || activity.instance_id == self.id
        });
        self.state.runtime_prepare_in_flight
            || self.state.content_apply_in_flight
            || external.as_ref().is_some_and(|activity| !matches!(activity.stage, InstallStage::Complete))
    }

    /// Download progress of whatever is installing this instance (the screen's own prepare, or another flow's).
    pub fn install_progress(&self, instance_name: &str) -> Option<InstallProgressView> {
        let external = install_activity::snapshot().filter(|activity| {
            activity.instance_id == instance_name || activity.instance_id == self.id
        });
        if let Some(progress) = self.state.runtime_latest_progress.as_ref() {
            if matches!(progress.stage, InstallStage::Complete)
                && !self.state.runtime_prepare_in_flight
                && external.as_ref().is_none_or(|activity| matches!(activity.stage, InstallStage::Complete))
            {
                return None;
            }
            return Some(InstallProgressView {
                fraction: Some(progress_fraction(progress)),
                stage: stage_label(progress.stage).into_owned(),
                bytes_per_second: progress.bytes_per_second,
                eta_seconds: progress.eta_seconds,
                message: progress.message.clone(),
                downloaded_files: progress.downloaded_files,
                total_files: progress.total_files,
                downloaded_bytes: progress.downloaded_bytes,
                total_bytes: progress.total_bytes,
            });
        }
        if self.state.runtime_prepare_in_flight {
            return Some(InstallProgressView {
                fraction: None,
                stage: tr!("Starting installation...").into_owned(),
                bytes_per_second: 0.0,
                eta_seconds: None,
                message: String::new(),
                downloaded_files: 0,
                total_files: 0,
                downloaded_bytes: 0,
                total_bytes: None,
            });
        }
        let activity = external?;
        if matches!(activity.stage, InstallStage::Complete) {
            return None;
        }
        Some(InstallProgressView {
            fraction: Some(progress_fraction_from_activity(&activity)),
            stage: stage_label(activity.stage).into_owned(),
            bytes_per_second: activity.bytes_per_second,
            eta_seconds: activity.eta_seconds,
            message: activity.message.clone(),
            downloaded_files: activity.downloaded_files,
            total_files: activity.total_files,
            downloaded_bytes: activity.downloaded_bytes,
            total_bytes: activity.total_bytes,
        })
    }
}

#[derive(Clone, Debug)]
pub struct InstallProgressView {
    /// `None` = indeterminate.
    pub fraction: Option<f32>,
    pub stage: String,
    pub bytes_per_second: f64,
    pub eta_seconds: Option<u64>,
    pub message: String,
    pub downloaded_files: u32,
    pub total_files: u32,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

pub fn human_bytes(bytes: u64) -> String {
    format_bytes(bytes)
}

pub fn time_ago(timestamp_ms: Option<u64>) -> String {
    format_time_ago(timestamp_ms, current_time_millis())
}

// -- Screenshots ----------------------------------------------------------------------------------------------------

pub fn screenshots_tick(state: &mut InstanceScreenState, instance_root: &Path) {
    let should_scan = state.last_screenshot_scan_at.is_none_or(|last| last.elapsed() >= INSTANCE_SCREENSHOT_SCAN_INTERVAL);
    if should_scan {
        refresh_instance_screenshots(state, instance_root, false);
    }
    if state.pending_delete_screenshot_key.as_ref().is_some_and(|pending| {
        !state.screenshots.iter().any(|screenshot| screenshot_key(screenshot.path.as_path()) == *pending)
    }) {
        state.pending_delete_screenshot_key = None;
    }
    if state
        .screenshot_viewer
        .as_ref()
        .is_some_and(|viewer| !state.screenshots.iter().any(|shot| screenshot_key(shot.path.as_path()) == viewer.screenshot_key))
    {
        state.screenshot_viewer = None;
    }
}

pub fn key_of(path: &Path) -> String {
    screenshot_key(path)
}

pub fn request_screenshot_delete(state: &mut InstanceScreenState, key: &str) {
    let Some(shot) = state.screenshots.iter().find(|shot| screenshot_key(shot.path.as_path()) == key).cloned() else { return };
    request_instance_screenshot_delete(state, key.to_owned(), shot.path, shot.file_name);
}

// -- Logs -----------------------------------------------------------------------------------------------------------

pub fn logs_tick(state: &mut InstanceScreenState, instance_root: &Path) {
    let should_scan = state.last_log_scan_at.is_none_or(|last| last.elapsed() >= INSTANCE_LOG_SCAN_INTERVAL);
    if should_scan {
        refresh_instance_logs(state, instance_root, false);
    }
    sync_selected_instance_log(state);
}

pub fn logs_scan_interval() -> Duration {
    INSTANCE_LOG_SCAN_INTERVAL
}

pub fn select_log(state: &mut InstanceScreenState, path: PathBuf) {
    state.selected_log_path = Some(path);
    load_selected_instance_log(state);
}

pub fn log_size_label(size_bytes: u64) -> String {
    format_log_file_size(size_bytes)
}

pub const MAX_LOG_LINES: usize = MAX_INSTANCE_LOG_LINES;

// -- Exports --------------------------------------------------------------------------------------------------------

/// The top-level entries of the instance root the export dialogs offer, kept fresh in the background.
pub fn root_entries(state: &mut InstanceScreenState, server: bool, root: &Path) -> (Vec<(String, bool)>, bool, bool) {
    let cache = if server { &mut state.export_server_root_entries } else { &mut state.export_vtmpack_root_entries };
    let listing = cache.poll(root);
    let entries = listing.entries.iter().map(|entry| (entry.name.clone(), entry.is_dir)).collect();
    (entries, listing.loaded, listing.root_exists)
}

pub fn sync_vtmpack_options(state: &mut InstanceScreenState, root: &Path) {
    vtmpack::sync_vtmpack_export_options(root, &mut state.export_vtmpack_options);
}

pub fn sync_server_options(state: &mut InstanceScreenState, root: &Path) {
    sync_server_export_options(root, &mut state.export_server_included_root_entries);
}

pub fn default_vtmpack_selection(entry: &str) -> bool {
    vtmpack::default_vtmpack_root_entry_selected(entry)
}

pub fn start_vtmpack_export(state: &mut InstanceScreenState, instance: &InstanceRecord, root: PathBuf, output: PathBuf) {
    let metadata = VtmpackInstanceMetadata {
        id: instance.id.clone(),
        name: instance.name.clone(),
        game_version: instance.game_version.clone(),
        modloader: instance.modloader.clone(),
        modloader_version: instance.modloader_version.clone(),
    };
    let options = state.export_vtmpack_options.clone();
    request_vtmpack_export(state, metadata, root, vtmpack::enforce_vtmpack_extension(output), options);
}

pub fn start_vtmpatch_export(state: &mut InstanceScreenState, instance_name: String, root: PathBuf, base: PathBuf, output: PathBuf) {
    let options = state.export_vtmpack_options.clone();
    request_vtmpatch_export(state, instance_name, root, base, vtmpack::enforce_vtmpatch_extension(output), options);
}

pub fn default_vtmpack_name(instance_name: &str) -> String {
    vtmpack::default_vtmpack_file_name(instance_name)
}

pub fn default_vtmpatch_name(instance_name: &str) -> String {
    vtmpack::default_vtmpatch_file_name(instance_name)
}

pub fn start_server_export(state: &mut InstanceScreenState, instance: &InstanceRecord, root: PathBuf, config: &Config) {
    let output = default_server_export_output_path(instance, config);
    let included = state.export_server_included_root_entries.clone();
    request_server_export(state, instance.clone(), root, output, included, config.force_java_21_minimum());
}

// -- Moving ---------------------------------------------------------------------------------------------------------

/// Checks a typed move destination; `Err` says what is wrong with it.
pub fn validate_move_destination(text: &str) -> Result<PathBuf, String> {
    validate_move_dest(text)
}

pub fn start_move(state: &mut InstanceScreenState, source: PathBuf, destination: PathBuf) {
    request_move_instance(state, source, destination);
}

pub fn move_bytes_label(bytes: u64) -> String {
    super::instance_move_modal::format_bytes(bytes)
}

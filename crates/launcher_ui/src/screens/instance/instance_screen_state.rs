use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct MoveInstanceProgress {
    pub total_bytes: u64,
    pub bytes_done: u64,
    pub total_files: usize,
    pub files_done: usize,
    pub active_file_count: usize,
    pub active_files: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum MoveInstanceResult {
    Complete {
        dest_path: PathBuf,
    },
    #[allow(dead_code)]
    Failed {
        reason: String,
    },
}
use std::time::Instant;

use config::Config;
use content_resolver::{InstalledContentHashCache, InstalledContentKind, ResolvedInstalledContent};
use installation::{
    InstallProgress, LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry, VersionCatalog,
    VersionCatalogFilter,
};
use vtmpack::{VtmpackExportOptions, VtmpackExportProgress, VtmpackExportStats};

use super::ContentApplyResult;
use super::ContentLookupResult;
use super::INSTALLED_CONTENT_PAGE_SIZES;
use super::InstalledContentCache;
use super::RuntimePrepareOutcome;

#[derive(Clone, Debug)]
pub struct InstalledContentEntryUiCache {
    pub description_source: String,
    pub description_width_bucket: u32,
    pub truncated_description: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InstanceScreenTab {
    #[default]
    Content,
    ScreenshotGallery,
    Logs,
}

impl InstanceScreenTab {
    pub const ALL: [Self; 3] = [Self::Content, Self::ScreenshotGallery, Self::Logs];

    pub fn label(self) -> &'static str {
        match self {
            Self::Content => "Content",
            Self::ScreenshotGallery => "Screenshot Gallery",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceScreenshotEntry {
    pub path: PathBuf,
    pub file_name: String,
    pub width: u32,
    pub height: u32,
    pub modified_at_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct InstanceScreenshotViewerState {
    pub screenshot_key: String,
    pub zoom: f32,
    pub pan_uv: crate::geom::Vec2,
}

#[derive(Clone, Debug)]
pub struct InstanceLogEntry {
    pub path: PathBuf,
    pub file_name: String,
    pub modified_at_ms: Option<u64>,
    pub size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct VtmpackExportOutcome {
    pub instance_name: String,
    pub output_path: PathBuf,
    pub is_patch: bool,
    pub result: Result<VtmpackExportStats, String>,
}

#[derive(Clone, Debug)]
pub struct ServerExportOutcome {
    pub instance_name: String,
    pub output_path: PathBuf,
    pub result: Result<String, String>,
}

#[derive(Clone, Debug)]
pub struct InstanceScreenState {
    pub running: bool,
    pub status_message: Option<String>,
    pub name_input: String,
    pub description_input: String,
    pub thumbnail_input: PathBuf,
    pub selected_modloader: usize,
    pub custom_modloader: String,
    pub game_version_input: String,
    pub modloader_version_input: String,
    pub memory_override_enabled: bool,
    pub memory_override_mib: u128,
    pub cli_args_input: String,
    pub env_vars_input: String,
    pub java_override_enabled: bool,
    pub java_override_runtime_major: Option<u8>,
    pub linux_set_opengl_driver: bool,
    pub linux_use_zink_driver: bool,
    pub discord_rich_presence_mod_installed: bool,
    pub runtime_overrides_saved_fingerprint: Option<String>,
    pub runtime_overrides_dirty_since: Option<(String, std::time::Instant)>,
    pub selected_content_tab: InstalledContentKind,
    pub installed_content_page_size: usize,
    pub installed_content_page: usize,
    pub installed_content_search_query: String,
    /// Only list entries with a known available update.
    pub installed_content_updatable_only: bool,
    pub installed_content_cache: InstalledContentCache,
    pub installed_content_entry_ui_cache: HashMap<String, InstalledContentEntryUiCache>,
    pub content_metadata_cache: HashMap<String, Option<ResolvedInstalledContent>>,
    pub content_hash_cache: Option<InstalledContentHashCache>,
    pub content_hash_cache_load_in_flight: bool,
    pub content_hash_cache_load_results:
        launcher_runtime::WorkerChannel<Result<InstalledContentHashCache, String>>,
    pub content_hash_cache_dirty: bool,
    pub content_hash_cache_dirty_since: Option<Instant>,
    pub content_hash_cache_serial: u64,
    pub content_hash_cache_save_in_flight: bool,
    pub content_hash_cache_save_results:
        launcher_runtime::WorkerChannel<(u64, Result<(), String>)>,
    pub content_apply_in_flight: bool,
    pub content_apply_results: launcher_runtime::WorkerChannel<ContentApplyResult>,
    pub content_lookup_in_flight: HashSet<String>,
    pub content_lookup_request_serial: u64,
    pub content_lookup_latest_serial_by_key: HashMap<String, u64>,
    pub content_lookup_retry_after_by_key: HashMap<String, Instant>,
    pub content_lookup_failure_count_by_key: HashMap<String, u8>,
    pub content_lookup_results: launcher_runtime::WorkerChannel<ContentLookupResult>,
    pub available_game_versions: Vec<MinecraftVersionEntry>,
    pub selected_game_version_index: usize,
    pub loader_support: LoaderSupportIndex,
    pub loader_versions: LoaderVersionIndex,
    pub modloader_versions_cache: BTreeMap<String, Vec<String>>,
    pub modloader_versions_in_flight: HashSet<String>,
    pub modloader_versions_results:
        launcher_runtime::WorkerChannel<(String, Result<Vec<String>, String>)>,
    pub modloader_versions_status_key: Option<String>,
    pub modloader_versions_status: Option<String>,
    pub incompatible_modloader_version_warning_key: Option<String>,
    pub version_catalog_filter: Option<VersionCatalogFilter>,
    pub version_catalog_error: Option<String>,
    pub version_catalog_in_flight: bool,
    pub version_catalog:
        launcher_runtime::WorkerChannel<(VersionCatalogFilter, Result<VersionCatalog, String>)>,
    pub runtime_prepare_in_flight: bool,
    pub runtime_prepare_results:
        launcher_runtime::WorkerChannel<(String, String, Result<RuntimePrepareOutcome, String>)>,
    pub runtime_progress: launcher_runtime::WorkerChannel<InstallProgress>,
    pub runtime_latest_progress: Option<InstallProgress>,
    pub runtime_last_notification_at: Option<Instant>,
    pub runtime_prepare_instance_root: Option<String>,
    pub runtime_prepare_user_key: Option<String>,
    pub active_tab: InstanceScreenTab,
    pub screenshots: Vec<InstanceScreenshotEntry>,
    pub last_screenshot_scan_at: Option<Instant>,
    pub screenshot_scan_in_flight: bool,
    pub screenshot_scan_request_serial: u64,
    pub screenshot_scan_results:
        launcher_runtime::WorkerChannel<(u64, Vec<InstanceScreenshotEntry>)>,
    pub screenshot_layout_revision: u64,
    pub screenshot_viewer: Option<InstanceScreenshotViewerState>,
    pub pending_delete_screenshot_key: Option<String>,
    pub delete_screenshot_in_flight: bool,
    pub delete_screenshot_results:
        launcher_runtime::WorkerChannel<(String, String, Result<(), String>)>,
    pub logs: Vec<InstanceLogEntry>,
    pub last_log_scan_at: Option<Instant>,
    pub log_scan_in_flight: bool,
    pub log_scan_request_serial: u64,
    pub log_scan_results: launcher_runtime::WorkerChannel<(u64, Vec<InstanceLogEntry>)>,
    pub selected_log_path: Option<PathBuf>,
    pub loaded_log_path: Option<PathBuf>,
    pub loaded_log_modified_at_ms: Option<u64>,
    pub loaded_log_lines: Vec<String>,
    pub loaded_log_error: Option<String>,
    pub loaded_log_truncated: bool,
    pub log_load_in_flight: bool,
    pub log_load_request_serial: u64,
    pub requested_log_load_path: Option<PathBuf>,
    pub requested_log_load_modified_at_ms: Option<u64>,
    pub log_load_results: launcher_runtime::WorkerChannel<(
        u64,
        PathBuf,
        Option<u64>,
        Result<(Vec<String>, bool), String>,
    )>,
    pub show_settings_modal: bool,
    pub show_export_vtmpack_modal: bool,
    pub show_export_server_modal: bool,
    pub export_vtmpack_patch_mode: bool,
    /// Base pack chosen for a patch export, waiting for the output location dialog.
    pub export_patch_base_path: Option<PathBuf>,
    pub export_vtmpack_root_entries: super::root_entries::RootEntries,
    pub export_server_root_entries: super::root_entries::RootEntries,
    pub export_vtmpack_options: VtmpackExportOptions,
    pub export_vtmpack_in_flight: bool,
    pub export_vtmpack_output_path: Option<PathBuf>,
    pub export_vtmpack_progress: launcher_runtime::WorkerChannel<VtmpackExportProgress>,
    pub export_vtmpack_latest_progress: Option<VtmpackExportProgress>,
    pub export_vtmpack_results: launcher_runtime::WorkerChannel<VtmpackExportOutcome>,
    pub export_server_included_root_entries: BTreeMap<String, bool>,
    pub export_server_in_flight: bool,
    pub export_server_output_path: Option<PathBuf>,
    pub export_server_progress: launcher_runtime::WorkerChannel<VtmpackExportProgress>,
    pub export_server_latest_progress: Option<VtmpackExportProgress>,
    pub export_server_results: launcher_runtime::WorkerChannel<ServerExportOutcome>,
    pub show_move_instance_modal: bool,
    pub show_move_instance_progress_modal: bool,
    pub move_instance_dest_input: String,
    pub move_instance_dest_valid: bool,
    pub move_instance_dest_error: Option<String>,
    pub move_instance_in_flight: bool,
    pub move_instance_latest_progress: Option<MoveInstanceProgress>,
    pub move_instance_dest_path: Option<PathBuf>,
    pub move_instance_completion_message: Option<String>,
    pub move_instance_completion_failed: bool,
    pub move_instance_last_layout_log_at: Option<Instant>,
    pub move_instance_pending_result: Option<MoveInstanceResult>,
    pub move_instance_progress_visible_until: Option<Instant>,
    pub move_instance_progress: launcher_runtime::WorkerChannel<MoveInstanceProgress>,
    pub move_instance_results: launcher_runtime::WorkerChannel<MoveInstanceResult>,
    pub launch_username: Option<String>,
    pub launch_user_key: Option<String>,
}

impl InstanceScreenState {
    pub fn from_instance(instance: &instances::InstanceRecord, config: &Config) -> Self {
        let (selected_modloader, custom_modloader) = super::runtime::split_modloader(&instance.modloader);
        let linux_set_opengl_driver = instances::linux_graphics_override_enabled(instance);
        let (_, linux_use_zink_driver) = instances::effective_linux_graphics_settings(
            instance,
            config.linux_set_opengl_driver(),
            config.linux_use_zink_driver(),
        );
        Self {
            running: false,
            status_message: None,
            name_input: instance.name.clone(),
            description_input: instance.description.clone().unwrap_or_default(),
            thumbnail_input: instance.thumbnail_path.clone().unwrap_or_default(),
            selected_modloader,
            custom_modloader,
            game_version_input: instance.game_version.clone(),
            modloader_version_input: instance.modloader_version.clone(),
            memory_override_enabled: instance.max_memory_mib.is_some(),
            memory_override_mib: instance
                .max_memory_mib
                .unwrap_or(config.default_instance_max_memory_mib()),
            cli_args_input: instance
                .cli_args
                .clone()
                .unwrap_or_else(|| config.default_instance_cli_args().to_owned()),
            env_vars_input: instance.env_vars.clone().unwrap_or_default(),
            java_override_enabled: instance.java_override_enabled,
            java_override_runtime_major: instance.java_override_runtime_major,
            linux_set_opengl_driver,
            linux_use_zink_driver,
            discord_rich_presence_mod_installed: instance.discord_rich_presence_mod_installed,
            runtime_overrides_saved_fingerprint: None,
            runtime_overrides_dirty_since: None,
            selected_content_tab: InstalledContentKind::Mods,
            installed_content_page_size: INSTALLED_CONTENT_PAGE_SIZES[1],
            installed_content_page: 1,
            installed_content_search_query: String::new(),
            installed_content_updatable_only: false,
            installed_content_cache: InstalledContentCache::default(),
            installed_content_entry_ui_cache: HashMap::new(),
            content_metadata_cache: HashMap::new(),
            content_hash_cache: None,
            content_hash_cache_load_in_flight: false,
            content_hash_cache_load_results: Default::default(),
            content_hash_cache_dirty: false,
            content_hash_cache_dirty_since: None,
            content_hash_cache_serial: 0,
            content_hash_cache_save_in_flight: false,
            content_hash_cache_save_results: Default::default(),
            content_apply_in_flight: false,
            content_apply_results: Default::default(),
            content_lookup_in_flight: HashSet::new(),
            content_lookup_request_serial: 0,
            content_lookup_latest_serial_by_key: HashMap::new(),
            content_lookup_retry_after_by_key: HashMap::new(),
            content_lookup_failure_count_by_key: HashMap::new(),
            content_lookup_results: Default::default(),
            available_game_versions: Vec::new(),
            selected_game_version_index: 0,
            loader_support: LoaderSupportIndex::default(),
            loader_versions: LoaderVersionIndex::default(),
            modloader_versions_cache: BTreeMap::new(),
            modloader_versions_in_flight: HashSet::new(),
            modloader_versions_results: Default::default(),
            modloader_versions_status_key: None,
            modloader_versions_status: None,
            incompatible_modloader_version_warning_key: None,
            version_catalog_filter: None,
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog: Default::default(),
            runtime_prepare_in_flight: false,
            runtime_prepare_results: Default::default(),
            runtime_progress: Default::default(),
            runtime_latest_progress: None,
            runtime_last_notification_at: None,
            runtime_prepare_instance_root: None,
            runtime_prepare_user_key: None,
            active_tab: InstanceScreenTab::Content,
            screenshots: Vec::new(),
            last_screenshot_scan_at: None,
            screenshot_scan_in_flight: false,
            screenshot_scan_request_serial: 0,
            screenshot_scan_results: Default::default(),
            screenshot_layout_revision: 0,
            screenshot_viewer: None,
            pending_delete_screenshot_key: None,
            delete_screenshot_in_flight: false,
            delete_screenshot_results: Default::default(),
            logs: Vec::new(),
            last_log_scan_at: None,
            log_scan_in_flight: false,
            log_scan_request_serial: 0,
            log_scan_results: Default::default(),
            selected_log_path: None,
            loaded_log_path: None,
            loaded_log_modified_at_ms: None,
            loaded_log_lines: Vec::new(),
            loaded_log_error: None,
            loaded_log_truncated: false,
            log_load_in_flight: false,
            log_load_request_serial: 0,
            requested_log_load_path: None,
            requested_log_load_modified_at_ms: None,
            log_load_results: Default::default(),
            show_settings_modal: false,
            show_export_vtmpack_modal: false,
            show_export_server_modal: false,
            export_vtmpack_patch_mode: false,
            export_patch_base_path: None,
            export_vtmpack_root_entries: Default::default(),
            export_server_root_entries: Default::default(),
            export_vtmpack_options: VtmpackExportOptions::default(),
            export_vtmpack_in_flight: false,
            export_vtmpack_output_path: None,
            export_vtmpack_progress: Default::default(),
            export_vtmpack_latest_progress: None,
            export_vtmpack_results: Default::default(),
            export_server_included_root_entries: BTreeMap::new(),
            export_server_in_flight: false,
            export_server_output_path: None,
            export_server_progress: Default::default(),
            export_server_latest_progress: None,
            export_server_results: Default::default(),
            show_move_instance_modal: false,
            show_move_instance_progress_modal: false,
            move_instance_dest_input: String::new(),
            move_instance_dest_valid: false,
            move_instance_dest_error: None,
            move_instance_in_flight: false,
            move_instance_latest_progress: None,
            move_instance_dest_path: None,
            move_instance_completion_message: None,
            move_instance_completion_failed: false,
            move_instance_last_layout_log_at: None,
            move_instance_pending_result: None,
            move_instance_progress_visible_until: None,
            move_instance_progress: Default::default(),
            move_instance_results: Default::default(),
            launch_username: None,
            launch_user_key: None,
        }
    }

    pub fn invalidate_installed_content_cache(&mut self) {
        self.installed_content_cache.clear();
        self.installed_content_entry_ui_cache.clear();
    }

    pub fn mark_screenshot_layout_dirty(&mut self) {
        self.screenshot_layout_revision = self.screenshot_layout_revision.saturating_add(1);
    }




}

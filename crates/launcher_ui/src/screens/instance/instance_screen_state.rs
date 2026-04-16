use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

#[derive(Clone, Debug)]
pub(super) struct MoveInstanceProgress {
    pub(super) total_bytes: u64,
    pub(super) bytes_done: u64,
    pub(super) total_files: usize,
    pub(super) files_done: usize,
    pub(super) active_file_count: usize,
    pub(super) active_files: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) enum MoveInstanceResult {
    Complete {
        dest_path: PathBuf,
    },
    #[allow(dead_code)]
    Failed {
        reason: String,
    },
}
use std::time::Instant;

use crate::ui::components::lazy_image_bytes::LazyImageBytes;
use crate::ui::components::virtual_masonry::CachedVirtualMasonryLayout;
use config::Config;
use content_resolver::{InstalledContentHashCache, InstalledContentKind, ResolvedInstalledContent};
use installation::{
    InstallProgress, LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry, VersionCatalog,
    VersionCatalogFilter,
};
use vtmpack::{VtmpackExportOptions, VtmpackExportProgress, VtmpackExportStats};

use super::{
    split_modloader, ContentApplyResult, ContentLookupResult, InstalledContentCache,
    RuntimePrepareOutcome, INSTALLED_CONTENT_PAGE_SIZES,
};

#[derive(Clone, Debug)]
pub(super) struct InstalledContentEntryUiCache {
    pub(super) description_source: String,
    pub(super) description_width_bucket: u32,
    pub(super) truncated_description: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum InstanceScreenTab {
    #[default]
    Content,
    ScreenshotGallery,
    Logs,
}

impl InstanceScreenTab {
    pub(super) const ALL: [Self; 3] = [Self::Content, Self::ScreenshotGallery, Self::Logs];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Content => "Content",
            Self::ScreenshotGallery => "Screenshot Gallery",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct InstanceScreenshotEntry {
    pub(super) path: PathBuf,
    pub(super) file_name: String,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) modified_at_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct InstanceScreenshotViewerState {
    pub(super) screenshot_key: String,
    pub(super) zoom: f32,
    pub(super) pan_uv: egui::Vec2,
}

#[derive(Clone, Debug)]
pub(super) struct InstanceLogEntry {
    pub(super) path: PathBuf,
    pub(super) file_name: String,
    pub(super) modified_at_ms: Option<u64>,
    pub(super) size_bytes: u64,
}

#[derive(Clone, Debug)]
pub(super) struct VtmpackExportOutcome {
    pub(super) instance_name: String,
    pub(super) output_path: PathBuf,
    pub(super) result: Result<VtmpackExportStats, String>,
}

#[derive(Clone, Debug)]
pub(super) struct ServerExportOutcome {
    pub(super) instance_name: String,
    pub(super) output_path: PathBuf,
    pub(super) result: Result<String, String>,
}

#[derive(Clone, Debug)]
pub(super) struct InstanceScreenState {
    pub(super) running: bool,
    pub(super) status_message: Option<String>,
    pub(super) name_input: String,
    pub(super) description_input: String,
    pub(super) thumbnail_input: PathBuf,
    pub(super) selected_modloader: usize,
    pub(super) custom_modloader: String,
    pub(super) game_version_input: String,
    pub(super) modloader_version_input: String,
    pub(super) memory_override_enabled: bool,
    pub(super) memory_override_mib: u128,
    pub(super) cli_args_input: String,
    pub(super) env_vars_input: String,
    pub(super) java_override_enabled: bool,
    pub(super) java_override_runtime_major: Option<u8>,
    pub(super) linux_set_opengl_driver: bool,
    pub(super) linux_use_zink_driver: bool,
    pub(super) discord_rich_presence_mod_installed: bool,
    pub(super) selected_content_tab: InstalledContentKind,
    pub(super) installed_content_page_size: usize,
    pub(super) installed_content_page: usize,
    pub(super) installed_content_cache: InstalledContentCache,
    pub(super) installed_content_entry_ui_cache: HashMap<String, InstalledContentEntryUiCache>,
    pub(super) content_metadata_cache: HashMap<String, Option<ResolvedInstalledContent>>,
    pub(super) content_hash_cache: Option<InstalledContentHashCache>,
    pub(super) content_hash_cache_dirty: bool,
    pub(super) content_hash_cache_dirty_since: Option<Instant>,
    pub(super) content_hash_cache_serial: u64,
    pub(super) content_hash_cache_save_in_flight: bool,
    pub(super) content_hash_cache_save_results_tx: Option<mpsc::Sender<(u64, Result<(), String>)>>,
    pub(super) content_hash_cache_save_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(u64, Result<(), String>)>>>>,
    pub(super) content_apply_in_flight: bool,
    pub(super) content_apply_results_tx: Option<mpsc::Sender<ContentApplyResult>>,
    pub(super) content_apply_results_rx: Option<Arc<Mutex<mpsc::Receiver<ContentApplyResult>>>>,
    pub(super) content_lookup_in_flight: HashSet<String>,
    pub(super) content_lookup_request_serial: u64,
    pub(super) content_lookup_latest_serial_by_key: HashMap<String, u64>,
    pub(super) content_lookup_retry_after_by_key: HashMap<String, Instant>,
    pub(super) content_lookup_failure_count_by_key: HashMap<String, u8>,
    pub(super) content_lookup_results_tx: Option<mpsc::Sender<ContentLookupResult>>,
    pub(super) content_lookup_results_rx: Option<Arc<Mutex<mpsc::Receiver<ContentLookupResult>>>>,
    pub(super) available_game_versions: Vec<MinecraftVersionEntry>,
    pub(super) selected_game_version_index: usize,
    pub(super) loader_support: LoaderSupportIndex,
    pub(super) loader_versions: LoaderVersionIndex,
    pub(super) modloader_versions_cache: BTreeMap<String, Vec<String>>,
    pub(super) modloader_versions_in_flight: HashSet<String>,
    pub(super) modloader_versions_results_tx:
        Option<mpsc::Sender<(String, Result<Vec<String>, String>)>>,
    pub(super) modloader_versions_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, Result<Vec<String>, String>)>>>>,
    pub(super) modloader_versions_status_key: Option<String>,
    pub(super) modloader_versions_status: Option<String>,
    pub(super) incompatible_modloader_version_warning_key: Option<String>,
    pub(super) version_catalog_filter: Option<VersionCatalogFilter>,
    pub(super) version_catalog_error: Option<String>,
    pub(super) version_catalog_in_flight: bool,
    pub(super) version_catalog_results_tx:
        Option<mpsc::Sender<(VersionCatalogFilter, Result<VersionCatalog, String>)>>,
    pub(super) version_catalog_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(VersionCatalogFilter, Result<VersionCatalog, String>)>>>>,
    pub(super) runtime_prepare_in_flight: bool,
    pub(super) runtime_prepare_results_tx:
        Option<mpsc::Sender<(String, String, Result<RuntimePrepareOutcome, String>)>>,
    pub(super) runtime_prepare_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, String, Result<RuntimePrepareOutcome, String>)>>>>,
    pub(super) runtime_progress_tx: Option<mpsc::Sender<InstallProgress>>,
    pub(super) runtime_progress_rx: Option<Arc<Mutex<mpsc::Receiver<InstallProgress>>>>,
    pub(super) runtime_latest_progress: Option<InstallProgress>,
    pub(super) runtime_last_notification_at: Option<Instant>,
    pub(super) runtime_prepare_instance_root: Option<String>,
    pub(super) runtime_prepare_user_key: Option<String>,
    pub(super) active_tab: InstanceScreenTab,
    pub(super) screenshots: Vec<InstanceScreenshotEntry>,
    pub(super) last_screenshot_scan_at: Option<Instant>,
    pub(super) screenshot_scan_in_flight: bool,
    pub(super) screenshot_scan_request_serial: u64,
    pub(super) screenshot_scan_results_tx:
        Option<mpsc::Sender<(u64, Vec<InstanceScreenshotEntry>)>>,
    pub(super) screenshot_scan_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(u64, Vec<InstanceScreenshotEntry>)>>>>,
    pub(super) screenshot_images: LazyImageBytes,
    pub(super) screenshot_layout_revision: u64,
    pub(super) screenshot_masonry_layout_cache: Option<CachedVirtualMasonryLayout>,
    pub(super) screenshot_viewer: Option<InstanceScreenshotViewerState>,
    pub(super) pending_delete_screenshot_key: Option<String>,
    pub(super) delete_screenshot_in_flight: bool,
    pub(super) delete_screenshot_results_tx:
        Option<mpsc::Sender<(String, String, Result<(), String>)>>,
    pub(super) delete_screenshot_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, String, Result<(), String>)>>>>,
    pub(super) logs: Vec<InstanceLogEntry>,
    pub(super) last_log_scan_at: Option<Instant>,
    pub(super) log_scan_in_flight: bool,
    pub(super) log_scan_request_serial: u64,
    pub(super) log_scan_results_tx: Option<mpsc::Sender<(u64, Vec<InstanceLogEntry>)>>,
    pub(super) log_scan_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(u64, Vec<InstanceLogEntry>)>>>>,
    pub(super) selected_log_path: Option<PathBuf>,
    pub(super) loaded_log_path: Option<PathBuf>,
    pub(super) loaded_log_modified_at_ms: Option<u64>,
    pub(super) loaded_log_lines: Vec<String>,
    pub(super) loaded_log_error: Option<String>,
    pub(super) loaded_log_truncated: bool,
    pub(super) log_load_in_flight: bool,
    pub(super) log_load_request_serial: u64,
    pub(super) requested_log_load_path: Option<PathBuf>,
    pub(super) requested_log_load_modified_at_ms: Option<u64>,
    pub(super) log_load_results_tx: Option<
        mpsc::Sender<(
            u64,
            PathBuf,
            Option<u64>,
            Result<(Vec<String>, bool), String>,
        )>,
    >,
    pub(super) log_load_results_rx: Option<
        Arc<
            Mutex<
                mpsc::Receiver<(
                    u64,
                    PathBuf,
                    Option<u64>,
                    Result<(Vec<String>, bool), String>,
                )>,
            >,
        >,
    >,
    pub(super) show_settings_modal: bool,
    pub(super) show_export_vtmpack_modal: bool,
    pub(super) show_export_server_modal: bool,
    pub(super) export_vtmpack_options: VtmpackExportOptions,
    pub(super) export_vtmpack_in_flight: bool,
    pub(super) export_vtmpack_output_path: Option<PathBuf>,
    pub(super) export_vtmpack_progress_tx: Option<mpsc::Sender<VtmpackExportProgress>>,
    pub(super) export_vtmpack_progress_rx:
        Option<Arc<Mutex<mpsc::Receiver<VtmpackExportProgress>>>>,
    pub(super) export_vtmpack_latest_progress: Option<VtmpackExportProgress>,
    pub(super) export_vtmpack_results_tx: Option<mpsc::Sender<VtmpackExportOutcome>>,
    pub(super) export_vtmpack_results_rx: Option<Arc<Mutex<mpsc::Receiver<VtmpackExportOutcome>>>>,
    pub(super) export_server_included_root_entries: BTreeMap<String, bool>,
    pub(super) export_server_in_flight: bool,
    pub(super) export_server_output_path: Option<PathBuf>,
    pub(super) export_server_progress_tx: Option<mpsc::Sender<VtmpackExportProgress>>,
    pub(super) export_server_progress_rx: Option<Arc<Mutex<mpsc::Receiver<VtmpackExportProgress>>>>,
    pub(super) export_server_latest_progress: Option<VtmpackExportProgress>,
    pub(super) export_server_results_tx: Option<mpsc::Sender<ServerExportOutcome>>,
    pub(super) export_server_results_rx: Option<Arc<Mutex<mpsc::Receiver<ServerExportOutcome>>>>,
    pub(super) show_move_instance_modal: bool,
    pub(super) show_move_instance_progress_modal: bool,
    pub(super) move_instance_dest_input: String,
    pub(super) move_instance_dest_valid: bool,
    pub(super) move_instance_dest_error: Option<String>,
    pub(super) move_instance_in_flight: bool,
    pub(super) move_instance_latest_progress: Option<MoveInstanceProgress>,
    pub(super) move_instance_dest_path: Option<PathBuf>,
    pub(super) move_instance_completion_message: Option<String>,
    pub(super) move_instance_completion_failed: bool,
    pub(super) move_instance_last_layout_log_at: Option<Instant>,
    pub(super) move_instance_pending_result: Option<MoveInstanceResult>,
    pub(super) move_instance_progress_visible_until: Option<Instant>,
    pub(super) move_instance_progress_tx: Option<mpsc::Sender<MoveInstanceProgress>>,
    pub(super) move_instance_progress_rx: Option<Arc<Mutex<mpsc::Receiver<MoveInstanceProgress>>>>,
    pub(super) move_instance_results_tx: Option<mpsc::Sender<MoveInstanceResult>>,
    pub(super) move_instance_results_rx: Option<Arc<Mutex<mpsc::Receiver<MoveInstanceResult>>>>,
    pub(super) launch_username: Option<String>,
    pub(super) launch_user_key: Option<String>,
}

impl InstanceScreenState {
    pub(super) fn from_instance(instance: &instances::InstanceRecord, config: &Config) -> Self {
        let (selected_modloader, custom_modloader) = split_modloader(&instance.modloader);
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
            selected_content_tab: InstalledContentKind::Mods,
            installed_content_page_size: INSTALLED_CONTENT_PAGE_SIZES[1],
            installed_content_page: 1,
            installed_content_cache: InstalledContentCache::default(),
            installed_content_entry_ui_cache: HashMap::new(),
            content_metadata_cache: HashMap::new(),
            content_hash_cache: None,
            content_hash_cache_dirty: false,
            content_hash_cache_dirty_since: None,
            content_hash_cache_serial: 0,
            content_hash_cache_save_in_flight: false,
            content_hash_cache_save_results_tx: None,
            content_hash_cache_save_results_rx: None,
            content_apply_in_flight: false,
            content_apply_results_tx: None,
            content_apply_results_rx: None,
            content_lookup_in_flight: HashSet::new(),
            content_lookup_request_serial: 0,
            content_lookup_latest_serial_by_key: HashMap::new(),
            content_lookup_retry_after_by_key: HashMap::new(),
            content_lookup_failure_count_by_key: HashMap::new(),
            content_lookup_results_tx: None,
            content_lookup_results_rx: None,
            available_game_versions: Vec::new(),
            selected_game_version_index: 0,
            loader_support: LoaderSupportIndex::default(),
            loader_versions: LoaderVersionIndex::default(),
            modloader_versions_cache: BTreeMap::new(),
            modloader_versions_in_flight: HashSet::new(),
            modloader_versions_results_tx: None,
            modloader_versions_results_rx: None,
            modloader_versions_status_key: None,
            modloader_versions_status: None,
            incompatible_modloader_version_warning_key: None,
            version_catalog_filter: None,
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog_results_tx: None,
            version_catalog_results_rx: None,
            runtime_prepare_in_flight: false,
            runtime_prepare_results_tx: None,
            runtime_prepare_results_rx: None,
            runtime_progress_tx: None,
            runtime_progress_rx: None,
            runtime_latest_progress: None,
            runtime_last_notification_at: None,
            runtime_prepare_instance_root: None,
            runtime_prepare_user_key: None,
            active_tab: InstanceScreenTab::Content,
            screenshots: Vec::new(),
            last_screenshot_scan_at: None,
            screenshot_scan_in_flight: false,
            screenshot_scan_request_serial: 0,
            screenshot_scan_results_tx: None,
            screenshot_scan_results_rx: None,
            screenshot_images: LazyImageBytes::default(),
            screenshot_layout_revision: 0,
            screenshot_masonry_layout_cache: None,
            screenshot_viewer: None,
            pending_delete_screenshot_key: None,
            delete_screenshot_in_flight: false,
            delete_screenshot_results_tx: None,
            delete_screenshot_results_rx: None,
            logs: Vec::new(),
            last_log_scan_at: None,
            log_scan_in_flight: false,
            log_scan_request_serial: 0,
            log_scan_results_tx: None,
            log_scan_results_rx: None,
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
            log_load_results_tx: None,
            log_load_results_rx: None,
            show_settings_modal: false,
            show_export_vtmpack_modal: false,
            show_export_server_modal: false,
            export_vtmpack_options: VtmpackExportOptions::default(),
            export_vtmpack_in_flight: false,
            export_vtmpack_output_path: None,
            export_vtmpack_progress_tx: None,
            export_vtmpack_progress_rx: None,
            export_vtmpack_latest_progress: None,
            export_vtmpack_results_tx: None,
            export_vtmpack_results_rx: None,
            export_server_included_root_entries: BTreeMap::new(),
            export_server_in_flight: false,
            export_server_output_path: None,
            export_server_progress_tx: None,
            export_server_progress_rx: None,
            export_server_latest_progress: None,
            export_server_results_tx: None,
            export_server_results_rx: None,
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
            move_instance_progress_tx: None,
            move_instance_progress_rx: None,
            move_instance_results_tx: None,
            move_instance_results_rx: None,
            launch_username: None,
            launch_user_key: None,
        }
    }

    pub(super) fn invalidate_installed_content_cache(&mut self) {
        self.installed_content_cache.clear();
        self.installed_content_entry_ui_cache.clear();
    }

    pub(super) fn mark_screenshot_layout_dirty(&mut self) {
        self.screenshot_layout_revision = self.screenshot_layout_revision.saturating_add(1);
        self.screenshot_masonry_layout_cache = None;
    }

    pub(super) fn purge_screenshot_state(&mut self, ctx: &egui::Context) {
        self.screenshots.clear();
        self.last_screenshot_scan_at = None;
        self.screenshot_scan_in_flight = false;
        self.screenshot_scan_results_tx = None;
        self.screenshot_scan_results_rx = None;
        self.screenshot_images.clear(ctx);
        self.screenshot_viewer = None;
        self.pending_delete_screenshot_key = None;
        self.delete_screenshot_in_flight = false;
        self.delete_screenshot_results_tx = None;
        self.delete_screenshot_results_rx = None;
        self.mark_screenshot_layout_dirty();
    }

    pub(super) fn purge_heavy_state(&mut self, ctx: &egui::Context) {
        self.status_message = None;
        self.selected_content_tab = InstalledContentKind::Mods;
        self.installed_content_page = 1;
        self.installed_content_cache.clear();
        self.installed_content_entry_ui_cache.clear();
        self.content_metadata_cache.clear();
        self.content_hash_cache = None;
        self.content_hash_cache_dirty = false;
        self.content_hash_cache_dirty_since = None;
        self.content_hash_cache_serial = 0;
        self.content_hash_cache_save_in_flight = false;
        self.content_hash_cache_save_results_tx = None;
        self.content_hash_cache_save_results_rx = None;
        self.content_apply_in_flight = false;
        self.content_apply_results_tx = None;
        self.content_apply_results_rx = None;
        self.content_lookup_in_flight.clear();
        self.content_lookup_request_serial = 0;
        self.content_lookup_latest_serial_by_key.clear();
        self.content_lookup_retry_after_by_key.clear();
        self.content_lookup_failure_count_by_key.clear();
        self.content_lookup_results_tx = None;
        self.content_lookup_results_rx = None;
        self.available_game_versions.clear();
        self.selected_game_version_index = 0;
        self.loader_support = LoaderSupportIndex::default();
        self.loader_versions = LoaderVersionIndex::default();
        self.modloader_versions_cache.clear();
        self.modloader_versions_in_flight.clear();
        self.modloader_versions_results_tx = None;
        self.modloader_versions_results_rx = None;
        self.modloader_versions_status_key = None;
        self.modloader_versions_status = None;
        self.incompatible_modloader_version_warning_key = None;
        self.version_catalog_filter = None;
        self.version_catalog_error = None;
        self.version_catalog_in_flight = false;
        self.version_catalog_results_tx = None;
        self.version_catalog_results_rx = None;
        self.runtime_prepare_in_flight = false;
        self.runtime_prepare_results_tx = None;
        self.runtime_prepare_results_rx = None;
        self.runtime_progress_tx = None;
        self.runtime_progress_rx = None;
        self.runtime_latest_progress = None;
        self.runtime_last_notification_at = None;
        self.runtime_prepare_instance_root = None;
        self.runtime_prepare_user_key = None;
        self.active_tab = InstanceScreenTab::Content;
        self.screenshots.clear();
        self.last_screenshot_scan_at = None;
        self.screenshot_scan_in_flight = false;
        self.screenshot_scan_request_serial = 0;
        self.screenshot_scan_results_tx = None;
        self.screenshot_scan_results_rx = None;
        self.screenshot_images.clear(ctx);
        self.screenshot_layout_revision = 0;
        self.screenshot_masonry_layout_cache = None;
        self.screenshot_viewer = None;
        self.pending_delete_screenshot_key = None;
        self.delete_screenshot_in_flight = false;
        self.delete_screenshot_results_tx = None;
        self.delete_screenshot_results_rx = None;
        self.logs.clear();
        self.last_log_scan_at = None;
        self.log_scan_in_flight = false;
        self.log_scan_request_serial = 0;
        self.log_scan_results_tx = None;
        self.log_scan_results_rx = None;
        self.selected_log_path = None;
        self.loaded_log_path = None;
        self.loaded_log_modified_at_ms = None;
        self.loaded_log_lines.clear();
        self.loaded_log_error = None;
        self.loaded_log_truncated = false;
        self.log_load_in_flight = false;
        self.log_load_request_serial = 0;
        self.requested_log_load_path = None;
        self.requested_log_load_modified_at_ms = None;
        self.log_load_results_tx = None;
        self.log_load_results_rx = None;
        self.show_settings_modal = false;
        self.show_export_vtmpack_modal = false;
        self.show_export_server_modal = false;
        self.export_vtmpack_in_flight = false;
        self.export_vtmpack_output_path = None;
        self.export_vtmpack_progress_tx = None;
        self.export_vtmpack_progress_rx = None;
        self.export_vtmpack_latest_progress = None;
        self.export_vtmpack_results_tx = None;
        self.export_vtmpack_results_rx = None;
        self.export_server_included_root_entries.clear();
        self.export_server_in_flight = false;
        self.export_server_output_path = None;
        self.export_server_progress_tx = None;
        self.export_server_progress_rx = None;
        self.export_server_latest_progress = None;
        self.export_server_results_tx = None;
        self.export_server_results_rx = None;
        self.show_move_instance_modal = false;
        self.show_move_instance_progress_modal = false;
        self.move_instance_dest_valid = false;
        self.move_instance_dest_error = None;
        self.move_instance_in_flight = false;
        self.move_instance_latest_progress = None;
        self.move_instance_dest_path = None;
        self.move_instance_completion_message = None;
        self.move_instance_completion_failed = false;
        self.move_instance_last_layout_log_at = None;
        self.move_instance_pending_result = None;
        self.move_instance_progress_visible_until = None;
        self.move_instance_progress_tx = None;
        self.move_instance_progress_rx = None;
        self.move_instance_results_tx = None;
        self.move_instance_results_rx = None;
        self.launch_username = None;
        self.launch_user_key = None;
    }
}

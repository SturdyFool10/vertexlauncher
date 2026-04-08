use config::{
    Config, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
    JavaRuntimeVersion,
};
use content_resolver::{
    InstalledContentFile, InstalledContentHashCache, InstalledContentKind,
    InstalledContentResolver, ResolveInstalledContentRequest,
};
use directories::UserDirs;
use egui::{TextureOptions, Ui, scroll_area::ScrollSource};
use flate2::read::GzDecoder;
use installation::{
    DownloadPolicy, InstallProgress, InstallProgressCallback, InstallStage, LaunchRequest,
    LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry, VersionCatalog,
    display_user_path, ensure_game_files, ensure_openjdk_runtime, fetch_loader_versions_for_game,
    fetch_version_catalog_with_refresh, is_instance_running_for_account, launch_instance,
    normalize_path_key, running_instance_for_account, stop_running_instance_for_account,
};
use instances::{
    InstanceStore, record_instance_launch_usage, set_instance_settings, set_instance_versions,
};
use managed_content::load_managed_content_identities;
use std::{
    collections::{BTreeMap, HashMap, HashSet, hash_map::DefaultHasher},
    ffi::OsStr,
    fs,
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use textui::TextUi;
use textui_egui::prelude::*;
use ui_foundation::{
    DialogPreset, danger_button, dialog_options, is_compact_width, primary_button,
    secondary_button, selectable_row_button, show_dialog, tab_button, themed_text_input,
};
use vtmpack::{
    VTMPACK_EXTENSION, VtmpackInstanceMetadata, VtmpackProviderMode, default_vtmpack_file_name,
    default_vtmpack_root_entry_selected, enforce_vtmpack_extension,
    export_instance_as_vtmpack_with_progress, list_exportable_root_entries,
    sync_vtmpack_export_options,
};

use crate::app::tokio_runtime;
use crate::desktop;
use crate::screens::{AppScreen, LaunchAuthContext};
use crate::ui::{
    components::{
        icon_button, image_textures,
        lazy_image_bytes::{LazyImageBytes, LazyImageBytesStatus},
        remote_tiled_image, settings_widgets,
        virtual_masonry::{build_virtual_masonry_layout, render_virtualized_masonry},
    },
    modal, style,
};
use crate::{assets, console, install_activity, notification, privacy};

mod content;
mod content_lookup_result;
mod installed_content_cache;
mod installed_entry_render_result;
mod instance_screen_output;
mod instance_screen_state;
mod move_instance;
mod platform;
mod runtime;
mod runtime_prepare_operation;
mod runtime_prepare_outcome;

use content::ContentApplyResult;
use content::{poll_content_lookup_results, render_installed_content_section};
use content_lookup_result::ContentLookupResult;
use installed_content_cache::InstalledContentCache;
use installed_entry_render_result::InstalledEntryRenderResult;
pub use instance_screen_output::InstanceScreenOutput;
use instance_screen_state::{
    InstalledContentEntryUiCache, InstanceLogEntry, InstanceScreenState, InstanceScreenTab,
    InstanceScreenshotEntry, InstanceScreenshotViewerState, MoveInstanceResult,
    ServerExportOutcome, VtmpackExportOutcome,
};
use move_instance::{
    poll_move_instance_progress, poll_move_instance_results, request_move_instance,
};
use platform::{
    effective_linux_graphics_settings_for_state, linux_instance_driver_settings_for_save,
    render_platform_specific_instance_settings_section,
};
use runtime::*;
use runtime_prepare_operation::RuntimePrepareOperation;
use runtime_prepare_outcome::RuntimePrepareOutcome;

use super::{console as console_screen, platform as screen_platform};

const RESERVED_SYSTEM_MEMORY_MIB: u128 = 4 * 1024;
const FALLBACK_TOTAL_MEMORY_MIB: u128 = 20 * 1024;
const MODLOADER_OPTIONS: [&str; 6] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt", "Custom"];
const CUSTOM_MODLOADER_INDEX: usize = MODLOADER_OPTIONS.len() - 1;
const INSTALLED_CONTENT_SCROLLBAR_RESERVE: f32 = 18.0;
const INSTALLED_CONTENT_PAGE_SIZES: [usize; 4] = [10, 25, 50, 100];
const INSTANCE_TABS_HEIGHT: f32 = 38.0;
const INSTANCE_SCREENSHOT_SCAN_INTERVAL: Duration = Duration::from_secs(10);
const INSTANCE_LOG_SCAN_INTERVAL: Duration = Duration::from_secs(3);
const INSTANCE_SCREENSHOT_TILE_GAP: f32 = 10.0;
const INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM: f32 = 1.0;
const INSTANCE_SCREENSHOT_VIEWER_MAX_ZOOM: f32 = 8.0;
const INSTANCE_SCREENSHOT_VIEWER_ZOOM_STEP: f32 = 0.12;
const INSTANCE_SCREENSHOT_VIEWER_SCROLL_ZOOM_SENSITIVITY: f32 = 0.0015;
const INSTANCE_SCREENSHOT_OVERSCAN: f32 = 420.0;
const INSTANCE_SCREENSHOT_MIN_COLUMN_WIDTH: f32 = 180.0;
const MAX_INSTANCE_LOG_LINES: usize = 12_000;
const INSTANCE_SCREENSHOT_COPY_BUTTON_SIZE: f32 = 28.0;
const INSTANCE_TOP_TAB_ID_KEY: &str = "instance_top_tab_id";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstancePresenceSection {
    Content,
    Screenshots,
    Logs,
}

impl Default for InstancePresenceSection {
    fn default() -> Self {
        Self::Content
    }
}

#[derive(Default)]
struct MemorySliderMaxState {
    detected_total_mib: Option<u128>,
    load_complete: bool,
    rx: Option<mpsc::Receiver<Option<u128>>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct InstanceScreenshotTileAction {
    open_viewer: bool,
    request_delete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstanceScreenshotOverlayAction {
    Copy,
    Delete,
}

#[derive(Clone, Copy, Debug, Default)]
struct InstanceScreenshotOverlayResult {
    action: Option<InstanceScreenshotOverlayAction>,
    contains_pointer: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct InstanceScreenshotOverlayButtonResult {
    clicked: bool,
    contains_pointer: bool,
}

fn instance_screen_state_id(instance_id: &str) -> egui::Id {
    egui::Id::new(("instance_screen_state", instance_id))
}

pub fn purge_inactive_state(ctx: &egui::Context, selected_instance_id: Option<&str>) {
    let Some(instance_id) = selected_instance_id else {
        return;
    };
    let state_id = instance_screen_state_id(instance_id);
    ctx.data_mut(|data| {
        let Some(mut state) = data.get_temp::<InstanceScreenState>(state_id) else {
            return;
        };
        state.purge_heavy_state(ctx);
        data.insert_temp(state_id, state);
    });
}

pub fn purge_screenshot_state(ctx: &egui::Context, selected_instance_id: Option<&str>) {
    let Some(instance_id) = selected_instance_id else {
        return;
    };
    let state_id = instance_screen_state_id(instance_id);
    ctx.data_mut(|data| {
        let Some(mut state) = data.get_temp::<InstanceScreenState>(state_id) else {
            return;
        };
        state.purge_screenshot_state(ctx);
        data.insert_temp(state_id, state);
    });
}

pub fn instance_content_resource_packs_tab_id(ctx: &egui::Context) -> Option<egui::Id> {
    content::installed_content_tab_id(ctx, content_resolver::InstalledContentKind::ResourcePacks)
}

pub fn instance_content_shader_packs_tab_id(ctx: &egui::Context) -> Option<egui::Id> {
    content::installed_content_tab_id(ctx, content_resolver::InstalledContentKind::ShaderPacks)
}

pub fn instance_top_content_tab_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new((
            INSTANCE_TOP_TAB_ID_KEY,
            InstanceScreenTab::Content,
        )))
    })
}

pub fn instance_top_screenshots_tab_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new((
            INSTANCE_TOP_TAB_ID_KEY,
            InstanceScreenTab::ScreenshotGallery,
        )))
    })
}

pub fn instance_top_logs_tab_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new((
            INSTANCE_TOP_TAB_ID_KEY,
            InstanceScreenTab::Logs,
        )))
    })
}

pub fn set_gamepad_screenshot_viewer_input(ctx: &egui::Context, pan: egui::Vec2, zoom: f32) {
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::new("instance_screenshot_viewer_gamepad_pan"), pan);
        data.insert_temp(
            egui::Id::new("instance_screenshot_viewer_gamepad_zoom"),
            zoom,
        );
    });
}

pub(super) fn handle_escape(ctx: &egui::Context, selected_instance_id: Option<&str>) -> bool {
    let Some(instance_id) = selected_instance_id else {
        return false;
    };
    let state_id = instance_screen_state_id(instance_id);
    let mut handled = false;
    ctx.data_mut(|data| {
        let Some(mut state) = data.get_temp::<InstanceScreenState>(state_id) else {
            return;
        };
        if state.pending_delete_screenshot_key.is_some() {
            if !state.delete_screenshot_in_flight {
                tracing::info!(
                    target: "vertexlauncher/screenshots",
                    instance_id,
                    "Instance screenshot delete confirmation closed by escape."
                );
                state.pending_delete_screenshot_key = None;
            }
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.screenshot_viewer.take().is_some() {
            tracing::info!(
                target: "vertexlauncher/screenshots",
                instance_id,
                "Instance screenshot viewer closed by escape."
            );
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.show_export_vtmpack_modal {
            if !state.export_vtmpack_in_flight {
                state.show_export_vtmpack_modal = false;
            }
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.show_export_server_modal {
            if !state.export_server_in_flight {
                state.show_export_server_modal = false;
            }
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.show_move_instance_progress_modal {
            // Non-dismissable while in flight
            if !state.move_instance_in_flight {
                state.show_move_instance_progress_modal = false;
            }
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.show_move_instance_modal {
            if !state.move_instance_in_flight {
                state.show_move_instance_modal = false;
            }
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.show_settings_modal {
            state.show_settings_modal = false;
            data.insert_temp(state_id, state);
            handled = true;
        }
    });
    handled
}

pub fn presence_section(
    ctx: &egui::Context,
    selected_instance_id: Option<&str>,
) -> InstancePresenceSection {
    let Some(instance_id) = selected_instance_id else {
        return InstancePresenceSection::Content;
    };
    let state_id = instance_screen_state_id(instance_id);
    let state = ctx.data_mut(|data| data.get_temp::<InstanceScreenState>(state_id));
    match state.map(|state| state.active_tab).unwrap_or_default() {
        InstanceScreenTab::Content => InstancePresenceSection::Content,
        InstanceScreenTab::ScreenshotGallery => InstancePresenceSection::Screenshots,
        InstanceScreenTab::Logs => InstancePresenceSection::Logs,
    }
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    active_account_owns_minecraft: bool,
    streamer_mode: bool,
    instances: &mut InstanceStore,
    config: &mut Config,
    account_avatars_by_key: &HashMap<String, Vec<u8>>,
) -> InstanceScreenOutput {
    ui.ctx()
        .options_mut(|options| options.reduce_texture_memory = true);
    let mut output = InstanceScreenOutput::default();
    let body_style = style::body(ui);
    let muted_style = style::muted(ui);

    let Some(instance_id) = selected_instance_id else {
        let _ = text_ui.label(
            ui,
            "instance_screen_empty_body",
            "Select an instance from the left sidebar or click + to create one.",
            &body_style,
        );
        return output;
    };

    let Some(instance_snapshot) = instances.find(instance_id).cloned() else {
        let _ = text_ui.label(
            ui,
            "instance_screen_missing_body",
            "Selected instance no longer exists.",
            &body_style,
        );
        return output;
    };

    let state_id = instance_screen_state_id(instance_id);
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<InstanceScreenState>(state_id))
        .unwrap_or_else(|| InstanceScreenState::from_instance(&instance_snapshot, config));
    let previous_tab = state.active_tab;

    poll_background_tasks(&mut state, config, instances, instance_id);
    poll_vtmpack_export_progress(&mut state);
    poll_vtmpack_export_results(&mut state);
    poll_server_export_progress(&mut state);
    poll_server_export_results(&mut state);
    poll_move_instance_progress(&mut state);
    if let Some(result) = poll_move_instance_results(&mut state) {
        match result {
            MoveInstanceResult::Complete { ref dest_path } => {
                if let Some(instance) = instances.find_mut(instance_id) {
                    instance.instance_root_override = Some(dest_path.clone());
                    output.instances_changed = true;
                }
                state.move_instance_completion_message =
                    Some(format!("Instance moved to {}.", dest_path.display()));
                state.move_instance_completion_failed = false;
                state.status_message = Some(format!("Instance moved to {}.", dest_path.display()));
            }
            MoveInstanceResult::Failed { ref reason } => {
                state.move_instance_completion_message =
                    Some(format!("Instance move failed: {reason}"));
                state.move_instance_completion_failed = true;
                state.status_message = Some(format!("Instance move failed: {reason}"));
            }
        }
    }
    poll_instance_screenshot_scan_results(&mut state);
    poll_instance_log_scan_results(&mut state);
    poll_instance_log_load_results(&mut state);
    sync_version_catalog(&mut state, config.include_snapshots_and_betas(), false);
    if state.version_catalog_in_flight
        || !state.modloader_versions_in_flight.is_empty()
        || state.runtime_prepare_in_flight
        || state.content_apply_in_flight
        || state.screenshot_scan_in_flight
        || state.delete_screenshot_in_flight
        || state.log_scan_in_flight
        || state.log_load_in_flight
        || state.export_vtmpack_in_flight
        || state.export_server_in_flight
        || state.move_instance_in_flight
    {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let selected_game_version_for_loader = selected_game_version(&state).to_owned();
    ensure_selected_modloader_is_supported(&mut state, selected_game_version_for_loader.as_str());

    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let instance_root_path = instances::instance_root_path(&installations_root, &instance_snapshot);
    poll_instance_screenshot_delete_results(&mut state, instance_root_path.as_path());
    let content_download_policy = DownloadPolicy {
        max_concurrent_downloads: config.download_max_concurrent().max(1),
        max_download_bps: config.parsed_download_speed_limit_bps(),
    };

    let _ = text_ui.label(
        ui,
        ("instance_screen_root", instance_id),
        &format!("Root: {}", instance_root_path.display()),
        &muted_style,
    );
    ui.add_space(12.0);

    let selected_game_version_for_runtime = selected_game_version(&state).to_owned();
    let external_activity = install_activity::snapshot().filter(|activity| {
        activity.instance_id == state.name_input
            || activity.instance_id == instance_snapshot.name
            || activity.instance_id == instance_id
    });
    let external_install_active = external_activity
        .as_ref()
        .is_some_and(|activity| !matches!(activity.stage, InstallStage::Complete))
        || state.content_apply_in_flight;
    render_runtime_row(
        ui,
        text_ui,
        &mut state,
        instance_id,
        instance_root_path.as_path(),
        selected_game_version_for_runtime.as_str(),
        config,
        external_install_active,
        active_username,
        active_launch_auth,
        active_account_owns_minecraft,
        streamer_mode,
        account_avatars_by_key,
    );
    render_install_feedback(
        ui,
        text_ui,
        instance_id,
        state.runtime_latest_progress.as_ref(),
        external_activity.as_ref(),
        state.runtime_prepare_in_flight,
    );
    ui.add_space(10.0);
    output.instances_changed |= render_instance_settings_modal(
        ui.ctx(),
        text_ui,
        instance_id,
        &mut state,
        instances,
        config,
    );
    render_move_instance_modal(
        ui.ctx(),
        text_ui,
        instance_id,
        &mut state,
        instances,
        config,
    );
    render_move_instance_progress_modal(ui.ctx(), text_ui, instance_id, &mut state);
    render_export_vtmpack_modal(
        ui.ctx(),
        text_ui,
        instance_id,
        &mut state,
        instances,
        config,
    );
    render_export_server_modal(
        ui.ctx(),
        text_ui,
        instance_id,
        &mut state,
        instances,
        config,
    );
    ui.add_space(10.0);
    render_instance_tab_row(ui, text_ui, &mut state.active_tab);
    if previous_tab == InstanceScreenTab::ScreenshotGallery
        && state.active_tab != InstanceScreenTab::ScreenshotGallery
    {
        state.purge_screenshot_state(ui.ctx());
    }
    state.screenshot_images.begin_frame(ui.ctx());
    let screenshot_images_updated = state.screenshot_images.poll(ui.ctx());
    ui.add_space(12.0);

    match state.active_tab {
        InstanceScreenTab::Content => {
            render_installed_content_section(
                ui,
                text_ui,
                instance_id,
                instance_root_path.as_path(),
                &content_download_policy,
                &mut state,
                external_install_active,
                &mut output,
            );

            let mut retained_image_keys = HashSet::new();
            retain_instance_viewer_image(&mut state, &mut retained_image_keys);
            state
                .screenshot_images
                .retain_loaded(ui.ctx(), &retained_image_keys);
        }
        InstanceScreenTab::ScreenshotGallery => {
            let should_scan = state
                .last_screenshot_scan_at
                .is_none_or(|last| last.elapsed() >= INSTANCE_SCREENSHOT_SCAN_INTERVAL);
            if should_scan {
                refresh_instance_screenshots(&mut state, instance_root_path.as_path(), false);
            }
            if state.screenshot_viewer.as_ref().is_some_and(|viewer| {
                !state.screenshots.iter().any(|screenshot| {
                    screenshot_key(screenshot.path.as_path()) == viewer.screenshot_key
                })
            }) {
                tracing::info!(
                    target: "vertexlauncher/screenshots",
                    instance_id,
                    "Instance screenshot viewer closed because the selected screenshot disappeared from the gallery state."
                );
                state.screenshot_viewer = None;
            }
            if state
                .pending_delete_screenshot_key
                .as_ref()
                .is_some_and(|pending| {
                    !state
                        .screenshots
                        .iter()
                        .any(|screenshot| screenshot_key(screenshot.path.as_path()) == *pending)
                })
            {
                state.pending_delete_screenshot_key = None;
            }
            let mut retained_image_keys = HashSet::new();
            render_instance_screenshot_gallery(ui, text_ui, &mut state, &mut retained_image_keys);
            retain_instance_viewer_image(&mut state, &mut retained_image_keys);
            state
                .screenshot_images
                .retain_loaded(ui.ctx(), &retained_image_keys);
        }
        InstanceScreenTab::Logs => {
            let should_scan = state
                .last_log_scan_at
                .is_none_or(|last| last.elapsed() >= INSTANCE_LOG_SCAN_INTERVAL);
            if should_scan {
                refresh_instance_logs(&mut state, instance_root_path.as_path(), false);
            }
            sync_selected_instance_log(&mut state);
            render_instance_logs_tab(ui, text_ui, &mut state);
            ui.ctx().request_repaint_after(Duration::from_millis(250));

            let mut retained_image_keys = HashSet::new();
            retain_instance_viewer_image(&mut state, &mut retained_image_keys);
            state
                .screenshot_images
                .retain_loaded(ui.ctx(), &retained_image_keys);
        }
    }

    if screenshot_images_updated
        || (state.screenshot_images.has_in_flight()
            && (state.active_tab == InstanceScreenTab::ScreenshotGallery
                || state.screenshot_viewer.is_some()))
    {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    render_instance_screenshot_viewer_modal(ui.ctx(), text_ui, &mut state);
    render_instance_delete_screenshot_modal(
        ui.ctx(),
        text_ui,
        &mut state,
        instance_root_path.as_path(),
    );

    output.presence_section = match state.active_tab {
        InstanceScreenTab::Content => InstancePresenceSection::Content,
        InstanceScreenTab::ScreenshotGallery => InstancePresenceSection::Screenshots,
        InstanceScreenTab::Logs => InstancePresenceSection::Logs,
    };
    ui.ctx().data_mut(|d| d.insert_temp(state_id, state));
    output
}

fn render_instance_tab_row(ui: &mut Ui, text_ui: &mut TextUi, active_tab: &mut InstanceScreenTab) {
    let tabs = InstanceScreenTab::ALL.map(|tab| (tab, tab.label()));
    let spacing = 8.0;
    let width =
        ((ui.available_width() - spacing * (tabs.len() as f32 - 1.0)) / tabs.len() as f32).max(0.0);
    ui.push_id("instance_tab_row", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            for &(tab, label) in &tabs {
                let selected = *active_tab == tab;
                let response = text_ui.selectable_button(
                    ui,
                    ("fill_tab_row", label),
                    label,
                    selected,
                    &tab_button(ui, selected, egui::vec2(width, INSTANCE_TABS_HEIGHT)),
                );
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new((INSTANCE_TOP_TAB_ID_KEY, tab)), response.id)
                });
                if response.clicked() {
                    *active_tab = tab;
                }
            }
        });
    });
}

fn render_instance_screenshot_gallery(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
    retained_image_keys: &mut HashSet<String>,
) {
    let title_style = style::body_strong(ui);
    let body_style = style::muted(ui);
    let _ = text_ui.label(
        ui,
        "instance_screenshot_gallery_title",
        "Screenshot Gallery",
        &title_style,
    );
    let summary = if state.screenshot_scan_in_flight && state.screenshots.is_empty() {
        "Loading screenshots...".to_owned()
    } else if state.screenshots.is_empty() {
        "No screenshots found for this instance.".to_owned()
    } else {
        format!(
            "{} screenshots found for this instance.",
            state.screenshots.len()
        )
    };
    let _ = text_ui.label(
        ui,
        "instance_screenshot_gallery_summary",
        summary.as_str(),
        &body_style,
    );
    ui.add_space(8.0);

    if state.screenshots.is_empty() {
        return;
    }

    let layout = build_virtual_masonry_layout(
        ui,
        INSTANCE_SCREENSHOT_MIN_COLUMN_WIDTH,
        INSTANCE_SCREENSHOT_TILE_GAP,
        3,
        state.screenshots.len(),
        state.screenshot_layout_revision,
        &mut state.screenshot_masonry_layout_cache,
        |index, column_width| {
            instance_screenshot_tile_height(&state.screenshots[index], column_width)
        },
    );

    let mut open_key = None;
    let mut delete_key = None;
    let screenshots = &state.screenshots;
    let screenshot_images = &mut state.screenshot_images;
    egui::ScrollArea::vertical()
        .id_salt("instance_screenshot_gallery_scroll")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            render_virtualized_masonry(
                ui,
                &layout,
                INSTANCE_SCREENSHOT_TILE_GAP,
                viewport,
                INSTANCE_SCREENSHOT_OVERSCAN,
                |column_ui, index, tile_height| {
                    let action = render_instance_screenshot_tile(
                        column_ui,
                        screenshot_images,
                        &screenshots[index],
                        tile_height,
                        retained_image_keys,
                    );
                    if action.open_viewer {
                        open_key = Some(screenshot_key(screenshots[index].path.as_path()));
                    }
                    if action.request_delete {
                        delete_key = Some(screenshot_key(screenshots[index].path.as_path()));
                    }
                },
            );
        });

    if let Some(screenshot_key) = open_key {
        state.screenshot_viewer = Some(InstanceScreenshotViewerState {
            screenshot_key,
            zoom: INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM,
            pan_uv: egui::Vec2::ZERO,
        });
    }
    if let Some(screenshot_key) = delete_key {
        state.pending_delete_screenshot_key = Some(screenshot_key);
    }
}

fn instance_screenshot_tile_height(screenshot: &InstanceScreenshotEntry, column_width: f32) -> f32 {
    column_width / screenshot_aspect_ratio(screenshot).max(0.01)
}

fn render_instance_screenshot_tile(
    ui: &mut Ui,
    screenshot_images: &mut LazyImageBytes,
    screenshot: &InstanceScreenshotEntry,
    tile_height: f32,
    retained_image_keys: &mut HashSet<String>,
) -> InstanceScreenshotTileAction {
    let width = ui.available_width().max(1.0);
    let tile_size = egui::vec2(width, tile_height);
    let (rect, _) = ui.allocate_exact_size(tile_size, egui::Sense::hover());
    let mut image_response = ui.interact(
        rect,
        ui.id().with((
            "instance_screenshot_tile",
            screenshot_key(screenshot.path.as_path()),
        )),
        egui::Sense::click(),
    );
    let image_key = screenshot_uri(screenshot.path.as_path(), screenshot.modified_at_ms);
    let image_status = screenshot_images.request(image_key.clone(), screenshot.path.clone());
    let image_bytes = screenshot_images.bytes(image_key.as_str());
    retained_image_keys.insert(image_key.clone());
    if let Some(bytes) = image_bytes.as_ref() {
        match image_textures::request_texture(
            ui.ctx(),
            image_key,
            Arc::clone(bytes),
            TextureOptions::LINEAR,
        ) {
            image_textures::ManagedTextureStatus::Ready(texture) => {
                egui::Image::from_texture(&texture)
                    .fit_to_exact_size(rect.size())
                    .corner_radius(egui::CornerRadius::same(14))
                    .paint_at(ui, rect);
            }
            image_textures::ManagedTextureStatus::Loading => {
                paint_instance_screenshot_placeholder(ui, rect, LazyImageBytesStatus::Loading);
            }
            image_textures::ManagedTextureStatus::Failed => {
                paint_instance_screenshot_placeholder(ui, rect, LazyImageBytesStatus::Failed);
            }
        }
    } else {
        paint_instance_screenshot_placeholder(ui, rect, image_status);
    }

    let tile_contains_pointer = ui_pointer_over_rect(ui, rect);
    let overlay_memory_id = image_response.id.with("instance_screenshot_overlay_active");
    let overlay_was_active = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(overlay_memory_id))
        .unwrap_or(false);
    let mut overlay_clicked = false;
    let mut action = InstanceScreenshotTileAction::default();
    let mut overlay_result = InstanceScreenshotOverlayResult::default();
    if tile_contains_pointer || overlay_was_active {
        overlay_result = render_instance_screenshot_overlay_action(
            ui,
            rect,
            "instance_gallery",
            screenshot,
            image_bytes.as_deref(),
            image_status == LazyImageBytesStatus::Loading,
        );
        match overlay_result.action {
            Some(InstanceScreenshotOverlayAction::Copy) => {
                overlay_clicked = true;
            }
            Some(InstanceScreenshotOverlayAction::Delete) => {
                overlay_clicked = true;
                action.request_delete = true;
            }
            None => {}
        }
    }
    let overlay_active = tile_contains_pointer || overlay_result.contains_pointer;
    ui.ctx()
        .data_mut(|data| data.insert_temp(overlay_memory_id, overlay_active));
    let stroke = if overlay_active {
        ui.visuals().widgets.hovered.bg_stroke
    } else {
        ui.visuals().widgets.inactive.bg_stroke
    };
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(14),
        stroke,
        egui::StrokeKind::Inside,
    );

    let label_bg_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 10.0, rect.max.y - 34.0),
        egui::pos2(rect.max.x - 10.0, rect.max.y - 10.0),
    );
    ui.painter().rect_filled(
        label_bg_rect,
        egui::CornerRadius::same(8),
        egui::Color32::from_rgba_premultiplied(6, 9, 14, 185),
    );
    ui.painter().text(
        egui::pos2(label_bg_rect.min.x + 8.0, label_bg_rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!(
            "{} | {}",
            screenshot.file_name,
            format_time_ago(screenshot.modified_at_ms, current_time_millis())
        ),
        egui::TextStyle::Body.resolve(ui.style()),
        egui::Color32::WHITE,
    );

    image_response = image_response.on_hover_text(format!(
        "{}\n{}x{}\n{}",
        screenshot.file_name,
        screenshot.width,
        screenshot.height,
        screenshot.path.display()
    ));
    action.open_viewer = image_response.clicked() && !overlay_clicked;
    action
}

fn ui_pointer_over_rect(ui: &Ui, rect: egui::Rect) -> bool {
    ui.input(|input| {
        input
            .pointer
            .interact_pos()
            .or_else(|| input.pointer.hover_pos())
            .is_some_and(|pointer_pos| rect.contains(pointer_pos))
    })
}

fn paint_instance_screenshot_placeholder(
    ui: &mut Ui,
    rect: egui::Rect,
    image_status: LazyImageBytesStatus,
) {
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(14),
        ui.visuals().widgets.inactive.bg_fill,
    );
    let label = match image_status {
        LazyImageBytesStatus::Loading => "Loading...",
        LazyImageBytesStatus::Failed => "Failed to load",
        LazyImageBytesStatus::Ready | LazyImageBytesStatus::Unrequested => "Waiting...",
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        ui.visuals().weak_text_color(),
    );
}

fn retain_instance_viewer_image(
    state: &mut InstanceScreenState,
    retained_image_keys: &mut HashSet<String>,
) {
    let Some(viewer_key) = state
        .screenshot_viewer
        .as_ref()
        .map(|viewer| viewer.screenshot_key.as_str())
    else {
        return;
    };
    let Some(screenshot) = state
        .screenshots
        .iter()
        .find(|entry| screenshot_key(entry.path.as_path()) == viewer_key)
    else {
        return;
    };
    let image_key = screenshot_uri(screenshot.path.as_path(), screenshot.modified_at_ms);
    retained_image_keys.insert(image_key.clone());
    state
        .screenshot_images
        .request(image_key, screenshot.path.clone());
}

fn render_instance_screenshot_viewer_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
) {
    let Some(selected_screenshot_key) = state
        .screenshot_viewer
        .as_ref()
        .map(|viewer| viewer.screenshot_key.clone())
    else {
        return;
    };
    let Some(screenshot) = state
        .screenshots
        .iter()
        .find(|entry| screenshot_key(entry.path.as_path()) == selected_screenshot_key)
        .cloned()
    else {
        tracing::info!(
            target: "vertexlauncher/screenshots",
            screenshot_key = selected_screenshot_key.as_str(),
            "Instance screenshot viewer closed because the screenshot entry was no longer available."
        );
        state.screenshot_viewer = None;
        return;
    };
    let Some(viewer_state) = state.screenshot_viewer.as_mut() else {
        return;
    };
    let image_key = screenshot_uri(screenshot.path.as_path(), screenshot.modified_at_ms);
    let image_status = state
        .screenshot_images
        .request(image_key.clone(), screenshot.path.clone());
    let image_bytes = state.screenshot_images.bytes(image_key.as_str());
    let gamepad_pan = ctx
        .data(|data| {
            data.get_temp::<egui::Vec2>(egui::Id::new("instance_screenshot_viewer_gamepad_pan"))
        })
        .unwrap_or(egui::Vec2::ZERO);
    let gamepad_zoom = ctx
        .data(|data| data.get_temp::<f32>(egui::Id::new("instance_screenshot_viewer_gamepad_zoom")))
        .unwrap_or(0.0);
    let frame_dt = ctx.input(|input| input.stable_dt).clamp(1.0 / 240.0, 0.05);

    let mut close_requested = false;
    let mut delete_requested = false;
    let response = show_dialog(
        ctx,
        dialog_options("instance_screenshot_viewer_window", DialogPreset::Viewer),
        |ui| {
            let title_style = style::section_heading(ui);
            let body_style = style::muted_single_line(ui);

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let _ = text_ui.label(
                        ui,
                        "instance_screenshot_viewer_title",
                        screenshot.file_name.as_str(),
                        &title_style,
                    );
                    let details = format!(
                        "{}x{} | {}",
                        screenshot.width,
                        screenshot.height,
                        format_time_ago(screenshot.modified_at_ms, current_time_millis())
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_screenshot_viewer_details",
                        details.as_str(),
                        &body_style,
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if text_ui
                        .button(
                            ui,
                            "instance_screenshot_viewer_close",
                            "Close",
                            &secondary_button(ui, egui::vec2(92.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                    if text_ui
                        .button(
                            ui,
                            "instance_screenshot_viewer_delete",
                            "Delete",
                            &danger_button(ui, egui::vec2(92.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        delete_requested = true;
                    }
                    if ui
                        .add_enabled_ui(image_bytes.is_some(), |ui| {
                            text_ui.button(
                                ui,
                                "instance_screenshot_viewer_copy",
                                "Copy",
                                &secondary_button(ui, egui::vec2(82.0, style::CONTROL_HEIGHT)),
                            )
                        })
                        .inner
                        .clicked()
                        && let Some(bytes) = image_bytes.as_deref()
                    {
                        copy_instance_screenshot_to_clipboard(
                            ui.ctx(),
                            screenshot.file_name.as_str(),
                            bytes,
                        );
                    }
                    if text_ui
                        .button(
                            ui,
                            "instance_screenshot_viewer_reset",
                            "Reset",
                            &secondary_button(ui, egui::vec2(82.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        viewer_state.zoom = INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM;
                        viewer_state.pan_uv = egui::Vec2::ZERO;
                    }
                    if text_ui
                        .button(
                            ui,
                            "instance_screenshot_viewer_zoom_in",
                            "+",
                            &secondary_button(ui, egui::vec2(40.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        viewer_state.zoom = adjust_instance_screenshot_zoom(viewer_state.zoom, 1.0);
                        clamp_instance_screenshot_pan(viewer_state);
                    }
                    if text_ui
                        .button(
                            ui,
                            "instance_screenshot_viewer_zoom_out",
                            "-",
                            &secondary_button(ui, egui::vec2(40.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        viewer_state.zoom =
                            adjust_instance_screenshot_zoom(viewer_state.zoom, -1.0);
                        clamp_instance_screenshot_pan(viewer_state);
                    }
                });
            });
            ui.add_space(12.0);

            let canvas_size = ui.available_size().max(egui::vec2(1.0, 1.0));
            let (canvas_rect, response) = ui.allocate_exact_size(canvas_size, egui::Sense::drag());
            ui.painter().rect_filled(
                canvas_rect,
                egui::CornerRadius::same(12),
                ui.visuals().faint_bg_color,
            );

            let image_rect = instance_fit_rect_to_aspect(
                canvas_rect.shrink(8.0),
                screenshot_aspect_ratio(&screenshot),
            );
            if response.hovered() {
                let scroll_delta = ui.ctx().input(|input| input.smooth_scroll_delta.y);
                if scroll_delta.abs() > 0.0 {
                    viewer_state.zoom = adjust_instance_screenshot_zoom_with_scroll(
                        viewer_state.zoom,
                        scroll_delta,
                    );
                    clamp_instance_screenshot_pan(viewer_state);
                    ui.ctx().request_repaint();
                }
            }
            if response.dragged() && viewer_state.zoom > INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM {
                let visible_fraction =
                    1.0 / viewer_state.zoom.max(INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM);
                let delta = ui.ctx().input(|input| input.pointer.delta());
                viewer_state.pan_uv.x -= delta.x / image_rect.width().max(1.0) * visible_fraction;
                viewer_state.pan_uv.y -= delta.y / image_rect.height().max(1.0) * visible_fraction;
                clamp_instance_screenshot_pan(viewer_state);
                ui.ctx().request_repaint();
            }
            if gamepad_zoom.abs() > 0.05 {
                let zoom_scale = (1.0 + gamepad_zoom * 1.8 * frame_dt).clamp(0.7, 1.3);
                viewer_state.zoom = (viewer_state.zoom * zoom_scale).clamp(
                    INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM,
                    INSTANCE_SCREENSHOT_VIEWER_MAX_ZOOM,
                );
                clamp_instance_screenshot_pan(viewer_state);
                ui.ctx().request_repaint();
            }
            if viewer_state.zoom > INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM
                && (gamepad_pan.x.abs() > 0.05 || gamepad_pan.y.abs() > 0.05)
            {
                let visible_fraction =
                    1.0 / viewer_state.zoom.max(INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM);
                let pan_speed = 1.35 * 0.2 * frame_dt * visible_fraction;
                viewer_state.pan_uv.x += gamepad_pan.x * pan_speed;
                viewer_state.pan_uv.y += gamepad_pan.y * pan_speed;
                clamp_instance_screenshot_pan(viewer_state);
                ui.ctx().request_repaint();
            }

            if let Some(bytes) = image_bytes.as_ref() {
                match image_textures::request_texture(
                    ui.ctx(),
                    image_key.clone(),
                    Arc::clone(bytes),
                    TextureOptions::LINEAR,
                ) {
                    image_textures::ManagedTextureStatus::Ready(texture) => {
                        egui::Image::from_texture(&texture)
                            .fit_to_exact_size(image_rect.size())
                            .maintain_aspect_ratio(false)
                            .uv(instance_viewer_uv_rect(viewer_state))
                            .corner_radius(egui::CornerRadius::same(12))
                            .paint_at(ui, image_rect);
                    }
                    image_textures::ManagedTextureStatus::Loading => {
                        ui.painter().rect_filled(
                            image_rect,
                            egui::CornerRadius::same(12),
                            ui.visuals().widgets.inactive.bg_fill,
                        );
                        ui.painter().text(
                            image_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Loading screenshot...",
                            egui::TextStyle::Button.resolve(ui.style()),
                            ui.visuals().weak_text_color(),
                        );
                    }
                    image_textures::ManagedTextureStatus::Failed => {
                        ui.painter().rect_filled(
                            image_rect,
                            egui::CornerRadius::same(12),
                            ui.visuals().widgets.inactive.bg_fill,
                        );
                        ui.painter().text(
                            image_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Failed to load screenshot",
                            egui::TextStyle::Button.resolve(ui.style()),
                            ui.visuals().weak_text_color(),
                        );
                    }
                }
            } else {
                ui.painter().rect_filled(
                    image_rect,
                    egui::CornerRadius::same(12),
                    ui.visuals().widgets.inactive.bg_fill,
                );
                let label = if image_status == LazyImageBytesStatus::Failed {
                    "Failed to load screenshot"
                } else {
                    "Loading screenshot..."
                };
                ui.painter().text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::TextStyle::Button.resolve(ui.style()),
                    ui.visuals().weak_text_color(),
                );
            }
            ui.painter().rect_stroke(
                image_rect,
                egui::CornerRadius::same(12),
                ui.visuals().widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );
        },
    );
    close_requested |= response.close_requested;

    if delete_requested {
        tracing::info!(
            target: "vertexlauncher/screenshots",
            screenshot_key = selected_screenshot_key.as_str(),
            "Instance screenshot viewer requested delete."
        );
        state.pending_delete_screenshot_key = Some(selected_screenshot_key.clone());
    }
    if close_requested {
        tracing::info!(
            target: "vertexlauncher/screenshots",
            screenshot_key = selected_screenshot_key.as_str(),
            "Instance screenshot viewer closed by explicit close button."
        );
        state.screenshot_viewer = None;
    }
}

fn render_instance_delete_screenshot_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
    instance_root: &Path,
) {
    let Some(pending_screenshot_key) = state.pending_delete_screenshot_key.clone() else {
        return;
    };
    let Some(screenshot) = state
        .screenshots
        .iter()
        .find(|entry| screenshot_key(entry.path.as_path()) == pending_screenshot_key)
        .cloned()
    else {
        state.pending_delete_screenshot_key = None;
        return;
    };

    let danger = ctx.style().visuals.error_fg_color;
    let mut cancel_requested = false;
    let mut delete_requested = false;
    let response = show_dialog(
        ctx,
        dialog_options("instance_delete_screenshot_modal", DialogPreset::Confirm),
        |ui| {
            let heading_style = style::heading_color(ui, 28.0, 32.0, danger);
            let body_style = style::body(ui);
            let muted_style = style::muted(ui);

            let path_label = screenshot.path.display().to_string();
            let _ = text_ui.label(
                ui,
                ("instance_delete_screenshot_heading", path_label.clone()),
                "Delete Screenshot?",
                &heading_style,
            );
            let _ = text_ui.label(
                ui,
                ("instance_delete_screenshot_body", path_label.clone()),
                &format!(
                    "Delete \"{}\" from disk? This permanently removes the screenshot.",
                    screenshot.file_name
                ),
                &body_style,
            );
            let _ = text_ui.label(
                ui,
                ("instance_delete_screenshot_root", path_label.clone()),
                &format!("Instance root: {}", instance_root.display()),
                &muted_style,
            );
            let _ = text_ui.label(
                ui,
                ("instance_delete_screenshot_path", path_label),
                &format!("Path: {}", screenshot.path.display()),
                &muted_style,
            );

            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled_ui(!state.delete_screenshot_in_flight, |ui| {
                        text_ui.button(
                            ui,
                            "instance_delete_screenshot_confirm",
                            "Delete",
                            &danger_button(ui, egui::vec2(120.0, 34.0)),
                        )
                    })
                    .inner
                    .clicked()
                {
                    delete_requested = true;
                }
                if ui
                    .add_enabled_ui(!state.delete_screenshot_in_flight, |ui| {
                        text_ui.button(
                            ui,
                            "instance_delete_screenshot_cancel",
                            "Cancel",
                            &secondary_button(ui, egui::vec2(120.0, 34.0)),
                        )
                    })
                    .inner
                    .clicked()
                {
                    cancel_requested = true;
                }
                if state.delete_screenshot_in_flight {
                    ui.spinner();
                }
            });
        },
    );
    cancel_requested |= response.close_requested && !state.delete_screenshot_in_flight;

    if cancel_requested && !state.delete_screenshot_in_flight {
        state.pending_delete_screenshot_key = None;
        return;
    }

    if delete_requested {
        request_instance_screenshot_delete(
            state,
            pending_screenshot_key,
            screenshot.path.clone(),
            screenshot.file_name.clone(),
        );
    }
}

fn render_instance_screenshot_overlay_action(
    ui: &mut Ui,
    tile_rect: egui::Rect,
    scope: &str,
    screenshot: &InstanceScreenshotEntry,
    copy_bytes: Option<&[u8]>,
    copy_loading: bool,
) -> InstanceScreenshotOverlayResult {
    let screenshot_key = screenshot_key(screenshot.path.as_path());
    let mut result = InstanceScreenshotOverlayResult::default();
    let copy_result = render_instance_screenshot_overlay_button(
        ui,
        tile_rect,
        scope,
        screenshot_key.as_str(),
        "instance_screenshot_copy_button",
        assets::COPY_SVG,
        ui.visuals().text_color(),
        if copy_loading {
            "Image is still loading"
        } else {
            "Copy image to clipboard"
        },
        8.0,
        copy_bytes.is_some(),
    );
    result.contains_pointer |= copy_result.contains_pointer;
    if copy_result.clicked {
        let Some(bytes) = copy_bytes else {
            return result;
        };
        copy_instance_screenshot_to_clipboard(ui.ctx(), screenshot.file_name.as_str(), bytes);
        result.action = Some(InstanceScreenshotOverlayAction::Copy);
        return result;
    }
    let delete_result = render_instance_screenshot_overlay_button(
        ui,
        tile_rect,
        scope,
        screenshot_key.as_str(),
        "instance_screenshot_delete_button",
        assets::TRASH_X_SVG,
        ui.visuals().error_fg_color,
        "Delete screenshot",
        8.0 + INSTANCE_SCREENSHOT_COPY_BUTTON_SIZE + 6.0,
        true,
    );
    result.contains_pointer |= delete_result.contains_pointer;
    if delete_result.clicked {
        result.action = Some(InstanceScreenshotOverlayAction::Delete);
    }
    result
}

fn render_instance_screenshot_overlay_button(
    ui: &mut Ui,
    tile_rect: egui::Rect,
    scope: &str,
    screenshot_key: &str,
    id_source: &str,
    icon_svg: &[u8],
    icon_color: egui::Color32,
    tooltip: &str,
    x_offset: f32,
    enabled: bool,
) -> InstanceScreenshotOverlayButtonResult {
    let button_rect = egui::Rect::from_min_size(
        tile_rect.min + egui::vec2(x_offset, 8.0),
        egui::vec2(
            INSTANCE_SCREENSHOT_COPY_BUTTON_SIZE,
            INSTANCE_SCREENSHOT_COPY_BUTTON_SIZE,
        ),
    );
    let themed_svg = apply_color_to_svg(icon_svg, icon_color);
    let icon_color_key = format!(
        "{:02x}{:02x}{:02x}",
        icon_color.r(),
        icon_color.g(),
        icon_color.b()
    );
    let uri = format!("bytes://instance/{id_source}/{scope}-{screenshot_key}-{icon_color_key}.svg");
    let response = ui.interact(
        button_rect,
        ui.id().with((id_source, scope, screenshot_key)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let button_contains_pointer = ui_pointer_over_rect(ui, button_rect);
    let button_pressed = button_contains_pointer && ui.input(|input| input.pointer.primary_down());
    let fill = if response.is_pointer_button_down_on() || button_pressed {
        ui.visuals().widgets.active.bg_fill
    } else if button_contains_pointer {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        egui::Color32::from_rgba_premultiplied(12, 16, 24, 210)
    };
    ui.painter()
        .rect_filled(button_rect, egui::CornerRadius::same(8), fill);
    ui.painter().rect_stroke(
        button_rect,
        egui::CornerRadius::same(8),
        ui.visuals().widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let icon_rect = egui::Rect::from_center_size(button_rect.center(), egui::vec2(14.0, 14.0));
    egui::Image::from_bytes(uri, themed_svg)
        .fit_to_exact_size(icon_rect.size())
        .tint(if enabled {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_white_alpha(120)
        })
        .paint_at(ui, icon_rect);
    let clicked = response.clicked();
    let contains_pointer = button_contains_pointer || response.is_pointer_button_down_on();
    if button_contains_pointer {
        let _ = egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            response.id.with("tooltip"),
            egui::PopupAnchor::Pointer,
        )
        .gap(12.0)
        .show(|ui| {
            ui.label(tooltip);
        });
    }
    InstanceScreenshotOverlayButtonResult {
        clicked,
        contains_pointer,
    }
}

fn copy_instance_screenshot_to_clipboard(ctx: &egui::Context, label: &str, bytes: &[u8]) {
    match decode_clipboard_color_image(bytes) {
        Ok(image) => {
            ctx.copy_image(image);
            notification::info!("instance/screenshots", "Copied '{}' to clipboard.", label);
        }
        Err(err) => {
            notification::error!(
                "instance/screenshots",
                "Failed to copy '{}' to clipboard: {}",
                label,
                err
            );
        }
    }
}

fn render_instance_logs_tab(ui: &mut Ui, text_ui: &mut TextUi, state: &mut InstanceScreenState) {
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
            let _ = text_ui.label(
                ui,
                "instance_logs_error",
                error,
                &style::error_text(ui),
            );
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

fn ensure_instance_screenshot_scan_channel(state: &mut InstanceScreenState) {
    if state.screenshot_scan_results_tx.is_some() && state.screenshot_scan_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(u64, Vec<InstanceScreenshotEntry>)>();
    state.screenshot_scan_results_tx = Some(tx);
    state.screenshot_scan_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn poll_instance_screenshot_scan_results(state: &mut InstanceScreenState) {
    let Some(rx) = state.screenshot_scan_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/instance",
            "Instance screenshot-scan receiver mutex was poisoned."
        );
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok((request_id, screenshots)) => {
                if request_id != state.screenshot_scan_request_serial {
                    continue;
                }
                state.screenshots = screenshots;
                state.mark_screenshot_layout_dirty();
                state.last_screenshot_scan_at = Some(Instant::now());
                state.screenshot_scan_in_flight = false;
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/instance",
                    "Instance screenshot-scan worker disconnected unexpectedly."
                );
                state.screenshot_scan_in_flight = false;
                break;
            }
        }
    }
}

fn ensure_instance_screenshot_delete_channel(state: &mut InstanceScreenState) {
    if state.delete_screenshot_results_tx.is_some() && state.delete_screenshot_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, String, Result<(), String>)>();
    state.delete_screenshot_results_tx = Some(tx);
    state.delete_screenshot_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_instance_screenshot_delete(
    state: &mut InstanceScreenState,
    screenshot_key: String,
    path: PathBuf,
    file_name: String,
) {
    if state.delete_screenshot_in_flight {
        return;
    }

    ensure_instance_screenshot_delete_channel(state);
    let Some(tx) = state.delete_screenshot_results_tx.as_ref().cloned() else {
        return;
    };

    state.delete_screenshot_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = fs::remove_file(path.as_path()).map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %path.display(), error = %err, context = "delete instance screenshot");
            format!("failed to remove {}: {err}", path.display())
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

fn poll_instance_screenshot_delete_results(state: &mut InstanceScreenState, instance_root: &Path) {
    let Some(rx) = state.delete_screenshot_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/instance",
            "Instance screenshot-delete receiver mutex was poisoned."
        );
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok((screenshot_key, file_name, result)) => {
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
                        notification::info!(
                            "instance/screenshots",
                            "Deleted '{}' from disk.",
                            file_name
                        );
                    }
                    Err(err) => {
                        state.pending_delete_screenshot_key = None;
                        notification::error!(
                            "instance/screenshots",
                            "Failed to delete '{}': {}",
                            file_name,
                            err
                        );
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/instance",
                    "Instance screenshot-delete worker disconnected unexpectedly."
                );
                state.delete_screenshot_in_flight = false;
                state.pending_delete_screenshot_key = None;
                break;
            }
        }
    }
}

fn refresh_instance_screenshots(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    force: bool,
) {
    if state.screenshot_scan_in_flight && !force {
        return;
    }

    ensure_instance_screenshot_scan_channel(state);
    let Some(tx) = state.screenshot_scan_results_tx.as_ref().cloned() else {
        return;
    };
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

fn ensure_instance_log_scan_channel(state: &mut InstanceScreenState) {
    if state.log_scan_results_tx.is_some() && state.log_scan_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(u64, Vec<InstanceLogEntry>)>();
    state.log_scan_results_tx = Some(tx);
    state.log_scan_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn poll_instance_log_scan_results(state: &mut InstanceScreenState) {
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

fn refresh_instance_logs(state: &mut InstanceScreenState, instance_root: &Path, force: bool) {
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

fn sync_selected_instance_log(state: &mut InstanceScreenState) {
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

fn poll_instance_log_load_results(state: &mut InstanceScreenState) {
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

fn is_supported_screenshot_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(OsStr::to_str)
            .map(|extension| extension.to_ascii_lowercase()),
        Some(extension)
            if matches!(
                extension.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
            )
    )
}

fn screenshot_aspect_ratio(screenshot: &InstanceScreenshotEntry) -> f32 {
    screenshot.width as f32 / screenshot.height.max(1) as f32
}

fn screenshot_key(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn screenshot_uri(path: &Path, modified_at_ms: Option<u64>) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    modified_at_ms.hash(&mut hasher);
    format!("bytes://instance/screenshot/{}.png", hasher.finish())
}

fn instance_fit_rect_to_aspect(rect: egui::Rect, aspect_ratio: f32) -> egui::Rect {
    let safe_aspect = aspect_ratio.max(0.01);
    let rect_aspect = rect.width() / rect.height().max(1.0);
    if rect_aspect > safe_aspect {
        let width = rect.height() * safe_aspect;
        let x = rect.center().x - width * 0.5;
        egui::Rect::from_min_size(egui::pos2(x, rect.min.y), egui::vec2(width, rect.height()))
    } else {
        let height = rect.width() / safe_aspect;
        let y = rect.center().y - height * 0.5;
        egui::Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(rect.width(), height))
    }
}

fn adjust_instance_screenshot_zoom(current_zoom: f32, direction: f32) -> f32 {
    (current_zoom + direction * INSTANCE_SCREENSHOT_VIEWER_ZOOM_STEP).clamp(
        INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM,
        INSTANCE_SCREENSHOT_VIEWER_MAX_ZOOM,
    )
}

fn adjust_instance_screenshot_zoom_with_scroll(current_zoom: f32, scroll_delta: f32) -> f32 {
    let scale =
        (1.0 + scroll_delta * INSTANCE_SCREENSHOT_VIEWER_SCROLL_ZOOM_SENSITIVITY).clamp(0.7, 1.3);
    (current_zoom * scale).clamp(
        INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM,
        INSTANCE_SCREENSHOT_VIEWER_MAX_ZOOM,
    )
}

fn clamp_instance_screenshot_pan(viewer_state: &mut InstanceScreenshotViewerState) {
    let visible_fraction = 1.0 / viewer_state.zoom.max(INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM);
    let max_offset = (1.0 - visible_fraction) * 0.5;
    viewer_state.pan_uv.x = viewer_state.pan_uv.x.clamp(-max_offset, max_offset);
    viewer_state.pan_uv.y = viewer_state.pan_uv.y.clamp(-max_offset, max_offset);
}

fn instance_viewer_uv_rect(viewer_state: &InstanceScreenshotViewerState) -> egui::Rect {
    let visible_fraction = 1.0 / viewer_state.zoom.max(INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM);
    let half = visible_fraction * 0.5;
    let center = egui::pos2(0.5 + viewer_state.pan_uv.x, 0.5 + viewer_state.pan_uv.y);
    egui::Rect::from_min_max(
        egui::pos2(center.x - half, center.y - half),
        egui::pos2(center.x + half, center.y + half),
    )
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

fn decode_clipboard_color_image(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    let rgba = image::load_from_memory(bytes)
        .map_err(|err| err.to_string())?
        .to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

fn render_instance_settings_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &mut InstanceStore,
    config: &mut Config,
) -> bool {
    if !state.show_settings_modal {
        return false;
    }

    let mut instances_changed = false;
    let mut close_requested = false;
    let response = modal::show_window(
        ctx,
        "Instance Settings",
        modal::ModalOptions::new(
            egui::Id::new(("instance_settings_modal", instance_id)),
            modal::ModalLayout::centered(
                modal::AxisSizing::new(0.92, 1.0, f32::INFINITY),
                modal::AxisSizing::new(0.92, 1.0, f32::INFINITY),
            ),
        )
        .with_layer(modal::ModalLayer::Base)
        .with_dismiss_behavior(modal::DismissBehavior::EscapeAndScrim),
        |ui| {
            let muted_style = style::muted(ui);
            let section_style = style::subtitle(ui);
            let body_style = style::body(ui);
            let action_button_style = ButtonOptions {
                min_size: egui::vec2(220.0, 34.0),
                text_color: ui.visuals().widgets.active.fg_stroke.color,
                fill: ui.visuals().selection.bg_fill,
                fill_hovered: ui.visuals().selection.bg_fill.gamma_multiply(1.1),
                fill_active: ui.visuals().selection.bg_fill.gamma_multiply(0.9),
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().selection.stroke,
                ..ButtonOptions::default()
            };
            let refresh_style = style::neutral_button_with_min_size(ui, egui::vec2(190.0, 30.0));
            let reinstall_button_style =
                style::neutral_button_with_min_size(ui, egui::vec2(220.0, 34.0));
            egui::ScrollArea::vertical()
                .id_salt(("instance_settings_modal_scroll", instance_id))
                .scroll_source(ScrollSource {
                    scroll_bar: true,
                    drag: false,
                    mouse_wheel: true,
                })
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_modal_heading", instance_id),
                        "Instance Settings",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_modal_description", instance_id),
                        "Manage this profile's metadata, version stack, runtime overrides, and maintenance actions.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_versions_heading", instance_id),
                        "Metadata & Versions",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_versions_description", instance_id),
                        "Display info, Minecraft version, and modloader selection for this instance.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_name_input", instance_id),
                        "Name",
                        Some("Display name shown in the sidebar."),
                        &mut state.name_input,
                    );
                    ui.add_space(6.0);
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_description_input", instance_id),
                        "Description (optional)",
                        Some("Optional note shown in library instance tiles."),
                        &mut state.description_input,
                    );
                    ui.add_space(6.0);

                    let mut thumbnail_input = state.thumbnail_input.as_os_str().to_string_lossy().into_owned();
                    let thumbnail_changed = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_thumbnail_input", instance_id),
                        "Thumbnail path (optional)",
                        Some("Local image path for this instance."),
                        &mut thumbnail_input,
                    )
                    .changed();
                    if thumbnail_changed {
                        let trimmed = thumbnail_input.trim();
                        state.thumbnail_input = if trimmed.is_empty() {
                            PathBuf::new()
                        } else {
                            PathBuf::from(trimmed)
                        };
                    }
                    ui.add_space(6.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_refresh_versions", instance_id),
                            "Refresh version list",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        sync_version_catalog(state, config.include_snapshots_and_betas(), true);
                        state.modloader_versions_cache.clear();
                        state.modloader_versions_status = None;
                        state.modloader_versions_status_key = None;
                    }
                    if state.version_catalog_in_flight {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let _ = text_ui.label(
                                ui,
                                ("instance_versions_loading", instance_id),
                                "Fetching version catalog...",
                                &muted_style,
                            );
                        });
                    }

                    if let Some(catalog_error) = state.version_catalog_error.as_deref() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_version_catalog_error", instance_id),
                            catalog_error,
                            &style::error_text(ui),
                        );
                    }

                    let version_labels: Vec<String> = state
                        .available_game_versions
                        .iter()
                        .map(MinecraftVersionEntry::display_label)
                        .collect();
                    let version_refs: Vec<&str> =
                        version_labels.iter().map(String::as_str).collect();
                    if !version_refs.is_empty() {
                        let mut selected_index = state
                            .selected_game_version_index
                            .min(version_refs.len().saturating_sub(1));
                        let response = settings_widgets::dropdown_row(
                            text_ui,
                            ui,
                            ("instance_game_version_dropdown", instance_id),
                            "Minecraft game version",
                            Some("Pick from available Minecraft versions."),
                            &mut selected_index,
                            &version_refs,
                        );
                        if response.changed() {
                            state.selected_game_version_index = selected_index;
                            if let Some(version) = state.available_game_versions.get(selected_index)
                            {
                                state.game_version_input = version.id.clone();
                            }
                        }
                    } else {
                        let _ = text_ui.label(
                            ui,
                            ("instance_game_version_empty", instance_id),
                            "No game versions available yet.",
                            &muted_style,
                        );
                    }
                    ui.add_space(6.0);

                    let selected_game_version_for_loader = selected_game_version(state).to_owned();
                    ensure_selected_modloader_is_supported(
                        state,
                        selected_game_version_for_loader.as_str(),
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_modloader_label", instance_id),
                        "Modloader",
                        &body_style,
                    );
                    ui.add_space(4.0);
                    render_modloader_selector(
                        ui,
                        text_ui,
                        state,
                        instance_id,
                        selected_game_version_for_loader.as_str(),
                    );
                    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
                        ui.add_space(6.0);
                        let _ = settings_widgets::full_width_text_input_row(
                            text_ui,
                            ui,
                            ("instance_custom_modloader_input", instance_id),
                            "Custom modloader id",
                            Some("Use any custom modloader name."),
                            &mut state.custom_modloader,
                        );
                    }
                    ui.add_space(6.0);

                    let selected_modloader_label = selected_modloader_value(state);
                    let modloader_versions_key = modloader_versions_cache_key(
                        selected_modloader_label.as_str(),
                        selected_game_version_for_loader.as_str(),
                    );
                    let available_modloader_versions =
                        selected_modloader_versions(state, selected_game_version_for_loader.as_str())
                            .to_vec();
                    if state.selected_modloader == 0 {
                        state.modloader_version_input.clear();
                    } else {
                        let mut resolved_modloader_versions = available_modloader_versions;
                        let should_fetch_remote = state.selected_modloader != CUSTOM_MODLOADER_INDEX
                            && resolved_modloader_versions.is_empty();
                        if should_fetch_remote {
                            if let Some(cached) =
                                state.modloader_versions_cache.get(&modloader_versions_key)
                            {
                                resolved_modloader_versions = cached.clone();
                            } else {
                                request_modloader_versions(
                                    state,
                                    selected_modloader_label.as_str(),
                                    selected_game_version_for_loader.as_str(),
                                    false,
                                );
                            }
                        }

                        let in_flight = state
                            .modloader_versions_in_flight
                            .contains(&modloader_versions_key);
                        if in_flight {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                let _ = text_ui.label(
                                    ui,
                                    ("instance_modloader_versions_fetching", instance_id),
                                    "Fetching modloader versions...",
                                    &muted_style,
                                );
                            });
                        }

                        if state.modloader_versions_status_key.as_deref()
                            == Some(modloader_versions_key.as_str())
                            && let Some(status) = state.modloader_versions_status.as_deref()
                        {
                            let is_error = status.starts_with("Failed");
                            let _ = text_ui.label(
                                ui,
                                ("instance_modloader_versions_status", instance_id),
                                status,
                                &if is_error { style::error_text(ui) } else { style::muted(ui) },
                            );
                        }

                        let modloader_version_options: Vec<String> =
                            resolved_modloader_versions.clone();

                        // Auto-select the first (latest) version if none is currently set.
                        if state.modloader_version_input.trim().is_empty() {
                            if let Some(first) = modloader_version_options.first() {
                                state.modloader_version_input = first.clone();
                            }
                        }

                        let option_refs: Vec<&str> = modloader_version_options
                            .iter()
                            .map(String::as_str)
                            .collect();
                        let current_modloader_version = state.modloader_version_input.trim().to_owned();
                        let mut selected_index = modloader_version_options
                            .iter()
                            .position(|entry| entry == &current_modloader_version)
                            .unwrap_or(0);
                        if settings_widgets::full_width_dropdown_row(
                            text_ui,
                            ui,
                            ("instance_modloader_version_dropdown", instance_id),
                            "Modloader version",
                            Some("Cataloged by loader+Minecraft compatibility and cached once per day."),
                            &mut selected_index,
                            &option_refs,
                        )
                        .changed()
                        {
                            if let Some(selected) = modloader_version_options.get(selected_index) {
                                state.modloader_version_input = selected.clone();
                            }
                        }

                        if state.selected_modloader != CUSTOM_MODLOADER_INDEX {
                            let refresh_clicked = ui
                                .add_enabled_ui(!in_flight, |ui| {
                                    text_ui.button(
                                        ui,
                                        ("instance_modloader_versions_refresh", instance_id),
                                        "Refresh modloader versions",
                                        &refresh_style,
                                    )
                                })
                                .inner
                                .clicked();
                            if refresh_clicked {
                                request_modloader_versions(
                                    state,
                                    selected_modloader_label.as_str(),
                                    selected_game_version_for_loader.as_str(),
                                    true,
                                );
                            }
                        }

                        if resolved_modloader_versions.is_empty()
                            && state.selected_modloader != CUSTOM_MODLOADER_INDEX
                        {
                            let _ = text_ui.label(
                                ui,
                                ("instance_modloader_versions_unavailable", instance_id),
                                "No cataloged modloader versions were found for this Minecraft version.",
                                &muted_style,
                            );
                        }
                    }

                    ui.add_space(8.0);

                    let trimmed_name = state.name_input.trim();
                    let requested_modloader = selected_modloader_value(state);
                    let requested_game_version = state.game_version_input.trim().to_owned();
                    let validation_error = if trimmed_name.is_empty() {
                        Some("Name cannot be empty.".to_owned())
                    } else if requested_game_version.is_empty() {
                        Some("Minecraft game version cannot be empty.".to_owned())
                    } else if requested_modloader.trim().is_empty() {
                        Some("Modloader cannot be empty.".to_owned())
                    } else if support_catalog_ready(state)
                        && !state
                            .loader_support
                            .supports_loader(
                                requested_modloader.as_str(),
                                requested_game_version.as_str(),
                            )
                        && state.selected_modloader != CUSTOM_MODLOADER_INDEX
                    {
                        Some(format!(
                            "{} is not available for Minecraft {}.",
                            requested_modloader, requested_game_version
                        ))
                    } else {
                        resolve_modloader_version_for_settings(
                            state,
                            requested_modloader.as_str(),
                            requested_game_version.as_str(),
                        )
                        .err()
                    };
                    let can_save_versions = validation_error.is_none();
                    if let Some(error) = validation_error.as_deref() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_save_versions_validation_error", instance_id),
                            error,
                            &style::error_text(ui),
                        );
                        ui.add_space(6.0);
                    }

                    let save_versions_clicked = ui
                        .add_enabled_ui(can_save_versions, |ui| {
                            text_ui.button(
                                ui,
                                ("instance_save_versions", instance_id),
                                "Save metadata & versions",
                                &action_button_style,
                            )
                        })
                        .inner
                        .clicked();
                    let reinstall_enabled =
                        can_save_versions && !state.runtime_prepare_in_flight && !state.running;
                    let reinstall_clicked = ui
                        .add_enabled_ui(reinstall_enabled, |ui| {
                            text_ui.button(
                                ui,
                                ("instance_reinstall_profile", instance_id),
                                "Reinstall Profile",
                                &reinstall_button_style,
                            )
                        })
                        .inner
                        .clicked();
                    if save_versions_clicked {
                        match save_instance_metadata_and_versions(state, instance_id, instances) {
                            Ok(()) => {
                                instances_changed = true;
                                if let Some(saved) = instances.find(instance_id) {
                                    tracing::info!(
                                        target: "vertexlauncher/ui/instance",
                                        instance_id = %instance_id,
                                        saved_modloader = %saved.modloader,
                                        saved_game_version = %saved.game_version,
                                        saved_modloader_version = %saved.modloader_version,
                                        "Saved instance metadata and versions."
                                    );
                                }
                                state.status_message =
                                    Some("Saved metadata and version settings.".to_owned());
                            }
                            Err(err) => {
                                tracing::warn!(
                                    target: "vertexlauncher/ui/instance",
                                    instance_id = %instance_id,
                                    error = %err,
                                    "Failed to save instance metadata and versions."
                                );
                                state.status_message = Some(err);
                            }
                        }
                    }
                    if reinstall_clicked {
                        match save_instance_metadata_and_versions(state, instance_id, instances) {
                            Ok(()) => {
                                instances_changed = true;
                                let game_version = state.game_version_input.trim().to_owned();
                                let modloader = selected_modloader_value(state);
                                if let Some(saved_instance) = instances.find(instance_id).cloned() {
                                    let modloader_version = normalize_optional(
                                        saved_instance.modloader_version.as_str(),
                                    );
                                    let installations_root =
                                        config.minecraft_installations_root_path().to_path_buf();
                                    let instance_root = instances::instance_root_path(
                                        &installations_root,
                                        &saved_instance,
                                    );
                                    let (linux_set_opengl_driver, linux_use_zink_driver) =
                                        effective_linux_graphics_settings_for_state(state, config);
                                    request_runtime_prepare(
                                        state,
                                        RuntimePrepareOperation::ReinstallProfile,
                                        instance_root,
                                        game_version.clone(),
                                        modloader.clone(),
                                        modloader_version,
                                        effective_required_java_major(
                                            config,
                                            game_version.as_str(),
                                        ),
                                        choose_java_executable(
                                            config,
                                            state.java_override_enabled,
                                            state.java_override_runtime_major,
                                            effective_required_java_major(
                                                config,
                                                game_version.as_str(),
                                            ),
                                        ),
                                        config.download_max_concurrent(),
                                        config.parsed_download_speed_limit_bps(),
                                        linux_set_opengl_driver,
                                        linux_use_zink_driver,
                                        config.default_instance_max_memory_mib(),
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                    );
                                } else {
                                    state.status_message =
                                        Some("Instance was removed before reinstall.".to_owned());
                                }
                            }
                            Err(err) => {
                                state.status_message = Some(err);
                            }
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_heading", instance_id),
                        "Runtime Overrides",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_runtime_overrides_description", instance_id),
                        "Per-instance overrides for memory, JVM arguments, and Java runtime selection.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = settings_widgets::toggle_row(
                        text_ui,
                        ui,
                        "Override max memory for this instance",
                        Some("When disabled, launcher instance default memory is used."),
                        &mut state.memory_override_enabled,
                    );
                    ui.add_space(6.0);

                    let (memory_slider_max, memory_slider_pending) = memory_slider_max_mib();
                    if memory_slider_pending {
                        ui.ctx().request_repaint_after(Duration::from_millis(50));
                    }
                    if state.memory_override_enabled {
                        let mut memory_mib = state
                            .memory_override_mib
                            .clamp(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, memory_slider_max);
                        let response = settings_widgets::u128_slider_with_input_row(
                            text_ui,
                            ui,
                            ("instance_memory_override", instance_id),
                            "Max memory allocation (MiB)",
                            Some("Per-instance memory limit."),
                            &mut memory_mib,
                            INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
                            memory_slider_max,
                            INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
                        );
                        if response.changed() {
                            state.memory_override_mib = memory_mib;
                        }
                        ui.add_space(6.0);
                    }

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_cli_args_override", instance_id),
                        "JVM args override (optional)",
                        Some("Leave blank to use launcher instance default JVM args."),
                        &mut state.cli_args_input,
                    );
                    let _ = settings_widgets::toggle_row(
                        text_ui,
                        ui,
                        "This modpack has a rich presence mod",
                        Some("When enabled, Vertex clears its own Discord Rich Presence for this instance after launch so the mod inside Minecraft can take over."),
                        &mut state.discord_rich_presence_mod_installed,
                    );
                    ui.add_space(8.0);

                    ui.add_space(8.0);

                    let _ = settings_widgets::toggle_row(
                        text_ui,
                        ui,
                        "Override Java runtime for this instance",
                        Some("When enabled, this instance will use the selected configured global Java path."),
                        &mut state.java_override_enabled,
                    );
                    ui.add_space(6.0);

                    let java_options = configured_java_path_options(config);
                    if state.java_override_enabled {
                        if java_options.is_empty() {
                            let _ = text_ui.label(
                                ui,
                                ("instance_java_override_no_options", instance_id),
                                "No configured global Java paths found. Add at least one Java path in Settings first.",
                                &style::error_text(ui),
                            );
                        } else {
                            if state
                                .java_override_runtime_major
                                .is_none_or(|major| !java_options.iter().any(|(m, _)| *m == major))
                            {
                                state.java_override_runtime_major = java_options.first().map(|(major, _)| *major);
                            }
                            let option_labels: Vec<&str> =
                                java_options.iter().map(|(_, label)| label.as_str()).collect();
                            let mut selected_index = java_options
                                .iter()
                                .position(|(major, _)| Some(*major) == state.java_override_runtime_major)
                                .unwrap_or(0);
                            if settings_widgets::full_width_dropdown_row(
                                text_ui,
                                ui,
                                ("instance_java_override_runtime", instance_id),
                                "Java path override",
                                Some("Select which configured Java path this instance should use."),
                                &mut selected_index,
                                &option_labels,
                            )
                            .changed()
                            {
                                state.java_override_runtime_major =
                                    java_options.get(selected_index).map(|(major, _)| *major);
                            }
                        }
                    }
                    ui.add_space(8.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_save_settings", instance_id),
                            "Save instance settings",
                            &action_button_style,
                        )
                        .clicked()
                    {
                        let java_override_runtime_major = if state.java_override_enabled {
                            if java_options.is_empty() {
                                state.status_message = Some(
                                    "Cannot save Java override: configure at least one global Java path in Settings."
                                        .to_owned(),
                                );
                                None
                            } else {
                                let selected = state.java_override_runtime_major.and_then(|major| {
                                    java_options
                                        .iter()
                                        .find_map(|(candidate, _)| (*candidate == major).then_some(major))
                                });
                                selected.or_else(|| java_options.first().map(|(major, _)| *major))
                            }
                        } else {
                            None
                        };
                        if !state.java_override_enabled || java_override_runtime_major.is_some() {
                            let memory_override = if state.memory_override_enabled {
                                Some(
                                    state
                                        .memory_override_mib
                                        .clamp(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, memory_slider_max),
                                )
                            } else {
                                None
                            };
                            let cli_override = normalize_optional(state.cli_args_input.as_str());
                            let (
                                linux_set_opengl_driver,
                                linux_use_zink_driver,
                            ) = linux_instance_driver_settings_for_save(
                                state,
                                instances.find(instance_id),
                            );
                            match set_instance_settings(
                                instances,
                                instance_id,
                                memory_override,
                                cli_override,
                                state.java_override_enabled,
                                java_override_runtime_major,
                                linux_set_opengl_driver,
                                linux_use_zink_driver,
                                state.discord_rich_presence_mod_installed,
                            ) {
                                Ok(()) => {
                                    instances_changed = true;
                                    state.status_message = Some("Saved instance settings.".to_owned());
                                }
                                Err(err) => state.status_message = Some(err.to_string()),
                            }
                        }
                    }

                    render_platform_specific_instance_settings_section(
                        ui,
                        text_ui,
                        state,
                        instance_id,
                        &section_style,
                        &muted_style,
                    );

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_actions_heading", instance_id),
                        "Maintenance & Actions",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_actions_description", instance_id),
                        "Open the instance folder and commit any metadata or runtime changes.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_open_folder", instance_id),
                            "Open Instance Folder",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        if let Some(instance) = instances.find(instance_id) {
                            let installations_root =
                                config.minecraft_installations_root_path().to_path_buf();
                            let instance_root =
                                instances::instance_root_path(&installations_root, instance);
                            match desktop::open_in_file_manager(instance_root.as_path()) {
                                Ok(()) => {
                                    state.status_message = Some(format!(
                                        "Opened instance folder: {}",
                                        instance_root.display()
                                    ));
                                }
                                Err(err) => {
                                    state.status_message =
                                        Some(format!("Failed to open instance folder: {err}"));
                                }
                            }
                        } else {
                            state.status_message =
                                Some("Instance was removed before opening its folder.".to_owned());
                        }
                    }
                    ui.add_space(6.0);
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_vtmpack", instance_id),
                            "Export .vtmpack...",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        state.show_export_vtmpack_modal = true;
                    }
                    ui.add_space(6.0);
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_server_zip", instance_id),
                            "Auto-generate server zip...",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        state.show_export_server_modal = true;
                    }
                    ui.add_space(6.0);
                    if text_ui
                        .button(
                            ui,
                            ("instance_move_instance", instance_id),
                            "Move Instance...",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        if let Some(instance) = instances.find(instance_id) {
                            let installations_root =
                                config.minecraft_installations_root_path().to_path_buf();
                            let current_root =
                                instances::instance_root_path(&installations_root, instance);
                            state.move_instance_dest_input =
                                current_root.display().to_string();
                        } else {
                            state.move_instance_dest_input = String::new();
                        }
                        state.move_instance_dest_valid = false;
                        state.move_instance_dest_error = None;
                        state.show_move_instance_modal = true;
                    }
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if text_ui
                            .button(
                                ui,
                                ("instance_settings_close", instance_id),
                                "Done",
                                &action_button_style,
                            )
                            .clicked()
                        {
                            close_requested = true;
                        }
                    });
                });
        },
    );

    if response.close_requested || close_requested {
        state.show_settings_modal = false;
    }
    instances_changed
}

fn render_export_vtmpack_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &InstanceStore,
    config: &Config,
) {
    if !state.show_export_vtmpack_modal {
        return;
    }

    let mut close_requested = false;
    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let instance_root = instances
        .find(instance_id)
        .map(|instance| instances::instance_root_path(&installations_root, instance));
    if let Some(instance_root) = instance_root.as_deref() {
        sync_vtmpack_export_options(instance_root, &mut state.export_vtmpack_options);
    }
    let mut export_requested = false;
    let response = show_dialog(
        ctx,
        dialog_options(
            ("instance_export_vtmpack_modal", instance_id),
            DialogPreset::Form,
        ),
        |ui| {
            let title_style = style::modal_title(ui);
            let body_style = style::muted(ui);
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_title", instance_id),
                "Export .vtmpack",
                &title_style,
            );
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_body", instance_id),
                "Choose whether the exported pack may reference CurseForge metadata directly, then select which top-level files and folders from the Minecraft root should be bundled into the pack.",
                &body_style,
            );
            ui.add_space(12.0);

            if state.export_vtmpack_in_flight {
                let progress = state.export_vtmpack_latest_progress.as_ref();
                let progress_fraction = progress
                    .and_then(|progress| {
                        (progress.total_steps > 0).then_some(
                            progress.completed_steps as f32 / progress.total_steps as f32,
                        )
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                let progress_label = progress
                    .map(|progress| progress.message.as_str())
                    .unwrap_or("Starting export...");
                let progress_counts = progress
                    .map(|progress| {
                        format!(
                            "{} of {} steps complete",
                            progress.completed_steps.min(progress.total_steps),
                            progress.total_steps
                        )
                    })
                    .unwrap_or_else(|| "Preparing export task...".to_owned());

                ui.horizontal(|ui| {
                    ui.spinner();
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_vtmpack_progress_title", instance_id),
                        "Export in progress",
                        &style::stat_label(ui),
                    );
                });
                ui.add_space(12.0);
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_progress_message", instance_id),
                    progress_label,
                    &body_style,
                );
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_progress_counts", instance_id),
                    progress_counts.as_str(),
                    &body_style,
                );
                if let Some(path) = state.export_vtmpack_output_path.as_ref() {
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_vtmpack_progress_path", instance_id),
                        &format!("Destination: {}", path.display()),
                        &style::muted(ui),
                    );
                }
                ui.add_space(10.0);
                ui.add(
                    egui::ProgressBar::new(progress_fraction)
                        .desired_width(ui.available_width())
                        .show_percentage(),
                );
            } else {
                for provider_mode in [
                    VtmpackProviderMode::IncludeCurseForge,
                    VtmpackProviderMode::ExcludeCurseForge,
                ] {
                    let selected = state.export_vtmpack_options.provider_mode == provider_mode;
                    if ui.radio(selected, provider_mode.label()).clicked() {
                        state.export_vtmpack_options.provider_mode = provider_mode;
                    }
                }

                ui.add_space(12.0);
                let explanation = match state.export_vtmpack_options.provider_mode {
                    VtmpackProviderMode::IncludeCurseForge => {
                        "Managed CurseForge entries stay downloadable in the pack manifest."
                    }
                    VtmpackProviderMode::ExcludeCurseForge => {
                        "CurseForge metadata is removed from the export. CurseForge-managed files are bundled into the pack unless they already use Modrinth as the selected source."
                    }
                };
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_explanation", instance_id),
                    explanation,
                    &body_style,
                );

                ui.add_space(16.0);
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_include_label", instance_id),
                    "Include top-level entries from the Minecraft root",
                    &style::stat_label(ui),
                );
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_include_help", instance_id),
                    "Defaults to mods, resourcepacks, shaderpacks, and config. You can also include any other top-level files or folders found in the instance root.",
                    &body_style,
                );
                ui.add_space(8.0);

                if let Some(instance_root) = instance_root.as_deref() {
                    if !instance_root.is_dir() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_export_vtmpack_missing_root", instance_id),
                            &format!(
                                "Instance root directory not found: {}",
                                instance_root.display()
                            ),
                            &body_style,
                        );
                    } else {
                        let entries = list_exportable_root_entries(instance_root);
                        ui.set_width(ui.available_width());
                        if entries.is_empty() {
                            let _ = text_ui.label(
                                ui,
                                ("instance_export_vtmpack_empty_root", instance_id),
                                "No files or folders found in the instance root.",
                                &body_style,
                            );
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt(("instance_export_vtmpack_entries_scroll", instance_id))
                                .max_height(360.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    for entry in entries {
                                        let checked = state
                                            .export_vtmpack_options
                                            .included_root_entries
                                            .entry(entry.clone())
                                            .or_insert_with(|| {
                                                default_vtmpack_root_entry_selected(&entry)
                                            });
                                        let label = if instance_root.join(entry.as_str()).is_dir() {
                                            format!("{entry}/")
                                        } else {
                                            entry.clone()
                                        };
                                        ui.checkbox(checked, label);
                                    }
                                });
                        }
                    }
                } else {
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_vtmpack_missing_instance", instance_id),
                        "Instance root is unavailable, so folder selection cannot be shown.",
                        &body_style,
                    );
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_vtmpack_cancel", instance_id),
                            "Cancel",
                            &secondary_button(ui, egui::vec2(120.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_vtmpack_confirm", instance_id),
                            "Choose file",
                            &primary_button(ui, egui::vec2(140.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        export_requested = true;
                    }
                });
            }
        },
    );
    close_requested |= response.close_requested;

    if close_requested && !state.export_vtmpack_in_flight {
        state.show_export_vtmpack_modal = false;
    }

    if export_requested {
        if let Some(instance) = instances.find(instance_id) {
            let instance_root = instances::instance_root_path(&installations_root, instance);
            let default_file_name = default_vtmpack_file_name(instance.name.as_str());
            let selected_output = rfd::FileDialog::new()
                .set_title("Export Modpack")
                .set_file_name(default_file_name.as_str())
                .add_filter("Vertex Modpack", &[VTMPACK_EXTENSION])
                .save_file();

            if let Some(selected_path) = selected_output {
                let output_path = enforce_vtmpack_extension(selected_path);
                let pack_instance = VtmpackInstanceMetadata {
                    id: instance.id.clone(),
                    name: instance.name.clone(),
                    game_version: instance.game_version.clone(),
                    modloader: instance.modloader.clone(),
                    modloader_version: instance.modloader_version.clone(),
                };
                request_vtmpack_export(
                    state,
                    pack_instance,
                    instance_root,
                    output_path,
                    state.export_vtmpack_options.clone(),
                );
                state.show_export_vtmpack_modal = true;
            }
        } else {
            state.status_message = Some("Instance was removed before export.".to_owned());
            state.show_export_vtmpack_modal = false;
        }
    }

    state.show_export_vtmpack_modal =
        state.show_export_vtmpack_modal || state.export_vtmpack_in_flight;
}

fn ensure_vtmpack_export_channels(state: &mut InstanceScreenState) {
    if state.export_vtmpack_progress_tx.is_none() || state.export_vtmpack_progress_rx.is_none() {
        let (tx, rx) = mpsc::channel();
        state.export_vtmpack_progress_tx = Some(tx);
        state.export_vtmpack_progress_rx = Some(Arc::new(Mutex::new(rx)));
    }
    if state.export_vtmpack_results_tx.is_none() || state.export_vtmpack_results_rx.is_none() {
        let (tx, rx) = mpsc::channel();
        state.export_vtmpack_results_tx = Some(tx);
        state.export_vtmpack_results_rx = Some(Arc::new(Mutex::new(rx)));
    }
}

fn request_vtmpack_export(
    state: &mut InstanceScreenState,
    instance: VtmpackInstanceMetadata,
    instance_root: PathBuf,
    output_path: PathBuf,
    options: vtmpack::VtmpackExportOptions,
) {
    if state.export_vtmpack_in_flight {
        state.show_export_vtmpack_modal = true;
        return;
    }

    ensure_vtmpack_export_channels(state);
    let Some(progress_tx) = state.export_vtmpack_progress_tx.as_ref().cloned() else {
        state.status_message = Some("Failed to start .vtmpack export progress channel.".to_owned());
        return;
    };
    let Some(results_tx) = state.export_vtmpack_results_tx.as_ref().cloned() else {
        state.status_message = Some("Failed to start .vtmpack export result channel.".to_owned());
        return;
    };

    state.export_vtmpack_in_flight = true;
    state.export_vtmpack_output_path = Some(output_path.clone());
    state.export_vtmpack_latest_progress = None;
    state.show_export_vtmpack_modal = true;
    state.status_message = Some(format!(
        "Exporting {} to {}...",
        instance.name,
        output_path.display()
    ));

    let instance_name = instance.name.clone();
    let export_path_for_task = output_path.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let instance_name_for_progress = instance_name.clone();
        let output_path_for_progress = output_path.clone();
        let result = export_instance_as_vtmpack_with_progress(
            &instance,
            instance_root.as_path(),
            export_path_for_task.as_path(),
            &options,
            |progress| {
                if let Err(err) = progress_tx.send(progress) {
                    tracing::error!(
                        target: "vertexlauncher/instance_export",
                        instance_name = %instance_name_for_progress,
                        output_path = %output_path_for_progress.display(),
                        error = %err,
                        "Failed to deliver vtmpack export progress update."
                    );
                }
            },
        );
        if let Err(err) = results_tx.send(VtmpackExportOutcome {
            instance_name,
            output_path,
            result,
        }) {
            tracing::error!(
                target: "vertexlauncher/instance_export",
                error = %err,
                "Failed to deliver vtmpack export result."
            );
        }
    });
}

fn poll_vtmpack_export_progress(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.export_vtmpack_progress_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            "vtmpack export progress worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    "vtmpack export progress receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel && !state.export_vtmpack_in_flight {
        state.export_vtmpack_progress_tx = None;
        state.export_vtmpack_progress_rx = None;
    }

    if let Some(update) = updates.into_iter().last() {
        state.export_vtmpack_latest_progress = Some(update);
    }
}

fn poll_vtmpack_export_results(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.export_vtmpack_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            "vtmpack export result worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    "vtmpack export result receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    for update in updates {
        state.export_vtmpack_in_flight = false;
        state.export_vtmpack_latest_progress = None;
        state.export_vtmpack_output_path = None;
        state.show_export_vtmpack_modal = false;
        match update.result {
            Ok(stats) => {
                state.status_message = Some(format!(
                    "Exported {} ({} bundled mods, {} config files, {} additional files) to {}",
                    update.instance_name,
                    stats.bundled_mod_files,
                    stats.config_files,
                    stats.additional_files,
                    update.output_path.display()
                ));
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    instance_name = %update.instance_name,
                    output_path = %update.output_path.display(),
                    error = %err,
                    "vtmpack export failed."
                );
                state.status_message = Some(format!("Failed to export .vtmpack: {err}"));
            }
        }
    }

    if should_reset_channel && state.export_vtmpack_in_flight {
        state.export_vtmpack_in_flight = false;
        state.export_vtmpack_latest_progress = None;
        state.export_vtmpack_output_path = None;
        state.show_export_vtmpack_modal = false;
        state.status_message =
            Some("Failed to export .vtmpack: export task stopped unexpectedly.".to_owned());
    }

    if should_reset_channel || !state.export_vtmpack_in_flight {
        state.export_vtmpack_progress_tx = None;
        state.export_vtmpack_progress_rx = None;
        state.export_vtmpack_results_tx = None;
        state.export_vtmpack_results_rx = None;
    }
}

fn default_server_root_entry_selected(entry: &str) -> bool {
    matches!(
        entry,
        "mods"
            | "config"
            | "defaultconfigs"
            | "kubejs"
            | "scripts"
            | "serverconfig"
            | "libraries"
            | "versions"
    )
}

fn sync_server_export_options(
    instance_root: &Path,
    included_root_entries: &mut BTreeMap<String, bool>,
) {
    let available_entries = list_exportable_root_entries(instance_root);
    let available_set = available_entries.iter().cloned().collect::<HashSet<_>>();
    included_root_entries.retain(|entry, _| available_set.contains(entry));
    for entry in available_entries {
        included_root_entries
            .entry(entry.clone())
            .or_insert_with(|| default_server_root_entry_selected(entry.as_str()));
    }
}

fn render_export_server_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &InstanceStore,
    config: &Config,
) {
    if !state.show_export_server_modal {
        return;
    }

    let mut close_requested = false;
    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let instance_root = instances
        .find(instance_id)
        .map(|instance| instances::instance_root_path(&installations_root, instance));
    if let Some(instance_root) = instance_root.as_deref() {
        sync_server_export_options(
            instance_root,
            &mut state.export_server_included_root_entries,
        );
    }
    let mut export_requested = false;
    let response = show_dialog(
        ctx,
        dialog_options(
            ("instance_export_server_modal", instance_id),
            DialogPreset::Form,
        ),
        |ui| {
            let title_style = style::modal_title(ui);
            let body_style = style::muted(ui);
            let _ = text_ui.label(
                ui,
                ("instance_export_server_title", instance_id),
                "Auto-generate server zip",
                &title_style,
            );
            let _ = text_ui.label(
                ui,
                ("instance_export_server_body", instance_id),
                "Builds a portable server package in your Downloads folder using this instance's files. CurseForge-managed mods that cannot be resolved on Modrinth by hash are listed as unknowns in the report.",
                &body_style,
            );
            ui.add_space(12.0);

            if state.export_server_in_flight {
                let progress = state.export_server_latest_progress.as_ref();
                let progress_fraction = progress
                    .and_then(|progress| {
                        (progress.total_steps > 0).then_some(
                            progress.completed_steps as f32 / progress.total_steps as f32,
                        )
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                let progress_label = progress
                    .map(|progress| progress.message.as_str())
                    .unwrap_or("Starting export...");
                let progress_counts = progress
                    .map(|progress| {
                        format!(
                            "{} of {} steps complete",
                            progress.completed_steps.min(progress.total_steps),
                            progress.total_steps
                        )
                    })
                    .unwrap_or_else(|| "Preparing export task...".to_owned());
                ui.horizontal(|ui| {
                    ui.spinner();
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_server_progress_title", instance_id),
                        "Server export in progress",
                        &style::stat_label(ui),
                    );
                });
                ui.add_space(12.0);
                let _ = text_ui.label(
                    ui,
                    ("instance_export_server_progress_message", instance_id),
                    progress_label,
                    &body_style,
                );
                let _ = text_ui.label(
                    ui,
                    ("instance_export_server_progress_counts", instance_id),
                    progress_counts.as_str(),
                    &body_style,
                );
                if let Some(path) = state.export_server_output_path.as_ref() {
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_server_progress_path", instance_id),
                        &format!("Destination: {}", path.display()),
                        &style::muted(ui),
                    );
                }
                ui.add_space(10.0);
                ui.add(
                    egui::ProgressBar::new(progress_fraction)
                        .desired_width(ui.available_width())
                        .show_percentage(),
                );
            } else {
                let _ = text_ui.label(
                    ui,
                    ("instance_export_server_include_label", instance_id),
                    "Include top-level entries from the Minecraft root",
                    &style::stat_label(ui),
                );
                let _ = text_ui.label(
                    ui,
                    ("instance_export_server_include_help", instance_id),
                    "Defaults to common server directories. You can enable or disable any top-level file or folder before export.",
                    &body_style,
                );
                ui.add_space(8.0);

                if let Some(instance_root) = instance_root.as_deref() {
                    let entries = list_exportable_root_entries(instance_root);
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt(("instance_export_server_entries_scroll", instance_id))
                        .max_height(360.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for entry in entries {
                                let checked = state
                                    .export_server_included_root_entries
                                    .entry(entry.clone())
                                    .or_insert_with(|| {
                                        default_server_root_entry_selected(entry.as_str())
                                    });
                                let label = if instance_root.join(entry.as_str()).is_dir() {
                                    format!("{entry}/")
                                } else {
                                    entry.clone()
                                };
                                ui.checkbox(checked, label);
                            }
                        });
                } else {
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_server_missing_instance", instance_id),
                        "Instance root is unavailable, so folder selection cannot be shown.",
                        &body_style,
                    );
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_server_cancel", instance_id),
                            "Cancel",
                            &secondary_button(ui, egui::vec2(120.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_server_confirm", instance_id),
                            "Build zip in Downloads",
                            &primary_button(ui, egui::vec2(196.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        export_requested = true;
                    }
                });
            }
        },
    );
    close_requested |= response.close_requested;

    if close_requested && !state.export_server_in_flight {
        state.show_export_server_modal = false;
    }

    if export_requested {
        if let Some(instance) = instances.find(instance_id) {
            let instance_root = instances::instance_root_path(&installations_root, instance);
            let output_path = default_server_export_output_path(instance, config);
            request_server_export(
                state,
                instance.clone(),
                instance_root,
                output_path,
                state.export_server_included_root_entries.clone(),
                config.force_java_21_minimum(),
            );
            state.show_export_server_modal = true;
        } else {
            state.status_message = Some("Instance was removed before export.".to_owned());
            state.show_export_server_modal = false;
        }
    }

    state.show_export_server_modal =
        state.show_export_server_modal || state.export_server_in_flight;
}

fn ensure_server_export_channels(state: &mut InstanceScreenState) {
    if state.export_server_progress_tx.is_none() || state.export_server_progress_rx.is_none() {
        let (tx, rx) = mpsc::channel();
        state.export_server_progress_tx = Some(tx);
        state.export_server_progress_rx = Some(Arc::new(Mutex::new(rx)));
    }
    if state.export_server_results_tx.is_none() || state.export_server_results_rx.is_none() {
        let (tx, rx) = mpsc::channel();
        state.export_server_results_tx = Some(tx);
        state.export_server_results_rx = Some(Arc::new(Mutex::new(rx)));
    }
}

fn request_server_export(
    state: &mut InstanceScreenState,
    instance: instances::InstanceRecord,
    instance_root: PathBuf,
    output_path: PathBuf,
    included_root_entries: BTreeMap<String, bool>,
    force_java_21_minimum: bool,
) {
    if state.export_server_in_flight {
        state.show_export_server_modal = true;
        return;
    }

    ensure_server_export_channels(state);
    let Some(progress_tx) = state.export_server_progress_tx.as_ref().cloned() else {
        state.status_message = Some("Failed to start server export progress channel.".to_owned());
        return;
    };
    let Some(results_tx) = state.export_server_results_tx.as_ref().cloned() else {
        state.status_message = Some("Failed to start server export result channel.".to_owned());
        return;
    };

    state.export_server_in_flight = true;
    state.export_server_output_path = Some(output_path.clone());
    state.export_server_latest_progress = None;
    state.show_export_server_modal = true;
    state.status_message = Some(format!(
        "Building server zip for {} at {}...",
        instance.name,
        output_path.display()
    ));

    let instance_name = instance.name.clone();
    let output_path_for_task = output_path.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let instance_name_for_progress = instance_name.clone();
        let output_path_for_progress = output_path.clone();
        let result = tokio_runtime::spawn_blocking(move || {
            export_instance_as_server_zip_with_progress(
                &instance,
                instance_root.as_path(),
                output_path_for_task.as_path(),
                &included_root_entries,
                force_java_21_minimum,
                |progress| {
                    if let Err(err) = progress_tx.send(progress) {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            instance_name = %instance_name_for_progress,
                            output_path = %output_path_for_progress.display(),
                            error = %err,
                            "Failed to deliver server export progress update."
                        );
                    }
                },
            )
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        if let Err(err) = results_tx.send(ServerExportOutcome {
            instance_name,
            output_path,
            result,
        }) {
            tracing::error!(
                target: "vertexlauncher/instance_export",
                error = %err,
                "Failed to deliver server export result."
            );
        }
    });
}

fn poll_server_export_progress(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.export_server_progress_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            "Server export progress worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    "Server export progress receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel && !state.export_server_in_flight {
        state.export_server_progress_tx = None;
        state.export_server_progress_rx = None;
    }

    if let Some(update) = updates.into_iter().last() {
        state.export_server_latest_progress = Some(update);
    }
}

fn poll_server_export_results(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.export_server_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            "Server export result worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    "Server export result receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    for update in updates {
        state.export_server_in_flight = false;
        state.export_server_latest_progress = None;
        state.export_server_output_path = None;
        state.show_export_server_modal = false;
        match update.result {
            Ok(summary) => {
                state.status_message = Some(format!(
                    "Server zip ready for {} at {}. {summary}",
                    update.instance_name,
                    update.output_path.display()
                ));
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    instance_name = %update.instance_name,
                    output_path = %update.output_path.display(),
                    error = %err,
                    "Server export failed."
                );
                state.status_message = Some(format!("Failed to export server zip: {err}"));
            }
        }
    }

    if should_reset_channel && state.export_server_in_flight {
        state.export_server_in_flight = false;
        state.export_server_latest_progress = None;
        state.export_server_output_path = None;
        state.show_export_server_modal = false;
        state.status_message =
            Some("Failed to export server zip: export task stopped unexpectedly.".to_owned());
    }

    if should_reset_channel || !state.export_server_in_flight {
        state.export_server_progress_tx = None;
        state.export_server_progress_rx = None;
        state.export_server_results_tx = None;
        state.export_server_results_rx = None;
    }
}

fn default_server_export_output_path(
    instance: &instances::InstanceRecord,
    config: &Config,
) -> PathBuf {
    let downloads_dir = UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(PathBuf::from))
        .unwrap_or_else(|| config.minecraft_installations_root_path().to_path_buf());
    let base_name = format!(
        "{}-server-{}-{}",
        sanitize_file_stem(instance.name.as_str()),
        sanitize_file_stem(instance.game_version.as_str()),
        sanitize_file_stem(instance.modloader.as_str()),
    );
    unique_file_path(downloads_dir.as_path(), base_name.as_str(), "zip")
}

fn sanitize_file_stem(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        let keep = lower.is_ascii_alphanumeric();
        if keep {
            out.push(lower);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "instance".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn unique_file_path(parent: &Path, stem: &str, extension: &str) -> PathBuf {
    let mut attempt = 0u32;
    loop {
        let file_name = if attempt == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{attempt}.{extension}")
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
        attempt = attempt.saturating_add(1);
    }
}

fn export_instance_as_server_zip_with_progress<F>(
    instance: &instances::InstanceRecord,
    instance_root: &Path,
    output_path: &Path,
    included_root_entries: &BTreeMap<String, bool>,
    force_java_21_minimum: bool,
    mut progress: F,
) -> Result<String, String>
where
    F: FnMut(vtmpack::VtmpackExportProgress),
{
    progress(vtmpack::VtmpackExportProgress {
        message: "Scanning instance files...".to_owned(),
        completed_steps: 0,
        total_steps: 1,
    });
    let included_files = collect_server_export_files(instance_root, included_root_entries)?;
    let manifest = managed_content::load_content_manifest(instance_root);
    let unknowns = classify_curseforge_unknown_mods_for_server_export(
        instance_root,
        &manifest,
        &mut progress,
        included_files.len(),
    );

    let required_java = runtime::required_java_major(instance.game_version.as_str()).map(|major| {
        if force_java_21_minimum && major < 21 {
            21
        } else {
            major
        }
    });

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create server export directory {}: {err}",
                parent.display()
            )
        })?;
    }

    let output_file = fs::File::create(output_path)
        .map_err(|err| format!("failed to create {}: {err}", output_path.display()))?;
    let mut zip = zip::ZipWriter::new(output_file);
    let file_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let script_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let mut completed_steps = 0usize;
    let total_steps = included_files.len() + 5;
    for file in &included_files {
        let relative = file.strip_prefix(instance_root).unwrap_or(file.as_path());
        let zip_path = normalize_zip_path(relative);
        progress(vtmpack::VtmpackExportProgress {
            message: format!("Adding {}", relative.display()),
            completed_steps,
            total_steps,
        });
        zip.start_file(zip_path.as_str(), file_options)
            .map_err(|err| format!("failed to add {zip_path} to zip: {err}"))?;
        let mut input = fs::File::open(file.as_path())
            .map_err(|err| format!("failed to open {}: {err}", file.display()))?;
        std::io::copy(&mut input, &mut zip)
            .map_err(|err| format!("failed to write {} to zip: {err}", file.display()))?;
        completed_steps += 1;
    }

    progress(vtmpack::VtmpackExportProgress {
        message: "Writing launch scripts...".to_owned(),
        completed_steps,
        total_steps,
    });
    zip.start_file("start-server.sh", script_options)
        .map_err(|err| format!("failed to add start-server.sh: {err}"))?;
    zip.write_all(build_server_start_script_sh(required_java).as_bytes())
        .map_err(|err| format!("failed to write start-server.sh: {err}"))?;
    completed_steps += 1;

    zip.start_file("start-server.bat", script_options)
        .map_err(|err| format!("failed to add start-server.bat: {err}"))?;
    zip.write_all(build_server_start_script_bat(required_java).as_bytes())
        .map_err(|err| format!("failed to write start-server.bat: {err}"))?;
    completed_steps += 1;

    progress(vtmpack::VtmpackExportProgress {
        message: "Writing server build report...".to_owned(),
        completed_steps,
        total_steps,
    });
    zip.start_file("VERTEX_SERVER_BUILD_REPORT.txt", file_options)
        .map_err(|err| format!("failed to add VERTEX_SERVER_BUILD_REPORT.txt: {err}"))?;
    zip.write_all(
        build_server_export_report(instance, included_root_entries, required_java, &unknowns)
            .as_bytes(),
    )
    .map_err(|err| format!("failed to write server build report: {err}"))?;
    completed_steps += 1;

    progress(vtmpack::VtmpackExportProgress {
        message: "Writing EULA placeholder...".to_owned(),
        completed_steps,
        total_steps,
    });
    zip.start_file("eula.txt", file_options)
        .map_err(|err| format!("failed to add eula.txt: {err}"))?;
    zip.write_all(b"eula=false\n")
        .map_err(|err| format!("failed to write eula.txt: {err}"))?;
    completed_steps += 1;

    progress(vtmpack::VtmpackExportProgress {
        message: "Finalizing zip archive...".to_owned(),
        completed_steps,
        total_steps,
    });
    let _ = zip
        .finish()
        .map_err(|err| format!("failed to finalize zip archive: {err}"))?;
    completed_steps += 1;

    progress(vtmpack::VtmpackExportProgress {
        message: "Server export complete.".to_owned(),
        completed_steps,
        total_steps,
    });
    Ok(format!(
        "{} files included, {} CurseForge-managed unknown mods flagged.",
        included_files.len(),
        unknowns.len()
    ))
}

fn collect_server_export_files(
    instance_root: &Path,
    included_root_entries: &BTreeMap<String, bool>,
) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for (entry, included) in included_root_entries {
        if !*included {
            continue;
        }
        let path = instance_root.join(entry);
        if !path.exists() {
            continue;
        }
        if path.is_file() {
            files.push(path);
            continue;
        }
        collect_regular_files_recursive(path.as_path(), &mut files)
            .map_err(|err| format!("failed to collect files under {}: {err}", path.display()))?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_regular_files_recursive(root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let entries = fs::read_dir(root)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_regular_files_recursive(path.as_path(), out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn classify_curseforge_unknown_mods_for_server_export<F>(
    instance_root: &Path,
    manifest: &managed_content::ContentInstallManifest,
    progress: &mut F,
    initial_step_offset: usize,
) -> Vec<String>
where
    F: FnMut(vtmpack::VtmpackExportProgress),
{
    let candidates = manifest
        .projects
        .values()
        .filter(|project| {
            project.selected_source == Some(managed_content::ManagedContentSource::CurseForge)
                && normalize_path_key(project.file_path.as_path()).starts_with("mods/")
        })
        .map(|project| {
            let display_name = project.name.trim().to_owned();
            let name = if display_name.is_empty() {
                project.file_path.to_string_lossy().into_owned()
            } else {
                display_name
            };
            (
                name,
                instance_root.join(project.file_path.as_path()),
                project.file_path.to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();
    let mut unknowns = Vec::new();
    if candidates.is_empty() {
        return unknowns;
    }

    let modrinth = modrinth::Client::default();
    let total_steps = initial_step_offset + candidates.len() + 1;
    let mut completed = initial_step_offset;
    for (name, path, relative) in candidates {
        progress(vtmpack::VtmpackExportProgress {
            message: format!("Classifying CurseForge mod {relative}..."),
            completed_steps: completed,
            total_steps,
        });
        completed += 1;

        if !path.is_file() {
            unknowns.push(format!("{name} ({relative}) - file missing"));
            continue;
        }
        let hashes = match modrinth::hash_file_sha1_and_sha512_hex(path.as_path()) {
            Ok(values) => values,
            Err(error) => {
                unknowns.push(format!("{name} ({relative}) - hash failed: {error}"));
                continue;
            }
        };
        let (sha1, sha512) = hashes;
        let matched = modrinth
            .get_version_from_hash(sha512.as_str(), "sha512")
            .ok()
            .flatten()
            .is_some()
            || modrinth
                .get_version_from_hash(sha1.as_str(), "sha1")
                .ok()
                .flatten()
                .is_some();
        if !matched {
            unknowns.push(format!("{name} ({relative})"));
        }
    }
    progress(vtmpack::VtmpackExportProgress {
        message: "CurseForge unknown-classification complete.".to_owned(),
        completed_steps: completed,
        total_steps,
    });
    unknowns
}

fn build_server_export_report(
    instance: &instances::InstanceRecord,
    included_root_entries: &BTreeMap<String, bool>,
    required_java: Option<u8>,
    unknowns: &[String],
) -> String {
    let mut report = String::new();
    report.push_str("Vertex Auto-Generated Server Package\n");
    report.push_str("===================================\n\n");
    report.push_str(format!("Instance: {}\n", instance.name).as_str());
    report.push_str(format!("Minecraft: {}\n", instance.game_version).as_str());
    report.push_str(format!("Modloader: {}\n", instance.modloader).as_str());
    if let Some(version) = normalize_optional(instance.modloader_version.as_str()) {
        report.push_str(format!("Modloader version: {version}\n").as_str());
    }
    if let Some(java_major) = required_java {
        report.push_str(format!("Recommended Java: {java_major}\n").as_str());
    } else {
        report.push_str("Recommended Java: unknown\n");
    }
    report.push('\n');
    report.push_str("Included top-level entries:\n");
    for entry in included_root_entries
        .iter()
        .filter_map(|(entry, enabled)| enabled.then_some(entry.as_str()))
    {
        report.push_str(format!("- {entry}\n").as_str());
    }
    report.push('\n');
    if unknowns.is_empty() {
        report.push_str("CurseForge unknowns:\n- none\n\n");
    } else {
        report.push_str("CurseForge unknowns (not found on Modrinth by hash):\n");
        for entry in unknowns {
            report.push_str(format!("- {entry}\n").as_str());
        }
        report.push('\n');
    }
    report.push_str("How to start:\n");
    report.push_str("- Linux/macOS: ./start-server.sh\n");
    report.push_str("- Windows: start-server.bat\n");
    report.push_str(
        "- If startup fails, check unknowns above first. Some mods may still be required or incompatible on dedicated server.\n",
    );
    report
}

fn build_server_start_script_sh(required_java: Option<u8>) -> String {
    let java_note = required_java
        .map(|major| format!("echo \"Recommended Java major: {major}\""))
        .unwrap_or_else(|| "echo \"Recommended Java major: unknown\"".to_owned());
    format!(
        "#!/usr/bin/env bash
set -euo pipefail
{java_note}

jar=\"$(find . -maxdepth 6 -type f \\( -iname '*server*.jar' -o -iname '*forge*.jar' -o -iname '*fabric*.jar' -o -iname '*quilt*.jar' -o -iname '*neoforge*.jar' \\) | head -n 1)\"
if [ -z \"$jar\" ]; then
  jar=\"$(find . -maxdepth 6 -type f -iname '*.jar' | head -n 1)\"
fi
jar=\"${{jar#./}}\"

if [ -z \"$jar\" ]; then
  echo \"No server jar found in this directory.\"
  echo \"Check VERTEX_SERVER_BUILD_REPORT.txt for guidance.\"
  exit 1
fi

echo \"Starting server using $jar\"
java -Xms2G -Xmx4G -jar \"$jar\" nogui
"
    )
}

fn build_server_start_script_bat(required_java: Option<u8>) -> String {
    let java_note = required_java
        .map(|major| format!("echo Recommended Java major: {major}"))
        .unwrap_or_else(|| "echo Recommended Java major: unknown".to_owned());
    format!(
        "@echo off
setlocal EnableDelayedExpansion
{java_note}

set \"jar=\"
for /r %%f in (*server*.jar) do (
  if not defined jar set \"jar=%%f\"
)
if not defined jar (
  for /r %%f in (*forge*.jar *fabric*.jar *quilt*.jar *neoforge*.jar) do (
    if not defined jar set \"jar=%%f\"
  )
)
if not defined jar (
  for /r %%f in (*.jar) do (
  if not defined jar set \"jar=%%f\"
  )
)

if not defined jar (
  echo No server jar found in this directory.
  echo Check VERTEX_SERVER_BUILD_REPORT.txt for guidance.
  exit /b 1
)

echo Starting server using !jar!
java -Xms2G -Xmx4G -jar \"!jar!\" nogui
"
    )
}

fn normalize_zip_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn modified_millis(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(duration.as_millis() as u64)
}

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn format_time_ago(timestamp_ms: Option<u64>, now_ms: u64) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "unknown".to_owned();
    };
    let elapsed_ms = now_ms.saturating_sub(timestamp_ms);
    let elapsed_seconds = elapsed_ms / 1_000;
    if elapsed_seconds < 60 {
        return "just now".to_owned();
    }
    let elapsed_minutes = elapsed_seconds / 60;
    if elapsed_minutes < 60 {
        return format!("{elapsed_minutes}m ago");
    }
    let elapsed_hours = elapsed_minutes / 60;
    if elapsed_hours < 24 {
        return format!("{elapsed_hours}h ago");
    }
    let elapsed_days = elapsed_hours / 24;
    if elapsed_days < 30 {
        return format!("{elapsed_days}d ago");
    }
    let elapsed_months = elapsed_days / 30;
    if elapsed_months < 12 {
        return format!("{elapsed_months}mo ago");
    }
    let elapsed_years = elapsed_days / 365;
    format!("{elapsed_years}y ago")
}

fn apply_color_to_svg(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}

fn ensure_selected_modloader_is_supported(state: &mut InstanceScreenState, game_version: &str) {
    if !support_catalog_ready(state) {
        state.incompatible_modloader_version_warning_key = None;
        return;
    }
    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
        state.incompatible_modloader_version_warning_key = None;
        return;
    }

    let selected_label = MODLOADER_OPTIONS
        .get(state.selected_modloader)
        .copied()
        .unwrap_or(MODLOADER_OPTIONS[0]);
    let entered_modloader_version = state.modloader_version_input.trim();
    if entered_modloader_version.is_empty()
        || is_latest_modloader_version_alias(entered_modloader_version)
    {
        state.incompatible_modloader_version_warning_key = None;
        return;
    }
    let Some(known_versions) = state
        .loader_versions
        .versions_for_loader(selected_label, game_version)
    else {
        state.incompatible_modloader_version_warning_key = None;
        return;
    };
    if known_versions
        .iter()
        .any(|version| version.eq_ignore_ascii_case(entered_modloader_version))
    {
        state.incompatible_modloader_version_warning_key = None;
        return;
    }

    let warning_key = format!(
        "{}\n{}\n{}",
        selected_label, game_version, entered_modloader_version
    );
    if state.incompatible_modloader_version_warning_key.as_deref() == Some(warning_key.as_str()) {
        return;
    }
    state.incompatible_modloader_version_warning_key = Some(warning_key);

    tracing::warn!(
        target: "vertexlauncher/ui/instance",
        selected_modloader = %selected_label,
        game_version = %game_version,
        selected_modloader_version = %entered_modloader_version,
        "Selected modloader version is not currently marked compatible for this game version; keeping user selection."
    );
}

fn support_catalog_ready(state: &InstanceScreenState) -> bool {
    state.version_catalog_include_snapshots.is_some() && state.version_catalog_error.is_none()
}

fn validate_move_dest(path_str: &str) -> Result<PathBuf, String> {
    let path_str = path_str.trim();
    if path_str.is_empty() {
        return Err("Please enter or browse to a destination folder.".to_owned());
    }
    let path = PathBuf::from(path_str);
    if path.is_dir() {
        // Existing folder is only valid if it's empty.
        match std::fs::read_dir(&path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err("The destination folder must be empty.".to_owned());
                }
            }
            Err(err) => {
                return Err(format!("Cannot read destination folder: {err}"));
            }
        }
        return Ok(path);
    }
    if path.exists() {
        return Err("The destination path already exists and is not a folder.".to_owned());
    }
    // Non-existent path: parent directory must exist.
    match path.parent() {
        Some(parent) if parent.as_os_str().is_empty() || parent.is_dir() => {}
        Some(parent) => {
            return Err(format!(
                "Parent folder does not exist: {}",
                parent.display()
            ));
        }
        None => {
            return Err("The destination path is invalid.".to_owned());
        }
    }
    Ok(path)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn compact_path_label(path: &str, max_chars: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_chars || max_chars < 9 {
        return path.to_owned();
    }

    let keep_each_side = (max_chars.saturating_sub(3)) / 2;
    let prefix: String = path.chars().take(keep_each_side).collect();
    let suffix: String = path
        .chars()
        .rev()
        .take(keep_each_side)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

fn render_move_instance_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &InstanceStore,
    config: &Config,
) {
    if !state.show_move_instance_modal {
        return;
    }
    let showing_move_progress =
        state.move_instance_in_flight || state.move_instance_completion_message.is_some();
    let modal_layout = if showing_move_progress {
        modal::ModalLayout::centered(
            modal::AxisSizing::new(0.52, 0.0, f32::INFINITY),
            modal::AxisSizing::new(0.34, 0.0, f32::INFINITY),
        )
        .with_viewport_margin_fraction(egui::vec2(0.04, 0.06))
    } else {
        modal::ModalLayout::centered(
            modal::AxisSizing::new(0.48, 0.0, f32::INFINITY),
            modal::AxisSizing::new(0.24, 0.0, f32::INFINITY),
        )
        .with_viewport_margin_fraction(egui::vec2(0.04, 0.08))
    };
    let mut close_requested = false;

    let mut move_requested = false;
    let response = modal::show_window(
        ctx,
        "Move Instance",
        modal::ModalOptions::new(
            egui::Id::new(("instance_move_modal", instance_id)),
            modal_layout,
        )
        .with_layer(modal::ModalLayer::Elevated)
        .with_dismiss_behavior(if state.move_instance_in_flight {
            modal::DismissBehavior::None
        } else {
            modal::DismissBehavior::EscapeAndScrim
        }),
        |ui| {
            let body_style = style::muted(ui);
            let title_style = style::modal_title(ui);
            let error_style = style::error_text(ui);
            let action_button_style = ButtonOptions::default();

            let _ = text_ui.label(
                ui,
                ("instance_move_title", instance_id),
                if state.move_instance_in_flight {
                    "Moving Instance"
                } else if state.move_instance_completion_message.is_some() {
                    if state.move_instance_completion_failed {
                        "Move Failed"
                    } else {
                        "Move Complete"
                    }
                } else {
                    "Move Instance"
                },
                &title_style,
            );
            ui.add_space(6.0);
            let _ = text_ui.label(
                ui,
                ("instance_move_body", instance_id),
                if state.move_instance_in_flight {
                    "Vertex is moving this instance now."
                } else if state.move_instance_completion_message.is_some() {
                    "Review the result, then close this dialog."
                } else {
                    "Choose a destination folder. It can be an empty existing folder or a new path that will be created."
                },
                &body_style,
            );
            ui.add_space(12.0);

            if !state.move_instance_in_flight && state.move_instance_completion_message.is_none() {
                let input_changed = ui
                    .horizontal(|ui| {
                        let input_width = (ui.available_width() - 124.0).max(180.0);
                        let response = themed_text_input(
                            text_ui,
                            ui,
                            ("move_instance_dest", instance_id),
                            &mut state.move_instance_dest_input,
                            InputOptions {
                                desired_width: Some(input_width),
                                placeholder_text: Some(
                                    "Choose an empty folder or enter a new destination path"
                                        .to_owned(),
                                ),
                                ..InputOptions::default()
                            },
                        );
                        let browse_clicked = text_ui
                            .button(
                                ui,
                                ("move_instance_browse", instance_id),
                                "Browse...",
                                &action_button_style,
                            )
                            .clicked();
                        if browse_clicked {
                            if let Some(picked) = rfd::FileDialog::new()
                                .set_title("Choose Destination Folder")
                                .pick_folder()
                            {
                                state.move_instance_dest_input = picked.display().to_string();
                                return true;
                            }
                        }
                        response.changed()
                    })
                    .inner;

                if input_changed {
                    match validate_move_dest(&state.move_instance_dest_input) {
                        Ok(_) => {
                            state.move_instance_dest_valid = true;
                            state.move_instance_dest_error = None;
                        }
                        Err(msg) => {
                            state.move_instance_dest_valid = false;
                            state.move_instance_dest_error = Some(msg);
                        }
                    }
                }

                if let Some(ref err_msg) = state.move_instance_dest_error.clone() {
                    ui.add_space(4.0);
                    let _ = text_ui.label(
                        ui,
                        ("move_instance_error", instance_id),
                        err_msg.as_str(),
                        &error_style,
                    );
                }
            } else {
                let available_width = ui.available_width();
                // At 13pt, a typical path character is ~7px wide. Subtract prefix headroom.
                let path_char_budget = ((available_width / 7.0) as usize).max(16);
                let weak_text_color = ui.visuals().weak_text_color();

                let (
                    bytes_done,
                    total_bytes,
                    total_files,
                    files_done,
                    active_file_count,
                    active_file,
                ) = if let Some(ref progress) = state.move_instance_latest_progress {
                    (
                        progress.bytes_done,
                        progress.total_bytes,
                        progress.total_files,
                        progress.files_done,
                        progress.active_file_count,
                        progress.active_files.first().cloned(),
                    )
                } else {
                    (0, 0, 0, 0, 0, None)
                };
                let progress_fraction = if total_bytes > 0 {
                    (bytes_done as f64 / total_bytes as f64).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                let status_style = if state.move_instance_completion_failed {
                    LabelOptions { color: ui.visuals().error_fg_color, ..style::stat_label(ui) }
                } else {
                    style::stat_label(ui)
                };
                let detail_style = LabelOptions {
                    color: weak_text_color,
                    ..style::caption(ui)
                };
                let status_text = if state.move_instance_in_flight {
                    "Moving files..."
                } else if state.move_instance_completion_failed {
                    "Move failed"
                } else {
                    "Move complete"
                };
                let bytes_text = if total_bytes > 0 {
                    if state.move_instance_in_flight {
                        format!(
                            "{} / {}",
                            format_bytes(bytes_done),
                            format_bytes(total_bytes)
                        )
                    } else {
                        format!("{} transferred", format_bytes(total_bytes))
                    }
                } else if state.move_instance_in_flight {
                    "preparing...".to_owned()
                } else {
                    String::new()
                };
                let files_text = if total_files > 0 {
                    if state.move_instance_in_flight {
                        format!("{files_done} / {total_files} files")
                    } else {
                        format!("{total_files} files moved")
                    }
                } else if state.move_instance_in_flight {
                    "scanning files...".to_owned()
                } else {
                    String::new()
                };
                let active_text = if state.move_instance_in_flight {
                    if let Some(ref file_path) = active_file {
                        let prefix = "Current file: ";
                        let path_chars = path_char_budget.saturating_sub(prefix.len());
                        format!("{prefix}{}", compact_path_label(file_path, path_chars))
                    } else if active_file_count > 0 {
                        format!("{active_file_count} threads active")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                let destination_text = state.move_instance_dest_path.as_ref().map(|dest_path| {
                    let prefix = "Destination: ";
                    let path_str = dest_path.display().to_string();
                    let path_chars = path_char_budget.saturating_sub(prefix.len());
                    format!("{prefix}{}", compact_path_label(&path_str, path_chars))
                });

                // Header: status left, percentage right (when byte data is available)
                ui.horizontal(|ui| {
                    if state.move_instance_in_flight {
                        ui.spinner();
                        ui.add_space(2.0);
                    }
                    let _ = text_ui.label(
                        ui,
                        ("instance_move_progress_status_inline", instance_id),
                        status_text,
                        &status_style,
                    );
                    if total_bytes > 0 && state.move_instance_in_flight {
                        let pct = (progress_fraction * 100.0) as u32;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{pct}%"))
                                    .size(15.0)
                                    .strong()
                                    .color(weak_text_color),
                            );
                        });
                    }
                });
                ui.add_space(6.0);
                ui.add(
                    egui::ProgressBar::new(progress_fraction).desired_width(ui.available_width()),
                );
                ui.add_space(8.0);

                // Stats: two-column when wide enough (bytes left, file count right), else stacked
                let show_two_col =
                    available_width >= 300.0 && !bytes_text.is_empty() && !files_text.is_empty();
                if show_two_col {
                    ui.horizontal(|ui| {
                        let _ = text_ui.label(
                            ui,
                            ("instance_move_progress_bytes_inline", instance_id),
                            bytes_text.as_str(),
                            &detail_style,
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(files_text.as_str())
                                    .size(13.0)
                                    .color(weak_text_color),
                            );
                        });
                    });
                } else {
                    if !bytes_text.is_empty() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_move_progress_bytes_inline", instance_id),
                            bytes_text.as_str(),
                            &detail_style,
                        );
                    }
                    if !files_text.is_empty() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_move_progress_files_inline", instance_id),
                            files_text.as_str(),
                            &detail_style,
                        );
                    }
                }
                if !active_text.is_empty() {
                    let _ = text_ui.label(
                        ui,
                        ("instance_move_progress_active_inline", instance_id),
                        active_text.as_str(),
                        &detail_style,
                    );
                }
                if let Some(destination_text) = destination_text.as_deref() {
                    let _ = text_ui.label(
                        ui,
                        ("instance_move_progress_dest_inline", instance_id),
                        destination_text,
                        &detail_style,
                    );
                }
                if let Some(message) = state.move_instance_completion_message.as_deref() {
                    ui.add_space(6.0);
                    let _ = text_ui.label(
                        ui,
                        ("instance_move_progress_message_inline", instance_id),
                        message,
                        &if state.move_instance_completion_failed { style::error_text(ui) } else { style::body(ui) },
                    );
                }
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if !state.move_instance_in_flight
                    && state.move_instance_completion_message.is_none()
                {
                    let move_enabled = state.move_instance_dest_valid;
                    ui.add_enabled_ui(move_enabled, |ui| {
                        if text_ui
                            .button(
                                ui,
                                ("move_instance_confirm", instance_id),
                                "Move",
                                &ButtonOptions::default(),
                            )
                            .clicked()
                        {
                            move_requested = true;
                        }
                    });
                    ui.add_space(6.0);
                    if text_ui
                        .button(
                            ui,
                            ("move_instance_cancel", instance_id),
                            "Cancel",
                            &action_button_style,
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                } else {
                    let done_enabled = !state.move_instance_in_flight
                        && state.move_instance_completion_message.is_some();
                    let done_label = if state.move_instance_completion_failed {
                        "Close"
                    } else {
                        "Done"
                    };
                    let done_clicked = ui
                        .add_enabled_ui(done_enabled, |ui| {
                            text_ui.button(
                                ui,
                                ("move_instance_done_inline", instance_id),
                                done_label,
                                &action_button_style,
                            )
                        })
                        .inner
                        .clicked();
                    if done_clicked {
                        close_requested = true;
                    }
                }
            });
        },
    );

    if move_requested {
        if let Some(instance) = instances.find(instance_id) {
            let installations_root = config.minecraft_installations_root_path().to_path_buf();
            let source_root = instances::instance_root_path(&installations_root, instance);
            let dest_root = PathBuf::from(state.move_instance_dest_input.trim());
            state.move_instance_dest_path = Some(dest_root.clone());
            request_move_instance(state, source_root, dest_root);
            state.show_move_instance_modal = true;
            state.show_move_instance_progress_modal = false;
        }
    }

    if response.close_requested || close_requested {
        if !state.move_instance_in_flight {
            state.move_instance_completion_message = None;
            state.move_instance_completion_failed = false;
            state.move_instance_latest_progress = None;
            state.move_instance_dest_path = None;
        }
        state.show_move_instance_modal = false;
    }
}

fn render_move_instance_progress_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
) {
    let _ = (ctx, text_ui, instance_id, state);
}

fn memory_slider_max_mib() -> (u128, bool) {
    static CACHED: OnceLock<Mutex<MemorySliderMaxState>> = OnceLock::new();
    let cache = CACHED.get_or_init(|| Mutex::new(MemorySliderMaxState::default()));
    let mut total_mib = None;
    let mut pending = false;

    if let Ok(mut state) = cache.lock() {
        if !state.load_complete {
            if let Some(rx) = state.rx.as_ref() {
                match rx.try_recv() {
                    Ok(result) => {
                        state.detected_total_mib = result;
                        state.load_complete = true;
                        state.rx = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        pending = true;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        state.load_complete = true;
                        state.rx = None;
                    }
                }
            }

            if !state.load_complete && state.rx.is_none() {
                let (tx, rx) = mpsc::channel::<Option<u128>>();
                state.rx = Some(rx);
                pending = true;
                let _ = tokio_runtime::spawn_detached(async move {
                    let result = screen_platform::detect_total_memory_mib();
                    if let Err(err) = tx.send(result) {
                        tracing::error!(
                            target: "vertexlauncher/instance",
                            error = %err,
                            "Failed to deliver server-export memory probe result."
                        );
                    }
                });
            }
        }
        total_mib = state.detected_total_mib;
        pending |= !state.load_complete;
    }

    let max_mib = total_mib
        .unwrap_or(FALLBACK_TOTAL_MEMORY_MIB)
        .saturating_sub(RESERVED_SYSTEM_MEMORY_MIB)
        .max(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN);
    (max_mib, pending)
}

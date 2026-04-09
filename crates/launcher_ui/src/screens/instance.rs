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
    sync::Arc,
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
#[path = "instance_exports.rs"]
mod instance_exports;
#[path = "instance_logs.rs"]
mod instance_logs;
#[path = "instance_move_modal.rs"]
mod instance_move_modal;
mod instance_screen_output;
mod instance_screen_state;
#[path = "instance_screenshots.rs"]
mod instance_screenshots;
#[path = "instance_settings_modal.rs"]
mod instance_settings_modal;
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
use instance_exports::*;
use instance_logs::*;
use instance_move_modal::*;
pub use instance_screen_output::InstanceScreenOutput;
use instance_screen_state::{
    InstalledContentEntryUiCache, InstanceLogEntry, InstanceScreenState, InstanceScreenTab,
    InstanceScreenshotEntry, InstanceScreenshotViewerState, MoveInstanceResult,
    ServerExportOutcome, VtmpackExportOutcome,
};
use instance_screenshots::*;
use instance_settings_modal::*;
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

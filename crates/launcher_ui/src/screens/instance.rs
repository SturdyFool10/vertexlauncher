use config::{
    Config, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
    JavaRuntimeVersion,
};
use content_resolver::{
    InstalledContentFile, InstalledContentHashCache, InstalledContentKind,
    InstalledContentResolver, ResolveInstalledContentRequest,
};
use egui::Ui;
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
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    ffi::OsStr,
    fs,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use textui::{ButtonOptions, LabelOptions, TextUi, TooltipOptions};
use vtmpack::{
    VTMPACK_EXTENSION, VtmpackInstanceMetadata, VtmpackProviderMode, default_vtmpack_file_name,
    default_vtmpack_root_entry_selected, enforce_vtmpack_extension, export_instance_as_vtmpack,
    list_exportable_root_entries, sync_vtmpack_export_options,
};

use crate::app::tokio_runtime;
use crate::desktop;
use crate::screens::{AppScreen, LaunchAuthContext};
use crate::ui::{
    components::{
        icon_button,
        lazy_image_bytes::{LazyImageBytes, LazyImageBytesStatus},
        remote_tiled_image, settings_widgets,
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
    InstanceScreenshotEntry, InstanceScreenshotViewerState,
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
const INSTANCE_SCREENSHOT_PRELOAD_VIEWPORTS: f32 = 0.75;
const INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM: f32 = 1.0;
const INSTANCE_SCREENSHOT_VIEWER_MAX_ZOOM: f32 = 8.0;
const INSTANCE_SCREENSHOT_VIEWER_ZOOM_STEP: f32 = 0.2;
const MAX_INSTANCE_SCREENSHOTS: usize = 120;
const MAX_INSTANCE_LOG_LINES: usize = 12_000;
const INSTANCE_SCREENSHOT_COPY_BUTTON_SIZE: f32 = 28.0;

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

fn instance_screen_state_id(instance_id: &str) -> egui::Id {
    egui::Id::new(("instance_screen_state", instance_id))
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
                state.pending_delete_screenshot_key = None;
            }
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.screenshot_viewer.take().is_some() {
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.show_export_vtmpack_modal {
            state.show_export_vtmpack_modal = false;
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

    poll_background_tasks(&mut state, config, instances, instance_id);
    poll_instance_screenshot_scan_results(&mut state);
    poll_instance_log_scan_results(&mut state);
    poll_instance_log_load_results(&mut state);
    let screenshot_images_updated = state.screenshot_images.poll();
    sync_version_catalog(&mut state, config.include_snapshots_and_betas(), false);
    if state.version_catalog_in_flight
        || !state.modloader_versions_in_flight.is_empty()
        || state.runtime_prepare_in_flight
        || state.content_apply_in_flight
        || state.screenshot_scan_in_flight
        || state.delete_screenshot_in_flight
        || state.log_scan_in_flight
        || state.log_load_in_flight
    {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let selected_game_version_for_loader = selected_game_version(&state).to_owned();
    ensure_selected_modloader_is_supported(&mut state, selected_game_version_for_loader.as_str());

    let installations_root = std::path::PathBuf::from(config.minecraft_installations_root());
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
    render_export_vtmpack_modal(
        ui.ctx(),
        text_ui,
        instance_id,
        &mut state,
        instances,
        config,
    );
    ui.add_space(10.0);
    render_instance_tab_row(ui, &mut state.active_tab);
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
                &mut output,
            );

            let mut retained_image_keys = HashSet::new();
            retain_instance_viewer_image(&mut state, &mut retained_image_keys);
            state.screenshot_images.retain_loaded(&retained_image_keys);
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
            state.screenshot_images.retain_loaded(&retained_image_keys);
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
            state.screenshot_images.retain_loaded(&retained_image_keys);
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

    ui.ctx().data_mut(|d| d.insert_temp(state_id, state));
    output
}

fn render_instance_tab_row(ui: &mut Ui, active_tab: &mut InstanceScreenTab) {
    let item_spacing = 8.0;
    let tab_count = InstanceScreenTab::ALL.len() as f32;
    let width = ((ui.available_width() - item_spacing * (tab_count - 1.0)) / tab_count).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = item_spacing;
        for tab in InstanceScreenTab::ALL {
            let selected = *active_tab == tab;
            let button =
                egui::Button::new(egui::RichText::new(tab.label()).size(15.0).strong().color(
                    if selected {
                        ui.visuals().widgets.active.fg_stroke.color
                    } else {
                        ui.visuals().text_color()
                    },
                ))
                .min_size(egui::vec2(width, INSTANCE_TABS_HEIGHT))
                .fill(if selected {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                })
                .stroke(if selected {
                    ui.visuals().selection.stroke
                } else {
                    ui.visuals().widgets.inactive.bg_stroke
                })
                .corner_radius(egui::CornerRadius::same(10));
            if ui
                .add_sized([width, INSTANCE_TABS_HEIGHT], button)
                .clicked()
            {
                *active_tab = tab;
            }
        }
    });
}

fn render_instance_screenshot_gallery(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
    retained_image_keys: &mut HashSet<String>,
) {
    let title_style = style::heading(ui, 18.0, 24.0);
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

    let available_width = ui.available_width().max(1.0);
    let column_count: usize = if available_width >= 980.0 {
        3
    } else if available_width >= 620.0 {
        2
    } else {
        1
    };
    let column_width = ((available_width
        - INSTANCE_SCREENSHOT_TILE_GAP * (column_count.saturating_sub(1) as f32))
        / column_count as f32)
        .max(180.0);
    let assignments =
        build_instance_screenshot_mosaic(&state.screenshots, column_count, column_width);

    let mut open_key = None;
    let mut delete_key = None;
    egui::ScrollArea::vertical()
        .id_salt("instance_screenshot_gallery_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.columns(column_count, |columns| {
                for (column_ui, items) in columns.iter_mut().zip(assignments.iter()) {
                    column_ui.spacing_mut().item_spacing.y = INSTANCE_SCREENSHOT_TILE_GAP;
                    let preload_rect = instance_screenshot_preload_rect(column_ui);
                    for &(index, tile_height) in items {
                        let screenshot = state.screenshots[index].clone();
                        let action = render_instance_screenshot_tile(
                            column_ui,
                            &mut state.screenshot_images,
                            &screenshot,
                            tile_height,
                            preload_rect,
                            retained_image_keys,
                        );
                        if action.open_viewer {
                            open_key = Some(screenshot_key(screenshot.path.as_path()));
                        }
                        if action.request_delete {
                            delete_key = Some(screenshot_key(screenshot.path.as_path()));
                        }
                    }
                }
            });
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

fn build_instance_screenshot_mosaic(
    screenshots: &[InstanceScreenshotEntry],
    column_count: usize,
    column_width: f32,
) -> Vec<Vec<(usize, f32)>> {
    let mut assignments = vec![Vec::new(); column_count];
    let mut heights = vec![0.0; column_count];
    for (index, screenshot) in screenshots.iter().enumerate() {
        let tile_height = instance_screenshot_tile_height(screenshot, column_width, index);
        let column_index = heights
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        assignments[column_index].push((index, tile_height));
        heights[column_index] += tile_height + INSTANCE_SCREENSHOT_TILE_GAP;
    }
    assignments
}

fn instance_screenshot_tile_height(
    screenshot: &InstanceScreenshotEntry,
    column_width: f32,
    _index: usize,
) -> f32 {
    column_width / screenshot_aspect_ratio(screenshot).max(0.01)
}

fn render_instance_screenshot_tile(
    ui: &mut Ui,
    screenshot_images: &mut LazyImageBytes,
    screenshot: &InstanceScreenshotEntry,
    tile_height: f32,
    preload_rect: egui::Rect,
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
    let should_preload = rects_overlap(rect, preload_rect);
    let image_status = if should_preload {
        retained_image_keys.insert(image_key.clone());
        screenshot_images.request(image_key.clone(), screenshot.path.clone())
    } else {
        screenshot_images.status(image_key.as_str())
    };
    let image_bytes = screenshot_images.bytes(image_key.as_str());
    if let Some(bytes) = image_bytes.as_ref() {
        egui::Image::from_bytes(image_key, Arc::clone(bytes))
            .fit_to_exact_size(rect.size())
            .corner_radius(egui::CornerRadius::same(14))
            .paint_at(ui, rect);
    } else {
        paint_instance_screenshot_placeholder(ui, rect, image_status);
    }

    let tile_hovered = image_response.hovered();
    let mut overlay_clicked = false;
    let mut action = InstanceScreenshotTileAction::default();
    if tile_hovered {
        match render_instance_screenshot_overlay_action(
            ui,
            rect,
            "instance_gallery",
            screenshot,
            image_bytes.as_deref(),
            image_status == LazyImageBytesStatus::Loading,
        ) {
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
    let stroke = if tile_hovered {
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

fn instance_screenshot_preload_rect(ui: &Ui) -> egui::Rect {
    let clip_rect = ui.clip_rect();
    let margin = (clip_rect.height() * INSTANCE_SCREENSHOT_PRELOAD_VIEWPORTS).max(220.0);
    egui::Rect::from_min_max(
        egui::pos2(clip_rect.min.x, clip_rect.min.y - margin),
        egui::pos2(clip_rect.max.x, clip_rect.max.y + margin),
    )
}

fn rects_overlap(left: egui::Rect, right: egui::Rect) -> bool {
    left.min.x <= right.max.x
        && left.max.x >= right.min.x
        && left.min.y <= right.max.y
        && left.max.y >= right.min.y
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
        .cloned()
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

    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_width = (viewport_rect.width() * 0.92).max(320.0);
    let modal_height = (viewport_rect.height() * 0.9).max(280.0);
    let modal_pos = egui::pos2(
        (viewport_rect.center().x - modal_width * 0.5)
            .clamp(viewport_rect.left(), viewport_rect.right() - modal_width),
        (viewport_rect.center().y - modal_height * 0.5)
            .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_height),
    );
    let mut open = true;
    let mut close_requested = false;
    let mut delete_requested = false;
    modal::show_scrim(ctx, "instance_screenshot_viewer_scrim", viewport_rect);

    egui::Window::new("Instance Screenshot Viewer")
        .id(egui::Id::new("instance_screenshot_viewer_window"))
        .order(egui::Order::Foreground)
        .open(&mut open)
        .fixed_pos(modal_pos)
        .fixed_size(egui::vec2(modal_width, modal_height))
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .hscroll(false)
        .vscroll(false)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            let title_style = style::heading(ui, 24.0, 28.0);
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
                    if ui.button("Close").clicked() {
                        close_requested = true;
                    }
                    if ui.button("Delete").clicked() {
                        delete_requested = true;
                    }
                    if ui
                        .add_enabled(image_bytes.is_some(), egui::Button::new("Copy"))
                        .clicked()
                        && let Some(bytes) = image_bytes.as_deref()
                    {
                        copy_instance_screenshot_to_clipboard(
                            ui.ctx(),
                            screenshot.file_name.as_str(),
                            bytes,
                        );
                    }
                    if ui.button("Reset").clicked() {
                        viewer_state.zoom = INSTANCE_SCREENSHOT_VIEWER_MIN_ZOOM;
                        viewer_state.pan_uv = egui::Vec2::ZERO;
                    }
                    if ui.button("+").clicked() {
                        viewer_state.zoom = adjust_instance_screenshot_zoom(viewer_state.zoom, 1.0);
                        clamp_instance_screenshot_pan(viewer_state);
                    }
                    if ui.button("-").clicked() {
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
                ui.visuals().widgets.noninteractive.bg_fill,
            );

            let image_rect = instance_fit_rect_to_aspect(
                canvas_rect.shrink(8.0),
                screenshot_aspect_ratio(&screenshot),
            );
            if response.hovered() {
                let scroll_delta = ui.ctx().input(|input| input.smooth_scroll_delta.y);
                if scroll_delta.abs() > 0.0 {
                    viewer_state.zoom =
                        adjust_instance_screenshot_zoom(viewer_state.zoom, scroll_delta.signum());
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

            if let Some(bytes) = image_bytes.as_ref() {
                egui::Image::from_bytes(image_key, Arc::clone(bytes))
                    .fit_to_exact_size(image_rect.size())
                    .maintain_aspect_ratio(false)
                    .uv(instance_viewer_uv_rect(viewer_state))
                    .paint_at(ui, image_rect);
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
        });

    if delete_requested {
        state.pending_delete_screenshot_key = Some(selected_screenshot_key);
    }
    if !open || close_requested {
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

    let viewport_rect = ctx.input(|input| input.content_rect());
    let modal_size = egui::vec2(viewport_rect.width().min(520.0), 250.0);
    let modal_pos = egui::pos2(
        (viewport_rect.center().x - modal_size.x * 0.5)
            .clamp(viewport_rect.left(), viewport_rect.right() - modal_size.x),
        (viewport_rect.center().y - modal_size.y * 0.5)
            .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_size.y),
    );
    let danger = ctx.style().visuals.error_fg_color;
    let mut cancel_requested = false;
    let mut delete_requested = false;
    modal::show_scrim(ctx, "instance_delete_screenshot_modal_scrim", viewport_rect);

    egui::Window::new("Delete Instance Screenshot")
        .id(egui::Id::new("instance_delete_screenshot_modal"))
        .order(egui::Order::Foreground)
        .fixed_pos(modal_pos)
        .fixed_size(modal_size)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
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
                let delete_button =
                    egui::Button::new(egui::RichText::new("Delete").color(egui::Color32::WHITE))
                        .fill(danger.gamma_multiply(0.84))
                        .stroke(egui::Stroke::new(1.0, danger))
                        .min_size(egui::vec2(120.0, 34.0))
                        .corner_radius(egui::CornerRadius::same(8));
                let cancel_button = egui::Button::new("Cancel")
                    .min_size(egui::vec2(120.0, 34.0))
                    .corner_radius(egui::CornerRadius::same(8));
                if ui
                    .add_enabled(!state.delete_screenshot_in_flight, delete_button)
                    .clicked()
                {
                    delete_requested = true;
                }
                if ui
                    .add_enabled(!state.delete_screenshot_in_flight, cancel_button)
                    .clicked()
                {
                    cancel_requested = true;
                }
                if state.delete_screenshot_in_flight {
                    ui.spinner();
                }
            });
        });

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
) -> Option<InstanceScreenshotOverlayAction> {
    let screenshot_key = screenshot_key(screenshot.path.as_path());
    if render_instance_screenshot_overlay_button(
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
    ) {
        let Some(bytes) = copy_bytes else {
            return None;
        };
        copy_instance_screenshot_to_clipboard(ui.ctx(), screenshot.file_name.as_str(), bytes);
        return Some(InstanceScreenshotOverlayAction::Copy);
    }
    if render_instance_screenshot_overlay_button(
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
    ) {
        return Some(InstanceScreenshotOverlayAction::Delete);
    }
    None
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
) -> bool {
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
    let fill = if response.is_pointer_button_down_on() {
        ui.visuals().widgets.active.bg_fill
    } else if response.hovered() {
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
    let _ = response.on_hover_text(tooltip);
    clicked
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
    let title_style = style::heading(ui, 18.0, 24.0);
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
    let sidebar_width = (full_size.x * 0.28).clamp(220.0, 320.0);
    let logs_snapshot = state.logs.clone();
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(sidebar_width, full_size.y),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("instance_logs_file_list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for log in &logs_snapshot {
                            let selected = state.selected_log_path.as_ref() == Some(&log.path);
                            let mut label = log.file_name.clone();
                            if log.size_bytes > 0 {
                                label.push_str(&format!(
                                    "\n{} | {}",
                                    format_log_file_size(log.size_bytes),
                                    format_time_ago(log.modified_at_ms, current_time_millis())
                                ));
                            }
                            let response = ui.add_sized(
                                egui::vec2(ui.available_width(), 44.0),
                                egui::Button::new(egui::RichText::new(label).color(if selected {
                                    ui.visuals().selection.stroke.color
                                } else {
                                    ui.visuals().text_color()
                                }))
                                .selected(selected)
                                .fill(if selected {
                                    ui.visuals().selection.bg_fill
                                } else {
                                    ui.visuals().widgets.inactive.bg_fill
                                })
                                .stroke(if selected {
                                    ui.visuals().selection.stroke
                                } else {
                                    ui.visuals().widgets.inactive.bg_stroke
                                })
                                .corner_radius(egui::CornerRadius::same(8)),
                            );
                            if response.clicked() {
                                state.selected_log_path = Some(log.path.clone());
                                load_selected_instance_log(state);
                            }
                            ui.add_space(6.0);
                        }
                    });
            },
        );
        ui.add_space(12.0);
        ui.allocate_ui_with_layout(
            egui::vec2((full_size.x - sidebar_width - 12.0).max(1.0), full_size.y),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                if let Some(selected_log_path) = state.selected_log_path.as_ref() {
                    let log_name = selected_log_path
                        .file_name()
                        .and_then(OsStr::to_str)
                        .unwrap_or("Log");
                    let _ =
                        text_ui.label(ui, "instance_logs_selected_name", log_name, &title_style);
                    let mut details = selected_log_path.display().to_string();
                    if state.loaded_log_truncated {
                        details
                            .push_str(&format!(" | showing last {} lines", MAX_INSTANCE_LOG_LINES));
                    }
                    let _ = text_ui.label(
                        ui,
                        "instance_logs_selected_path",
                        details.as_str(),
                        &body_style,
                    );
                    ui.add_space(8.0);
                    if state.log_load_in_flight {
                        let _ = text_ui.label(
                            ui,
                            "instance_logs_loading",
                            "Loading log contents...",
                            &body_style,
                        );
                        ui.add_space(8.0);
                    }
                    if let Some(error) = state.loaded_log_error.as_deref() {
                        let _ = text_ui.label(
                            ui,
                            "instance_logs_error",
                            error,
                            &LabelOptions {
                                color: ui.visuals().error_fg_color,
                                wrap: true,
                                ..LabelOptions::default()
                            },
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
                    );
                } else {
                    let _ = text_ui.label(
                        ui,
                        "instance_logs_no_selection",
                        "Select a log file from the left to view it.",
                        &body_style,
                    );
                }
            },
        );
    });
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
        return;
    };
    while let Ok((request_id, screenshots)) = receiver.try_recv() {
        if request_id != state.screenshot_scan_request_serial {
            continue;
        }
        state.screenshots = screenshots;
        state.last_screenshot_scan_at = Some(Instant::now());
        state.screenshot_scan_in_flight = false;
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
        let result = tokio_runtime::spawn_blocking(move || {
            fs::remove_file(path.as_path()).map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| format!("instance screenshot delete task join error: {err}"))
        .and_then(|inner| inner);
        let _ = tx.send((screenshot_key, file_name, result));
    });
}

fn poll_instance_screenshot_delete_results(state: &mut InstanceScreenState, instance_root: &Path) {
    let Some(rx) = state.delete_screenshot_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        return;
    };
    while let Ok((screenshot_key, file_name, result)) = receiver.try_recv() {
        state.delete_screenshot_in_flight = false;
        match result {
            Ok(()) => {
                if state
                    .screenshot_viewer
                    .as_ref()
                    .is_some_and(|viewer| viewer.screenshot_key == screenshot_key)
                {
                    state.screenshot_viewer = None;
                }
                state.pending_delete_screenshot_key = None;
                refresh_instance_screenshots(state, instance_root, true);
                notification::info!("instance/screenshots", "Deleted '{}' from disk.", file_name);
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
    let _ = tokio_runtime::spawn_detached(async move {
        let outcome = tokio_runtime::spawn_blocking(move || {
            collect_instance_screenshots(instance_root.as_path())
        })
        .await;
        match outcome {
            Ok(screenshots) => {
                let _ = tx.send((request_id, screenshots));
            }
            Err(error) => {
                tracing::warn!(
                    target: "vertexlauncher/instance/screenshots",
                    error = %error,
                    "instance screenshot scan task failed to join"
                );
            }
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
    screenshots.truncate(MAX_INSTANCE_SCREENSHOTS);
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
        return;
    };
    while let Ok((request_id, logs)) = receiver.try_recv() {
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
        let outcome =
            tokio_runtime::spawn_blocking(move || collect_instance_logs(instance_root.as_path()))
                .await;
        match outcome {
            Ok(logs) => {
                let _ = tx.send((request_id, logs));
            }
            Err(error) => {
                tracing::warn!(
                    target: "vertexlauncher/instance/logs",
                    error = %error,
                    "instance log scan task failed to join"
                );
            }
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
        return;
    };
    while let Ok((request_id, path, modified_at_ms, result)) = receiver.try_recv() {
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
        let outcome = tokio_runtime::spawn_blocking(move || {
            read_instance_log_lines(path_for_worker.as_path())
        })
        .await;
        match outcome {
            Ok(result) => {
                let _ = tx.send((request_id, selected_log_path, modified_at_ms, result));
            }
            Err(error) => {
                tracing::warn!(
                    target: "vertexlauncher/instance/logs",
                    error = %error,
                    path = %selected_log_path.display(),
                    "instance log load task failed to join"
                );
            }
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
    let mut open = state.show_settings_modal;
    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_width = (viewport_rect.width() * 0.92).max(1.0);
    let modal_height = (viewport_rect.height() * 0.92).max(1.0);
    let modal_pos_x = (viewport_rect.center().x - modal_width * 0.5)
        .clamp(viewport_rect.left(), viewport_rect.right() - modal_width);
    let modal_pos_y = (viewport_rect.center().y - modal_height * 0.5)
        .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_height);
    let modal_pos = egui::pos2(modal_pos_x, modal_pos_y);
    let modal_size = egui::vec2(modal_width, modal_height);
    let mut close_requested = false;
    modal::show_scrim(
        ctx,
        ("instance_settings_modal_scrim", instance_id),
        viewport_rect,
    );

    egui::Window::new("Instance Settings")
        .id(egui::Id::new(("instance_settings_modal", instance_id)))
        .order(egui::Order::Foreground)
        .open(&mut open)
        .fixed_pos(modal_pos)
        .fixed_size(modal_size)
        .collapsible(false)
        .title_bar(false)
        .resizable(false)
        .movable(false)
        .hscroll(false)
        .vscroll(false)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            let muted_style = style::muted(ui);
            let section_style = style::heading(ui, 22.0, 26.0);
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
            let refresh_style =
                style::neutral_button_with_min_size(ui, egui::vec2(190.0, 30.0));
            let reinstall_button_style =
                style::neutral_button_with_min_size(ui, egui::vec2(220.0, 34.0));

            egui::ScrollArea::vertical()
                .id_salt(("instance_settings_modal_scroll", instance_id))
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

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_thumbnail_input", instance_id),
                        "Thumbnail path (optional)",
                        Some("Local image path for this instance."),
                        &mut state.thumbnail_input,
                    );
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
                            &LabelOptions {
                                color: ui.visuals().error_fg_color,
                                wrap: true,
                                ..LabelOptions::default()
                            },
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
                                &LabelOptions {
                                    color: if is_error {
                                        ui.visuals().error_fg_color
                                    } else {
                                        ui.visuals().weak_text_color()
                                    },
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
                            );
                        }

                        let mut modloader_version_options: Vec<String> =
                            Vec::with_capacity(resolved_modloader_versions.len() + 1);
                        modloader_version_options.push("Latest available".to_owned());
                        modloader_version_options.extend(resolved_modloader_versions.iter().cloned());
                        let option_refs: Vec<&str> = modloader_version_options
                            .iter()
                            .map(String::as_str)
                            .collect();
                        let current_modloader_version = state.modloader_version_input.trim().to_owned();
                        let mut selected_index = if current_modloader_version.is_empty() {
                            0
                        } else {
                            modloader_version_options
                                .iter()
                                .position(|entry| entry == &current_modloader_version)
                                .unwrap_or(0)
                        };
                        if !current_modloader_version.is_empty() && selected_index == 0 {
                            state.modloader_version_input.clear();
                        }
                        if settings_widgets::full_width_dropdown_row(
                            text_ui,
                            ui,
                            ("instance_modloader_version_dropdown", instance_id),
                            "Modloader version",
                            Some("Cataloged by loader+Minecraft compatibility and cached once per day. Pick Latest available for automatic selection."),
                            &mut selected_index,
                            &option_refs,
                        )
                        .changed()
                        {
                            if selected_index == 0 {
                                state.modloader_version_input.clear();
                            } else if let Some(selected) = modloader_version_options.get(selected_index) {
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
                            &LabelOptions {
                                color: ui.visuals().error_fg_color,
                                wrap: true,
                                ..LabelOptions::default()
                            },
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
                                        PathBuf::from(config.minecraft_installations_root());
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
                                &LabelOptions {
                                    color: ui.visuals().error_fg_color,
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
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
                                PathBuf::from(config.minecraft_installations_root());
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
        });

    if close_requested {
        open = false;
    }
    state.show_settings_modal = open;
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

    let mut open = state.show_export_vtmpack_modal;
    let mut close_requested = false;
    let viewport_rect = ctx.input(|i| i.content_rect());
    let installations_root = PathBuf::from(config.minecraft_installations_root());
    let instance_root = instances
        .find(instance_id)
        .map(|instance| instances::instance_root_path(&installations_root, instance));
    if let Some(instance_root) = instance_root.as_deref() {
        sync_vtmpack_export_options(instance_root, &mut state.export_vtmpack_options);
    }
    let modal_width = viewport_rect.width().min(560.0).max(320.0);
    let modal_height = viewport_rect.height().min(520.0).max(300.0);
    let modal_pos = egui::pos2(
        (viewport_rect.center().x - modal_width * 0.5)
            .clamp(viewport_rect.left(), viewport_rect.right() - modal_width),
        (viewport_rect.center().y - modal_height * 0.5)
            .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_height),
    );
    modal::show_scrim(
        ctx,
        ("instance_export_vtmpack_modal_scrim", instance_id),
        viewport_rect,
    );

    let mut export_requested = false;
    egui::Window::new("Export .vtmpack")
        .id(egui::Id::new(("instance_export_vtmpack_modal", instance_id)))
        .order(egui::Order::Foreground)
        .open(&mut open)
        .fixed_pos(modal_pos)
        .fixed_size(egui::vec2(modal_width, modal_height))
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .hscroll(false)
        .vscroll(true)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            let title_style = style::heading(ui, 26.0, 30.0);
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
                &LabelOptions {
                    font_size: 18.0,
                    line_height: 22.0,
                    weight: 600,
                    color: ui.visuals().text_color(),
                    wrap: false,
                    ..LabelOptions::default()
                },
            );
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_include_help", instance_id),
                "Defaults to mods, resourcepacks, shaderpacks, and config. You can also include any other top-level files or folders found in the instance root.",
                &body_style,
            );
            ui.add_space(8.0);

            if let Some(instance_root) = instance_root.as_deref() {
                let entries = list_exportable_root_entries(instance_root);
                egui::ScrollArea::vertical()
                    .id_salt(("instance_export_vtmpack_entries_scroll", instance_id))
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for entry in entries {
                            let checked = state
                                .export_vtmpack_options
                                .included_root_entries
                                .entry(entry.clone())
                                .or_insert_with(|| default_vtmpack_root_entry_selected(&entry));
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
                        &ButtonOptions::default(),
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
                        &ButtonOptions::default(),
                    )
                    .clicked()
                {
                    export_requested = true;
                }
            });
        });

    if close_requested {
        open = false;
    }

    if export_requested {
        open = false;
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
                match export_instance_as_vtmpack(
                    &pack_instance,
                    instance_root.as_path(),
                    output_path.as_path(),
                    &state.export_vtmpack_options,
                ) {
                    Ok(stats) => {
                        state.status_message = Some(format!(
                            "Exported {} ({} bundled mods, {} config files, {} additional files) to {}",
                            instance.name,
                            stats.bundled_mod_files,
                            stats.config_files,
                            stats.additional_files,
                            output_path.display()
                        ));
                    }
                    Err(err) => {
                        state.status_message = Some(format!("Failed to export .vtmpack: {err}"));
                    }
                }
            }
        } else {
            state.status_message = Some("Instance was removed before export.".to_owned());
        }
    }

    state.show_export_vtmpack_modal = open;
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
        return;
    }
    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
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
        return;
    }
    let Some(known_versions) = state
        .loader_versions
        .versions_for_loader(selected_label, game_version)
    else {
        return;
    };
    if known_versions
        .iter()
        .any(|version| version.eq_ignore_ascii_case(entered_modloader_version))
    {
        return;
    }

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
                    let result =
                        tokio_runtime::spawn_blocking(screen_platform::detect_total_memory_mib)
                            .await
                            .ok()
                            .flatten();
                    let _ = tx.send(result);
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

use std::ffi::OsStr;

use super::*;

#[path = "home_screenshots/screenshot_candidate.rs"]
mod screenshot_candidate;
#[path = "home_screenshots/screenshot_entry.rs"]
mod screenshot_entry;
#[path = "home_screenshots/screenshot_overlay_action.rs"]
mod screenshot_overlay_action;
#[path = "home_screenshots/screenshot_overlay_button_result.rs"]
mod screenshot_overlay_button_result;
#[path = "home_screenshots/screenshot_overlay_result.rs"]
mod screenshot_overlay_result;
#[path = "home_screenshots/screenshot_result_channel.rs"]
mod screenshot_result_channel;
#[path = "home_screenshots/screenshot_scan_instance.rs"]
mod screenshot_scan_instance;
#[path = "home_screenshots/screenshot_scan_message.rs"]
mod screenshot_scan_message;
#[path = "home_screenshots/screenshot_scan_request.rs"]
mod screenshot_scan_request;
#[path = "home_screenshots/screenshot_tile_action.rs"]
mod screenshot_tile_action;
#[path = "home_screenshots/screenshot_viewer_state.rs"]
mod screenshot_viewer_state;

pub(super) use self::screenshot_candidate::ScreenshotCandidate;
pub(super) use self::screenshot_entry::ScreenshotEntry;
use self::screenshot_overlay_action::ScreenshotOverlayAction;
use self::screenshot_overlay_button_result::ScreenshotOverlayButtonResult;
use self::screenshot_overlay_result::ScreenshotOverlayResult;
use self::screenshot_result_channel::ScreenshotResultChannel;
pub(super) use self::screenshot_scan_instance::ScreenshotScanInstance;
use self::screenshot_scan_message::ScreenshotScanMessage;
pub(super) use self::screenshot_scan_request::ScreenshotScanRequest;
use self::screenshot_tile_action::ScreenshotTileAction;
pub(super) use self::screenshot_viewer_state::ScreenshotViewerState;

static SCREENSHOT_RESULTS: OnceLock<Mutex<ScreenshotResultChannel>> = OnceLock::new();

pub(super) fn purge_screenshot_state(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        let Some(mut state) = data.get_temp::<HomeState>(home_state_id()) else {
            return;
        };
        state.purge_screenshot_state(ctx);
        data.insert_temp(home_state_id(), state);
    });
}

pub(super) fn set_gamepad_screenshot_viewer_input(ctx: &egui::Context, pan: egui::Vec2, zoom: f32) {
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::new("home_screenshot_viewer_gamepad_pan"), pan);
        data.insert_temp(egui::Id::new("home_screenshot_viewer_gamepad_zoom"), zoom);
    });
}

fn screenshot_results() -> &'static Mutex<ScreenshotResultChannel> {
    SCREENSHOT_RESULTS.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<ScreenshotScanMessage>();
        Mutex::new(ScreenshotResultChannel {
            rx: result_rx,
            tx: result_tx,
        })
    })
}

pub(super) fn build_screenshot_scan_request(
    instances: &InstanceStore,
    config: &Config,
) -> ScreenshotScanRequest {
    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let screenshot_instances = instances
        .instances
        .iter()
        .map(|instance| ScreenshotScanInstance {
            instance_name: instance.name.clone(),
            screenshots_dir: instance_root_path(installations_root.as_path(), instance)
                .join("screenshots"),
        })
        .collect();

    ScreenshotScanRequest {
        scanned_instance_count: instances.instances.len(),
        instances: screenshot_instances,
    }
}

pub(super) fn poll_screenshot_results(state: &mut HomeState) {
    let Ok(channel) = screenshot_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            request_id = state.latest_requested_screenshot_scan_id,
            "Home screenshot results receiver mutex was poisoned while polling screenshot entries."
        );
        return;
    };

    let mut messages = Vec::new();
    while let Ok(msg) = channel.rx.try_recv() {
        messages.push(msg);
    }
    drop(channel);

    let mut any_entries = false;
    for msg in messages {
        match msg {
            ScreenshotScanMessage::EntryLoaded { request_id, entry } => {
                if request_id != state.latest_requested_screenshot_scan_id {
                    continue;
                }
                state.screenshots.push(entry);
                any_entries = true;
            }
            ScreenshotScanMessage::TaskDone { request_id } => {
                if request_id != state.latest_requested_screenshot_scan_id {
                    continue;
                }
                state.screenshot_tasks_done += 1;
                let all_pages_spawned =
                    state.screenshot_loaded_count >= state.screenshot_candidates.len();
                if state.screenshot_scan_ready
                    && all_pages_spawned
                    && state.screenshot_tasks_done >= state.screenshot_tasks_total
                {
                    state.screenshot_scan_pending = false;
                    state.last_screenshot_scan_at = Some(Instant::now());
                }
            }
        }
    }

    if any_entries {
        state.screenshots.sort_by(|a, b| {
            b.modified_at_ms
                .unwrap_or(0)
                .cmp(&a.modified_at_ms.unwrap_or(0))
                .then_with(|| a.file_name.cmp(&b.file_name))
        });
        state.mark_screenshot_layout_dirty();
    }
}

fn spawn_screenshot_load_page(state: &mut HomeState, request_id: u64, page_size: usize) {
    let start = state.screenshot_loaded_count;
    let end = (start + page_size).min(state.screenshot_candidates.len());
    if start >= end {
        return;
    }

    let Ok(channel) = screenshot_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            request_id,
            start = start,
            end = end,
            page_size,
            "Home screenshot results channel mutex was poisoned while spawning screenshot page load."
        );
        return;
    };
    let result_tx = channel.tx.clone();
    drop(channel);

    state.screenshot_loaded_count = end;
    state.screenshot_tasks_total += end - start;

    for candidate in state.screenshot_candidates[start..end].iter().cloned() {
        let tx = result_tx.clone();
        tokio_runtime::spawn_detached(async move {
            let entry = tokio_runtime::spawn_blocking(move || {
                let Ok((width, height)) = image::image_dimensions(&candidate.path) else {
                    return None;
                };
                if width == 0 || height == 0 {
                    return None;
                }
                Some(ScreenshotEntry {
                    instance_name: candidate.instance_name,
                    path: candidate.path,
                    file_name: candidate.file_name,
                    width,
                    height,
                    modified_at_ms: candidate.modified_at_ms,
                })
            })
            .await
            .ok()
            .flatten();
            if let Some(entry) = entry {
                if let Err(err) = tx.send(ScreenshotScanMessage::EntryLoaded { request_id, entry })
                {
                    tracing::error!(
                        target: "vertexlauncher/home",
                        request_id,
                        error = %err,
                        "Failed to deliver home screenshot entry."
                    );
                }
            }
            if let Err(err) = tx.send(ScreenshotScanMessage::TaskDone { request_id }) {
                tracing::error!(
                    target: "vertexlauncher/home",
                    request_id,
                    error = %err,
                    "Failed to deliver home screenshot task completion."
                );
            }
        });
    }
}

fn ensure_delete_screenshot_channel(state: &mut HomeState) {
    if state.delete_screenshot_results_tx.is_some() && state.delete_screenshot_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, String, Result<(), String>)>();
    state.delete_screenshot_results_tx = Some(tx);
    state.delete_screenshot_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_screenshot_delete(
    state: &mut HomeState,
    screenshot_key: String,
    path: PathBuf,
    file_name: String,
) {
    if state.delete_screenshot_in_flight {
        return;
    }

    ensure_delete_screenshot_channel(state);
    let Some(tx) = state.delete_screenshot_results_tx.as_ref().cloned() else {
        return;
    };

    state.delete_screenshot_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = fs::remove_file(path.as_path()).map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %path.display(), error = %err, context = "delete home screenshot");
            format!("failed to remove {}: {err}", path.display())
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

pub(super) fn poll_delete_screenshot_results(
    state: &mut HomeState,
    instances: &InstanceStore,
    config: &Config,
) {
    let Some(rx) = state.delete_screenshot_results_rx.as_ref() else {
        return;
    };

    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    match rx.lock() {
        Ok(receiver) => loop {
            match receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!(
                        target: "vertexlauncher/home",
                        "Home screenshot-delete worker disconnected unexpectedly."
                    );
                    should_reset_channel = true;
                    break;
                }
            }
        },
        Err(_) => {
            tracing::error!(
                target: "vertexlauncher/home",
                "Home screenshot-delete receiver mutex was poisoned."
            );
            should_reset_channel = true;
        }
    }

    if should_reset_channel {
        state.delete_screenshot_results_tx = None;
        state.delete_screenshot_results_rx = None;
        state.delete_screenshot_in_flight = false;
        notification::error!(
            "home/screenshots",
            "Screenshot delete worker stopped unexpectedly."
        );
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
                notification::info!("home/screenshots", "Deleted '{}' from disk.", file_name);
            }
            Err(err) => {
                state.pending_delete_screenshot_key = None;
                notification::error!(
                    "home/screenshots",
                    "Failed to delete '{}': {}",
                    file_name,
                    err
                );
            }
        }
    }
}

pub(super) fn handle_escape(ctx: &egui::Context) -> bool {
    let state_id = home_state_id();
    let mut handled = false;
    ctx.data_mut(|data| {
        let Some(mut state) = data.get_temp::<HomeState>(state_id) else {
            return;
        };
        if state.pending_delete_screenshot_key.is_some() {
            if !state.delete_screenshot_in_flight {
                tracing::info!(
                    target: "vertexlauncher/screenshots",
                    "Home screenshot delete confirmation closed by escape."
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
                "Home screenshot viewer closed by escape."
            );
            data.insert_temp(state_id, state);
            handled = true;
        }
    });
    handled
}

pub(super) fn refresh_screenshot_state(
    state: &mut HomeState,
    instances: &InstanceStore,
    config: &Config,
    force: bool,
) {
    if state.screenshot_scan_pending && !force {
        return;
    }

    let request_id = state.latest_requested_screenshot_scan_id.saturating_add(1);
    let request = build_screenshot_scan_request(instances, config);

    state.latest_requested_screenshot_scan_id = request_id;
    state.screenshot_scan_pending = true;
    state.screenshot_scan_ready = true;
    state.screenshot_tasks_total = 0;
    state.screenshot_tasks_done = 0;
    state.screenshots.clear();
    state.mark_screenshot_layout_dirty();

    let candidates = collect_screenshot_candidates(&request);
    state.scanned_screenshot_instance_count = request.scanned_instance_count;
    state.screenshot_candidates = candidates;
    state.screenshot_loaded_count = 0;

    if state.screenshot_candidates.is_empty() {
        state.screenshot_scan_pending = false;
        state.last_screenshot_scan_at = Some(Instant::now());
    } else {
        spawn_screenshot_load_page(state, request_id, SCREENSHOT_PAGE_SIZE);
    }
}

fn collect_screenshot_candidates(request: &ScreenshotScanRequest) -> Vec<ScreenshotCandidate> {
    let mut candidates = Vec::new();
    for instance in &request.instances {
        let Ok(entries) = fs::read_dir(&instance.screenshots_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !is_supported_screenshot_path(path.as_path()) {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            candidates.push(ScreenshotCandidate {
                instance_name: instance.instance_name.clone(),
                file_name,
                modified_at_ms: modified_millis(path.as_path()),
                path,
            });
        }
    }
    candidates.sort_by(|a, b| {
        b.modified_at_ms
            .unwrap_or(0)
            .cmp(&a.modified_at_ms.unwrap_or(0))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    candidates
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

pub(super) fn render_screenshot_gallery(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut HomeState,
    retained_image_keys: &mut HashSet<String>,
    metrics: HomeUiMetrics,
) {
    let title_style = style::body_strong(ui);
    let body_style = style::muted(ui);
    let _ = text_ui.label(ui, "home_screenshots_title", "Screenshots", &title_style);
    let total_candidates = state.screenshot_candidates.len();
    let loaded_count = state.screenshots.len();
    let summary = if state.screenshot_scan_pending && state.screenshots.is_empty() {
        "Loading screenshots...".to_owned()
    } else if state.screenshots.is_empty() {
        "No screenshots found in any instance.".to_owned()
    } else if state.screenshot_loaded_count < total_candidates {
        format!(
            "Showing {loaded_count} of {total_candidates} screenshots — scroll down to load more."
        )
    } else {
        format!("{loaded_count} screenshots across your instances.")
    };
    let _ = text_ui.label(
        ui,
        "home_screenshots_summary",
        summary.as_str(),
        &body_style,
    );
    ui.add_space(style::SPACE_SM);

    if state.screenshots.is_empty() {
        return;
    }

    let layout = build_virtual_masonry_layout(
        ui,
        metrics.screenshot_min_column_width,
        SCREENSHOT_TILE_GAP,
        3,
        state.screenshots.len(),
        state.screenshot_layout_revision,
        &mut state.screenshot_masonry_layout_cache,
        |index, column_width| screenshot_tile_height(&state.screenshots[index], column_width),
    );

    let mut open_screenshot_key = None;
    let mut delete_screenshot_key = None;
    let mut should_load_more = false;
    egui::ScrollArea::vertical()
        .id_salt("home_screenshots_scroll")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            ui.add_space(style::SPACE_SM);
            let screenshots = &state.screenshots;
            let screenshot_images = &mut state.screenshot_images;
            render_virtualized_masonry(
                ui,
                &layout,
                SCREENSHOT_TILE_GAP,
                viewport,
                HOME_SCREENSHOT_OVERSCAN,
                |column_ui, index, tile_height| {
                    let action = render_screenshot_tile(
                        column_ui,
                        text_ui,
                        screenshot_images,
                        &screenshots[index],
                        tile_height,
                        retained_image_keys,
                        metrics,
                    );
                    if action.open_viewer {
                        open_screenshot_key = Some(screenshots[index].key());
                    }
                    if action.request_delete {
                        delete_screenshot_key = Some(screenshots[index].key());
                    }
                },
            );
            should_load_more = state.screenshot_loaded_count < state.screenshot_candidates.len()
                && viewport.bottom() >= ui.min_rect().bottom() - viewport.height().max(320.0);
        });

    if should_load_more {
        let request_id = state.latest_requested_screenshot_scan_id;
        spawn_screenshot_load_page(state, request_id, SCREENSHOT_PAGE_SIZE);
    }

    if let Some(screenshot_key) = open_screenshot_key {
        if let Some(entry) = state
            .screenshots
            .iter()
            .find(|e| e.key() == screenshot_key)
            .cloned()
        {
            state.screenshot_viewer = Some(ScreenshotViewerState {
                screenshot_key,
                entry_snapshot: entry,
                zoom: SCREENSHOT_VIEWER_MIN_ZOOM,
                pan_uv: egui::Vec2::ZERO,
            });
        }
    }
    if let Some(screenshot_key) = delete_screenshot_key {
        state.pending_delete_screenshot_key = Some(screenshot_key);
    }
}

fn screenshot_tile_height(screenshot: &ScreenshotEntry, column_width: f32) -> f32 {
    column_width / screenshot.aspect_ratio().max(0.01)
}

fn render_screenshot_tile(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    screenshot_images: &mut LazyImageBytes,
    screenshot: &ScreenshotEntry,
    tile_height: f32,
    retained_image_keys: &mut HashSet<String>,
    metrics: HomeUiMetrics,
) -> ScreenshotTileAction {
    let width = ui.available_width().max(1.0);
    let tile_size = egui::vec2(width, tile_height);
    let (rect, _) = ui.allocate_exact_size(tile_size, egui::Sense::hover());
    let mut image_response = ui.interact(
        rect,
        ui.id().with(("home_screenshot_tile", screenshot.key())),
        egui::Sense::click(),
    );
    let image_has_focus = image_response.has_focus();
    let image_key = screenshot.uri();
    retained_image_keys.insert(image_key.clone());
    let image_status = screenshot_images.request(image_key.clone(), screenshot.path.clone());
    let image_bytes = screenshot_images.bytes(image_key.as_str());
    if let Some(bytes) = image_bytes.as_ref() {
        match image_textures::request_texture(
            ui.ctx(),
            image_key.clone(),
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
                paint_screenshot_tile_placeholder(
                    ui,
                    text_ui,
                    rect,
                    LazyImageBytesStatus::Loading,
                );
            }
            image_textures::ManagedTextureStatus::Failed => {
                paint_screenshot_tile_placeholder(
                    ui,
                    text_ui,
                    rect,
                    LazyImageBytesStatus::Failed,
                );
            }
        }
    } else {
        paint_screenshot_tile_placeholder(ui, text_ui, rect, image_status);
    }

    let tile_contains_pointer = ui_pointer_over_rect(ui, rect);
    let overlay_memory_id = image_response.id.with("home_screenshot_overlay_active");
    let overlay_was_active = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(overlay_memory_id))
        .unwrap_or(false);
    let mut overlay_clicked = false;
    let mut action = ScreenshotTileAction::default();
    let mut overlay_result = ScreenshotOverlayResult::default();
    if tile_contains_pointer || overlay_was_active {
        overlay_result = render_screenshot_overlay_action(
            ui,
            text_ui,
            rect,
            "home_gallery",
            screenshot,
            image_bytes.as_deref(),
            image_status == LazyImageBytesStatus::Loading,
            metrics,
        );
        match overlay_result.action {
            Some(ScreenshotOverlayAction::Copy) => {
                overlay_clicked = true;
            }
            Some(ScreenshotOverlayAction::Delete) => {
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
    } else if image_has_focus {
        ui.visuals().selection.stroke
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
        Color32::from_rgba_premultiplied(6, 9, 14, 185),
    );
    let age_label = format_time_ago(screenshot.modified_at_ms, current_time_millis());
    let label = format!("{} | {}", screenshot.instance_name, age_label);
    let label_style = LabelOptions {
        color: Color32::WHITE,
        wrap: false,
        ..style::body(ui)
    };
    let label_text_rect = label_bg_rect.shrink2(egui::vec2(8.0, 0.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(label_text_rect), |ui| {
        ui.set_clip_rect(label_text_rect.intersect(ui.clip_rect()));
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let _ = text_ui.label(
                ui,
                ("home_screenshot_tile_label", screenshot.key()),
                label.as_str(),
                &label_style,
            );
        });
    });

    image_response = image_response.on_hover_text(format!(
        "{}\n{}\n{}",
        screenshot.instance_name,
        screenshot.file_name,
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

fn paint_screenshot_tile_placeholder(
    ui: &mut Ui,
    text_ui: &mut TextUi,
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
    let label_style = LabelOptions {
        color: ui.visuals().weak_text_color(),
        ..style::body_strong(ui)
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                let _ = text_ui.label(
                    ui,
                    ("home_screenshot_placeholder", label),
                    label,
                    &label_style,
                );
            },
        );
    });
}

pub(super) fn retain_home_viewer_image(
    state: &mut HomeState,
    retained_image_keys: &mut HashSet<String>,
) {
    let Some(viewer) = state.screenshot_viewer.as_ref() else {
        return;
    };
    let snapshot = viewer.entry_snapshot.clone();
    let image_key = snapshot.uri();
    retained_image_keys.insert(image_key.clone());
    state.screenshot_images.request(image_key, snapshot.path);
}

fn paint_home_screenshot_centered_status(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    rect: egui::Rect,
    label: &str,
) {
    let label_style = LabelOptions {
        color: ui.visuals().weak_text_color(),
        ..style::body_strong(ui)
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                let _ = text_ui.label(
                    ui,
                    ("home_screenshot_center_status", label),
                    label,
                    &label_style,
                );
            },
        );
    });
}

pub(super) fn render_screenshot_viewer_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut HomeState,
    metrics: HomeUiMetrics,
) {
    let Some(screenshot_key) = state
        .screenshot_viewer
        .as_ref()
        .map(|viewer_state| viewer_state.screenshot_key.clone())
    else {
        return;
    };
    let live_entry = state
        .screenshots
        .iter()
        .find(|entry| entry.key() == screenshot_key)
        .cloned();
    if live_entry.is_none() && !state.screenshot_scan_pending {
        tracing::info!(
            target: "vertexlauncher/screenshots",
            screenshot_key = screenshot_key.as_str(),
            "Home screenshot viewer closed because the screenshot entry was no longer available."
        );
        state.screenshot_viewer = None;
        return;
    }
    let Some(viewer_state) = state.screenshot_viewer.as_mut() else {
        return;
    };
    if let Some(ref entry) = live_entry {
        viewer_state.entry_snapshot = entry.clone();
    }
    let screenshot = live_entry.unwrap_or_else(|| viewer_state.entry_snapshot.clone());
    let image_key = screenshot.uri();
    let image_status = state
        .screenshot_images
        .request(image_key.clone(), screenshot.path.clone());
    let image_bytes = state.screenshot_images.bytes(image_key.as_str());
    let gamepad_pan = ctx
        .data(|data| {
            data.get_temp::<egui::Vec2>(egui::Id::new("home_screenshot_viewer_gamepad_pan"))
        })
        .unwrap_or(egui::Vec2::ZERO);
    let gamepad_zoom = ctx
        .data(|data| data.get_temp::<f32>(egui::Id::new("home_screenshot_viewer_gamepad_zoom")))
        .unwrap_or(0.0);
    let frame_dt = ctx.input(|input| input.stable_dt).clamp(1.0 / 240.0, 0.05);

    let now_ms = current_time_millis();
    let mut close_requested = false;
    let mut delete_requested = false;
    let request_reset_focus = modal_default_focus_requested(
        ctx,
        ("home_screenshot_viewer_window", screenshot_key.as_str()),
    );
    let response = show_dialog(
        ctx,
        dialog_options("home_screenshot_viewer_window", DialogPreset::Viewer),
        |ui| {
            let title_style = style::section_heading(ui);
            let body_style = style::muted_single_line(ui);

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let _ = text_ui.label(
                        ui,
                        "home_screenshot_viewer_title",
                        screenshot.file_name.as_str(),
                        &title_style,
                    );
                    let details = format!(
                        "{} | {}x{} | {}",
                        screenshot.instance_name,
                        screenshot.width,
                        screenshot.height,
                        format_time_ago(screenshot.modified_at_ms, now_ms)
                    );
                    let _ = text_ui.label(
                        ui,
                        "home_screenshot_viewer_details",
                        details.as_str(),
                        &body_style,
                    );
                });
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    if text_ui
                        .button(
                            ui,
                            "home_screenshot_viewer_close",
                            "Close",
                            &secondary_button(
                                ui,
                                egui::vec2(metrics.action_button_width, style::CONTROL_HEIGHT),
                            ),
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                    if text_ui
                        .button(
                            ui,
                            "home_screenshot_viewer_delete",
                            "Delete",
                            &danger_button(
                                ui,
                                egui::vec2(metrics.action_button_width, style::CONTROL_HEIGHT),
                            ),
                        )
                        .clicked()
                    {
                        delete_requested = true;
                    }
                    if ui
                        .add_enabled_ui(image_bytes.is_some(), |ui| {
                            text_ui.button(
                                ui,
                                "home_screenshot_viewer_copy",
                                "Copy",
                                &secondary_button(
                                    ui,
                                    egui::vec2(
                                        (metrics.action_button_width - 10.0).max(80.0),
                                        style::CONTROL_HEIGHT,
                                    ),
                                ),
                            )
                        })
                        .inner
                        .clicked()
                        && let Some(bytes) = image_bytes.as_deref()
                    {
                        copy_screenshot_to_clipboard(
                            ui.ctx(),
                            screenshot.file_name.as_str(),
                            bytes,
                        );
                    }
                    let reset_response = text_ui.button(
                        ui,
                        "home_screenshot_viewer_reset",
                        "Reset",
                        &secondary_button(
                            ui,
                            egui::vec2(
                                (metrics.action_button_width - 10.0).max(80.0),
                                style::CONTROL_HEIGHT,
                            ),
                        ),
                    );
                    if request_reset_focus {
                        reset_response.request_focus();
                    }
                    if reset_response.clicked() {
                        viewer_state.zoom = SCREENSHOT_VIEWER_MIN_ZOOM;
                        viewer_state.pan_uv = egui::Vec2::ZERO;
                    }
                    if text_ui
                        .button(
                            ui,
                            "home_screenshot_viewer_zoom_in",
                            "+",
                            &secondary_button(
                                ui,
                                egui::vec2(
                                    metrics
                                        .screenshot_overlay_button_size
                                        .max(style::CONTROL_HEIGHT),
                                    style::CONTROL_HEIGHT,
                                ),
                            ),
                        )
                        .clicked()
                    {
                        viewer_state.zoom = adjust_viewer_zoom(viewer_state.zoom, 1.0);
                        clamp_viewer_pan(viewer_state);
                    }
                    if text_ui
                        .button(
                            ui,
                            "home_screenshot_viewer_zoom_out",
                            "-",
                            &secondary_button(
                                ui,
                                egui::vec2(
                                    metrics
                                        .screenshot_overlay_button_size
                                        .max(style::CONTROL_HEIGHT),
                                    style::CONTROL_HEIGHT,
                                ),
                            ),
                        )
                        .clicked()
                    {
                        viewer_state.zoom = adjust_viewer_zoom(viewer_state.zoom, -1.0);
                        clamp_viewer_pan(viewer_state);
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

            let image_rect = fit_rect_to_aspect(canvas_rect.shrink(8.0), screenshot.aspect_ratio());
            if response.hovered() {
                let scroll_delta = ui.ctx().input(|input| input.smooth_scroll_delta.y);
                if scroll_delta.abs() > 0.0 {
                    viewer_state.zoom =
                        adjust_viewer_zoom_with_scroll(viewer_state.zoom, scroll_delta);
                    clamp_viewer_pan(viewer_state);
                    ui.ctx().request_repaint();
                }
            }
            if response.dragged() && viewer_state.zoom > SCREENSHOT_VIEWER_MIN_ZOOM {
                let visible_fraction = 1.0 / viewer_state.zoom.max(SCREENSHOT_VIEWER_MIN_ZOOM);
                let delta = ui.ctx().input(|input| input.pointer.delta());
                viewer_state.pan_uv.x -= delta.x / image_rect.width().max(1.0) * visible_fraction;
                viewer_state.pan_uv.y -= delta.y / image_rect.height().max(1.0) * visible_fraction;
                clamp_viewer_pan(viewer_state);
                ui.ctx().request_repaint();
            }
            if gamepad_zoom.abs() > 0.05 {
                let zoom_scale = (1.0 + gamepad_zoom * 1.8 * frame_dt).clamp(0.7, 1.3);
                viewer_state.zoom = (viewer_state.zoom * zoom_scale)
                    .clamp(SCREENSHOT_VIEWER_MIN_ZOOM, SCREENSHOT_VIEWER_MAX_ZOOM);
                clamp_viewer_pan(viewer_state);
                ui.ctx().request_repaint();
            }
            if viewer_state.zoom > SCREENSHOT_VIEWER_MIN_ZOOM
                && (gamepad_pan.x.abs() > 0.05 || gamepad_pan.y.abs() > 0.05)
            {
                let visible_fraction = 1.0 / viewer_state.zoom.max(SCREENSHOT_VIEWER_MIN_ZOOM);
                let pan_speed = 1.35 * 0.2 * frame_dt * visible_fraction;
                viewer_state.pan_uv.x += gamepad_pan.x * pan_speed;
                viewer_state.pan_uv.y += gamepad_pan.y * pan_speed;
                clamp_viewer_pan(viewer_state);
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
                            .uv(viewer_uv_rect(viewer_state))
                            .corner_radius(egui::CornerRadius::same(12))
                            .paint_at(ui, image_rect);
                    }
                    image_textures::ManagedTextureStatus::Loading => {
                        ui.painter().rect_filled(
                            image_rect,
                            egui::CornerRadius::same(12),
                            ui.visuals().widgets.inactive.bg_fill,
                        );
                        paint_home_screenshot_centered_status(
                            ui,
                            text_ui,
                            image_rect,
                            "Loading screenshot...",
                        );
                    }
                    image_textures::ManagedTextureStatus::Failed => {
                        ui.painter().rect_filled(
                            image_rect,
                            egui::CornerRadius::same(12),
                            ui.visuals().widgets.inactive.bg_fill,
                        );
                        paint_home_screenshot_centered_status(
                            ui,
                            text_ui,
                            image_rect,
                            "Failed to load screenshot",
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
                paint_home_screenshot_centered_status(ui, text_ui, image_rect, label);
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
            screenshot_key = screenshot_key.as_str(),
            "Home screenshot viewer requested delete."
        );
        state.pending_delete_screenshot_key = Some(screenshot_key.clone());
    }
    if close_requested {
        tracing::info!(
            target: "vertexlauncher/screenshots",
            screenshot_key = screenshot_key.as_str(),
            "Home screenshot viewer closed by explicit close button."
        );
        state.screenshot_viewer = None;
    }
}

pub(super) fn render_delete_screenshot_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut HomeState,
    _instances: &InstanceStore,
    _config: &Config,
    metrics: HomeUiMetrics,
) {
    let Some(screenshot_key) = state.pending_delete_screenshot_key.clone() else {
        return;
    };
    let Some(screenshot) = state
        .screenshots
        .iter()
        .find(|entry| entry.key() == screenshot_key)
        .cloned()
    else {
        state.pending_delete_screenshot_key = None;
        return;
    };

    let danger = ctx.global_style().visuals.error_fg_color;
    let mut cancel_requested = false;
    let mut delete_requested = false;
    let request_cancel_focus = modal_default_focus_requested(
        ctx,
        ("home_delete_screenshot_modal", screenshot_key.as_str()),
    );
    let response = show_dialog(
        ctx,
        dialog_options("home_delete_screenshot_modal", DialogPreset::Confirm),
        |ui| {
            let heading_style = style::heading_color(ui, 28.0, 32.0, danger);
            let body_style = style::body(ui);
            let muted_style = style::muted(ui);

            let _ = text_ui.label(
                ui,
                (
                    "home_delete_screenshot_heading",
                    screenshot.path.display().to_string(),
                ),
                "Delete Screenshot?",
                &heading_style,
            );
            let _ = text_ui.label(
                ui,
                (
                    "home_delete_screenshot_body",
                    screenshot.path.display().to_string(),
                ),
                &format!(
                    "Delete \"{}\" from disk? This permanently removes the screenshot.",
                    screenshot.file_name
                ),
                &body_style,
            );
            let _ = text_ui.label(
                ui,
                (
                    "home_delete_screenshot_instance",
                    screenshot.path.display().to_string(),
                ),
                &format!("Instance: {}", screenshot.instance_name),
                &muted_style,
            );
            let _ = text_ui.label(
                ui,
                (
                    "home_delete_screenshot_path",
                    screenshot.path.display().to_string(),
                ),
                &format!("Path: {}", screenshot.path.display()),
                &muted_style,
            );

            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled_ui(!state.delete_screenshot_in_flight, |ui| {
                        text_ui.button(
                            ui,
                            "home_delete_screenshot_confirm",
                            "Delete",
                            &danger_button(
                                ui,
                                egui::vec2(metrics.action_button_width, style::CONTROL_HEIGHT),
                            ),
                        )
                    })
                    .inner
                    .clicked()
                {
                    delete_requested = true;
                }
                let cancel_response = ui
                    .add_enabled_ui(!state.delete_screenshot_in_flight, |ui| {
                        text_ui.button(
                            ui,
                            "home_delete_screenshot_cancel",
                            "Cancel",
                            &secondary_button(
                                ui,
                                egui::vec2(metrics.action_button_width, style::CONTROL_HEIGHT),
                            ),
                        )
                    })
                    .inner;
                if request_cancel_focus {
                    cancel_response.request_focus();
                }
                if cancel_response.clicked() {
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
        request_screenshot_delete(
            state,
            screenshot_key,
            screenshot.path.clone(),
            screenshot.file_name.clone(),
        );
    }
}

fn render_screenshot_overlay_action(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    tile_rect: egui::Rect,
    scope: &str,
    screenshot: &ScreenshotEntry,
    copy_bytes: Option<&[u8]>,
    copy_loading: bool,
    metrics: HomeUiMetrics,
) -> ScreenshotOverlayResult {
    let screenshot_key = screenshot.key();
    let mut result = ScreenshotOverlayResult::default();
    let copy_result = render_screenshot_overlay_button(
        ui,
        text_ui,
        tile_rect,
        scope,
        screenshot_key.as_str(),
        "home_screenshot_copy_button",
        assets::COPY_SVG,
        ui.visuals().text_color(),
        if copy_loading {
            "Image is still loading"
        } else {
            "Copy image to clipboard"
        },
        8.0,
        copy_bytes.is_some(),
        metrics,
    );
    result.contains_pointer |= copy_result.contains_pointer;
    if copy_result.clicked {
        let Some(bytes) = copy_bytes else {
            return result;
        };
        copy_screenshot_to_clipboard(ui.ctx(), screenshot.file_name.as_str(), bytes);
        result.action = Some(ScreenshotOverlayAction::Copy);
        return result;
    }
    let delete_result = render_screenshot_overlay_button(
        ui,
        text_ui,
        tile_rect,
        scope,
        screenshot_key.as_str(),
        "home_screenshot_delete_button",
        assets::TRASH_X_SVG,
        ui.visuals().error_fg_color,
        "Delete screenshot",
        8.0 + metrics.screenshot_overlay_button_size + 6.0,
        true,
        metrics,
    );
    result.contains_pointer |= delete_result.contains_pointer;
    if delete_result.clicked {
        result.action = Some(ScreenshotOverlayAction::Delete);
    }
    result
}

fn render_screenshot_overlay_button(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    tile_rect: egui::Rect,
    scope: &str,
    screenshot_key: &str,
    id_source: &str,
    icon_svg: &[u8],
    icon_color: Color32,
    tooltip: &str,
    x_offset: f32,
    enabled: bool,
    metrics: HomeUiMetrics,
) -> ScreenshotOverlayButtonResult {
    let button_rect = egui::Rect::from_min_size(
        tile_rect.min + egui::vec2(x_offset, 8.0),
        egui::vec2(
            metrics.screenshot_overlay_button_size,
            metrics.screenshot_overlay_button_size,
        ),
    );
    let themed_svg = apply_color_to_svg(icon_svg, icon_color);
    let icon_color_key = format!(
        "{:02x}{:02x}{:02x}",
        icon_color.r(),
        icon_color.g(),
        icon_color.b()
    );
    let uri = format!("bytes://home/{id_source}/{scope}-{screenshot_key}-{icon_color_key}.svg");
    let response = ui.interact(
        button_rect,
        ui.id().with((id_source, scope, screenshot_key)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let has_focus = response.has_focus();
    let button_contains_pointer = ui_pointer_over_rect(ui, button_rect);
    let button_pressed = button_contains_pointer && ui.input(|input| input.pointer.primary_down());
    let fill = if response.is_pointer_button_down_on() || button_pressed {
        ui.visuals().widgets.active.bg_fill
    } else if button_contains_pointer || has_focus {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        Color32::from_rgba_premultiplied(12, 16, 24, 210)
    };
    ui.painter()
        .rect_filled(button_rect, egui::CornerRadius::same(8), fill);
    ui.painter().rect_stroke(
        button_rect,
        egui::CornerRadius::same(8),
        if has_focus {
            ui.visuals().selection.stroke
        } else {
            ui.visuals().widgets.inactive.bg_stroke
        },
        egui::StrokeKind::Inside,
    );
    if has_focus {
        ui.painter().rect_stroke(
            button_rect.expand(2.0),
            egui::CornerRadius::same(10),
            egui::Stroke::new(
                (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }
    let icon_size = (metrics.screenshot_overlay_button_size * 0.5).clamp(12.0, 16.0);
    let icon_rect =
        egui::Rect::from_center_size(button_rect.center(), egui::vec2(icon_size, icon_size));
    egui::Image::from_bytes(uri, themed_svg)
        .fit_to_exact_size(icon_rect.size())
        .tint(if enabled {
            Color32::WHITE
        } else {
            Color32::from_white_alpha(120)
        })
        .paint_at(ui, icon_rect);
    let clicked = response.clicked();
    if button_contains_pointer {
        let _ = egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            response.id.with("tooltip"),
            egui::PopupAnchor::Pointer,
        )
        .gap(12.0)
        .show(|ui| {
            let tooltip_style = LabelOptions {
                color: ui.visuals().text_color(),
                ..style::body(ui)
            };
            let _ = text_ui.label(
                ui,
                ("home_screenshot_overlay_tooltip", id_source, tooltip),
                tooltip,
                &tooltip_style,
            );
        });
    }
    ScreenshotOverlayButtonResult {
        clicked,
        contains_pointer: button_contains_pointer || response.is_pointer_button_down_on(),
    }
}

fn copy_screenshot_to_clipboard(ctx: &egui::Context, label: &str, bytes: &[u8]) {
    match decode_color_image(bytes) {
        Ok(image) => {
            ctx.copy_image(image);
            notification::info!("home/screenshots", "Copied '{}' to clipboard.", label);
        }
        Err(err) => {
            notification::error!(
                "home/screenshots",
                "Failed to copy '{}' to clipboard: {}",
                label,
                err
            );
        }
    }
}

fn decode_color_image(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    let rgba = image::load_from_memory(bytes)
        .map_err(|err| err.to_string())?
        .to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

fn fit_rect_to_aspect(rect: egui::Rect, aspect_ratio: f32) -> egui::Rect {
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

fn adjust_viewer_zoom(current_zoom: f32, direction: f32) -> f32 {
    (current_zoom + direction * SCREENSHOT_VIEWER_ZOOM_STEP)
        .clamp(SCREENSHOT_VIEWER_MIN_ZOOM, SCREENSHOT_VIEWER_MAX_ZOOM)
}

fn adjust_viewer_zoom_with_scroll(current_zoom: f32, scroll_delta: f32) -> f32 {
    let scale = (1.0 + scroll_delta * SCREENSHOT_VIEWER_SCROLL_ZOOM_SENSITIVITY).clamp(0.7, 1.3);
    (current_zoom * scale).clamp(SCREENSHOT_VIEWER_MIN_ZOOM, SCREENSHOT_VIEWER_MAX_ZOOM)
}

fn clamp_viewer_pan(viewer_state: &mut ScreenshotViewerState) {
    let visible_fraction = 1.0 / viewer_state.zoom.max(SCREENSHOT_VIEWER_MIN_ZOOM);
    let max_offset = (1.0 - visible_fraction) * 0.5;
    viewer_state.pan_uv.x = viewer_state.pan_uv.x.clamp(-max_offset, max_offset);
    viewer_state.pan_uv.y = viewer_state.pan_uv.y.clamp(-max_offset, max_offset);
}

fn viewer_uv_rect(viewer_state: &ScreenshotViewerState) -> egui::Rect {
    let visible_fraction = 1.0 / viewer_state.zoom.max(SCREENSHOT_VIEWER_MIN_ZOOM);
    let half = visible_fraction * 0.5;
    let center = egui::pos2(0.5 + viewer_state.pan_uv.x, 0.5 + viewer_state.pan_uv.y);
    egui::Rect::from_min_max(
        egui::pos2(center.x - half, center.y - half),
        egui::pos2(center.x + half, center.y + half),
    )
}

use super::*;

#[path = "instance_screenshots/instance_screenshot_overlay_action.rs"]
mod instance_screenshot_overlay_action;
#[path = "instance_screenshots/instance_screenshot_overlay_button_result.rs"]
mod instance_screenshot_overlay_button_result;
#[path = "instance_screenshots/instance_screenshot_overlay_result.rs"]
mod instance_screenshot_overlay_result;
#[path = "instance_screenshots/instance_screenshot_tile_action.rs"]
mod instance_screenshot_tile_action;

use self::instance_screenshot_overlay_action::InstanceScreenshotOverlayAction;
use self::instance_screenshot_overlay_button_result::InstanceScreenshotOverlayButtonResult;
use self::instance_screenshot_overlay_result::InstanceScreenshotOverlayResult;
use self::instance_screenshot_tile_action::InstanceScreenshotTileAction;

pub(super) fn render_instance_screenshot_gallery(
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

pub(super) fn retain_instance_viewer_image(
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

pub(super) fn render_instance_screenshot_viewer_modal(
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

pub(super) fn render_instance_delete_screenshot_modal(
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

    let danger = ctx.global_style().visuals.error_fg_color;
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

fn ensure_instance_screenshot_scan_channel(state: &mut InstanceScreenState) {
    if state.screenshot_scan_results_tx.is_some() && state.screenshot_scan_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(u64, Vec<InstanceScreenshotEntry>)>();
    state.screenshot_scan_results_tx = Some(tx);
    state.screenshot_scan_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn poll_instance_screenshot_scan_results(state: &mut InstanceScreenState) {
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

pub(super) fn poll_instance_screenshot_delete_results(
    state: &mut InstanceScreenState,
    instance_root: &Path,
) {
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

pub(super) fn refresh_instance_screenshots(
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

pub(super) fn screenshot_key(path: &Path) -> String {
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

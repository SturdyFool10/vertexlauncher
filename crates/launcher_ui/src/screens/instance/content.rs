use super::content_lookup_result::ContentLookupResultEntry;
use super::*;
use content_resolver::detect_installed_content_kind;
use std::collections::{BTreeMap, HashSet};
use ui_foundation::fill_tab_row;

const CONTENT_HASH_CACHE_FLUSH_DEBOUNCE: Duration = Duration::from_millis(750);
const CONTENT_LOOKUP_REPAINT_INTERVAL: Duration = Duration::from_millis(100);
const CONTENT_LOOKUP_BATCH_SIZE: usize = 24;
const CONTENT_UPDATE_PREFETCH_BATCH_SIZE: usize = 4;
const CONTENT_UPDATE_LOG_TARGET: &str = "vertexlauncher/content_update";

#[derive(Clone, Debug)]
pub(super) struct ContentApplyResult {
    pub(super) kind: InstalledContentKind,
    pub(super) focus_lookup_keys: Vec<String>,
    pub(super) refresh_all_content: bool,
    pub(super) status_message: Result<String, String>,
}

pub(super) fn render_installed_content_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    instance_root: &Path,
    download_policy: &DownloadPolicy,
    state: &mut InstanceScreenState,
    external_install_active: bool,
    output: &mut InstanceScreenOutput,
) {
    let content_browser_locked = state.runtime_prepare_in_flight || external_install_active;
    let content_browser_locked_reason =
        "Content Browser is unavailable while instance prep is running.";
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);
    poll_content_apply_results(state, instance_root);
    ensure_content_hash_cache_loaded(state, instance_root);

    let (open_browser_response, add_menu_button) =
        render_joined_content_browser_controls(ui, text_ui, instance_id, !content_browser_locked);
    if content_browser_locked {
        let _ = open_browser_response
            .clone()
            .on_disabled_hover_text(content_browser_locked_reason);
    }
    if open_browser_response.clicked() {
        output.requested_screen = Some(AppScreen::ContentBrowser);
    }

    let popup_id = ui.id().with(("instance_add_content_popup", instance_id));
    let _ = egui::Popup::menu(&add_menu_button)
        .id(popup_id)
        .width(240.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            let popup_button_style = style::neutral_button_with_min_size(
                ui,
                egui::vec2(ui.available_width().max(120.0), style::CONTROL_HEIGHT),
            );
            let add_local_response = ui
                .add_enabled_ui(!state.content_apply_in_flight, |ui| {
                    text_ui.button(
                        ui,
                        ("instance_content_popup_add_local_files", instance_id),
                        "Add local file(s)",
                        &popup_button_style,
                    )
                })
                .inner;
            if add_local_response.clicked() {
                if let Some(selected_paths) = rfd::FileDialog::new()
                    .set_title("Add Local Content")
                    .pick_files()
                {
                    request_local_content_import(state, instance_root, selected_paths);
                }
            }

            if text_ui
                .button(
                    ui,
                    ("instance_content_popup_local", instance_id),
                    "Refresh local files",
                    &popup_button_style,
                )
                .clicked()
            {
                refresh_installed_content_state(state, instance_root);
                state.status_message =
                    Some("Refreshed installed content metadata and hash cache.".to_owned());
            }
        });

    ui.add_space(10.0);
    let previous_tab = state.selected_content_tab;
    fill_tab_row(
        text_ui,
        ui,
        ("instance_content_tab", instance_id),
        &mut state.selected_content_tab,
        &InstalledContentKind::ALL.map(|tab| (tab, tab.label())),
        30.0,
        6.0,
    );
    if state.selected_content_tab != previous_tab {
        state.installed_content_page = 1;
    }

    ui.add_space(10.0);

    let installed_files =
        installed_content_files_for_tab(state, instance_root, state.selected_content_tab);
    if installed_files.is_empty() {
        let _ = text_ui.label(
            ui,
            (
                "instance_content_empty",
                instance_id,
                state.selected_content_tab.label(),
            ),
            &format!(
                "No {} installed.",
                state.selected_content_tab.label().to_lowercase()
            ),
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        return;
    }

    let page_size = if INSTALLED_CONTENT_PAGE_SIZES.contains(&state.installed_content_page_size) {
        state.installed_content_page_size
    } else {
        INSTALLED_CONTENT_PAGE_SIZES[1]
    };
    state.installed_content_page_size = page_size;

    let total_items = installed_files.len();
    let total_pages = total_items.div_ceil(page_size).max(1);
    state.installed_content_page = state.installed_content_page.clamp(1, total_pages);

    ui.horizontal(|ui| {
        ui.set_min_height(style::CONTROL_HEIGHT);
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        let label_style = LabelOptions {
            line_height: style::CONTROL_HEIGHT - 6.0,
            color: ui.visuals().text_color(),
            wrap: false,
            ..LabelOptions::default()
        };
        let muted_label_style = LabelOptions {
            color: ui.visuals().weak_text_color(),
            ..label_style.clone()
        };

        let _ = text_ui.label(
            ui,
            ("instance_content_page_size_label", instance_id),
            "Items per page",
            &label_style,
        );
        egui::ComboBox::from_id_salt(("instance_content_page_size", instance_id))
            .selected_text(state.installed_content_page_size.to_string())
            .show_ui(ui, |ui| {
                for page_size in INSTALLED_CONTENT_PAGE_SIZES {
                    if ui
                        .selectable_value(
                            &mut state.installed_content_page_size,
                            page_size,
                            page_size.to_string(),
                        )
                        .changed()
                    {
                        state.installed_content_page = 1;
                    }
                }
            });

        let _ = text_ui.label(
            ui,
            ("instance_content_page_label", instance_id),
            "Page",
            &label_style,
        );
        egui::ComboBox::from_id_salt(("instance_content_page", instance_id))
            .selected_text(format!(
                "{} / {}",
                state.installed_content_page, total_pages
            ))
            .show_ui(ui, |ui| {
                for page in 1..=total_pages {
                    ui.selectable_value(
                        &mut state.installed_content_page,
                        page,
                        format!("Page {page}"),
                    );
                }
            });

        let _ = text_ui.label(
            ui,
            ("instance_content_page_total", instance_id),
            &format!("{total_items} installed"),
            &muted_label_style,
        );
    });

    let selected_game_version = selected_game_version(state).to_owned();
    let selected_modloader = selected_modloader_value(state).to_owned();
    prefetch_bulk_update_metadata(
        state,
        installed_files.as_ref(),
        state.selected_content_tab,
        selected_game_version.as_str(),
        selected_modloader.as_str(),
    );
    let has_bulk_update_available = tab_has_known_available_update(state, installed_files.as_ref());
    if let Some(delay) = installed_content_lookup_repaint_delay(state, installed_files.as_ref()) {
        ui.ctx().request_repaint_after(delay);
    }

    if has_bulk_update_available {
        ui.add_space(10.0);
        let bulk_update_label = bulk_update_button_label(state.selected_content_tab);
        let bulk_update_tooltip = bulk_update_button_tooltip(state.selected_content_tab);
        let bulk_update_clicked = render_bulk_update_button(
            ui,
            (
                "instance_content_bulk_update",
                instance_id,
                state.selected_content_tab.label(),
            ),
            bulk_update_label.as_str(),
            bulk_update_tooltip.as_str(),
            !state.content_apply_in_flight,
        );
        ui.add_space(8.0);

        if bulk_update_clicked {
            request_bulk_content_update(
                state,
                instance_root,
                state.selected_content_tab,
                selected_game_version.as_str(),
                selected_modloader.as_str(),
                download_policy,
            );
        }
    }

    let start_index = (state.installed_content_page - 1) * state.installed_content_page_size;
    let end_index = (start_index + state.installed_content_page_size).min(total_items);
    let visible_files = &installed_files[start_index..end_index];
    let _ = request_content_metadata_lookup_batch(
        state,
        visible_files,
        state.selected_content_tab,
        selected_game_version.as_str(),
        selected_modloader.as_str(),
        false,
        CONTENT_LOOKUP_BATCH_SIZE,
    );
    let delete_icon_color = ui.visuals().error_fg_color;
    let delete_button_icon_svg = apply_color_to_svg(assets::TRASH_X_SVG, delete_icon_color);
    let warning_icon_svg = apply_color_to_svg(assets::WARN_SVG, ui.visuals().warn_fg_color);

    let mut pending_delete: Option<(PathBuf, String)> = None;
    let mut pending_update: Option<(String, String, PathBuf)> = None;
    let scroll_height = ui.available_height().max(180.0);
    egui::ScrollArea::vertical()
        .id_salt((
            "instance_installed_content_scroll",
            instance_id,
            state.selected_content_tab.label(),
        ))
        .auto_shrink([false, false])
        .max_height(scroll_height)
        .show(ui, |ui| {
            let row_width = (ui.max_rect().width() - INSTALLED_CONTENT_SCROLLBAR_RESERVE).max(1.0);
            ui.set_min_width(row_width);
            ui.set_max_width(row_width);
            for (visible_index, entry) in visible_files.iter().enumerate() {
                let entry_index = start_index + visible_index;
                let metadata = state
                    .content_metadata_cache
                    .get(&entry.lookup_key)
                    .and_then(|meta| meta.as_ref());
                let display_name = metadata
                    .map(|value| value.entry.name.clone())
                    .unwrap_or_else(|| entry.file_name.clone());
                let description = metadata
                    .map(|value| {
                        if value.entry.summary.trim().is_empty() {
                            entry.file_name.clone()
                        } else {
                            value.entry.summary.clone()
                        }
                    })
                    .unwrap_or_else(|| entry.file_name.clone());
                let platform_label = metadata
                    .map(|value| value.entry.source.label().to_owned())
                    .unwrap_or_else(|| "Unknown".to_owned());
                let version_label =
                    metadata.and_then(|value| value.installed_version_label.clone());
                let warning_message = metadata.and_then(|value| value.warning_message.clone());
                let update_label = metadata
                    .and_then(|value| value.update.as_ref())
                    .map(|update| format!("Update: {}", update.latest_version_label));
                let update_version_id = metadata
                    .and_then(|value| value.update.as_ref())
                    .map(|update| update.latest_version_id.clone());
                let icon_url = metadata
                    .and_then(|value| value.entry.icon_url.as_deref())
                    .map(str::to_owned);

                let rendered = ui
                    .scope_builder(
                        egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(row_width, f32::INFINITY),
                        )),
                        |ui| {
                            ui.set_min_width(row_width);
                            ui.set_max_width(row_width);
                            render_installed_content_entry(
                                ui,
                                text_ui,
                                (instance_id, entry_index),
                                state,
                                entry,
                                display_name.as_str(),
                                description.as_str(),
                                platform_label.as_str(),
                                version_label.as_deref(),
                                warning_message.as_deref(),
                                update_label.as_deref(),
                                icon_url.as_deref(),
                                delete_button_icon_svg.as_slice(),
                                delete_icon_color,
                                warning_icon_svg.as_slice(),
                            )
                        },
                    )
                    .inner;

                if rendered.delete_clicked {
                    pending_delete = Some((entry.file_path.clone(), entry.lookup_key.clone()));
                } else if rendered.update_clicked {
                    if let Some(version_id) = update_version_id {
                        pending_update = Some((
                            entry.lookup_key.clone(),
                            version_id.to_owned(),
                            entry.file_path.clone(),
                        ));
                    }
                } else if rendered.open_clicked {
                    if content_browser_locked {
                        state.status_message = Some(
                            "Content Browser is unavailable while instance prep is running."
                                .to_owned(),
                        );
                    } else if let Some(metadata) = state
                        .content_metadata_cache
                        .get(&entry.lookup_key)
                        .and_then(|meta| meta.clone())
                    {
                        crate::screens::content_browser::request_open_detail_for_content(
                            metadata.entry,
                        );
                        output.requested_screen = Some(AppScreen::ContentBrowser);
                    } else {
                        state.status_message = Some(
                            "Still loading content metadata. Try again in a moment.".to_owned(),
                        );
                    }
                }
                ui.add_space(8.0);
            }
        });

    if let Some((path, lookup_key)) = pending_delete {
        request_content_delete(
            state,
            instance_root,
            state.selected_content_tab,
            lookup_key.as_str(),
            path.as_path(),
        );
    }

    if let Some((lookup_key, version_id, file_path)) = pending_update
        && let Some(metadata) = state
            .content_metadata_cache
            .get(&lookup_key)
            .and_then(|meta| meta.clone())
    {
        request_content_update(
            state,
            instance_root,
            lookup_key.as_str(),
            metadata.entry,
            file_path.as_path(),
            version_id.as_str(),
            selected_game_version.as_str(),
            selected_modloader.as_str(),
        );
    }

    if state.content_hash_cache_dirty {
        let repaint_after = state
            .content_hash_cache_dirty_since
            .map(|dirty_since| {
                CONTENT_HASH_CACHE_FLUSH_DEBOUNCE.saturating_sub(dirty_since.elapsed())
            })
            .unwrap_or(CONTENT_HASH_CACHE_FLUSH_DEBOUNCE);
        ui.ctx().request_repaint_after(repaint_after);
    }
    flush_content_hash_cache(state, instance_root);
}

fn installed_content_files_for_tab(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    tab: InstalledContentKind,
) -> Arc<[InstalledContentFile]> {
    if let Some(files) = state.installed_content_cache.files_by_tab.get(&tab) {
        return files.clone();
    }

    let managed_identities = state
        .installed_content_cache
        .managed_identities
        .get_or_insert_with(|| load_managed_content_identities(instance_root));
    let files: Arc<[InstalledContentFile]> =
        InstalledContentResolver::scan_installed_content_files(
            instance_root,
            tab,
            managed_identities,
        )
        .into();
    state
        .installed_content_cache
        .files_by_tab
        .insert(tab, files.clone());
    files
}

fn render_installed_content_entry(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id_source: impl std::hash::Hash + Copy,
    state: &mut InstanceScreenState,
    entry: &InstalledContentFile,
    display_name: &str,
    description: &str,
    platform_label: &str,
    version_label: Option<&str>,
    warning_message: Option<&str>,
    update_label: Option<&str>,
    icon_url: Option<&str>,
    delete_button_icon_svg: &[u8],
    delete_icon_color: egui::Color32,
    warning_icon_svg: &[u8],
) -> InstalledEntryRenderResult {
    const INSTALLED_TILE_GAP: f32 = 8.0;
    const INSTALLED_TILE_THUMBNAIL_FRAME_PADDING: f32 = 8.0;
    const INSTALLED_DESCRIPTION_LINE_HEIGHT: f32 = 20.0;
    const INSTALLED_DESCRIPTION_FRAME_Y_PADDING: i8 = 3;
    let available_width = ui.available_width().max(1.0);
    let tile_width = (available_width - (style::SPACE_XS * 2.0)).max(1.0);
    let side_padding = ((available_width - tile_width) * 0.5).max(0.0);

    let (delete_clicked, update_clicked, open_clicked) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if side_padding > 0.0 {
                ui.add_space(side_padding);
            }

            let frame_response = egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
                .inner_margin(egui::Margin::same(style::SPACE_MD as i8))
                .show(ui, |ui| {
                    ui.set_min_width(tile_width);
                    ui.set_max_width(tile_width);

                    let mut delete_clicked = false;
                    let mut update_clicked = false;
                    let action_button_width = 28.0;
                    let content_width = ui.available_width().max(1.0);
                    let thumbnail_size = ((content_width - 52.0) * 0.14).clamp(32.0, 48.0);
                    let thumbnail_frame_size =
                        thumbnail_size + INSTALLED_TILE_THUMBNAIL_FRAME_PADDING;
                    let thumbnail_lane_height = 92.0_f32.max(thumbnail_frame_size);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        let delete_button_id =
                            format!("instance-content-delete-{}", entry.lookup_key);
                        if render_installed_content_action_button(
                            ui,
                            delete_button_id.as_str(),
                            delete_button_icon_svg,
                            delete_icon_color,
                            "Delete this content",
                            action_button_width,
                            action_button_width,
                        ) {
                            delete_clicked = true;
                        }

                        ui.add_space(INSTALLED_TILE_GAP);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width().max(1.0), 0.0),
                            egui::Layout::left_to_right(egui::Align::TOP),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(thumbnail_frame_size, thumbnail_lane_height),
                                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                                    |ui| {
                                        egui::Frame::new()
                                            .fill(ui.visuals().extreme_bg_color)
                                            .stroke(egui::Stroke::NONE)
                                            .corner_radius(egui::CornerRadius::same(
                                                style::CORNER_RADIUS_SM,
                                            ))
                                            .inner_margin(egui::Margin::same(4))
                                            .show(ui, |ui| {
                                                render_content_thumbnail(
                                                    ui,
                                                    id_source,
                                                    icon_url,
                                                    thumbnail_size,
                                                );
                                            });
                                    },
                                );

                                ui.add_space(INSTALLED_TILE_GAP);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width().max(1.0), 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(ui.available_width().max(1.0));
                                        let _ = text_ui.label(
                                            ui,
                                            (id_source, "name"),
                                            display_name,
                                            &LabelOptions {
                                                font_size: 19.0,
                                                line_height: 24.0,
                                                weight: 700,
                                                color: ui.visuals().text_color(),
                                                wrap: true,
                                                ..LabelOptions::default()
                                            },
                                        );

                                        ui.add_space(4.0);
                                        ui.horizontal_wrapped(|ui| {
                                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                                            render_installed_content_badge(
                                                ui,
                                                text_ui,
                                                (id_source, "platform_badge"),
                                                platform_label,
                                                ui.visuals().selection.bg_fill,
                                                ui.visuals().selection.stroke.color,
                                            );
                                            if let Some(version_label) = version_label {
                                                render_installed_content_badge(
                                                    ui,
                                                    text_ui,
                                                    (id_source, "version_badge"),
                                                    version_label,
                                                    ui.visuals().widgets.inactive.bg_fill,
                                                    ui.visuals().text_color(),
                                                );
                                            }
                                            if let Some(update) = update_label {
                                                if render_installed_content_update_badge(
                                                    ui,
                                                    text_ui,
                                                    (id_source, "update_badge"),
                                                    update,
                                                    !state.content_apply_in_flight,
                                                ) {
                                                    update_clicked = true;
                                                }
                                            }
                                            if let Some(warning_message) = warning_message {
                                                render_installed_content_warning(
                                                    ui,
                                                    warning_message,
                                                    id_source,
                                                    warning_icon_svg,
                                                );
                                            }
                                        });

                                        ui.add_space(4.0);
                                        egui::Frame::new()
                                            .fill(ui.visuals().extreme_bg_color)
                                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                                            .corner_radius(egui::CornerRadius::same(
                                                style::CORNER_RADIUS_SM,
                                            ))
                                            .inner_margin(egui::Margin::symmetric(
                                                6,
                                                INSTALLED_DESCRIPTION_FRAME_Y_PADDING,
                                            ))
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width().max(1.0));
                                                let description_style = LabelOptions {
                                                    line_height: INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                    color: ui.visuals().text_color(),
                                                    wrap: false,
                                                    ..LabelOptions::default()
                                                };
                                                let truncated_description =
                                                    cached_truncated_description(
                                                        state,
                                                        text_ui,
                                                        ui,
                                                        entry.lookup_key.as_str(),
                                                        description,
                                                        ui.available_width().max(1.0),
                                                        &description_style,
                                                    );
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(
                                                        ui.available_width().max(1.0),
                                                        INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                    ),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        ui.set_min_height(
                                                            INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                        );
                                                        ui.set_max_height(
                                                            INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                        );
                                                        ui.set_width(ui.available_width().max(1.0));
                                                        let _ = text_ui.label(
                                                            ui,
                                                            (id_source, "description"),
                                                            truncated_description.as_str(),
                                                            &description_style,
                                                        );
                                                    },
                                                );
                                            });
                                    },
                                );
                            },
                        );
                    });

                    (delete_clicked, update_clicked)
                });

            if side_padding > 0.0 {
                ui.add_space(side_padding);
            }

            (
                frame_response.inner.0,
                frame_response.inner.1,
                frame_response.response.clicked(),
            )
        })
        .inner;

    InstalledEntryRenderResult {
        open_clicked: open_clicked && !delete_clicked && !update_clicked,
        delete_clicked,
        update_clicked,
    }
}

fn render_content_thumbnail(ui: &mut Ui, id_source: impl Hash, icon_url: Option<&str>, size: f32) {
    let size = egui::vec2(size, size);
    if let Some(icon_url) = icon_url {
        remote_tiled_image::show(
            ui,
            icon_url,
            size,
            (id_source, "remote-icon"),
            assets::LIBRARY_SVG,
        );
    } else {
        let mut hasher = DefaultHasher::new();
        id_source.hash(&mut hasher);
        ui.add(
            egui::Image::from_bytes(
                format!(
                    "bytes://instance/default-content-icon/{}.svg",
                    hasher.finish()
                ),
                assets::LIBRARY_SVG,
            )
            .fit_to_exact_size(size),
        );
    }
}

fn render_installed_content_action_button(
    ui: &mut Ui,
    icon_id: &str,
    themed_svg: &[u8],
    icon_color: egui::Color32,
    tooltip: &str,
    width: f32,
    height: f32,
) -> bool {
    let uri = format!(
        "bytes://instance-installed-content-action/{icon_id}-{:02x}{:02x}{:02x}.svg",
        icon_color.r(),
        icon_color.g(),
        icon_color.b()
    );
    let button_size = egui::vec2(width, height);
    let icon_size = (height - 10.0).max(12.0);
    let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());
    let visuals = ui.visuals();
    let button_fill = if response.is_pointer_button_down_on() {
        visuals.widgets.active.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.extreme_bg_color
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), button_fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        visuals.widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let image = egui::Image::from_bytes(uri, themed_svg.to_vec())
        .fit_to_exact_size(egui::vec2(icon_size, icon_size));
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(icon_size, icon_size));
    let _ = ui.put(icon_rect, image);

    response.on_hover_text(tooltip).clicked()
}

fn render_bulk_update_button(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + Copy,
    label: &str,
    tooltip: &str,
    enabled: bool,
) -> bool {
    ui.push_id(id_source, |ui| {
        let text_color = if enabled {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let fill = if enabled {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };
        let sense = if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let button_size = egui::vec2(ui.available_width().max(220.0), 34.0);
        let (rect, response) = ui.allocate_exact_size(button_size, sense);
        let button_fill = if enabled {
            if response.is_pointer_button_down_on() {
                fill.gamma_multiply(0.9)
            } else if response.hovered() {
                fill.gamma_multiply(1.08)
            } else {
                fill
            }
        } else {
            fill
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(8), button_fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(8),
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );

        let icon_size = 16.0;
        let icon_uri = format!(
            "bytes://instance-content-bulk-update/{:02x}{:02x}{:02x}.svg",
            text_color.r(),
            text_color.g(),
            text_color.b()
        );
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 12.0, rect.center().y - (icon_size * 0.5)),
            egui::vec2(icon_size, icon_size),
        );
        let refresh_icon_svg = apply_color_to_svg(assets::REFRESH_SVG, text_color);
        let _ = ui.put(
            icon_rect,
            egui::Image::from_bytes(icon_uri, refresh_icon_svg)
                .fit_to_exact_size(egui::vec2(icon_size, icon_size)),
        );
        ui.painter().text(
            egui::pos2(icon_rect.max.x + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(14.0),
            text_color,
        );

        response
            .on_hover_text(if enabled {
                tooltip
            } else {
                "A content operation is already in progress."
            })
            .clicked()
    })
    .inner
}

fn bulk_update_button_label(kind: InstalledContentKind) -> String {
    format!("Update all {}", kind.label().to_ascii_lowercase())
}

fn bulk_update_button_tooltip(kind: InstalledContentKind) -> String {
    if kind == InstalledContentKind::Mods {
        "Updates all mods to the latest compatible version, you typically should not update pre-made modpacks most of the time if you are playing Multiplayer, or if your modpack is complex".to_owned()
    } else {
        format!(
            "Updates all {} to the latest compatible version for this instance.",
            kind.label().to_ascii_lowercase()
        )
    }
}

fn render_installed_content_badge(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id_source: impl std::hash::Hash + Copy,
    label: &str,
    fill: egui::Color32,
    text_color: egui::Color32,
) {
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            let _ = text_ui.label(
                ui,
                id_source,
                label,
                &LabelOptions {
                    font_size: 13.0,
                    line_height: 16.0,
                    color: text_color,
                    wrap: false,
                    ..LabelOptions::default()
                },
            );
        });
}

fn render_installed_content_update_badge(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id_source: impl std::hash::Hash + Copy,
    label: &str,
    enabled: bool,
) -> bool {
    ui.push_id(id_source, |ui| {
        let text_color = if enabled {
            ui.visuals().warn_fg_color
        } else {
            ui.visuals().weak_text_color()
        };
        let fill = if enabled {
            ui.visuals().warn_fg_color.gamma_multiply(0.16)
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };
        let stroke_color = if enabled {
            ui.visuals().warn_fg_color
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke.color
        };
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                text_ui.button(
                    ui,
                    ("installed_content_update_badge", label),
                    label,
                    &ButtonOptions {
                        min_size: egui::vec2(0.0, 0.0),
                        corner_radius: 6,
                        text_color,
                        fill,
                        fill_hovered: fill.gamma_multiply(1.08),
                        fill_active: fill.gamma_multiply(0.92),
                        fill_selected: fill,
                        stroke: egui::Stroke::new(1.0, stroke_color),
                        ..ButtonOptions::default()
                    },
                )
            })
            .inner;
        response
            .on_hover_text(if enabled {
                "Update to this version"
            } else {
                "A content operation is already in progress"
            })
            .clicked()
    })
    .inner
}

fn prefetch_bulk_update_metadata(
    state: &mut InstanceScreenState,
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
) {
    if tab_has_known_available_update(state, installed_files) {
        return;
    }

    let _ = request_content_metadata_lookup_batch(
        state,
        installed_files,
        kind,
        game_version,
        loader,
        false,
        CONTENT_UPDATE_PREFETCH_BATCH_SIZE,
    );
}

fn tab_has_known_available_update(
    state: &InstanceScreenState,
    installed_files: &[InstalledContentFile],
) -> bool {
    installed_files.iter().any(|entry| {
        state
            .content_metadata_cache
            .get(&entry.lookup_key)
            .and_then(|resolution| resolution.as_ref())
            .and_then(|resolution| resolution.update.as_ref())
            .is_some()
    })
}

fn render_installed_content_warning(
    ui: &mut Ui,
    warning_message: &str,
    id_source: impl std::hash::Hash + Copy,
    warning_icon_svg: &[u8],
) {
    let mut hasher = DefaultHasher::new();
    id_source.hash(&mut hasher);
    let uri = format!(
        "bytes://instance-installed-content-warning/{}.svg",
        hasher.finish()
    );
    let response = ui.add(
        egui::Image::from_bytes(uri, warning_icon_svg.to_vec())
            .fit_to_exact_size(egui::vec2(16.0, 16.0)),
    );
    response.on_hover_text(warning_message);
}

fn ensure_content_hash_cache_loaded(state: &mut InstanceScreenState, instance_root: &Path) {
    if state.content_hash_cache.is_none() {
        state.content_hash_cache = Some(InstalledContentResolver::load_hash_cache(instance_root));
    }
}

fn flush_content_hash_cache(state: &mut InstanceScreenState, instance_root: &Path) {
    if !state.content_hash_cache_dirty {
        return;
    }
    let Some(dirty_since) = state.content_hash_cache_dirty_since else {
        state.content_hash_cache_dirty_since = Some(Instant::now());
        return;
    };
    if dirty_since.elapsed() < CONTENT_HASH_CACHE_FLUSH_DEBOUNCE {
        return;
    }

    if let Some(cache) = state.content_hash_cache.as_ref()
        && InstalledContentResolver::save_hash_cache(instance_root, cache).is_ok()
    {
        state.content_hash_cache_dirty = false;
        state.content_hash_cache_dirty_since = None;
    }
}

fn clear_content_hash_cache(state: &mut InstanceScreenState, instance_root: &Path) {
    state.content_hash_cache = Some(InstalledContentHashCache::default());
    state.content_hash_cache_dirty = false;
    state.content_hash_cache_dirty_since = None;
    let _ = InstalledContentResolver::clear_hash_cache(instance_root);
}

fn cached_truncated_description(
    state: &mut InstanceScreenState,
    text_ui: &mut TextUi,
    ui: &Ui,
    lookup_key: &str,
    description: &str,
    max_width: f32,
    label_options: &LabelOptions,
) -> String {
    let width_bucket = (max_width.max(0.0) / 8.0).round() as u32;
    if let Some(cache_entry) = state.installed_content_entry_ui_cache.get(lookup_key)
        && cache_entry.description_source == description
        && cache_entry.description_width_bucket == width_bucket
    {
        return cache_entry.truncated_description.clone();
    }

    let truncated_description = textui::truncate_single_line_text_with_ellipsis(
        text_ui,
        ui,
        description,
        max_width,
        label_options,
    );
    state.installed_content_entry_ui_cache.insert(
        lookup_key.to_owned(),
        InstalledContentEntryUiCache {
            description_source: description.to_owned(),
            description_width_bucket: width_bucket,
            truncated_description: truncated_description.clone(),
        },
    );
    truncated_description
}

fn ensure_content_lookup_channel(state: &mut InstanceScreenState) {
    if state.content_lookup_results_tx.is_some() && state.content_lookup_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<ContentLookupResult>();
    state.content_lookup_results_tx = Some(tx);
    state.content_lookup_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn should_request_content_metadata_lookup(
    state: &mut InstanceScreenState,
    lookup_key: &str,
    force_refresh: bool,
) -> bool {
    let now = Instant::now();
    if lookup_key.trim().is_empty() || state.content_lookup_in_flight.contains(lookup_key) {
        return false;
    }
    if !force_refresh {
        if state
            .content_metadata_cache
            .get(lookup_key)
            .is_some_and(|resolution| resolution.is_some())
        {
            return false;
        }
        if let Some(retry_at) = state.content_lookup_retry_after_by_key.get(lookup_key)
            && *retry_at > now
        {
            return false;
        }
    }
    true
}

fn mark_content_metadata_lookup_requested(
    state: &mut InstanceScreenState,
    lookup_key: &str,
) -> u64 {
    state.content_lookup_request_serial = state.content_lookup_request_serial.saturating_add(1);
    let request_serial = state.content_lookup_request_serial;
    state.content_lookup_in_flight.insert(lookup_key.to_owned());
    state.content_lookup_retry_after_by_key.remove(lookup_key);
    state
        .content_lookup_latest_serial_by_key
        .insert(lookup_key.to_owned(), request_serial);
    request_serial
}

#[allow(clippy::too_many_arguments)]
fn request_content_metadata_lookup_batch(
    state: &mut InstanceScreenState,
    files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
    force_refresh: bool,
    max_batch_size: usize,
) -> usize {
    if files.is_empty() || max_batch_size == 0 {
        return 0;
    }

    ensure_content_lookup_channel(state);
    let Some(tx) = state.content_lookup_results_tx.as_ref().cloned() else {
        return 0;
    };

    let mut work_items = Vec::new();
    for file in files {
        if work_items.len() >= max_batch_size {
            break;
        }
        if !should_request_content_metadata_lookup(state, file.lookup_key.as_str(), force_refresh) {
            continue;
        }
        let request_serial =
            mark_content_metadata_lookup_requested(state, file.lookup_key.as_str());
        work_items.push((request_serial, file.clone()));
    }
    if work_items.is_empty() {
        return 0;
    }
    let scheduled_count = work_items.len();

    let hash_cache = state.content_hash_cache.clone().unwrap_or_default();
    let game_version = game_version.trim().to_owned();
    let loader = loader.trim().to_owned();
    let _ = tokio_runtime::spawn_detached(async move {
        let result = resolve_installed_content_lookup_batch(
            work_items.as_slice(),
            kind,
            game_version.as_str(),
            loader.as_str(),
            hash_cache,
        );
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/instance_content",
                kind = %kind.folder_name(),
                scheduled_count,
                game_version = %game_version,
                loader = %loader,
                error = %err,
                "Failed to deliver installed-content metadata lookup result."
            );
        }
    });

    scheduled_count
}

pub(super) fn poll_content_lookup_results(state: &mut InstanceScreenState) {
    let Some(rx) = state.content_lookup_results_rx.as_ref() else {
        return;
    };
    let Ok(guard) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/instance_content",
            in_flight = state.content_lookup_in_flight.len(),
            tracked_keys = state.content_lookup_latest_serial_by_key.len(),
            "Instance content lookup receiver mutex was poisoned while polling metadata results."
        );
        return;
    };

    while let Ok(result) = guard.try_recv() {
        let cache = state
            .content_hash_cache
            .get_or_insert_with(InstalledContentHashCache::default);
        if cache.apply_updates(result.hash_cache_updates) {
            state.content_hash_cache_dirty = true;
            state.content_hash_cache_dirty_since = Some(Instant::now());
        }

        for result in result.results {
            let is_latest = state
                .content_lookup_latest_serial_by_key
                .get(result.lookup_key.as_str())
                .copied()
                == Some(result.request_serial);
            if !is_latest {
                continue;
            }
            state
                .content_lookup_in_flight
                .remove(result.lookup_key.as_str());
            if let Some(resolution) = result.resolution {
                state
                    .content_lookup_retry_after_by_key
                    .remove(result.lookup_key.as_str());
                state
                    .content_lookup_failure_count_by_key
                    .remove(result.lookup_key.as_str());
                state
                    .content_metadata_cache
                    .insert(result.lookup_key, Some(resolution));
                continue;
            }

            let failure_count = state
                .content_lookup_failure_count_by_key
                .entry(result.lookup_key.clone())
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
            state.content_lookup_retry_after_by_key.insert(
                result.lookup_key.clone(),
                Instant::now() + content_lookup_retry_delay(*failure_count),
            );
            state
                .content_metadata_cache
                .entry(result.lookup_key)
                .or_insert(None);
        }
    }
}

fn resolve_installed_content_lookup_batch(
    work_items: &[(u64, InstalledContentFile)],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    mut hash_cache: InstalledContentHashCache,
) -> ContentLookupResult {
    let installed_files = work_items
        .iter()
        .map(|(_, file)| file.clone())
        .collect::<Vec<_>>();
    let original_hash_cache = hash_cache.clone();
    let resolved_by_lookup_key = resolve_installed_content_metadata_batch(
        installed_files.as_slice(),
        kind,
        game_version,
        loader_label,
        &mut hash_cache,
    )
    .into_iter()
    .map(|(file, resolution)| (file.lookup_key, resolution))
    .collect::<std::collections::HashMap<_, _>>();

    ContentLookupResult {
        results: work_items
            .iter()
            .map(|(request_serial, file)| ContentLookupResultEntry {
                request_serial: *request_serial,
                lookup_key: file.lookup_key.clone(),
                resolution: resolved_by_lookup_key
                    .get(file.lookup_key.as_str())
                    .cloned(),
            })
            .collect(),
        hash_cache_updates: hash_cache_diff_updates(&original_hash_cache, &hash_cache),
    }
}

fn hash_cache_diff_updates(
    previous: &InstalledContentHashCache,
    current: &InstalledContentHashCache,
) -> Vec<content_resolver::InstalledContentHashCacheUpdate> {
    current
        .entries
        .iter()
        .filter(|(hash_key, resolution)| {
            previous.entries.get(hash_key.as_str()) != Some(*resolution)
        })
        .map(
            |(hash_key, resolution)| content_resolver::InstalledContentHashCacheUpdate {
                hash_key: hash_key.clone(),
                resolution: resolution.clone(),
            },
        )
        .collect()
}

fn refresh_cached_metadata_after_apply(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    kind: InstalledContentKind,
    focus_lookup_keys: &[String],
) {
    let managed_identities = load_managed_content_identities(instance_root);
    let installed_files: Arc<[InstalledContentFile]> =
        InstalledContentResolver::scan_installed_content_files(
            instance_root,
            kind,
            &managed_identities,
        )
        .into();
    let active_keys = installed_files
        .iter()
        .map(|entry| entry.lookup_key.clone())
        .collect::<HashSet<_>>();
    let key_prefix = format!("{}::", kind.folder_name());

    state.installed_content_cache.managed_identities = Some(managed_identities);
    state
        .installed_content_cache
        .files_by_tab
        .insert(kind, installed_files.clone());
    state
        .content_metadata_cache
        .retain(|key, _| !key.starts_with(key_prefix.as_str()) || active_keys.contains(key));
    state
        .installed_content_entry_ui_cache
        .retain(|key, _| !key.starts_with(key_prefix.as_str()) || active_keys.contains(key));
    state
        .content_lookup_in_flight
        .retain(|key| !key.starts_with(key_prefix.as_str()) || active_keys.contains(key));
    state
        .content_lookup_latest_serial_by_key
        .retain(|key, _| !key.starts_with(key_prefix.as_str()) || active_keys.contains(key));
    state
        .content_lookup_retry_after_by_key
        .retain(|key, _| !key.starts_with(key_prefix.as_str()) || active_keys.contains(key));
    state
        .content_lookup_failure_count_by_key
        .retain(|key, _| !key.starts_with(key_prefix.as_str()) || active_keys.contains(key));

    if installed_files.is_empty() {
        return;
    }

    let selected_game_version = selected_game_version(state).to_owned();
    let selected_modloader = selected_modloader_value(state).to_owned();
    let mut refresh_keys = HashSet::new();
    for lookup_key in focus_lookup_keys {
        if active_keys.contains(lookup_key) {
            refresh_keys.insert(lookup_key.clone());
        }
    }

    let (start_index, end_index) = if state.selected_content_tab == kind {
        let total_items = installed_files.len();
        let total_pages = total_items
            .div_ceil(state.installed_content_page_size.max(1))
            .max(1);
        state.installed_content_page = state.installed_content_page.clamp(1, total_pages);
        let start = (state.installed_content_page - 1) * state.installed_content_page_size;
        let end = (start + state.installed_content_page_size).min(total_items);
        (start, end)
    } else {
        (
            0,
            installed_files
                .len()
                .min(CONTENT_UPDATE_PREFETCH_BATCH_SIZE),
        )
    };

    for entry in &installed_files[start_index..end_index] {
        refresh_keys.insert(entry.lookup_key.clone());
    }

    let refresh_files = installed_files
        .iter()
        .filter(|entry| refresh_keys.contains(&entry.lookup_key))
        .cloned()
        .collect::<Vec<_>>();
    for entry in &refresh_files {
        state
            .content_lookup_in_flight
            .remove(entry.lookup_key.as_str());
        state
            .content_lookup_retry_after_by_key
            .remove(entry.lookup_key.as_str());
        state
            .content_lookup_failure_count_by_key
            .remove(entry.lookup_key.as_str());
    }
    let _ = request_content_metadata_lookup_batch(
        state,
        refresh_files.as_slice(),
        kind,
        selected_game_version.as_str(),
        selected_modloader.as_str(),
        true,
        refresh_files.len(),
    );

    prefetch_bulk_update_metadata(
        state,
        installed_files.as_ref(),
        kind,
        selected_game_version.as_str(),
        selected_modloader.as_str(),
    );
}

fn content_lookup_retry_delay(failure_count: u8) -> Duration {
    match failure_count {
        0 | 1 => Duration::from_secs(2),
        2 => Duration::from_secs(10),
        3 => Duration::from_secs(30),
        _ => Duration::from_secs(120),
    }
}

fn installed_content_lookup_repaint_delay(
    state: &InstanceScreenState,
    installed_files: &[InstalledContentFile],
) -> Option<Duration> {
    let now = Instant::now();
    let mut next_retry: Option<Duration> = None;

    for entry in installed_files {
        if state.content_lookup_in_flight.contains(&entry.lookup_key)
            || !state.content_metadata_cache.contains_key(&entry.lookup_key)
        {
            return Some(CONTENT_LOOKUP_REPAINT_INTERVAL);
        }

        if state
            .content_metadata_cache
            .get(&entry.lookup_key)
            .is_some_and(|resolution| resolution.is_none())
        {
            match state
                .content_lookup_retry_after_by_key
                .get(&entry.lookup_key)
            {
                Some(retry_at) if *retry_at > now => {
                    let remaining = retry_at.saturating_duration_since(now);
                    next_retry = Some(match next_retry {
                        Some(current) => current.min(remaining),
                        None => remaining,
                    });
                }
                _ => return Some(CONTENT_LOOKUP_REPAINT_INTERVAL),
            }
        }
    }

    next_retry
}

fn ensure_content_apply_channel(state: &mut InstanceScreenState) {
    if state.content_apply_results_tx.is_some() && state.content_apply_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<ContentApplyResult>();
    state.content_apply_results_tx = Some(tx);
    state.content_apply_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_content_update(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    lookup_key: &str,
    entry: modprovider::UnifiedContentEntry,
    installed_file_path: &Path,
    version_id: &str,
    game_version: &str,
    loader_label: &str,
) {
    let lookup_key = lookup_key.trim();
    let version_id = version_id.trim();
    if lookup_key.is_empty() || version_id.is_empty() || state.content_apply_in_flight {
        return;
    }

    ensure_content_apply_channel(state);
    let Some(tx) = state.content_apply_results_tx.as_ref().cloned() else {
        return;
    };

    let lookup_key = lookup_key.to_owned();
    let version_id = version_id.to_owned();
    let installed_file_path = installed_file_path.to_path_buf();
    let game_version = game_version.trim().to_owned();
    let loader_label = loader_label.trim().to_owned();
    let project_name = if entry.name.trim().is_empty() {
        "content".to_owned()
    } else {
        entry.name.clone()
    };
    let instance_name = state.name_input.clone();
    let instance_root = instance_root.to_path_buf();
    let kind = state.selected_content_tab;

    state.content_apply_in_flight = true;
    state.status_message = Some(format!("Updating {}...", project_name));
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance = %instance_name,
        lookup_key = %lookup_key,
        project = %project_name,
        version_id = %version_id,
        installed_path = %installed_file_path.display(),
        game_version = %game_version,
        loader = %loader_label,
        "starting individual content update"
    );
    install_activity::set_status(
        instance_name.as_str(),
        InstallStage::DownloadingCore,
        format!("Updating {}...", project_name),
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let result = crate::screens::content_browser::update_installed_content_to_version(
            instance_root.as_path(),
            &entry,
            installed_file_path.as_path(),
            version_id.as_str(),
            game_version.as_str(),
            loader_label.as_str(),
        );
        match &result {
            Ok(message) => tracing::info!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                lookup_key = %lookup_key,
                project = %project_name,
                "individual content update completed: {message}"
            ),
            Err(err) => tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                lookup_key = %lookup_key,
                project = %project_name,
                "individual content update failed: {err}"
            ),
        }
        let focus_lookup_keys = vec![lookup_key.clone()];
        if let Err(err) = tx.send(ContentApplyResult {
            kind,
            focus_lookup_keys,
            refresh_all_content: false,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                lookup_key = %lookup_key,
                project = %project_name,
                error = %err,
                "Failed to deliver individual content update result."
            );
        }
    });
}

fn render_joined_content_browser_controls(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    open_browser_enabled: bool,
) -> (egui::Response, egui::Response) {
    let total_width = ui.available_width().max(1.0);
    let control_height = 34.0;
    let icon_button_width = control_height;
    let label_width = (total_width - icon_button_width).max(120.0);
    let button_style =
        style::neutral_button_with_min_size(ui, egui::vec2(label_width, control_height));
    let cr = style::CORNER_RADIUS_SM;
    let label_radius = egui::CornerRadius {
        nw: cr,
        sw: cr,
        ne: 0,
        se: 0,
    };
    let icon_radius = egui::CornerRadius {
        nw: 0,
        sw: 0,
        ne: cr,
        se: cr,
    };
    let (outer_rect, _) = ui.allocate_exact_size(
        egui::vec2(total_width, control_height),
        egui::Sense::hover(),
    );
    let label_rect =
        egui::Rect::from_min_size(outer_rect.min, egui::vec2(label_width, control_height));
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(label_rect.max.x, outer_rect.min.y),
        egui::vec2(icon_button_width, control_height),
    );

    let label_response = ui.interact(
        label_rect,
        ui.id().with(("instance_add_content_label", instance_id)),
        if open_browser_enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let label_visuals = if open_browser_enabled {
        ui.style().interact(&label_response)
    } else {
        &ui.visuals().widgets.inactive
    };
    ui.painter().rect(
        label_rect,
        label_radius,
        label_visuals.bg_fill,
        label_visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let label_inner = label_rect.shrink2(egui::vec2(button_style.padding.x, 0.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(label_inner), |ui| {
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                let label_options = LabelOptions {
                    font_size: button_style.font_size,
                    line_height: button_style.line_height,
                    weight: 700,
                    color: if open_browser_enabled {
                        button_style.text_color
                    } else {
                        ui.visuals().weak_text_color()
                    },
                    wrap: false,
                    ..LabelOptions::default()
                };
                let _ = text_ui.label(
                    ui,
                    ("instance_add_content_label", instance_id),
                    "Open Content Browser",
                    &label_options,
                );
            },
        );
    });

    let icon_response = ui.interact(
        icon_rect,
        ui.id().with(("instance_add_content_plus", instance_id)),
        egui::Sense::click(),
    );
    let icon_visuals = ui.style().interact(&icon_response);
    ui.painter().rect(
        icon_rect,
        icon_radius,
        icon_visuals.bg_fill,
        icon_visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let icon_color = ui.visuals().text_color();
    let themed_svg = apply_color_to_svg(assets::PLUS_SVG, icon_color);
    let uri = format!(
        "bytes://instance/content-plus/{instance_id}-{:02x}{:02x}{:02x}.svg",
        icon_color.r(),
        icon_color.g(),
        icon_color.b()
    );
    let icon_size = (control_height - style::SPACE_MD * 2.0).clamp(12.0, 18.0);
    let icon_draw_rect =
        egui::Rect::from_center_size(icon_rect.center(), egui::vec2(icon_size, icon_size));
    egui::Image::from_bytes(uri, themed_svg).paint_at(ui, icon_draw_rect);

    (label_response, icon_response)
}

fn refresh_installed_content_state(state: &mut InstanceScreenState, instance_root: &Path) {
    state.invalidate_installed_content_cache();
    state.content_metadata_cache.clear();
    state.content_lookup_in_flight.clear();
    state.content_lookup_latest_serial_by_key.clear();
    state.content_lookup_retry_after_by_key.clear();
    state.content_lookup_failure_count_by_key.clear();
    clear_content_hash_cache(state, instance_root);
}

fn request_local_content_import(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    selected_paths: Vec<PathBuf>,
) {
    if state.content_apply_in_flight || selected_paths.is_empty() {
        return;
    }

    ensure_content_apply_channel(state);
    let Some(tx) = state.content_apply_results_tx.as_ref().cloned() else {
        return;
    };

    let instance_root = instance_root.to_path_buf();
    let instance_name = state.name_input.clone();
    state.content_apply_in_flight = true;
    state.status_message = Some(format!(
        "Adding {} local content file{}...",
        selected_paths.len(),
        if selected_paths.len() == 1 { "" } else { "s" }
    ));
    install_activity::set_status(
        instance_name.as_str(),
        InstallStage::DownloadingCore,
        "Adding local content files...".to_owned(),
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let result = import_local_content_files(instance_root.as_path(), selected_paths.as_slice());
        let focus_lookup_keys = selected_paths
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|value| value.to_string_lossy().to_string())
            })
            .collect();
        if let Err(err) = tx.send(ContentApplyResult {
            kind: InstalledContentKind::Mods,
            focus_lookup_keys,
            refresh_all_content: true,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                error = %err,
                "Failed to deliver local content import result."
            );
        }
    });
}

fn import_local_content_files(
    instance_root: &Path,
    selected_paths: &[PathBuf],
) -> Result<String, String> {
    let mut imported_total = 0usize;
    let mut counts_by_kind = BTreeMap::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    for source_path in selected_paths {
        let Some(file_name) = source_path.file_name() else {
            skipped.push(source_path.display().to_string());
            continue;
        };

        let Some(kind) = detect_installed_content_kind(source_path.as_path()) else {
            skipped.push(source_path.display().to_string());
            continue;
        };

        let destination_dir = instance_root.join(kind.folder_name());
        if let Err(err) = fs::create_dir_all(destination_dir.as_path()) {
            failures.push(format!(
                "{} -> {} ({err})",
                source_path.display(),
                destination_dir.display()
            ));
            continue;
        }

        let destination_path = destination_dir.join(file_name);
        if let Err(err) = fs::copy(source_path.as_path(), destination_path.as_path()) {
            failures.push(format!(
                "{} -> {} ({err})",
                source_path.display(),
                destination_path.display()
            ));
            continue;
        }

        imported_total += 1;
        *counts_by_kind.entry(kind.label()).or_insert(0usize) += 1;
    }

    let mut summary = counts_by_kind
        .into_iter()
        .map(|(label, count)| format!("{count} {label}"))
        .collect::<Vec<_>>();
    if summary.is_empty() {
        summary.push("no files".to_owned());
    }

    if imported_total == 0 {
        if !failures.is_empty() {
            return Err(format!(
                "Failed to add local content: {}.",
                failures.join("; ")
            ));
        }
        if !skipped.is_empty() {
            return Err(format!(
                "Could not determine where to place: {}.",
                skipped.join(", ")
            ));
        }
    }

    let mut message = format!("Added {}.", summary.join(", "));
    if !failures.is_empty() {
        message.push_str(" Failed to copy: ");
        message.push_str(failures.join("; ").as_str());
        message.push('.');
    }
    if !skipped.is_empty() {
        message.push_str(" Skipped unrecognized files: ");
        message.push_str(skipped.join(", ").as_str());
        message.push('.');
    }
    Ok(message)
}

fn request_content_delete(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    kind: InstalledContentKind,
    lookup_key: &str,
    path: &Path,
) {
    let lookup_key = lookup_key.trim();
    if lookup_key.is_empty() || state.content_apply_in_flight {
        return;
    }

    ensure_content_apply_channel(state);
    let Some(tx) = state.content_apply_results_tx.as_ref().cloned() else {
        return;
    };

    let lookup_key = lookup_key.to_owned();
    let path = path.to_path_buf();
    let instance_root = instance_root.to_path_buf();
    let instance_name = state.name_input.clone();
    let path_display = path.display().to_string();

    state.content_apply_in_flight = true;
    state.status_message = Some("Removing installed content...".to_owned());
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance = %instance_name,
        lookup_key = %lookup_key,
        path = %path_display,
        "starting installed content delete"
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let result = (|| -> Result<String, String> {
            let delete_result = if path.is_dir() {
                std::fs::remove_dir_all(path.as_path())
            } else {
                std::fs::remove_file(path.as_path())
            };
            delete_result.map_err(|err| format!("failed to remove {}: {err}", path.display()))?;

            managed_content::remove_content_manifest_entries_for_path(
                instance_root.as_path(),
                path.as_path(),
            )
            .map(|_| "Removed installed content.".to_owned())
            .map_err(|err| {
                format!(
                    "removed installed content, but failed to update the content manifest: {err}"
                )
            })
        })();

        if let Err(err) = tx.send(ContentApplyResult {
            kind,
            focus_lookup_keys: vec![lookup_key],
            refresh_all_content: false,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                path = %path_display,
                error = %err,
                "Failed to deliver installed content delete result."
            );
        }
    });
}

fn request_bulk_content_update(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    download_policy: &DownloadPolicy,
) {
    if state.content_apply_in_flight {
        return;
    }

    ensure_content_apply_channel(state);
    let Some(tx) = state.content_apply_results_tx.as_ref().cloned() else {
        return;
    };

    let instance_name = state.name_input.clone();
    let instance_root = instance_root.to_path_buf();
    let game_version = game_version.trim().to_owned();
    let loader_label = loader_label.trim().to_owned();
    let download_policy = download_policy.clone();
    let progress_instance_name = instance_name.clone();
    let progress: InstallProgressCallback = Arc::new(move |progress| {
        install_activity::set_progress(progress_instance_name.as_str(), &progress);
    });
    let operation_label = bulk_update_button_label(kind);
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance = %instance_name,
        kind = %kind.folder_name(),
        operation = %operation_label,
        game_version = %game_version,
        loader = %loader_label,
        "starting bulk content update"
    );

    state.content_apply_in_flight = true;
    state.status_message = Some(format!("{operation_label}..."));
    install_activity::set_status(
        instance_name.as_str(),
        InstallStage::DownloadingCore,
        format!("{operation_label}..."),
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            update_all_installed_content(
                instance_root.as_path(),
                kind,
                game_version.as_str(),
                loader_label.as_str(),
                &download_policy,
                Some(&progress),
            )
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        if result.is_ok() {
            install_activity::set_status(
                instance_name.as_str(),
                InstallStage::Complete,
                format!("{operation_label} complete."),
            );
        }
        match &result {
            Ok(message) => tracing::info!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                kind = %kind.folder_name(),
                operation = %operation_label,
                "bulk content update completed: {message}"
            ),
            Err(err) => tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                kind = %kind.folder_name(),
                operation = %operation_label,
                "bulk content update failed: {err}"
            ),
        }
        if let Err(err) = tx.send(ContentApplyResult {
            kind,
            focus_lookup_keys: Vec::new(),
            refresh_all_content: false,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                kind = %kind.folder_name(),
                operation = %operation_label,
                error = %err,
                "Failed to deliver bulk content update result."
            );
        }
    });
}

fn update_all_installed_content(
    instance_root: &Path,
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    download_policy: &DownloadPolicy,
    progress: Option<&InstallProgressCallback>,
) -> Result<String, String> {
    let mut updated_count = 0usize;
    let mut pass = 0usize;
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        kind = %kind.folder_name(),
        game_version = %game_version,
        loader = %loader_label,
        "scanning for bulk content updates"
    );

    loop {
        pass += 1;
        if pass > 512 {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                kind = %kind.folder_name(),
                updated_count,
                "aborting bulk content update after too many passes"
            );
            return Err(format!(
                "Stopped updating {} after too many passes.",
                kind.label().to_ascii_lowercase()
            ));
        }

        let managed_identities = load_managed_content_identities(instance_root);
        let installed_files = InstalledContentResolver::scan_installed_content_files(
            instance_root,
            kind,
            &managed_identities,
        );
        tracing::debug!(
            target: CONTENT_UPDATE_LOG_TARGET,
            instance_root = %instance_root.display(),
            kind = %kind.folder_name(),
            pass,
            installed_files = installed_files.len(),
            "scanned installed content for bulk update pass"
        );
        if installed_files.is_empty() {
            break;
        }

        let mut hash_cache = InstalledContentResolver::load_hash_cache(instance_root);
        let manifest = managed_content::load_content_manifest(instance_root);
        let mut cleaned_stale_duplicates = 0usize;
        let mut updates = Vec::new();
        for (file, resolution) in resolve_installed_content_metadata_batch(
            installed_files.as_slice(),
            kind,
            game_version,
            loader_label,
            &mut hash_cache,
        ) {
            let Some(update) = resolution.update.as_ref() else {
                continue;
            };
            if let Some(managed_path) = stale_managed_content_path_for_update(
                instance_root,
                &manifest,
                &file,
                &resolution,
                update.latest_version_id.as_str(),
            ) {
                tracing::warn!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    "removing stale duplicate content during bulk update pass file_path={} managed_path={} project={} latest_version_id={}",
                    file.file_path.display(),
                    managed_path.display(),
                    resolution.entry.name,
                    update.latest_version_id,
                );
                remove_stale_duplicate_content_path(file.file_path.as_path())?;
                cleaned_stale_duplicates += 1;
                continue;
            }
            updates.push(crate::screens::content_browser::BulkContentUpdate {
                entry: resolution.entry.clone(),
                installed_file_path: file.file_path.clone(),
                version_id: update.latest_version_id.clone(),
            });
        }

        if updates.is_empty() {
            if cleaned_stale_duplicates > 0 {
                tracing::info!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    "cleaned stale duplicate content during bulk update pass instance_root={} kind={} pass={} cleaned_duplicates={}",
                    instance_root.display(),
                    kind.folder_name(),
                    pass,
                    cleaned_stale_duplicates,
                );
                continue;
            }
            tracing::info!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                kind = %kind.folder_name(),
                pass,
                updated_count,
                "no further bulk content updates found"
            );
            break;
        }

        tracing::info!(
            target: CONTENT_UPDATE_LOG_TARGET,
            instance_root = %instance_root.display(),
            kind = %kind.folder_name(),
            pass,
            queued_updates = updates.len(),
            "applying queued bulk content updates"
        );
        let applied = crate::screens::content_browser::bulk_update_installed_content(
            instance_root,
            updates.as_slice(),
            game_version,
            loader_label,
            download_policy,
            progress,
        )?;
        if applied == 0 {
            tracing::warn!(
                target: CONTENT_UPDATE_LOG_TARGET,
                "bulk content update pass made no progress; stopping to avoid a no-op loop instance_root={} kind={} pass={} queued_updates={}",
                instance_root.display(),
                kind.folder_name(),
                pass,
                updates.len(),
            );
            break;
        }
        updated_count += applied;
    }

    let kind_label = kind.label().to_ascii_lowercase();
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        kind = %kind.folder_name(),
        updated_count,
        "finished bulk content update scan"
    );
    if updated_count == 0 {
        Ok(format!("No {kind_label} updates available."))
    } else if updated_count == 1 {
        Ok(format!("Updated 1 {} entry.", kind.content_type_key()))
    } else {
        Ok(format!("Updated {updated_count} {kind_label}."))
    }
}

fn remove_stale_duplicate_content_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove stale directory {}: {err}", path.display()))
    } else {
        std::fs::remove_file(path)
            .map_err(|err| format!("failed to remove stale file {}: {err}", path.display()))
    }
}

fn stale_managed_content_path_for_update(
    instance_root: &Path,
    manifest: &managed_content::ContentInstallManifest,
    file: &InstalledContentFile,
    resolution: &content_resolver::ResolvedInstalledContent,
    latest_version_id: &str,
) -> Option<PathBuf> {
    let project = manifest_project_for_entry(manifest, &resolution.entry)?;
    if project.selected_version_id.as_deref() != Some(latest_version_id) {
        return None;
    }

    let managed_path = instance_root.join(project.file_path.as_path());
    if !managed_path.exists()
        || content_paths_match(managed_path.as_path(), file.file_path.as_path())
    {
        return None;
    }

    Some(managed_path)
}

fn manifest_project_for_entry<'a>(
    manifest: &'a managed_content::ContentInstallManifest,
    entry: &modprovider::UnifiedContentEntry,
) -> Option<&'a managed_content::InstalledContentProject> {
    match entry.source {
        modprovider::ContentSource::Modrinth => {
            let project_id = entry.id.strip_prefix("modrinth:")?;
            manifest
                .projects
                .values()
                .find(|project| project.modrinth_project_id.as_deref() == Some(project_id))
        }
        modprovider::ContentSource::CurseForge => {
            let project_id = entry.id.strip_prefix("curseforge:")?.parse::<u64>().ok()?;
            manifest
                .projects
                .values()
                .find(|project| project.curseforge_project_id == Some(project_id))
        }
    }
}

fn content_paths_match(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }

    #[cfg(not(target_os = "windows"))]
    {
        left == right
    }
}

fn resolve_installed_content_for_update(
    file: &InstalledContentFile,
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    hash_cache: &mut InstalledContentHashCache,
) -> Option<content_resolver::ResolvedInstalledContent> {
    let request = ResolveInstalledContentRequest {
        file_path: file.file_path.clone(),
        disk_file_name: file.file_name.trim().to_owned(),
        lookup_query: file.lookup_query.trim().to_owned(),
        fallback_lookup_key: file
            .fallback_lookup_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        fallback_lookup_query: file
            .fallback_lookup_query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        managed_identity: file.managed_identity.clone(),
        kind,
        game_version: game_version.trim().to_owned(),
        loader: loader_label.trim().to_owned(),
    };
    let result = InstalledContentResolver::resolve(&request, hash_cache);
    let _ = hash_cache.apply_updates(result.hash_cache_updates);
    result.resolution
}

fn resolve_installed_content_metadata_batch(
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    hash_cache: &mut InstalledContentHashCache,
) -> Vec<(
    InstalledContentFile,
    content_resolver::ResolvedInstalledContent,
)> {
    let mut prefetched = vec![None; installed_files.len()];
    prefetch_modrinth_hash_updates(
        installed_files,
        kind,
        game_version,
        loader_label,
        hash_cache,
        prefetched.as_mut_slice(),
    );
    prefetch_managed_modrinth_updates(
        installed_files,
        kind,
        game_version,
        loader_label,
        prefetched.as_mut_slice(),
    );
    prefetch_managed_curseforge_updates(
        installed_files,
        kind,
        game_version,
        loader_label,
        prefetched.as_mut_slice(),
    );

    let mut resolved = Vec::new();
    for (index, file) in installed_files.iter().enumerate() {
        let resolution = prefetched[index].clone().or_else(|| {
            resolve_installed_content_for_update(file, kind, game_version, loader_label, hash_cache)
        });
        if let Some(resolution) = resolution {
            resolved.push((file.clone(), resolution));
        }
    }
    resolved
}

fn prefetch_modrinth_hash_updates(
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    hash_cache: &mut InstalledContentHashCache,
    prefetched: &mut [Option<content_resolver::ResolvedInstalledContent>],
) {
    #[derive(Clone)]
    struct HashWorkItem {
        index: usize,
        sha1: String,
        sha512: String,
    }

    let modrinth = modrinth::Client::default();
    let loaders = if kind == InstalledContentKind::Mods {
        modrinth_loader_slugs_for_update_prefetch(loader_label)
    } else {
        Vec::new()
    };
    let game_versions = normalized_game_versions_for_update_prefetch(game_version);
    let mut pending_sha512 = Vec::new();
    let mut pending_sha1 = Vec::new();
    let mut cached_sha512 = Vec::new();
    let mut cached_sha1 = Vec::new();
    let mut cache_updates = Vec::new();

    for (index, file) in installed_files.iter().enumerate() {
        if prefetched[index].is_some() {
            continue;
        }
        if !supports_modrinth_hash_prefetch(kind, file.file_path.as_path()) {
            continue;
        }

        let Ok((sha1, sha512)) = modrinth::hash_file_sha1_and_sha512_hex(file.file_path.as_path())
        else {
            continue;
        };
        let sha512_key = format!("sha512:{sha512}");
        let sha1_key = format!("sha1:{sha1}");

        if let Some(Some(resolution)) = hash_cache.entries.get(sha512_key.as_str()) {
            cached_sha512.push((index, sha512, resolution.clone()));
            continue;
        }
        if let Some(Some(resolution)) = hash_cache.entries.get(sha1_key.as_str()) {
            cached_sha1.push((index, sha1, resolution.clone()));
            continue;
        }

        let sha512_missing = !hash_cache.entries.contains_key(sha512_key.as_str());
        let sha1_missing = !hash_cache.entries.contains_key(sha1_key.as_str());
        let item = HashWorkItem {
            index,
            sha1,
            sha512,
        };
        if sha512_missing {
            pending_sha512.push(item);
        } else if sha1_missing {
            pending_sha1.push(item);
        } else {
            cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                hash_key: sha512_key,
                resolution: None,
            });
            cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                hash_key: sha1_key,
                resolution: None,
            });
        }
    }

    let cached_sha512_latest = modrinth
        .get_latest_versions_from_hashes(
            &cached_sha512
                .iter()
                .map(|(_, hash, _)| hash.clone())
                .collect::<Vec<_>>(),
            "sha512",
            loaders.as_slice(),
            game_versions.as_slice(),
        )
        .unwrap_or_default();
    for (index, hash, mut resolution) in cached_sha512 {
        resolution.update = modrinth_update_from_latest_prefetch(
            cached_sha512_latest.get(hash.as_str()),
            resolution.installed_version_id.as_deref(),
        );
        prefetched[index] = Some(resolution);
    }

    let cached_sha1_latest = modrinth
        .get_latest_versions_from_hashes(
            &cached_sha1
                .iter()
                .map(|(_, hash, _)| hash.clone())
                .collect::<Vec<_>>(),
            "sha1",
            loaders.as_slice(),
            game_versions.as_slice(),
        )
        .unwrap_or_default();
    for (index, hash, mut resolution) in cached_sha1 {
        resolution.update = modrinth_update_from_latest_prefetch(
            cached_sha1_latest.get(hash.as_str()),
            resolution.installed_version_id.as_deref(),
        );
        prefetched[index] = Some(resolution);
    }

    let mut pending_sha1_from_sha512 = Vec::new();
    if !pending_sha512.is_empty()
        && let Ok(versions_by_hash) = modrinth.get_versions_from_hashes(
            &pending_sha512
                .iter()
                .map(|item| item.sha512.clone())
                .collect::<Vec<_>>(),
            "sha512",
        )
    {
        let projects_by_id = modrinth_projects_by_id_prefetch(
            &modrinth,
            versions_by_hash
                .values()
                .map(|version| version.project_id.clone())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        let latest_by_hash = modrinth
            .get_latest_versions_from_hashes(
                &pending_sha512
                    .iter()
                    .map(|item| item.sha512.clone())
                    .collect::<Vec<_>>(),
                "sha512",
                loaders.as_slice(),
                game_versions.as_slice(),
            )
            .unwrap_or_default();
        for item in pending_sha512 {
            if let Some(version) = versions_by_hash.get(item.sha512.as_str()) {
                if let Some(project) = projects_by_id.get(version.project_id.as_str()) {
                    let resolution = modrinth_resolution_from_prefetched_hash(
                        project,
                        version,
                        latest_by_hash.get(item.sha512.as_str()),
                    );
                    cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                        hash_key: format!("sha512:{}", item.sha512),
                        resolution: Some(hash_cache_resolution_without_update_prefetch(
                            &resolution,
                        )),
                    });
                    cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                        hash_key: format!("sha1:{}", item.sha1),
                        resolution: Some(hash_cache_resolution_without_update_prefetch(
                            &resolution,
                        )),
                    });
                    prefetched[item.index] = Some(resolution);
                    continue;
                }
            }
            pending_sha1_from_sha512.push(item);
        }
    } else {
        pending_sha1_from_sha512 = pending_sha512;
    }
    pending_sha1.extend(pending_sha1_from_sha512);

    if !pending_sha1.is_empty()
        && let Ok(versions_by_hash) = modrinth.get_versions_from_hashes(
            &pending_sha1
                .iter()
                .map(|item| item.sha1.clone())
                .collect::<Vec<_>>(),
            "sha1",
        )
    {
        let projects_by_id = modrinth_projects_by_id_prefetch(
            &modrinth,
            versions_by_hash
                .values()
                .map(|version| version.project_id.clone())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        let latest_by_hash = modrinth
            .get_latest_versions_from_hashes(
                &pending_sha1
                    .iter()
                    .map(|item| item.sha1.clone())
                    .collect::<Vec<_>>(),
                "sha1",
                loaders.as_slice(),
                game_versions.as_slice(),
            )
            .unwrap_or_default();
        for item in pending_sha1 {
            if let Some(version) = versions_by_hash.get(item.sha1.as_str())
                && let Some(project) = projects_by_id.get(version.project_id.as_str())
            {
                let resolution = modrinth_resolution_from_prefetched_hash(
                    project,
                    version,
                    latest_by_hash.get(item.sha1.as_str()),
                );
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha512:{}", item.sha512),
                    resolution: Some(hash_cache_resolution_without_update_prefetch(&resolution)),
                });
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha1:{}", item.sha1),
                    resolution: Some(hash_cache_resolution_without_update_prefetch(&resolution)),
                });
                prefetched[item.index] = Some(resolution);
            } else {
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha512:{}", item.sha512),
                    resolution: None,
                });
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha1:{}", item.sha1),
                    resolution: None,
                });
            }
        }
    }

    let _ = hash_cache.apply_updates(cache_updates);
}

fn prefetch_managed_modrinth_updates(
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    prefetched: &mut [Option<content_resolver::ResolvedInstalledContent>],
) {
    #[derive(Clone)]
    struct ModrinthWorkItem {
        index: usize,
        project_id: String,
        version_id: String,
    }

    let mut work_items = Vec::new();
    for (index, file) in installed_files.iter().enumerate() {
        if prefetched[index].is_some() {
            continue;
        }
        let Some(identity) = file.managed_identity.as_ref() else {
            continue;
        };
        if identity.pack_managed {
            continue;
        }
        if identity.source != modprovider::ContentSource::Modrinth {
            continue;
        }
        let Some(project_id) = identity.modrinth_project_id.as_ref() else {
            continue;
        };
        let version_id = identity.selected_version_id.trim();
        if version_id.is_empty() {
            continue;
        }
        work_items.push(ModrinthWorkItem {
            index,
            project_id: project_id.clone(),
            version_id: version_id.to_owned(),
        });
    }
    if work_items.is_empty() {
        return;
    }

    let modrinth = modrinth::Client::default();
    let versions_by_id = match modrinth.get_versions(
        &work_items
            .iter()
            .map(|item| item.version_id.clone())
            .collect::<Vec<_>>(),
    ) {
        Ok(versions) => versions
            .into_iter()
            .map(|version| (version.id.clone(), version))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(_) => return,
    };
    let projects_by_id = modrinth_projects_by_id_prefetch(
        &modrinth,
        &work_items
            .iter()
            .map(|item| item.project_id.clone())
            .collect::<Vec<_>>(),
    );
    let loaders = if kind == InstalledContentKind::Mods {
        modrinth_loader_slugs_for_update_prefetch(loader_label)
    } else {
        Vec::new()
    };
    let game_versions = normalized_game_versions_for_update_prefetch(game_version);
    let latest_versions_by_project_id = modrinth_latest_versions_by_project_prefetch(
        &modrinth,
        &work_items
            .iter()
            .map(|item| item.project_id.clone())
            .collect::<Vec<_>>(),
        loaders.as_slice(),
        game_versions.as_slice(),
    );

    for item in work_items {
        let Some(version) = versions_by_id.get(item.version_id.as_str()) else {
            continue;
        };
        if version.project_id != item.project_id {
            continue;
        }
        let file = &installed_files[item.index];
        if !version_contains_file_name_for_update_prefetch(
            version.files.as_slice(),
            file.file_name.as_str(),
        ) {
            continue;
        }
        let Some(project) = projects_by_id.get(item.project_id.as_str()) else {
            continue;
        };

        prefetched[item.index] = Some(modrinth_resolution_from_prefetched_managed(
            project,
            version,
            latest_versions_by_project_id.get(item.project_id.as_str()),
        ));
    }
}

fn prefetch_managed_curseforge_updates(
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    prefetched: &mut [Option<content_resolver::ResolvedInstalledContent>],
) {
    let Some(curseforge) = curseforge::Client::from_env() else {
        return;
    };

    #[derive(Clone, Copy)]
    struct CurseForgeWorkItem {
        index: usize,
        project_id: u64,
        version_id: u64,
    }

    let mut work_items = Vec::new();
    for (index, file) in installed_files.iter().enumerate() {
        if prefetched[index].is_some() {
            continue;
        }
        let Some(identity) = file.managed_identity.as_ref() else {
            continue;
        };
        if identity.pack_managed {
            continue;
        }
        if identity.source != modprovider::ContentSource::CurseForge {
            continue;
        }
        let Some(project_id) = identity.curseforge_project_id else {
            continue;
        };
        let Ok(version_id) = identity.selected_version_id.trim().parse::<u64>() else {
            continue;
        };
        work_items.push(CurseForgeWorkItem {
            index,
            project_id,
            version_id,
        });
    }
    if work_items.is_empty() {
        return;
    }

    let installed_files_by_id = match curseforge.get_files(
        &work_items
            .iter()
            .map(|item| item.version_id)
            .collect::<Vec<_>>(),
    ) {
        Ok(files) => files
            .into_iter()
            .map(|file| (file.id, file))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(_) => return,
    };
    let projects_by_id = match curseforge.get_mods(
        &work_items
            .iter()
            .map(|item| item.project_id)
            .collect::<Vec<_>>(),
    ) {
        Ok(projects) => projects
            .into_iter()
            .map(|project| (project.id, project))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(_) => return,
    };

    let latest_file_ids = work_items
        .iter()
        .filter_map(|item| {
            projects_by_id.get(&item.project_id).and_then(|project| {
                select_curseforge_latest_file_id_prefetch(project, kind, game_version, loader_label)
            })
        })
        .collect::<Vec<_>>();
    let latest_files_by_id = match curseforge.get_files(latest_file_ids.as_slice()) {
        Ok(files) => files
            .into_iter()
            .map(|file| (file.id, file))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(_) => return,
    };

    for item in work_items {
        let Some(project) = projects_by_id.get(&item.project_id) else {
            continue;
        };
        let Some(latest_file_id) =
            select_curseforge_latest_file_id_prefetch(project, kind, game_version, loader_label)
        else {
            continue;
        };
        let Some(installed_file) = installed_files_by_id.get(&item.version_id) else {
            continue;
        };
        let Some(latest_file) = latest_files_by_id.get(&latest_file_id) else {
            continue;
        };

        let file = &installed_files[item.index];
        if !file_name_matches_for_update_prefetch(
            installed_file.file_name.as_str(),
            file.file_name.as_str(),
        ) || !file
            .file_path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                file_name_matches_for_update_prefetch(value, file.file_name.as_str())
            })
        {
            continue;
        }

        let update = if latest_file.id == item.version_id || latest_file.download_url.is_none() {
            None
        } else {
            Some(content_resolver::InstalledContentUpdate {
                latest_version_id: latest_file.id.to_string(),
                latest_version_label: non_empty_owned_for_update_prefetch(
                    latest_file.display_name.as_str(),
                )
                .unwrap_or_else(|| "Unknown update".to_owned()),
            })
        };

        prefetched[item.index] = Some(content_resolver::ResolvedInstalledContent {
            entry: modprovider::UnifiedContentEntry {
                id: format!("curseforge:{}", project.id),
                name: project.name.clone(),
                summary: project.summary.trim().to_owned(),
                content_type: kind.content_type_key().to_owned(),
                source: modprovider::ContentSource::CurseForge,
                project_url: project.website_url.clone(),
                icon_url: project.icon_url.clone(),
            },
            installed_version_id: Some(installed_file.id.to_string()),
            installed_version_label: non_empty_owned_for_update_prefetch(
                installed_file.display_name.as_str(),
            ),
            resolution_kind: content_resolver::InstalledContentResolutionKind::Managed,
            warning_message: None,
            update,
        });
    }
}

fn modrinth_projects_by_id_prefetch(
    modrinth: &modrinth::Client,
    project_ids: &[String],
) -> std::collections::HashMap<String, modrinth::Project> {
    modrinth
        .get_projects(project_ids)
        .unwrap_or_default()
        .into_iter()
        .map(|project| (project.project_id.clone(), project))
        .collect()
}

fn modrinth_latest_versions_by_project_prefetch(
    modrinth: &modrinth::Client,
    project_ids: &[String],
    loaders: &[String],
    game_versions: &[String],
) -> std::collections::HashMap<String, modrinth::ProjectVersion> {
    let mut latest_versions = std::collections::HashMap::new();
    let mut seen = HashSet::new();

    for project_id in project_ids {
        if !seen.insert(project_id.clone()) {
            continue;
        }
        let Ok(versions) =
            modrinth.list_project_versions(project_id.as_str(), loaders, game_versions)
        else {
            continue;
        };
        let Some(latest) = versions
            .into_iter()
            .filter(|version| !version.files.is_empty())
            .max_by(|left, right| left.date_published.cmp(&right.date_published))
        else {
            continue;
        };
        latest_versions.insert(project_id.clone(), latest);
    }

    latest_versions
}

fn modrinth_resolution_from_prefetched_hash(
    project: &modrinth::Project,
    version: &modrinth::ProjectVersion,
    latest: Option<&modrinth::ProjectVersion>,
) -> content_resolver::ResolvedInstalledContent {
    content_resolver::ResolvedInstalledContent {
        entry: modprovider::UnifiedContentEntry {
            id: format!("modrinth:{}", project.project_id),
            name: project.title.clone(),
            summary: project.description.trim().to_owned(),
            content_type: project.project_type.clone(),
            source: modprovider::ContentSource::Modrinth,
            project_url: Some(project.project_url.clone()),
            icon_url: project.icon_url.clone(),
        },
        installed_version_id: non_empty_owned_for_update_prefetch(version.id.as_str()),
        installed_version_label: non_empty_owned_for_update_prefetch(
            version.version_number.as_str(),
        ),
        resolution_kind: content_resolver::InstalledContentResolutionKind::ExactHash,
        warning_message: None,
        update: modrinth_update_from_latest_prefetch(latest, Some(version.id.as_str())),
    }
}

fn modrinth_resolution_from_prefetched_managed(
    project: &modrinth::Project,
    version: &modrinth::ProjectVersion,
    latest: Option<&modrinth::ProjectVersion>,
) -> content_resolver::ResolvedInstalledContent {
    content_resolver::ResolvedInstalledContent {
        entry: modprovider::UnifiedContentEntry {
            id: format!("modrinth:{}", project.project_id),
            name: project.title.clone(),
            summary: project.description.trim().to_owned(),
            content_type: project.project_type.clone(),
            source: modprovider::ContentSource::Modrinth,
            project_url: Some(project.project_url.clone()),
            icon_url: project.icon_url.clone(),
        },
        installed_version_id: non_empty_owned_for_update_prefetch(version.id.as_str()),
        installed_version_label: non_empty_owned_for_update_prefetch(
            version.version_number.as_str(),
        ),
        resolution_kind: content_resolver::InstalledContentResolutionKind::Managed,
        warning_message: None,
        update: modrinth_update_from_latest_prefetch(latest, Some(version.id.as_str())),
    }
}

fn modrinth_update_from_latest_prefetch(
    latest: Option<&modrinth::ProjectVersion>,
    installed_version_id: Option<&str>,
) -> Option<content_resolver::InstalledContentUpdate> {
    let latest = latest?;
    if installed_version_id.is_some_and(|value| value == latest.id) {
        return None;
    }

    Some(content_resolver::InstalledContentUpdate {
        latest_version_id: latest.id.clone(),
        latest_version_label: non_empty_owned_for_update_prefetch(latest.version_number.as_str())
            .unwrap_or_else(|| "Unknown update".to_owned()),
    })
}

fn hash_cache_resolution_without_update_prefetch(
    resolution: &content_resolver::ResolvedInstalledContent,
) -> content_resolver::ResolvedInstalledContent {
    let mut cached = resolution.clone();
    cached.update = None;
    cached
}

fn select_curseforge_latest_file_id_prefetch(
    project: &curseforge::Project,
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
) -> Option<u64> {
    let game_version = normalize_optional_for_update_prefetch(game_version);
    let mod_loader_type = if kind == InstalledContentKind::Mods {
        curseforge_loader_type_for_update_prefetch(loader_label)
    } else {
        None
    };

    project
        .latest_files_indexes
        .iter()
        .filter(|index| {
            game_version
                .as_deref()
                .is_none_or(|value| index.game_version.trim() == value)
        })
        .filter(|index| mod_loader_type.is_none_or(|value| index.mod_loader == Some(value)))
        .map(|index| index.file_id)
        .max()
}

fn supports_modrinth_hash_prefetch(kind: InstalledContentKind, path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };

    match kind {
        InstalledContentKind::Mods => extension.eq_ignore_ascii_case("jar"),
        InstalledContentKind::ResourcePacks
        | InstalledContentKind::ShaderPacks
        | InstalledContentKind::DataPacks => extension.eq_ignore_ascii_case("zip"),
    }
}

fn file_name_matches_for_update_prefetch(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    !left.is_empty() && left.eq_ignore_ascii_case(right)
}

fn version_contains_file_name_for_update_prefetch(
    files: &[modrinth::ProjectVersionFile],
    disk_file_name: &str,
) -> bool {
    files
        .iter()
        .any(|file| file_name_matches_for_update_prefetch(file.filename.as_str(), disk_file_name))
}

fn modrinth_loader_slugs_for_update_prefetch(loader: &str) -> Vec<String> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "fabric" => vec!["fabric".to_owned()],
        "forge" => vec!["forge".to_owned()],
        "neoforge" => vec!["neoforge".to_owned()],
        "quilt" => vec!["quilt".to_owned()],
        _ => Vec::new(),
    }
}

fn curseforge_loader_type_for_update_prefetch(loader: &str) -> Option<u32> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

fn normalized_game_versions_for_update_prefetch(game_version: &str) -> Vec<String> {
    normalize_optional_for_update_prefetch(game_version)
        .into_iter()
        .collect()
}

fn normalize_optional_for_update_prefetch(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn non_empty_owned_for_update_prefetch(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn poll_content_apply_results(state: &mut InstanceScreenState, instance_root: &Path) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.content_apply_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_content",
                            "Content-apply worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_content",
                    "Content-apply receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.content_apply_results_tx = None;
        state.content_apply_results_rx = None;
        state.content_apply_in_flight = false;
        install_activity::clear_instance(state.name_input.as_str());
        state.status_message = Some("Content apply worker stopped unexpectedly.".to_owned());
    }

    for result in updates {
        state.content_apply_in_flight = false;
        install_activity::clear_instance(state.name_input.as_str());
        match result.status_message {
            Ok(message) => {
                if result.refresh_all_content {
                    refresh_installed_content_state(state, instance_root);
                } else {
                    state.invalidate_installed_content_cache();
                    refresh_cached_metadata_after_apply(
                        state,
                        instance_root,
                        result.kind,
                        result.focus_lookup_keys.as_slice(),
                    );
                }
                state.status_message = Some(message);
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_content",
                    error = %err,
                    "Applying content changes failed."
                );
                state.status_message = Some(format!("Failed to apply content changes: {err}"));
            }
        }
    }
}

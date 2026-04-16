use super::content_lookup_result::ContentLookupResultEntry;
use super::*;
use content_resolver::detect_installed_content_kind;
use std::collections::{BTreeMap, HashSet};
use textui_egui::truncate_single_line_text_with_ellipsis;
use ui_foundation::tab_button;

#[path = "content_actions.rs"]
mod content_actions;
#[path = "content_updates.rs"]
mod content_updates;

use self::content_actions::{
    poll_content_apply_results, refresh_installed_content_state,
    render_joined_content_browser_controls, request_bulk_content_update, request_content_delete,
    request_content_update, request_local_content_import,
};
use self::content_updates::{
    bulk_update_button_label, bulk_update_button_tooltip, installed_content_lookup_repaint_delay,
    prefetch_bulk_update_metadata, resolve_installed_content_metadata_batch,
    tab_has_known_available_update,
};

const CONTENT_HASH_CACHE_FLUSH_DEBOUNCE: Duration = Duration::from_millis(750);
const CONTENT_LOOKUP_REPAINT_INTERVAL: Duration = Duration::from_millis(100);
const CONTENT_LOOKUP_BATCH_SIZE: usize = 24;
const CONTENT_UPDATE_PREFETCH_BATCH_SIZE: usize = 4;
const CONTENT_UPDATE_LOG_TARGET: &str = "vertexlauncher/content_update";
const INSTALLED_CONTENT_BADGE_FONT_SIZE: f32 = 13.0;
const INSTALLED_CONTENT_BADGE_LINE_HEIGHT: f32 = 16.0;
const INSTALLED_CONTENT_BADGE_PADDING_X: f32 = 6.0;
const INSTALLED_CONTENT_BADGE_PADDING_Y: f32 = 3.0;
const INSTANCE_CONTENT_TAB_ID_KEY: &str = "instance_content_tab_id";

pub(super) fn installed_content_tab_id(
    ctx: &egui::Context,
    kind: InstalledContentKind,
) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new((INSTANCE_CONTENT_TAB_ID_KEY, kind))))
}

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
    poll_content_hash_cache_save_results(state);
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
    render_installed_content_tab_row(ui, text_ui, instance_id, &mut state.selected_content_tab);
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
            &style::muted(ui),
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
            wrap: false,
            ..style::body(ui)
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
            text_ui,
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
    if state.content_hash_cache_save_in_flight {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    flush_content_hash_cache(state, instance_root);
}

fn ensure_content_hash_cache_save_channel(state: &mut InstanceScreenState) {
    if state.content_hash_cache_save_results_tx.is_some()
        && state.content_hash_cache_save_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<(u64, Result<(), String>)>();
    state.content_hash_cache_save_results_tx = Some(tx);
    state.content_hash_cache_save_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn poll_content_hash_cache_save_results(state: &mut InstanceScreenState) {
    let Some(rx) = state.content_hash_cache_save_results_rx.as_ref() else {
        return;
    };
    let Ok(guard) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/instance_content",
            "Content hash-cache save receiver mutex was poisoned while polling save results."
        );
        return;
    };

    while let Ok((saved_serial, result)) = guard.try_recv() {
        state.content_hash_cache_save_in_flight = false;
        match result {
            Ok(()) => {
                if saved_serial == state.content_hash_cache_serial {
                    state.content_hash_cache_dirty = false;
                    state.content_hash_cache_dirty_since = None;
                }
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/instance_content",
                    serial = saved_serial,
                    error = %err,
                    "Failed to save installed-content hash cache."
                );
                if state.content_hash_cache_dirty_since.is_none() {
                    state.content_hash_cache_dirty_since = Some(Instant::now());
                }
            }
        }
    }
}

fn render_installed_content_tab_row(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    active_tab: &mut InstalledContentKind,
) {
    let tabs = InstalledContentKind::ALL.map(|tab| (tab, tab.label()));
    let spacing = 6.0;
    let height = 30.0;
    let width =
        ((ui.available_width() - spacing * (tabs.len() as f32 - 1.0)) / tabs.len() as f32).max(0.0);
    ui.push_id(("instance_content_tab", instance_id), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            for &(tab, label) in &tabs {
                let selected = *active_tab == tab;
                let response = text_ui.selectable_button(
                    ui,
                    ("fill_tab_row", label),
                    label,
                    selected,
                    &tab_button(ui, selected, egui::vec2(width, height)),
                );
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new((INSTANCE_CONTENT_TAB_ID_KEY, tab)),
                        response.id,
                    )
                });
                if response.clicked() {
                    *active_tab = tab;
                }
            }
        });
    });
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
                                                wrap: true,
                                                ..style::stat_label(ui)
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
                                                    wrap: false,
                                                    ..style::body(ui)
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
    text_ui: &mut TextUi,
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
        let label_rect = egui::Rect::from_min_max(
            egui::pos2(icon_rect.max.x + 8.0, rect.top()),
            egui::pos2(rect.right() - 10.0, rect.bottom()),
        );
        let label_style = LabelOptions {
            font_size: 14.0,
            line_height: 18.0,
            color: text_color,
            ..style::body_strong(ui)
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(label_rect), |ui| {
            ui.set_clip_rect(label_rect.intersect(ui.clip_rect()));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let _ = text_ui.label(ui, ("instance_content_bulk_update_label", label), label, &label_style);
            });
        });

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
        .inner_margin(egui::Margin::symmetric(
            INSTALLED_CONTENT_BADGE_PADDING_X as i8,
            INSTALLED_CONTENT_BADGE_PADDING_Y as i8,
        ))
        .show(ui, |ui| {
            let _ = text_ui.label(
                ui,
                id_source,
                label,
                &LabelOptions {
                    font_size: INSTALLED_CONTENT_BADGE_FONT_SIZE,
                    line_height: INSTALLED_CONTENT_BADGE_LINE_HEIGHT,
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
                        font_size: INSTALLED_CONTENT_BADGE_FONT_SIZE,
                        line_height: INSTALLED_CONTENT_BADGE_LINE_HEIGHT,
                        padding: egui::vec2(
                            INSTALLED_CONTENT_BADGE_PADDING_X,
                            INSTALLED_CONTENT_BADGE_PADDING_Y,
                        ),
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
    if !state.content_hash_cache_dirty || state.content_hash_cache_save_in_flight {
        return;
    }
    let Some(dirty_since) = state.content_hash_cache_dirty_since else {
        state.content_hash_cache_dirty_since = Some(Instant::now());
        return;
    };
    if dirty_since.elapsed() < CONTENT_HASH_CACHE_FLUSH_DEBOUNCE {
        return;
    }

    let Some(cache) = state.content_hash_cache.clone() else {
        return;
    };
    ensure_content_hash_cache_save_channel(state);
    let Some(tx) = state.content_hash_cache_save_results_tx.as_ref().cloned() else {
        return;
    };
    let instance_root = instance_root.to_path_buf();
    let serial = state.content_hash_cache_serial;
    state.content_hash_cache_save_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            InstalledContentResolver::save_hash_cache(instance_root.as_path(), &cache)
                .map_err(|err| err.to_string())
        })
        .await;
        let result = match result {
            Ok(result) => result,
            Err(err) => Err(err.to_string()),
        };
        let _ = tx.send((serial, result));
    });
}

fn clear_content_hash_cache(state: &mut InstanceScreenState, instance_root: &Path) {
    state.content_hash_cache = Some(InstalledContentHashCache::default());
    state.content_hash_cache_dirty = false;
    state.content_hash_cache_dirty_since = None;
    state.content_hash_cache_serial = state.content_hash_cache_serial.saturating_add(1);
    state.content_hash_cache_save_in_flight = false;
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

    let truncated_description =
        truncate_single_line_text_with_ellipsis(text_ui, ui, description, max_width, label_options);
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
        let join = tokio_runtime::spawn_blocking({
            let game_version = game_version.clone();
            let loader = loader.clone();
            move || {
                resolve_installed_content_lookup_batch(
                    work_items.as_slice(),
                    kind,
                    game_version.as_str(),
                    loader.as_str(),
                    hash_cache,
                )
            }
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_content",
                    kind = %kind.folder_name(),
                    error = %err,
                    "Content metadata lookup worker panicked."
                );
                return;
            }
        };
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
            state.content_hash_cache_serial = state.content_hash_cache_serial.saturating_add(1);
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

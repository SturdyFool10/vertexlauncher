use super::*;

const CONTENT_HASH_CACHE_FLUSH_DEBOUNCE: Duration = Duration::from_millis(750);

pub(super) fn render_installed_content_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    instance_root: &Path,
    state: &mut InstanceScreenState,
    output: &mut InstanceScreenOutput,
) {
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);
    ensure_content_hash_cache_loaded(state, instance_root);

    let add_button_style = ButtonOptions {
        min_size: egui::vec2((ui.available_width() - 30.0).max(160.0), 34.0),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    };

    ui.horizontal(|ui| {
        if text_ui
            .button(
                ui,
                ("instance_add_content_label", instance_id),
                "Open Content Browser",
                &add_button_style,
            )
            .clicked()
        {
            output.requested_screen = Some(AppScreen::ContentBrowser);
        }

        let plus_button_id = format!("instance_add_content_plus_{instance_id}");
        let add_menu_button = icon_button::svg(
            ui,
            plus_button_id.as_str(),
            assets::PLUS_SVG,
            "Add content options",
            false,
            20.0,
        );

        let popup_id = ui.id().with(("instance_add_content_popup", instance_id));
        let _ = egui::Popup::menu(&add_menu_button)
            .id(popup_id)
            .width(220.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                let popup_button_style = ButtonOptions {
                    min_size: egui::vec2(ui.available_width().max(120.0), style::CONTROL_HEIGHT),
                    text_color: ui.visuals().text_color(),
                    fill: ui.visuals().widgets.inactive.bg_fill,
                    fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                    fill_active: ui.visuals().widgets.active.bg_fill,
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().widgets.inactive.bg_stroke,
                    ..ButtonOptions::default()
                };
                if text_ui
                    .button(
                        ui,
                        ("instance_content_popup_local", instance_id),
                        "Refresh local files",
                        &popup_button_style,
                    )
                    .clicked()
                {
                    state.invalidate_installed_content_cache();
                    state.content_metadata_cache.clear();
                    clear_content_hash_cache(state, instance_root);
                    state.status_message =
                        Some("Refreshed installed content metadata and hash cache.".to_owned());
                }
                if text_ui
                    .button(
                        ui,
                        ("instance_content_popup_mods", instance_id),
                        "Open content browser",
                        &popup_button_style,
                    )
                    .clicked()
                {
                    output.requested_screen = Some(AppScreen::ContentBrowser);
                }
            });
    });

    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for tab in InstalledContentKind::ALL {
            let selected = state.selected_content_tab == tab;
            let tab_style = ButtonOptions {
                min_size: egui::vec2(120.0, 30.0),
                text_color: ui.visuals().text_color(),
                fill: ui.visuals().widgets.inactive.bg_fill,
                fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                fill_active: ui.visuals().widgets.active.bg_fill,
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().widgets.inactive.bg_stroke,
                ..ButtonOptions::default()
            };
            if text_ui
                .selectable_button(
                    ui,
                    ("instance_content_tab", instance_id, tab.label()),
                    tab.label(),
                    selected,
                    &tab_style,
                )
                .clicked()
            {
                state.selected_content_tab = tab;
                state.installed_content_page = 1;
            }
        }
    });

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

    ui.add_space(10.0);
    let _ = text_ui.label(
        ui,
        ("instance_content_resolution_help", instance_id),
        "Vertex identifies Modrinth jars by file hash first. Unresolved files may come from unsupported providers or only match by filename heuristics.",
        &LabelOptions {
            color: ui.visuals().weak_text_color(),
            wrap: true,
            ..LabelOptions::default()
        },
    );
    ui.add_space(8.0);

    let start_index = (state.installed_content_page - 1) * state.installed_content_page_size;
    let end_index = (start_index + state.installed_content_page_size).min(total_items);
    let visible_files = &installed_files[start_index..end_index];
    let selected_game_version = selected_game_version(state).to_owned();
    let selected_modloader = selected_modloader_value(state).to_owned();
    let delete_icon_color = ui.visuals().error_fg_color;
    let delete_button_icon_svg = apply_color_to_svg(assets::TRASH_X_SVG, delete_icon_color);
    let warning_icon_svg = apply_color_to_svg(assets::WARN_SVG, ui.visuals().warn_fg_color);

    let mut pending_delete: Option<(PathBuf, String)> = None;
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
                if !state.content_metadata_cache.contains_key(&entry.lookup_key) {
                    request_content_metadata_lookup(
                        state,
                        entry.lookup_key.as_str(),
                        entry.file_path.as_path(),
                        entry.file_name.as_str(),
                        entry.lookup_query.as_str(),
                        entry.fallback_lookup_key.as_deref(),
                        entry.fallback_lookup_query.as_deref(),
                        entry.managed_identity.as_ref(),
                        state.selected_content_tab,
                        selected_game_version.as_str(),
                        selected_modloader.as_str(),
                    );
                }
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
                } else if rendered.open_clicked {
                    if let Some(metadata) = state
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
        let delete_result = if path.is_dir() {
            std::fs::remove_dir_all(path.as_path())
        } else {
            std::fs::remove_file(path.as_path())
        };
        match delete_result {
            Ok(()) => {
                state.invalidate_installed_content_cache();
                state.content_metadata_cache.remove(&lookup_key);
                state.installed_content_entry_ui_cache.remove(&lookup_key);
                state.status_message = Some("Removed installed content.".to_owned());
            }
            Err(err) => {
                state.status_message = Some(format!("Failed to remove content: {err}"));
            }
        }
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

    let (delete_clicked, open_clicked) = ui
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
                                                render_installed_content_badge(
                                                    ui,
                                                    text_ui,
                                                    (id_source, "update_badge"),
                                                    update,
                                                    ui.visuals().warn_fg_color.gamma_multiply(0.16),
                                                    ui.visuals().warn_fg_color,
                                                );
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

                    delete_clicked
                });

            if side_padding > 0.0 {
                ui.add_space(side_padding);
            }

            (frame_response.inner, frame_response.response.clicked())
        })
        .inner;

    InstalledEntryRenderResult {
        open_clicked: open_clicked && !delete_clicked,
        delete_clicked,
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

    let truncated_description = text_helpers::truncate_single_line_text_with_ellipsis(
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

#[allow(clippy::too_many_arguments)]
fn request_content_metadata_lookup(
    state: &mut InstanceScreenState,
    lookup_key: &str,
    file_path: &Path,
    disk_file_name: &str,
    lookup_query: &str,
    fallback_lookup_key: Option<&str>,
    fallback_lookup_query: Option<&str>,
    managed_identity: Option<&InstalledContentIdentity>,
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
) {
    if lookup_key.trim().is_empty()
        || state.content_lookup_in_flight.contains(lookup_key)
        || state.content_metadata_cache.contains_key(lookup_key)
    {
        return;
    }

    ensure_content_lookup_channel(state);
    let Some(tx) = state.content_lookup_results_tx.as_ref().cloned() else {
        return;
    };

    let key_for_state = lookup_key.to_owned();
    state.content_lookup_in_flight.insert(key_for_state.clone());
    let lookup_key = key_for_state;
    let request = ResolveInstalledContentRequest {
        file_path: file_path.to_path_buf(),
        disk_file_name: disk_file_name.trim().to_owned(),
        lookup_query: lookup_query.trim().to_owned(),
        fallback_lookup_key: fallback_lookup_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        fallback_lookup_query: fallback_lookup_query
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        managed_identity: managed_identity.cloned(),
        kind,
        game_version: game_version.trim().to_owned(),
        loader: loader.trim().to_owned(),
    };
    let hash_cache = state.content_hash_cache.clone().unwrap_or_default();

    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            InstalledContentResolver::resolve(&request, &hash_cache)
        })
        .await
        .ok();
        if let Some(result) = result {
            let _ = tx.send(ContentLookupResult {
                lookup_key,
                resolution: result.resolution,
                hash_cache_updates: result.hash_cache_updates,
            });
        }
    });
}

pub(super) fn poll_content_lookup_results(state: &mut InstanceScreenState) {
    let Some(rx) = state.content_lookup_results_rx.as_ref() else {
        return;
    };
    let Ok(guard) = rx.lock() else {
        return;
    };

    while let Ok(result) = guard.try_recv() {
        state
            .content_lookup_in_flight
            .remove(result.lookup_key.as_str());
        let cache = state
            .content_hash_cache
            .get_or_insert_with(InstalledContentHashCache::default);
        if cache.apply_updates(result.hash_cache_updates) {
            state.content_hash_cache_dirty = true;
            state.content_hash_cache_dirty_since = Some(Instant::now());
        }
        state
            .content_metadata_cache
            .insert(result.lookup_key, result.resolution);
    }
}

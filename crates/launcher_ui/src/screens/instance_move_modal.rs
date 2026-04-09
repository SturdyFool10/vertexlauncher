use super::*;

fn validate_move_dest(path_str: &str) -> Result<PathBuf, String> {
    let path_str = path_str.trim();
    if path_str.is_empty() {
        return Err("Please enter or browse to a destination folder.".to_owned());
    }
    let path = PathBuf::from(path_str);
    if path.is_dir() {
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
        format!("{bytes} B")
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

pub(super) fn render_move_instance_modal(
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
                        if browse_clicked
                            && let Some(picked) = rfd::FileDialog::new()
                                .set_title("Choose Destination Folder")
                                .pick_folder()
                        {
                            state.move_instance_dest_input = picked.display().to_string();
                            return true;
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
                    LabelOptions {
                        color: ui.visuals().error_fg_color,
                        ..style::stat_label(ui)
                    }
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
                        &if state.move_instance_completion_failed {
                            style::error_text(ui)
                        } else {
                            style::body(ui)
                        },
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

    if move_requested && let Some(instance) = instances.find(instance_id) {
        let installations_root = config.minecraft_installations_root_path().to_path_buf();
        let source_root = instances::instance_root_path(&installations_root, instance);
        let dest_root = PathBuf::from(state.move_instance_dest_input.trim());
        state.move_instance_dest_path = Some(dest_root.clone());
        request_move_instance(state, source_root, dest_root);
        state.show_move_instance_modal = true;
        state.show_move_instance_progress_modal = false;
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

pub(super) fn render_move_instance_progress_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
) {
    let _ = (ctx, text_ui, instance_id, state);
}

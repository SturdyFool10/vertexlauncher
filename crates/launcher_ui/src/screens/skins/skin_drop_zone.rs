use super::*;

pub(super) fn render_skin_drop_zone(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
) {
    let width = ui
        .available_width()
        .min(ui.clip_rect().width().max(1.0))
        .max(1.0);
    let height = style::CONTROL_HEIGHT_LG * 3.4;
    let drop_zone_id = ui.make_persistent_id("skins_drop_zone");
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let response = ui.interact(rect, drop_zone_id, Sense::click());
    let hovered_files = ui.input(|input| input.raw.hovered_files.clone());
    let dropped_files = ui.input(|input| input.raw.dropped_files.clone());
    let hovering_drop = !hovered_files.is_empty()
        && ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|pointer| rect.contains(pointer));
    let received_drop = !dropped_files.is_empty()
        && ui
            .ctx()
            .pointer_latest_pos()
            .is_some_and(|pointer| rect.contains(pointer));
    let focused = response.has_focus();
    let pressed = response.is_pointer_button_down_on();
    let fill = if hovering_drop {
        ui.visuals().selection.bg_fill.gamma_multiply(0.22)
    } else if pressed {
        ui.visuals().widgets.active.bg_fill.gamma_multiply(0.95)
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.92)
    } else if focused {
        ui.visuals().selection.bg_fill.gamma_multiply(0.12)
    } else {
        ui.visuals()
            .widgets
            .inactive
            .weak_bg_fill
            .gamma_multiply(0.7)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(14), fill);
    paint_dotted_drop_zone_stroke(
        ui,
        rect.shrink(1.5),
        if hovering_drop || focused {
            ui.visuals().selection.stroke.color
        } else if response.hovered() {
            ui.visuals().widgets.hovered.bg_stroke.color
        } else {
            ui.visuals().weak_text_color()
        },
    );
    if focused {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(16),
            Stroke::new(
                (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }

    let mut choose_style =
        style::neutral_button_with_min_size(ui, egui::vec2(220.0, style::CONTROL_HEIGHT));
    choose_style.fill = ui.visuals().widgets.inactive.bg_fill;
    choose_style.fill_hovered = ui.visuals().widgets.hovered.bg_fill;
    choose_style.fill_active = ui.visuals().widgets.active.bg_fill;
    choose_style.fill_selected = ui.visuals().selection.bg_fill.gamma_multiply(0.7);
    let content_rect = rect.shrink2(egui::vec2(18.0, 18.0));
    let title_style = style::section_heading(ui);
    let muted = style::muted(ui);
    let button_label_style = LabelOptions {
        font_size: choose_style.font_size,
        line_height: choose_style.line_height,
        color: choose_style.text_color,
        wrap: false,
        ..style::body(ui)
    };
    let title_size = text_ui.measure_text_size(ui, "Drag Skin Image here", &title_style);
    let or_size = text_ui.measure_text_size(ui, "or", &muted);
    let button_text_size = text_ui.measure_text_size(ui, "Choose Skin Image", &button_label_style);
    let button_size = egui::vec2(
        (button_text_size.x + choose_style.padding.x * 2.0).max(choose_style.min_size.x),
        (button_text_size.y + choose_style.padding.y * 2.0).max(choose_style.min_size.y),
    );
    let gap = style::SPACE_XS;
    let total_height = title_size.y + gap + or_size.y + gap + button_size.y;
    let mut current_y = content_rect.center().y - total_height * 0.5;

    let title_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.center().x - title_size.x * 0.5, current_y),
        title_size,
    );
    current_y += title_size.y + gap;
    let or_width = (or_size.x + 8.0).min(content_rect.width());
    let or_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.center().x - or_width * 0.5, current_y),
        egui::vec2(or_width, or_size.y),
    );
    current_y += or_size.y + gap;
    let button_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.center().x - button_size.x * 0.5, current_y),
        button_size,
    );
    let button_text_rect = egui::Rect::from_min_size(
        egui::pos2(
            button_rect.center().x - button_text_size.x * 0.5,
            button_rect.center().y - button_text_size.y * 0.5,
        ),
        button_text_size,
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(title_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let _ = text_ui.label(
                ui,
                "skins_drop_prompt",
                "Drag Skin Image here",
                &title_style,
            );
        },
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(or_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let _ = text_ui.label(ui, "skins_drop_prompt_or", "or", &muted);
        },
    );
    let button_fill = if pressed {
        choose_style.fill_active
    } else if response.hovered() {
        choose_style.fill_hovered
    } else if focused {
        choose_style.fill_selected
    } else {
        choose_style.fill
    };
    let button_stroke = if focused {
        ui.visuals().selection.stroke
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_stroke
    } else {
        choose_style.stroke
    };
    ui.painter().rect_filled(
        button_rect,
        CornerRadius::same(choose_style.corner_radius),
        button_fill,
    );
    ui.painter().rect_stroke(
        button_rect,
        CornerRadius::same(choose_style.corner_radius),
        button_stroke,
        egui::StrokeKind::Inside,
    );
    if focused {
        ui.painter().rect_stroke(
            button_rect.expand(2.0),
            CornerRadius::same(choose_style.corner_radius.saturating_add(2)),
            Stroke::new(
                (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(button_text_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let _ = text_ui.label(
                ui,
                "skins_pick_file_visual",
                "Choose Skin Image",
                &button_label_style,
            );
        },
    );
    if response.clicked() && !state.pick_skin_in_progress && !received_drop {
        state.pick_skin_file();
    }

    if received_drop && !state.pick_skin_in_progress {
        if let Some(file) = dropped_files.into_iter().next() {
            if let Some(path) = file.path {
                state.begin_loading_skin_from_path(path);
            } else if let Some(bytes) = file.bytes {
                let name = if file.name.trim().is_empty() {
                    "Dropped skin.png".to_owned()
                } else {
                    file.name
                };
                state.begin_loading_skin_from_bytes(PathBuf::from(name), bytes.as_ref().to_vec());
            } else {
                notification::error!(
                    "skin_manager",
                    "Dropped file did not include readable image data."
                );
            }
        }
    }
}

fn paint_dotted_drop_zone_stroke(ui: &Ui, rect: Rect, color: Color32) {
    let dash_step = 10.0;
    let dash_len = 4.0;
    let stroke = Stroke::new(1.5, color);

    let mut x = rect.left();
    while x <= rect.right() {
        ui.painter().line_segment(
            [
                egui::pos2(x, rect.top()),
                egui::pos2((x + dash_len).min(rect.right()), rect.top()),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(x, rect.bottom()),
                egui::pos2((x + dash_len).min(rect.right()), rect.bottom()),
            ],
            stroke,
        );
        x += dash_step;
    }

    let mut y = rect.top();
    while y <= rect.bottom() {
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), y),
                egui::pos2(rect.left(), (y + dash_len).min(rect.bottom())),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(rect.right(), y),
                egui::pos2(rect.right(), (y + dash_len).min(rect.bottom())),
            ],
            stroke,
        );
        y += dash_step;
    }
}

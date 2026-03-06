use std::hash::Hash;

use egui::{self, Align, Layout, Response, Sense, Ui};
use textui::{ButtonOptions, InputOptions, LabelOptions, TextUi, TooltipOptions};

use crate::{assets, ui::components::icon_button};

#[derive(Clone, Copy, Debug)]
struct ControlMetrics {
    right_padding: f32,
    control_height: f32,
    switch_width: f32,
    dropdown_width: f32,
    number_input_width: f32,
    icon_size: f32,
    control_gap: f32,
}

#[derive(Clone, Debug)]
struct FloatInputState {
    text: String,
    last_valid: f32,
}

#[derive(Clone, Debug)]
struct IntInputState {
    text: String,
    last_valid: i32,
}

pub fn toggle_row(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    label: &str,
    info_tooltip: Option<&str>,
    value: &mut bool,
) -> Response {
    let metrics = control_metrics(ui);
    let label_options = row_label_options(ui);

    ui.horizontal(|ui| {
        let mut label_response =
            text_ui.clickable_label(ui, ("toggle_label", label), label, &label_options);
        if label_response.clicked() {
            *value = !*value;
            label_response.mark_changed();
        }

        if info_tooltip.is_some() {
            ui.add_space(6.0);
            info_hint(text_ui, ui, ("toggle_info", label), info_tooltip);
        }

        let switch_response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(metrics.right_padding);
                switch(ui, value, metrics)
            })
            .inner;

        switch_response.union(label_response)
    })
    .inner
}

pub fn dropdown_row(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    label: &str,
    info_tooltip: Option<&str>,
    selected_index: &mut usize,
    options: &[&str],
) -> Response {
    let metrics = control_metrics(ui);
    let label_options = row_label_options(ui);

    ui.horizontal(|ui| {
        let label_response = text_ui.label(ui, ("dropdown_label", label), label, &label_options);

        if info_tooltip.is_some() {
            ui.add_space(6.0);
            info_hint(text_ui, ui, ("dropdown_info", label), info_tooltip);
        }

        let dropdown_response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(metrics.right_padding);
                ui.push_id(id_source, |ui| {
                    dropdown(text_ui, ui, selected_index, options, metrics)
                })
                .inner
            })
            .inner;

        dropdown_response.union(label_response)
    })
    .inner
}

pub fn float_stepper_row(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    label: &str,
    info_tooltip: Option<&str>,
    value: &mut f32,
    min: f32,
    max: f32,
    step: f32,
) -> Response {
    let metrics = control_metrics(ui);
    let id = ui.make_persistent_id(id_source);
    let input_id = id.with("float_input");
    let label_options = row_label_options(ui);

    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<FloatInputState>(id))
        .unwrap_or(FloatInputState {
            text: format_float(*value),
            last_valid: *value,
        });

    let row_response = ui
        .horizontal(|ui| {
            let label_response = text_ui.label(ui, ("float_label", label), label, &label_options);

            if info_tooltip.is_some() {
                ui.add_space(6.0);
                info_hint(text_ui, ui, ("float_info", label), info_tooltip);
            }

            let (controls_response, text_response, plus_clicked, minus_clicked) = ui
                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(metrics.right_padding);

                    let plus_response =
                        step_button(ui, "float_plus", assets::PLUS_SVG, "Increase", metrics);
                    ui.add_space(metrics.control_gap);

                    let mut input_options = number_input_options(ui, metrics);
                    input_options.desired_width = Some(metrics.number_input_width);
                    let text_response =
                        text_ui.singleline_input(ui, input_id, &mut state.text, &input_options);
                    ui.add_space(metrics.control_gap);

                    let minus_response =
                        step_button(ui, "float_minus", assets::MINUS_SVG, "Decrease", metrics);

                    let merged = plus_response
                        .clone()
                        .union(text_response.clone())
                        .union(minus_response.clone());

                    (
                        merged,
                        text_response,
                        plus_response.clicked(),
                        minus_response.clicked(),
                    )
                })
                .inner;

            sanitize_float_text(&mut state.text, min < 0.0);

            if let Some(parsed) = parse_float_text(&state.text) {
                if parsed >= min && parsed <= max {
                    *value = parsed;
                    state.last_valid = parsed;
                }
            }

            if plus_clicked {
                *value = (*value + step).clamp(min, max);
                state.last_valid = *value;
                state.text = format_float(*value);
            } else if minus_clicked {
                *value = (*value - step).clamp(min, max);
                state.last_valid = *value;
                state.text = format_float(*value);
            }

            if text_response.lost_focus() {
                if let Some(parsed) = parse_float_text(&state.text) {
                    if parsed >= min && parsed <= max {
                        *value = parsed;
                        state.last_valid = parsed;
                        state.text = format_float(parsed);
                    } else {
                        state.text = format_float(state.last_valid);
                    }
                } else {
                    state.text = format_float(state.last_valid);
                }
            }

            if !text_response.has_focus() {
                state.last_valid = *value;
                state.text = format_float(*value);
            }

            controls_response.union(label_response)
        })
        .inner;

    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    row_response
}

pub fn int_stepper_row(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    label: &str,
    info_tooltip: Option<&str>,
    value: &mut i32,
    min: i32,
    max: i32,
    step: i32,
) -> Response {
    let metrics = control_metrics(ui);
    let id = ui.make_persistent_id(id_source);
    let input_id = id.with("int_input");
    let label_options = row_label_options(ui);

    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<IntInputState>(id))
        .unwrap_or(IntInputState {
            text: value.to_string(),
            last_valid: *value,
        });

    let row_response = ui
        .horizontal(|ui| {
            let label_response = text_ui.label(ui, ("int_label", label), label, &label_options);

            if info_tooltip.is_some() {
                ui.add_space(6.0);
                info_hint(text_ui, ui, ("int_info", label), info_tooltip);
            }

            let (controls_response, text_response, plus_clicked, minus_clicked) = ui
                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(metrics.right_padding);

                    let plus_response =
                        step_button(ui, "int_plus", assets::PLUS_SVG, "Increase", metrics);
                    ui.add_space(metrics.control_gap);

                    let mut input_options = number_input_options(ui, metrics);
                    input_options.desired_width = Some(metrics.number_input_width);
                    let text_response =
                        text_ui.singleline_input(ui, input_id, &mut state.text, &input_options);
                    ui.add_space(metrics.control_gap);

                    let minus_response =
                        step_button(ui, "int_minus", assets::MINUS_SVG, "Decrease", metrics);

                    let merged = plus_response
                        .clone()
                        .union(text_response.clone())
                        .union(minus_response.clone());

                    (
                        merged,
                        text_response,
                        plus_response.clicked(),
                        minus_response.clicked(),
                    )
                })
                .inner;

            sanitize_int_text(&mut state.text, min < 0);

            if let Some(parsed) = parse_int_text(&state.text) {
                if parsed >= min && parsed <= max {
                    *value = parsed;
                    state.last_valid = parsed;
                }
            }

            if plus_clicked {
                *value = (*value + step).clamp(min, max);
                state.last_valid = *value;
                state.text = value.to_string();
            } else if minus_clicked {
                *value = (*value - step).clamp(min, max);
                state.last_valid = *value;
                state.text = value.to_string();
            }

            if text_response.lost_focus() {
                if let Some(parsed) = parse_int_text(&state.text) {
                    if parsed >= min && parsed <= max {
                        *value = parsed;
                        state.last_valid = parsed;
                        state.text = parsed.to_string();
                    } else {
                        state.text = state.last_valid.to_string();
                    }
                } else {
                    state.text = state.last_valid.to_string();
                }
            }

            if !text_response.has_focus() {
                state.last_valid = *value;
                state.text = value.to_string();
            }

            controls_response.union(label_response)
        })
        .inner;

    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    row_response
}

pub fn text_input_row(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    label: &str,
    info_tooltip: Option<&str>,
    value: &mut String,
) -> Response {
    let metrics = control_metrics(ui);
    let label_options = row_label_options(ui);
    let input_id = ui.make_persistent_id(id_source).with("text_input");

    ui.horizontal(|ui| {
        let label_response = text_ui.label(ui, ("text_label", label), label, &label_options);

        if info_tooltip.is_some() {
            ui.add_space(6.0);
            info_hint(text_ui, ui, ("text_info", label), info_tooltip);
        }

        let text_response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(metrics.right_padding);
                let mut input_options = text_input_options(ui, metrics);
                input_options.desired_width = Some(metrics.dropdown_width);
                input_options.min_width = metrics.dropdown_width;
                text_ui.singleline_input(ui, input_id, value, &input_options)
            })
            .inner;

        text_response.union(label_response)
    })
    .inner
}

pub fn info_hint(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    tooltip: Option<&str>,
) -> Response {
    let metrics = control_metrics(ui);
    let icon = themed_svg_image(
        "settings-info-circle",
        assets::INFO_CIRCLE_SVG,
        metrics.icon_size,
        ui.visuals().weak_text_color(),
    )
    .sense(Sense::hover())
    .fit_to_exact_size(egui::vec2(metrics.icon_size, metrics.icon_size));

    let response = ui.add(icon);
    if let Some(text) = tooltip {
        let mut tooltip_options = TooltipOptions::default();
        tooltip_options.text.color = ui.visuals().text_color();
        tooltip_options.background = ui.visuals().widgets.noninteractive.bg_fill;
        tooltip_options.stroke = ui.visuals().widgets.noninteractive.bg_stroke;
        text_ui.tooltip_for_response(ui, id_source, &response, text, &tooltip_options);
    }
    response
}

fn switch(ui: &mut Ui, value: &mut bool, metrics: ControlMetrics) -> Response {
    let desired_size = egui::vec2(metrics.switch_width, metrics.control_height);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click());

    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }

    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, "")
    });

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *value);
        let off_bg = ui.visuals().widgets.inactive.bg_fill;
        let on_bg = ui.visuals().selection.bg_fill;
        let bg_fill: egui::Color32 =
            egui::lerp(egui::Rgba::from(off_bg)..=egui::Rgba::from(on_bg), how_on).into();
        let bg_stroke = ui.visuals().widgets.inactive.bg_stroke;
        let corner_radius = rect.height() / 2.0;
        ui.painter().rect(
            rect,
            corner_radius,
            bg_fill,
            bg_stroke,
            egui::StrokeKind::Inside,
        );

        let knob_margin = (metrics.control_height * 0.10).clamp(2.0, 4.0);
        let knob_radius = (rect.height() - (knob_margin * 2.0)) / 2.0;
        let knob_x = egui::lerp(
            (rect.left() + knob_margin + knob_radius)..=(rect.right() - knob_margin - knob_radius),
            how_on,
        );
        let knob_center = egui::pos2(knob_x, rect.center().y);
        let knob_fill = ui.visuals().widgets.noninteractive.fg_stroke.color;
        ui.painter().circle(
            knob_center,
            knob_radius,
            knob_fill,
            egui::Stroke::new(1.0, bg_stroke.color),
        );
    }

    response
}

fn step_button(
    ui: &mut Ui,
    icon_id: &str,
    icon_bytes: &'static [u8],
    tooltip: &str,
    metrics: ControlMetrics,
) -> Response {
    icon_button::svg(
        ui,
        icon_id,
        icon_bytes,
        tooltip,
        false,
        metrics.control_height,
    )
}

fn dropdown(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    selected_index: &mut usize,
    options: &[&str],
    metrics: ControlMetrics,
) -> Response {
    let open_id = ui.id().with("settings_dropdown_open");

    let mut label_style = row_label_options(ui);
    label_style.wrap = false;

    let selected_text_raw = options.get(*selected_index).copied().unwrap_or("Select...");
    let selected_text = truncate_button_text_with_ellipsis(
        text_ui,
        ui,
        selected_text_raw,
        dropdown_text_budget(metrics),
        &label_style,
    );

    let (button_rect, mut response) = ui.allocate_exact_size(
        egui::vec2(metrics.dropdown_width, metrics.control_height),
        Sense::click(),
    );

    let mut interacted = ui.style().interact(&response);
    let mut text_color = interacted.text_color();

    let popup_response = egui::Popup::menu(&response)
        .id(open_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(metrics.dropdown_width);

            let mut popup_changed = false;
            let button_options = ButtonOptions {
                min_size: egui::vec2(metrics.dropdown_width - 4.0, metrics.control_height),
                corner_radius: 4,
                padding: egui::vec2(8.0, 4.0),
                text_color: ui.visuals().text_color(),
                fill: ui.visuals().widgets.inactive.bg_fill,
                fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                fill_active: ui.visuals().widgets.active.bg_fill,
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().widgets.inactive.bg_stroke,
                ..ButtonOptions::default()
            };

            for (index, option) in options.iter().enumerate() {
                let option_text = truncate_button_text_with_ellipsis(
                    text_ui,
                    ui,
                    option,
                    dropdown_text_budget(metrics),
                    &label_style,
                );
                let option_response = text_ui.selectable_button(
                    ui,
                    ("dropdown_option", index),
                    &option_text,
                    *selected_index == index,
                    &button_options,
                );
                if option_response.clicked() {
                    *selected_index = index;
                    popup_changed = true;
                    egui::Popup::close_id(ui.ctx(), open_id);
                }
            }

            popup_changed
        });

    let is_open = egui::Popup::is_id_open(ui.ctx(), open_id);
    if is_open {
        interacted = &ui.visuals().widgets.open;
        text_color = interacted.text_color();
    }

    ui.painter().rect(
        button_rect,
        6.0,
        interacted.bg_fill,
        interacted.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let icon_bytes = if is_open {
        assets::CHEVRON_UP_SVG
    } else {
        assets::CHEVRON_DOWN_SVG
    };
    let icon = themed_svg_image(
        "settings-dropdown-chevron",
        icon_bytes,
        metrics.icon_size,
        text_color,
    )
    .fit_to_exact_size(egui::vec2(metrics.icon_size, metrics.icon_size));
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(
            button_rect.right() - metrics.icon_size - 8.0,
            button_rect.center().y - metrics.icon_size * 0.5,
        ),
        egui::vec2(metrics.icon_size, metrics.icon_size),
    );
    ui.put(icon_rect, icon);

    label_style.color = text_color;
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(button_rect.left() + 8.0, button_rect.top()),
        egui::pos2(icon_rect.left() - 6.0, button_rect.bottom()),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
        ui.set_clip_rect(text_rect);
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            let _ = text_ui.label(ui, "dropdown_selected_text", &selected_text, &label_style);
        });
    });
    if popup_response
        .as_ref()
        .map(|inner| inner.inner)
        .unwrap_or(false)
    {
        response.mark_changed();
    }

    response
}

fn sanitize_float_text(text: &mut String, allow_negative: bool) {
    if text.is_empty() {
        return;
    }

    let mut out = String::with_capacity(text.len());
    let mut seen_dot = false;
    let mut seen_sign = false;

    for (index, ch) in text.chars().enumerate() {
        if ch.is_ascii_digit() {
            out.push(ch);
            continue;
        }

        if ch == '.' && !seen_dot {
            seen_dot = true;
            out.push(ch);
            continue;
        }

        if allow_negative && ch == '-' && index == 0 && !seen_sign {
            seen_sign = true;
            out.push(ch);
        }
    }

    *text = out;
}

fn sanitize_int_text(text: &mut String, allow_negative: bool) {
    if text.is_empty() {
        return;
    }

    let mut out = String::with_capacity(text.len());
    let mut seen_sign = false;

    for (index, ch) in text.chars().enumerate() {
        if ch.is_ascii_digit() {
            out.push(ch);
            continue;
        }

        if allow_negative && ch == '-' && index == 0 && !seen_sign {
            seen_sign = true;
            out.push(ch);
        }
    }

    *text = out;
}

fn parse_float_text(text: &str) -> Option<f32> {
    if text.is_empty() || text == "-" || text == "." || text == "-." {
        None
    } else {
        text.parse::<f32>().ok()
    }
}

fn parse_int_text(text: &str) -> Option<i32> {
    if text.is_empty() || text == "-" {
        None
    } else {
        text.parse::<i32>().ok()
    }
}

fn format_float(value: f32) -> String {
    let mut formatted = format!("{value:.3}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

fn row_label_options(ui: &Ui) -> LabelOptions {
    LabelOptions {
        font_size: 18.0,
        line_height: 24.0,
        color: ui.visuals().text_color(),
        wrap: false,
        ..LabelOptions::default()
    }
}

fn number_input_options(ui: &Ui, metrics: ControlMetrics) -> InputOptions {
    let selection = ui.visuals().selection.bg_fill;
    InputOptions {
        font_size: 17.0,
        line_height: 22.0,
        text_color: ui.visuals().text_color(),
        cursor_color: ui.visuals().text_cursor.stroke.color,
        selection_color: egui::Color32::from_rgba_premultiplied(
            selection.r(),
            selection.g(),
            selection.b(),
            92,
        ),
        selected_text_color: ui.visuals().text_color(),
        background_color: ui.visuals().widgets.inactive.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        desired_rows: 1,
        desired_width: Some(metrics.number_input_width),
        min_width: metrics.number_input_width,
        monospace: true,
        padding: egui::vec2(6.0, 4.0),
        ..InputOptions::default()
    }
}

fn text_input_options(ui: &Ui, metrics: ControlMetrics) -> InputOptions {
    let selection = ui.visuals().selection.bg_fill;
    InputOptions {
        font_size: 17.0,
        line_height: 22.0,
        text_color: ui.visuals().text_color(),
        cursor_color: ui.visuals().text_cursor.stroke.color,
        selection_color: egui::Color32::from_rgba_premultiplied(
            selection.r(),
            selection.g(),
            selection.b(),
            92,
        ),
        selected_text_color: ui.visuals().text_color(),
        background_color: ui.visuals().widgets.inactive.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        desired_rows: 1,
        desired_width: Some(metrics.dropdown_width),
        min_width: metrics.dropdown_width,
        monospace: false,
        padding: egui::vec2(6.0, 4.0),
        ..InputOptions::default()
    }
}

fn control_metrics(ui: &Ui) -> ControlMetrics {
    let viewport_width = ui.ctx().input(|i| i.content_rect().width()).max(320.0);
    let text_height = ui.text_style_height(&egui::TextStyle::Body).max(14.0);
    let control_height = (viewport_width * 0.024).clamp(22.0, 34.0);
    let control_gap = (control_height * 0.20).clamp(4.0, 8.0);
    let number_input_width = (viewport_width * 0.10).clamp(84.0, 150.0);
    let step_button_width = control_height;
    let number_selector_width =
        number_input_width + (step_button_width * 2.0) + (control_gap * 2.0);

    ControlMetrics {
        right_padding: (viewport_width * 0.01).clamp(8.0, 20.0),
        control_height,
        switch_width: (control_height * 1.95).clamp(42.0, 72.0),
        dropdown_width: number_selector_width,
        number_input_width,
        icon_size: text_height.clamp(14.0, 20.0),
        control_gap,
    }
}

fn dropdown_text_budget(metrics: ControlMetrics) -> f32 {
    let left_padding = 8.0;
    let right_padding = 8.0;
    let icon_gap = 6.0;
    (metrics.dropdown_width - metrics.icon_size - left_padding - right_padding - icon_gap).max(0.0)
}

fn truncate_button_text_with_ellipsis(
    text_ui: &mut TextUi,
    ui: &Ui,
    text: &str,
    max_width: f32,
    label_options: &LabelOptions,
) -> String {
    if text.is_empty() {
        return String::new();
    }

    if max_width <= 0.0 {
        return "...".to_owned();
    }

    let mut measure_width =
        |candidate: &str| -> f32 { text_ui.measure_text_size(ui, candidate, label_options).x };

    if measure_width(text) <= max_width {
        return text.to_owned();
    }

    let ellipsis = "...";
    if measure_width(ellipsis) > max_width {
        return String::new();
    }

    let mut cutoff = 0usize;
    for (index, _) in text
        .char_indices()
        .skip(1)
        .chain(std::iter::once((text.len(), '\0')))
    {
        let candidate = format!("{}{}", &text[..index], ellipsis);
        if measure_width(&candidate) <= max_width {
            cutoff = index;
        } else {
            break;
        }
    }

    if cutoff == 0 {
        ellipsis.to_owned()
    } else {
        format!("{}{}", &text[..cutoff], ellipsis)
    }
}

fn themed_svg_image(
    icon_id: &str,
    svg_bytes: &[u8],
    icon_size: f32,
    color: egui::Color32,
) -> egui::Image<'static> {
    let themed_svg = apply_svg_color(svg_bytes, color);
    let uri = format!(
        "bytes://vertex-settings-icons/{icon_id}-{:02x}{:02x}{:02x}.svg",
        color.r(),
        color.g(),
        color.b()
    );
    egui::Image::from_bytes(uri, themed_svg).fit_to_exact_size(egui::vec2(icon_size, icon_size))
}

fn apply_svg_color(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    String::from_utf8_lossy(svg_bytes)
        .replace("currentColor", &color_hex)
        .into_bytes()
}

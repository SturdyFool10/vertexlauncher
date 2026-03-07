use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use egui::Ui;
use instances::{InstanceRecord, InstanceStore};
use textui::{LabelOptions, TextUi};

use crate::{assets, ui::style};

const TILE_WIDTH: f32 = 300.0;
const TILE_THUMBNAIL_HEIGHT: f32 = 150.0;

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    instances: &InstanceStore,
) {
    if instances.instances.is_empty() {
        let _ = text_ui.label(
            ui,
            "library_empty_profiles",
            "No instances created yet.",
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("library_instance_tiles_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_MD, style::SPACE_MD);
                for instance in &instances.instances {
                    render_instance_tile(
                        ui,
                        text_ui,
                        instance,
                        selected_instance_id == Some(instance.id.as_str()),
                    );
                }
            });
        });
}

fn render_instance_tile(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance: &InstanceRecord,
    selected: bool,
) {
    let tile_fill = if selected {
        ui.visuals().selection.bg_fill.gamma_multiply(0.22)
    } else {
        ui.visuals().widgets.noninteractive.bg_fill
    };
    let tile_stroke = if selected {
        ui.visuals().selection.stroke
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };

    let frame = egui::Frame::new()
        .fill(tile_fill)
        .stroke(tile_stroke)
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(10));

    frame.show(ui, |ui| {
        ui.set_min_width(TILE_WIDTH);
        ui.set_max_width(TILE_WIDTH);
        ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_XS, style::SPACE_XS);
        ui.vertical(|ui| {
            render_instance_thumbnail(ui, instance);
            ui.add_space(style::SPACE_SM);

            let _ = text_ui.label(
                ui,
                ("library_instance_name", instance.id.as_str()),
                instance.name.as_str(),
                &LabelOptions {
                    font_size: 22.0,
                    line_height: 28.0,
                    weight: 700,
                    color: ui.visuals().text_color(),
                    wrap: true,
                    ..LabelOptions::default()
                },
            );
            ui.add_space(2.0);

            let _ = text_ui.label(
                ui,
                ("library_instance_version", instance.id.as_str()),
                &format!("Version: {}", instance.game_version),
                &LabelOptions {
                    color: ui.visuals().text_color(),
                    wrap: true,
                    ..LabelOptions::default()
                },
            );
            let _ = text_ui.label(
                ui,
                ("library_instance_modloader", instance.id.as_str()),
                &format!("Modloader: {}", instance.modloader),
                &LabelOptions {
                    color: ui.visuals().text_color(),
                    wrap: true,
                    ..LabelOptions::default()
                },
            );

            ui.add_space(6.0);
            let (description, muted) = if let Some(description) = instance
                .description
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                (description, false)
            } else {
                ("No description provided.", true)
            };
            let _ = text_ui.label(
                ui,
                ("library_instance_description", instance.id.as_str()),
                description,
                &LabelOptions {
                    color: if muted {
                        ui.visuals().weak_text_color()
                    } else {
                        ui.visuals().text_color()
                    },
                    wrap: true,
                    ..LabelOptions::default()
                },
            );
        });
    });
}

fn render_instance_thumbnail(ui: &mut Ui, instance: &InstanceRecord) {
    let thumbnail_width = (TILE_WIDTH - 20.0).max(120.0);
    let thumbnail_size = egui::vec2(thumbnail_width, TILE_THUMBNAIL_HEIGHT);

    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(6));

    frame.show(ui, |ui| {
        if let Some(path) = instance
            .thumbnail_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && let Ok(bytes) = std::fs::read(path)
        {
            let mut hasher = DefaultHasher::new();
            instance.id.hash(&mut hasher);
            path.hash(&mut hasher);
            let uri = format!(
                "bytes://library/instance-thumbnail/{:016x}",
                hasher.finish()
            );
            ui.add(egui::Image::from_bytes(uri, bytes).fit_to_exact_size(thumbnail_size));
            return;
        }

        let placeholder_size = egui::vec2(42.0, 42.0);
        let placeholder = egui::Image::from_bytes(
            format!("bytes://library/instance-thumbnail-default/{}", instance.id),
            assets::LIBRARY_SVG,
        )
        .fit_to_exact_size(placeholder_size);
        let (rect, _) = ui.allocate_exact_size(thumbnail_size, egui::Sense::hover());
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(6),
            ui.visuals().faint_bg_color,
        );
        let icon_rect = egui::Rect::from_center_size(rect.center(), placeholder_size);
        ui.put(icon_rect, placeholder);
    });
}

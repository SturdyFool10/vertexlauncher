use egui::{Color32, Ui, Vec2};
use textui_egui::prelude::*;

pub fn primary_button(ui: &Ui, min_size: Vec2) -> ButtonOptions {
    ButtonOptions {
        min_size,
        text_color: ui.visuals().widgets.active.fg_stroke.color,
        fill: ui.visuals().selection.bg_fill,
        fill_hovered: ui.visuals().selection.bg_fill.gamma_multiply(1.08),
        fill_active: ui.visuals().selection.bg_fill.gamma_multiply(0.92),
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().selection.stroke,
        ..ButtonOptions::default()
    }
}

pub fn secondary_button(ui: &Ui, min_size: Vec2) -> ButtonOptions {
    ButtonOptions {
        min_size,
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    }
}

pub fn danger_button(ui: &Ui, min_size: Vec2) -> ButtonOptions {
    let danger = ui.visuals().error_fg_color;
    ButtonOptions {
        min_size,
        text_color: Color32::WHITE,
        fill: danger.gamma_multiply(0.84),
        fill_hovered: danger,
        fill_active: danger.gamma_multiply(0.9),
        fill_selected: danger,
        stroke: egui::Stroke::new(1.0, danger),
        ..ButtonOptions::default()
    }
}

pub fn tab_button(ui: &Ui, selected: bool, min_size: Vec2) -> ButtonOptions {
    ButtonOptions {
        min_size,
        text_color: if selected {
            ui.visuals().widgets.active.fg_stroke.color
        } else {
            ui.visuals().text_color()
        },
        fill: if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().widgets.inactive.bg_fill
        },
        fill_hovered: if selected {
            ui.visuals().selection.bg_fill.gamma_multiply(1.04)
        } else {
            ui.visuals().widgets.hovered.bg_fill
        },
        fill_active: if selected {
            ui.visuals().selection.bg_fill.gamma_multiply(0.94)
        } else {
            ui.visuals().widgets.active.bg_fill
        },
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: if selected {
            ui.visuals().selection.stroke
        } else {
            ui.visuals().widgets.inactive.bg_stroke
        },
        corner_radius: 10,
        ..ButtonOptions::default()
    }
}

use egui::{self, Align, Context, Layout, RichText, Sense, TopBottomPanel, ViewportCommand};

use crate::{assets, screens::AppScreen, ui::components::icon_button};

const TOP_BAR_HEIGHT: f32 = 38.0;
const CONTROL_SLOT_WIDTH: f32 = 20.0;
const CONTROL_ICON_MAX_WIDTH: f32 = 20.0;
const CONTROL_GAP: f32 = 7.0;
const CONTROL_GROUP_PADDING: f32 = 12.0;

pub fn render(ctx: &Context, active_screen: AppScreen) {
    TopBottomPanel::top("window_top_bar")
        .exact_height(TOP_BAR_HEIGHT)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(egui::Margin::ZERO)
                .outer_margin(egui::Margin::ZERO)
                .stroke(egui::Stroke::new(
                    1.0,
                    ctx.style().visuals.widgets.noninteractive.bg_stroke.color,
                )),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let full_rect = ui.max_rect();
            let controls_width =
                (CONTROL_SLOT_WIDTH * 3.0) + (CONTROL_GAP * 2.0) + (CONTROL_GROUP_PADDING * 2.0);
            let controls_min_x = (full_rect.max.x - controls_width).max(full_rect.min.x);
            let drag_rect = egui::Rect::from_min_max(
                full_rect.min,
                egui::pos2(controls_min_x, full_rect.max.y),
            );
            let controls_rect = egui::Rect::from_min_max(
                egui::pos2(controls_min_x, full_rect.min.y),
                full_rect.max,
            );

            let drag_response = ui.interact(
                drag_rect,
                ui.id().with("top_bar_drag_region"),
                Sense::click_and_drag(),
            );
            if drag_response.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }

            ui.scope_builder(egui::UiBuilder::new().max_rect(drag_rect), |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new("Minecraft Launcher").strong());
                    ui.add_space(12.0);
                    ui.colored_label(ui.visuals().weak_text_color(), active_screen.label());
                });
            });

            ui.scope_builder(egui::UiBuilder::new().max_rect(controls_rect), |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(CONTROL_GROUP_PADDING);
                    render_controls(ui, ctx);
                    ui.add_space(CONTROL_GROUP_PADDING);
                });
            });
        });
}

fn render_controls(ui: &mut egui::Ui, ctx: &Context) {
    if render_control_button(ui, "close", assets::X_SVG, "Close").clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    ui.add_space(CONTROL_GAP);

    let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if is_maximized {
        if render_control_button(ui, "restore_down", assets::COPY_SVG, "Restore down").clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
        }
    } else if render_control_button(ui, "maximize", assets::CHEVRON_UP_SVG, "Maximize").clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
    }
    ui.add_space(CONTROL_GAP);

    if render_control_button(ui, "minimize", assets::CHEVRON_DOWN_SVG, "Minimize").clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

fn render_control_button(
    ui: &mut egui::Ui,
    icon_id: &str,
    icon_bytes: &'static [u8],
    tooltip: &str,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(CONTROL_SLOT_WIDTH, ui.available_height()),
        Layout::left_to_right(Align::Center),
        |ui| {
            icon_button::svg(
                ui,
                icon_id,
                icon_bytes,
                tooltip,
                false,
                CONTROL_ICON_MAX_WIDTH,
            )
        },
    )
    .inner
}

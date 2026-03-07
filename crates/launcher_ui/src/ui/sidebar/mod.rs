use egui::{Context, ScrollArea, SidePanel, Ui};

use crate::assets;
use crate::screens::AppScreen;
use crate::ui::components::icon_button;
use crate::ui::style;

mod app_nav;
mod profiles;

#[derive(Debug, Clone, Copy)]
struct SidebarLayout {
    nav_icon_width: f32,
}

#[derive(Debug, Clone)]
pub struct ProfileShortcut {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct SidebarOutput {
    pub selected_screen: Option<AppScreen>,
    pub selected_profile_id: Option<String>,
    pub create_instance_clicked: bool,
}

pub fn render(
    ctx: &Context,
    active_screen: AppScreen,
    profile_shortcuts: &[ProfileShortcut],
) -> SidebarOutput {
    let mut output = SidebarOutput::default();
    let viewport_width = ctx.input(|i| i.content_rect().width());
    let nav_icon_width = (viewport_width * 0.025).clamp(16.0, 40.0);
    let horizontal_padding = (viewport_width * 0.005).clamp(4.0, 12.0);
    let sidebar_width = nav_icon_width + (horizontal_padding * 2.0);
    let content_width = (sidebar_width - (horizontal_padding * 2.0)).max(1.0);
    let layout = SidebarLayout { nav_icon_width };

    SidePanel::left("task_bar_left")
        .resizable(false)
        .exact_width(sidebar_width)
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
        .show_separator_line(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                ui.add_space(horizontal_padding);
                ui.allocate_ui_with_layout(
                    egui::vec2(content_width, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| render_segments(ui, active_screen, profile_shortcuts, &mut output, layout),
                );
                ui.add_space(horizontal_padding);
            });
        });

    output
}

fn render_segments(
    ui: &mut Ui,
    active_screen: AppScreen,
    profile_shortcuts: &[ProfileShortcut],
    output: &mut SidebarOutput,
    layout: SidebarLayout,
) {
    let full_height = ui.available_height().max(1.0);
    let full_width = ui.available_width().max(1.0);
    ui.allocate_ui_with_layout(
        egui::vec2(full_width, full_height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_height(full_height);
            ui.add_space(style::SPACE_XS);
            app_nav::render(ui, active_screen, output, layout.nav_icon_width);

            ui.add_space(style::SPACE_MD);
            let create_response = ui
                .horizontal_centered(|ui| {
                    icon_button::svg(
                        ui,
                        "create_instance",
                        assets::PLUS_SVG,
                        "Create instance",
                        false,
                        layout.nav_icon_width,
                    )
                })
                .inner;
            if create_response.clicked() {
                output.create_instance_clicked = true;
            }

            ui.add_space(style::SPACE_LG);
            ui.separator();
            ui.add_space(style::SPACE_MD);
            let profiles_height = ui.available_height().max(1.0);
            ScrollArea::vertical()
                .id_salt("profiles_scroll")
                .auto_shrink([false, false])
                .max_height(profiles_height)
                .show(ui, |ui| {
                    ui.set_min_height(profiles_height);
                    profiles::render(ui, profile_shortcuts, output, layout.nav_icon_width)
                });
        },
    );
}

use egui::{Context, ScrollArea, SidePanel, Ui};
use std::path::PathBuf;
use textui::{ButtonOptions, TextUi};

use crate::assets;
use crate::screens::AppScreen;
use crate::ui::components::icon_button;
use crate::ui::instance_context_menu::InstanceContextAction;
use crate::ui::style;

mod app_nav;
mod profiles;

pub fn request_home_focus(ctx: &Context, requested: bool) {
    app_nav::request_home_focus(ctx, requested);
}

#[derive(Debug, Clone, Copy)]
struct SidebarLayout {
    nav_icon_width: f32,
}

#[derive(Debug, Clone)]
pub struct ProfileShortcut {
    pub id: String,
    pub name: String,
    pub thumbnail_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
/// User interactions emitted by the sidebar for the app shell to handle.
pub struct SidebarOutput {
    /// Requests navigation to a fixed application screen.
    pub selected_screen: Option<AppScreen>,
    /// Requests that the main view open a specific instance.
    pub selected_profile_id: Option<String>,
    /// Requests opening the create-instance flow.
    pub create_instance_clicked: bool,
    /// Requests opening the import-instance flow.
    pub import_instance_clicked: bool,
    /// Context-menu actions requested for a specific instance shortcut.
    ///
    /// The shell handles these centrally so sidebar interactions can reuse the
    /// same instance-open and delete-confirmation flows as the library screen.
    pub instance_context_actions: Vec<(String, InstanceContextAction)>,
}

pub fn render(
    ctx: &Context,
    active_screen: AppScreen,
    profile_shortcuts: &[ProfileShortcut],
    text_ui: &mut TextUi,
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
            let panel_rect = ui.max_rect();
            let content_rect = egui::Rect::from_min_max(
                egui::pos2(panel_rect.left() + horizontal_padding, panel_rect.top()),
                egui::pos2(panel_rect.right() - horizontal_padding, panel_rect.bottom()),
            );
            let _ = ui.allocate_rect(panel_rect, egui::Sense::hover());
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(content_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_width(content_width);
                    render_segments(
                        ui,
                        active_screen,
                        profile_shortcuts,
                        text_ui,
                        &mut output,
                        layout,
                    );
                },
            );
        });

    output
}

fn render_segments(
    ui: &mut Ui,
    active_screen: AppScreen,
    profile_shortcuts: &[ProfileShortcut],
    text_ui: &mut TextUi,
    output: &mut SidebarOutput,
    layout: SidebarLayout,
) {
    let row_height = layout.nav_icon_width.max(1.0);
    let button_gap = style::SPACE_SM;
    let nav_count = AppScreen::FIXED_NAV.len() as f32;
    let nav_stack_height = (nav_count * row_height) + ((nav_count - 1.0).max(0.0) * button_gap);
    let divider_height = 1.0;
    let desired_top_height = style::SPACE_XS
        + nav_stack_height
        + button_gap
        + row_height
        + style::SPACE_LG
        + divider_height
        + style::SPACE_MD;
    let full_rect = ui.available_rect_before_wrap();
    if full_rect.width() <= 0.0 || full_rect.height() <= 0.0 {
        return;
    }

    let min_bottom_height = row_height.max(8.0);
    let max_top_height = (full_rect.height() - min_bottom_height).max(0.0);
    let top_height = desired_top_height.min(max_top_height);
    let top_rect = egui::Rect::from_min_max(
        full_rect.min,
        egui::pos2(full_rect.max.x, full_rect.min.y + top_height),
    );
    let bottom_rect =
        egui::Rect::from_min_max(egui::pos2(full_rect.min.x, top_rect.max.y), full_rect.max);

    let _ = ui.allocate_rect(full_rect, egui::Sense::hover());

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(top_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_min_height(top_height);
            ui.add_space(style::SPACE_XS);
            app_nav::render(ui, active_screen, output, layout.nav_icon_width);

            ui.add_space(button_gap);
            let create_response = ui
                .allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        icon_button::svg(
                            ui,
                            "create_instance",
                            assets::PLUS_SVG,
                            "Profile actions",
                            false,
                            layout.nav_icon_width,
                        )
                    },
                )
                .inner;
            let create_menu_labels = ["Create from scratch", "Import profile"];
            let create_menu_width = popup_menu_width(ui, &create_menu_labels);
            let _ = egui::Popup::menu(&create_response)
                .id(ui.id().with("sidebar_create_instance_popup"))
                .width(create_menu_width)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                .show(|ui| {
                    let popup_button_style = ButtonOptions {
                        min_size: egui::vec2(
                            ui.available_width().max(120.0),
                            style::CONTROL_HEIGHT,
                        ),
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
                            "sidebar_create_from_scratch",
                            create_menu_labels[0],
                            &popup_button_style,
                        )
                        .clicked()
                    {
                        output.create_instance_clicked = true;
                    }
                    if text_ui
                        .button(
                            ui,
                            "sidebar_import_profile",
                            create_menu_labels[1],
                            &popup_button_style,
                        )
                        .clicked()
                    {
                        output.import_instance_clicked = true;
                    }
                });

            ui.add_space(style::SPACE_LG);
            let (divider_rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width().max(1.0), divider_height),
                egui::Sense::hover(),
            );
            ui.painter().hline(
                divider_rect.x_range(),
                divider_rect.center().y,
                egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
            );
            ui.add_space(style::SPACE_MD);
        },
    );

    if bottom_rect.height() > 0.0 {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(bottom_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.add_space(style::SPACE_LG);
                let scroll_height = ui.available_height().max(1.0);
                ScrollArea::vertical()
                    .id_salt("profiles_scroll_v4")
                    .auto_shrink([false, false])
                    .max_height(scroll_height)
                    .show(ui, |ui| {
                        profiles::render(ui, profile_shortcuts, output, layout.nav_icon_width)
                    });
            },
        );
    }
}

fn popup_menu_width(ui: &Ui, labels: &[&str]) -> f32 {
    let button_padding = ui.spacing().button_padding.x * 2.0;
    let item_spacing = ui.spacing().item_spacing.x * 2.0;
    let window_margin =
        f32::from(ui.spacing().window_margin.left + ui.spacing().window_margin.right);
    let button_font = egui::TextStyle::Button.resolve(ui.style());

    let widest_label = labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap(
                    (*label).to_owned(),
                    button_font.clone(),
                    ui.visuals().text_color(),
                )
                .size()
                .x
        })
        .fold(0.0, f32::max);

    (widest_label + button_padding + item_spacing + window_margin).ceil()
}

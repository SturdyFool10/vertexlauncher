use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use egui::{
    self, Align, Button, Context, CursorIcon, Layout, ResizeDirection, Sense, TopBottomPanel,
    ViewportCommand,
};
use textui::{ButtonOptions, LabelOptions, TextUi};

use crate::{
    assets,
    ui::{components::icon_button, style},
};

const TOP_BAR_HEIGHT: f32 = 38.0;
const CONTROL_SLOT_WIDTH: f32 = 20.0;
const CONTROL_ICON_MAX_WIDTH: f32 = 20.0;
const CONTROL_GAP: f32 = 7.0;
const CONTROL_GROUP_PADDING: f32 = 12.0;
const PROFILE_BUTTON_VERTICAL_PADDING: f32 = 5.0;
const PROFILE_TO_CONTROLS_GAP: f32 = style::SPACE_MD;
const ACTIVE_USER_TO_PROFILE_GAP: f32 = style::SPACE_SM;
const ACTIVE_USER_BUTTON_MIN_WIDTH: f32 = 132.0;
const PROFILE_POPUP_MIN_WIDTH: f32 = 310.0;
const RESIZE_GRAB_THICKNESS: f32 = 6.0;

#[derive(Debug, Clone, Default)]
pub struct TopBarOutput {
    pub start_sign_in: bool,
    pub select_account_id: Option<String>,
    pub remove_account_id: Option<String>,
    pub open_active_user_terminal: bool,
}

#[derive(Debug, Clone)]
pub struct ProfileAccountOption {
    pub profile_id: String,
    pub display_name: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ProfileUiModel<'a> {
    pub display_name: Option<&'a str>,
    pub avatar_png: Option<&'a [u8]>,
    pub sign_in_in_progress: bool,
    pub status_message: Option<&'a str>,
    pub accounts: &'a [ProfileAccountOption],
    pub user_instance_active: bool,
}

pub fn render(
    ctx: &Context,
    section_label: &str,
    text_ui: &mut TextUi,
    profile_ui: ProfileUiModel<'_>,
) -> TopBarOutput {
    let mut output = TopBarOutput::default();

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
            let profile_button_size =
                (TOP_BAR_HEIGHT - (PROFILE_BUTTON_VERTICAL_PADDING * 2.0)).max(1.0);
            let active_user_button_width = ACTIVE_USER_BUTTON_MIN_WIDTH;
            let active_user_visible = profile_ui.user_instance_active;
            let control_group_width =
                (CONTROL_SLOT_WIDTH * 3.0) + (CONTROL_GAP * 2.0) + (CONTROL_GROUP_PADDING * 2.0);
            let right_side_width = control_group_width
                + PROFILE_TO_CONTROLS_GAP
                + profile_button_size
                + if active_user_visible {
                    ACTIVE_USER_TO_PROFILE_GAP + active_user_button_width
                } else {
                    0.0
                };
            let controls_min_x = (full_rect.max.x - right_side_width).max(full_rect.min.x);
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
                    ui.add_space(style::SPACE_LG);
                    let mut section_style = LabelOptions {
                        font_size: 18.0,
                        line_height: 24.0,
                        wrap: false,
                        ..LabelOptions::default()
                    };
                    section_style.color = ui.visuals().weak_text_color();
                    let _ = text_ui.label(
                        ui,
                        ("topbar_screen", section_label),
                        section_label,
                        &section_style,
                    );
                });
            });

            ui.scope_builder(egui::UiBuilder::new().max_rect(controls_rect), |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(CONTROL_GROUP_PADDING);
                    render_controls(ui, ctx);
                    ui.add_space(PROFILE_TO_CONTROLS_GAP);

                    let profile_response =
                        render_profile_button(ui, text_ui, profile_ui, profile_button_size);
                    let direct_sign_in = profile_ui.display_name.is_none()
                        && profile_ui.accounts.is_empty()
                        && !profile_ui.sign_in_in_progress;
                    if direct_sign_in {
                        if profile_response.clicked() {
                            output.start_sign_in = true;
                        }
                    } else {
                        let profile_popup_id = ui.id().with("profile_selector_popup");
                        let _ = egui::Popup::menu(&profile_response)
                            .id(profile_popup_id)
                            .width(PROFILE_POPUP_MIN_WIDTH)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                render_profile_popup(
                                    ui,
                                    text_ui,
                                    profile_ui,
                                    &mut output,
                                    profile_popup_id,
                                );
                            });
                    }

                    if active_user_visible {
                        ui.add_space(ACTIVE_USER_TO_PROFILE_GAP);
                        if render_active_user_terminal_button(ui, profile_button_size).clicked() {
                            output.open_active_user_terminal = true;
                        }
                    }

                    ui.add_space(CONTROL_GROUP_PADDING);
                });
            });
        });

    output
}

pub fn handle_window_resize(ctx: &Context) {
    let (content_rect, pointer_pos, primary_pressed, is_maximized, is_fullscreen) =
        ctx.input(|i| {
            (
                i.content_rect(),
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.viewport().maximized.unwrap_or(false),
                i.viewport().fullscreen.unwrap_or(false),
            )
        });

    if is_maximized || is_fullscreen {
        return;
    }

    let Some(pointer_pos) = pointer_pos else {
        return;
    };

    let left = pointer_pos.x <= content_rect.left() + RESIZE_GRAB_THICKNESS;
    let right = pointer_pos.x >= content_rect.right() - RESIZE_GRAB_THICKNESS;
    let top = pointer_pos.y <= content_rect.top() + RESIZE_GRAB_THICKNESS;
    let bottom = pointer_pos.y >= content_rect.bottom() - RESIZE_GRAB_THICKNESS;

    let direction = if top && left {
        Some(ResizeDirection::NorthWest)
    } else if top && right {
        Some(ResizeDirection::NorthEast)
    } else if bottom && left {
        Some(ResizeDirection::SouthWest)
    } else if bottom && right {
        Some(ResizeDirection::SouthEast)
    } else if top {
        Some(ResizeDirection::North)
    } else if bottom {
        Some(ResizeDirection::South)
    } else if left {
        Some(ResizeDirection::West)
    } else if right {
        Some(ResizeDirection::East)
    } else {
        None
    };

    if let Some(direction) = direction {
        ctx.set_cursor_icon(resize_cursor_icon(direction));
        if primary_pressed {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
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

fn resize_cursor_icon(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
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

fn render_profile_button(
    ui: &mut egui::Ui,
    text_ui: &mut TextUi,
    profile_ui: ProfileUiModel<'_>,
    button_size: f32,
) -> egui::Response {
    if let Some(avatar_png) = profile_ui.avatar_png {
        let mut hasher = DefaultHasher::new();
        avatar_png.hash(&mut hasher);
        let uri = format!("bytes://vertex-profile/avatar-{:016x}.png", hasher.finish());
        let icon_size = (button_size - 8.0).clamp(10.0, button_size);
        let icon = egui::Image::from_bytes(uri, avatar_png.to_vec())
            .fit_to_exact_size(egui::vec2(icon_size, icon_size));

        let button = Button::image(icon)
            .frame(true)
            .stroke(egui::Stroke::new(
                1.0,
                ui.visuals().widgets.inactive.bg_stroke.color,
            ))
            .fill(if profile_ui.sign_in_in_progress {
                ui.visuals().widgets.active.weak_bg_fill
            } else {
                ui.visuals().widgets.inactive.weak_bg_fill
            });

        ui.add_sized([button_size, button_size], button)
    } else if profile_ui.display_name.is_none() && !profile_ui.sign_in_in_progress {
        let sign_in_style = ButtonOptions {
            min_size: egui::vec2((button_size * 3.2).clamp(68.0, 110.0), button_size),
            text_color: ui.visuals().text_color(),
            fill: ui.visuals().widgets.inactive.weak_bg_fill,
            fill_hovered: ui.visuals().widgets.hovered.weak_bg_fill,
            fill_active: ui.visuals().widgets.active.weak_bg_fill,
            fill_selected: ui.visuals().widgets.open.weak_bg_fill,
            stroke: egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color),
            ..ButtonOptions::default()
        };
        text_ui.button(ui, "profile_button_sign_in", "Sign in", &sign_in_style)
    } else {
        ui.allocate_ui_with_layout(
            egui::vec2(button_size, button_size),
            Layout::left_to_right(Align::Center),
            |ui| {
                icon_button::svg(
                    ui,
                    "profile_selector_default",
                    assets::USER_SVG,
                    "Profile selector",
                    profile_ui.sign_in_in_progress,
                    button_size,
                )
            },
        )
        .inner
    }
}

fn render_active_user_terminal_button(ui: &mut egui::Ui, button_height: f32) -> egui::Response {
    let text_color = ui.visuals().text_color();
    let themed_svg = apply_text_color(assets::TERMINAL_2_SVG, text_color);
    let uri = format!(
        "bytes://vertex-topbar/user-active-terminal-{:02x}{:02x}{:02x}.svg",
        text_color.r(),
        text_color.g(),
        text_color.b()
    );
    let icon_size = (button_height - 14.0).clamp(12.0, 18.0);
    let desired_size = egui::vec2(ACTIVE_USER_BUTTON_MIN_WIDTH, button_height);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());

    let fill = if response.is_pointer_button_down_on() {
        ui.visuals().widgets.active.weak_bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        ui.visuals().widgets.inactive.weak_bg_fill
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );

    let inner_rect = rect.shrink2(egui::vec2(8.0, 4.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            let _ = ui.add(
                egui::Label::new(egui::RichText::new("user active").color(text_color))
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
            ui.add_space(6.0);
            let icon = egui::Image::from_bytes(uri, themed_svg)
                .fit_to_exact_size(egui::vec2(icon_size, icon_size));
            ui.add(icon);
        });
    });

    response
}

fn apply_text_color(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}

fn render_profile_popup(
    ui: &mut egui::Ui,
    text_ui: &mut TextUi,
    profile_ui: ProfileUiModel<'_>,
    output: &mut TopBarOutput,
    popup_id: egui::Id,
) {
    ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_MD, style::SPACE_MD);
    let full_action_width = ui.available_width().max(220.0);

    let muted_text = ui.visuals().weak_text_color();
    let heading_style = LabelOptions {
        font_size: 18.0,
        line_height: 22.0,
        weight: 700,
        color: ui.visuals().text_color(),
        wrap: false,
        ..LabelOptions::default()
    };
    let body_style = LabelOptions {
        color: ui.visuals().text_color(),
        wrap: false,
        ..LabelOptions::default()
    };
    let mut muted_style = body_style.clone();
    muted_style.color = muted_text;

    let button_style = ButtonOptions {
        min_size: egui::vec2(full_action_width, style::CONTROL_HEIGHT),
        corner_radius: style::CORNER_RADIUS_SM,
        padding: egui::vec2(style::SPACE_MD, style::SPACE_XS),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().widgets.open.bg_fill,
        stroke: egui::Stroke::new(1.4, ui.visuals().widgets.hovered.bg_stroke.color),
        ..ButtonOptions::default()
    };

    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(egui::Stroke::new(1.0, ui.visuals().window_stroke.color))
        .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
        .inner_margin(egui::Margin::same(style::SPACE_LG as i8))
        .show(ui, |ui| {
            if let Some(name) = profile_ui.display_name {
                let _ = text_ui.label(
                    ui,
                    "profile_popup_signed_in",
                    &format!("Signed in as {name}"),
                    &heading_style,
                );
            } else {
                let _ = text_ui.label(
                    ui,
                    "profile_popup_signed_out",
                    "No Microsoft account signed in",
                    &heading_style,
                );
            }

            if let Some(message) = profile_ui.status_message {
                let _ = text_ui.label(ui, "profile_popup_status", message, &muted_style);
            }
        });

    if !profile_ui.accounts.is_empty() {
        ui.add_space(style::SPACE_XS / 2.0);
        let _ = text_ui.label(
            ui,
            "profile_popup_accounts_title",
            "Saved accounts",
            &muted_style,
        );

        egui::Frame::new()
            .fill(ui.visuals().window_fill)
            .stroke(egui::Stroke::new(1.0, ui.visuals().window_stroke.color))
            .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
            .inner_margin(egui::Margin::same(style::SPACE_MD as i8))
            .show(ui, |ui| {
                let mut list_button_style = button_style.clone();
                list_button_style.min_size = egui::vec2(150.0, style::CONTROL_HEIGHT);

                let mut remove_button_style = button_style.clone();
                remove_button_style.min_size = egui::vec2(72.0, style::CONTROL_HEIGHT);

                for account in profile_ui.accounts {
                    ui.horizontal(|ui| {
                        let label = if account.is_active {
                            format!("{} (active)", account.display_name)
                        } else {
                            account.display_name.clone()
                        };

                        if text_ui
                            .selectable_button(
                                ui,
                                ("profile_popup_account_select", &account.profile_id),
                                &label,
                                account.is_active,
                                &list_button_style,
                            )
                            .clicked()
                        {
                            output.select_account_id = Some(account.profile_id.clone());
                        }

                        if text_ui
                            .button(
                                ui,
                                ("profile_popup_account_remove", &account.profile_id),
                                "Remove",
                                &remove_button_style,
                            )
                            .clicked()
                        {
                            output.remove_account_id = Some(account.profile_id.clone());
                        }
                    });
                }
            });
    }

    ui.add_space(style::SPACE_XS / 2.0);
    ui.separator();
    ui.add_space(style::SPACE_XS / 2.0);

    let mut primary_button_style = button_style.clone();
    primary_button_style.min_size = egui::vec2(full_action_width, style::CONTROL_HEIGHT);
    primary_button_style.text_color = ui.visuals().text_color();
    primary_button_style.fill = ui.visuals().widgets.hovered.bg_fill;
    primary_button_style.fill_hovered = ui.visuals().widgets.open.bg_fill;
    primary_button_style.fill_active = ui.visuals().widgets.active.bg_fill;
    primary_button_style.fill_selected = ui.visuals().widgets.open.bg_fill;
    primary_button_style.stroke = egui::Stroke::new(1.8, ui.visuals().widgets.open.bg_stroke.color);

    if profile_ui.sign_in_in_progress {
        let mut pending_button_style = button_style.clone();
        pending_button_style.min_size = egui::vec2(full_action_width, style::CONTROL_HEIGHT);
        pending_button_style.stroke =
            egui::Stroke::new(1.4, ui.visuals().widgets.inactive.bg_stroke.color);
        ui.add_enabled_ui(false, |ui| {
            let _ = text_ui.button(
                ui,
                "profile_popup_signing_in",
                "Signing in with Microsoft...",
                &pending_button_style,
            );
        });
    } else if text_ui
        .button(
            ui,
            "profile_popup_signin_action",
            "Sign in",
            &primary_button_style,
        )
        .clicked()
    {
        output.start_sign_in = true;
        egui::Popup::open_id(ui.ctx(), popup_id);
    }
}

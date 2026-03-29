use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use egui::{
    self, Align, Context, CursorIcon, Layout, ResizeDirection, Sense, TopBottomPanel,
    ViewportCommand,
};
use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
use textui::{ButtonOptions, LabelOptions, TextUi};

use crate::{
    assets, privacy,
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
const ACTIVE_USER_BUTTON_MIN_WIDTH: f32 = 148.0;
const PROFILE_POPUP_MIN_WIDTH: f32 = 310.0;
const RESIZE_GRAB_THICKNESS: f32 = 6.0;
const PROFILE_BUTTON_CORNER_RADIUS: u8 = 10;

#[derive(Debug, Clone, Default)]
pub struct TopBarOutput {
    pub start_webview_sign_in: bool,
    pub start_device_code_sign_in: bool,
    pub open_device_code_browser: bool,
    pub select_account_id: Option<String>,
    pub remove_account_id: Option<String>,
    pub refresh_account_id: Option<String>,
    pub open_active_user_terminal: bool,
}

#[derive(Debug, Clone)]
pub struct ProfileAccountOption {
    pub profile_id: String,
    pub display_name: String,
    pub is_active: bool,
    pub is_failed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ProfileUiModel<'a> {
    pub display_name: Option<&'a str>,
    pub avatar_png: Option<&'a [u8]>,
    pub sign_in_in_progress: bool,
    pub auth_busy: bool,
    pub token_refresh_in_progress: bool,
    pub streamer_mode: bool,
    pub status_message: Option<&'a str>,
    pub accounts: &'a [ProfileAccountOption],
    pub user_instance_active: bool,
    pub device_code_prompt: Option<&'a auth::DeviceCodePrompt>,
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
                    let direct_sign_in = profile_ui.display_name.is_none() && !profile_ui.auth_busy;
                    if direct_sign_in {
                        if profile_response.clicked() {
                            output.start_webview_sign_in = true;
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
                        if render_active_user_terminal_button(ui, text_ui, profile_button_size)
                            .clicked()
                        {
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
        if render_control_button(ui, "restore_down", assets::RESTORE_SVG, "Restore down").clicked()
        {
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
        let avatar_hash = hasher.finish();
        let uri = format!("bytes://vertex-profile/avatar-rounded-{avatar_hash:016x}.png");
        let icon = egui::Image::from_bytes(
            uri,
            rounded_profile_avatar_png(avatar_hash, avatar_png, PROFILE_BUTTON_CORNER_RADIUS),
        )
        .fit_to_exact_size(egui::vec2(button_size.max(1.0), button_size.max(1.0)));

        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(button_size, button_size), egui::Sense::click());
        let fill = if profile_ui.auth_busy {
            ui.visuals().widgets.active.weak_bg_fill
        } else if response.is_pointer_button_down_on() {
            ui.visuals().widgets.active.weak_bg_fill
        } else if response.hovered() {
            ui.visuals().widgets.hovered.weak_bg_fill
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        };
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(PROFILE_BUTTON_CORNER_RADIUS),
            fill,
        );
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(PROFILE_BUTTON_CORNER_RADIUS),
            egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color),
            egui::StrokeKind::Inside,
        );
        let _ = ui.put(rect, icon);
        response
    } else if profile_ui.display_name.is_none() && profile_ui.auth_busy {
        render_profile_pending_button(ui, button_size)
    } else if profile_ui.display_name.is_none() {
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
                    profile_ui.auth_busy,
                    button_size,
                )
            },
        )
        .inner
    }
}

fn render_profile_pending_button(ui: &mut egui::Ui, button_size: f32) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(button_size, button_size), egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(PROFILE_BUTTON_CORNER_RADIUS),
        ui.visuals().widgets.active.weak_bg_fill,
    );
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(PROFILE_BUTTON_CORNER_RADIUS),
        egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space((button_size * 0.2).max(2.0));
            ui.spinner();
        });
    });
    response
}

fn rounded_profile_avatar_png(cache_key: u64, avatar_png: &[u8], radius: u8) -> Vec<u8> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Vec<u8>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(cache) = cache.lock()
        && let Some(bytes) = cache.get(&cache_key)
    {
        return bytes.clone();
    }

    let rounded = round_avatar_png_bytes(avatar_png, radius).unwrap_or_else(|| avatar_png.to_vec());

    if let Ok(mut cache) = cache.lock() {
        cache.insert(cache_key, rounded.clone());
    }

    rounded
}

fn round_avatar_png_bytes(avatar_png: &[u8], radius: u8) -> Option<Vec<u8>> {
    let mut image = image::load_from_memory(avatar_png).ok()?.to_rgba8();
    let width = image.width();
    let height = image.height();
    let radius = radius as i32;
    let width_i32 = i32::try_from(width).ok()?;
    let height_i32 = i32::try_from(height).ok()?;

    for y in 0..height {
        for x in 0..width {
            let x_i32 = i32::try_from(x).ok()?;
            let y_i32 = i32::try_from(y).ok()?;
            let dx = if x_i32 < radius {
                radius - 1 - x_i32
            } else if x_i32 >= width_i32 - radius {
                x_i32 - (width_i32 - radius)
            } else {
                0
            };
            let dy = if y_i32 < radius {
                radius - 1 - y_i32
            } else if y_i32 >= height_i32 - radius {
                y_i32 - (height_i32 - radius)
            } else {
                0
            };

            if dx > 0 || dy > 0 {
                let distance_sq = dx * dx + dy * dy;
                if distance_sq >= radius * radius {
                    image.get_pixel_mut(x, y).0[3] = 0;
                }
            }
        }
    }

    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(image.as_raw(), width, height, ColorType::Rgba8.into())
        .ok()?;
    Some(bytes)
}

fn render_active_user_terminal_button(
    ui: &mut egui::Ui,
    text_ui: &mut TextUi,
    button_height: f32,
) -> egui::Response {
    let text_color = ui.visuals().text_color();
    let themed_svg = apply_text_color(assets::TERMINAL_2_SVG, text_color);
    let uri = format!(
        "bytes://vertex-topbar/user-active-terminal-{:02x}{:02x}{:02x}.svg",
        text_color.r(),
        text_color.g(),
        text_color.b()
    );
    let icon_size = (button_height - 14.0).clamp(12.0, 18.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ACTIVE_USER_BUTTON_MIN_WIDTH, button_height),
        egui::Sense::click(),
    );
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
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let icon = egui::Image::from_bytes(uri, themed_svg)
                .fit_to_exact_size(egui::vec2(icon_size, icon_size));
            let _ = ui.add_sized(egui::vec2(icon_size, icon_size), icon);
            ui.add_space(6.0);

            let text_lane_width = ui.available_width().max(1.0);
            let text_lane_height = inner_rect.height().max(1.0);
            ui.allocate_ui_with_layout(
                egui::vec2(text_lane_width, text_lane_height),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_clip_rect(ui.max_rect());
                    let label_style = LabelOptions {
                        font_size: 16.0,
                        line_height: 20.0,
                        color: text_color,
                        weight: 700,
                        wrap: false,
                        ..LabelOptions::default()
                    };
                    let _ =
                        text_ui.label(ui, "topbar_user_active_label", "user active", &label_style);
                },
            );
        });
    });

    response
}

fn apply_text_color(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}

fn render_device_code_section(
    ui: &mut egui::Ui,
    text_ui: &mut TextUi,
    prompt: &auth::DeviceCodePrompt,
    output: &mut TopBarOutput,
    full_action_width: f32,
    button_style: &ButtonOptions,
) {
    let qr_url = prompt
        .verification_uri_complete
        .as_deref()
        .unwrap_or(prompt.verification_uri.as_str());

    // Instructional text — wraps naturally, normal size
    let instruction_style = LabelOptions {
        color: ui.visuals().text_color(),
        wrap: true,
        ..LabelOptions::default()
    };
    let _ = text_ui.label(
        ui,
        "device_code_instruction",
        "You can either scan this QR code and sign-in on another device, or copy the code below and sign in on this device...",
        &instruction_style,
    );

    // QR code: pre-allocate exact rect so nothing can expand outward
    let qr_size = (full_action_width * 0.7).clamp(120.0, 220.0);
    ui.allocate_ui_with_layout(
        egui::vec2(full_action_width, qr_size),
        Layout::top_down(Align::Center),
        |ui| {
            render_qr_code(ui, qr_url, qr_size);
        },
    );

    // Code row: build style then measure actual rendered size to drive layout
    let base = LabelOptions::default();
    let code_label_style = LabelOptions {
        font_size: base.font_size * 1.25,
        line_height: base.line_height * 1.25,
        weight: 700,
        color: ui.visuals().text_color(),
        wrap: false,
        ..base
    };
    let code_text_size =
        text_ui.measure_text_size(ui, prompt.user_code.as_str(), &code_label_style);
    let code_row_height = code_text_size.y + style::SPACE_MD * 2.0;
    let code_box_width = code_text_size.x + style::SPACE_MD * 2.0;
    let copy_btn_size = code_row_height; // square button matching row height
    let total_row_width = code_box_width + copy_btn_size;

    let cr = style::CORNER_RADIUS_SM;
    let box_radius = egui::CornerRadius {
        nw: cr,
        sw: cr,
        ne: 0,
        se: 0,
    };
    let btn_radius = egui::CornerRadius {
        nw: 0,
        sw: 0,
        ne: cr,
        se: cr,
    };

    // Outer container centers the joined control horizontally
    ui.allocate_ui_with_layout(
        egui::vec2(full_action_width, code_row_height),
        Layout::top_down(Align::Center),
        |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(total_row_width, code_row_height),
                Layout::right_to_left(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;

                    // Copy SVG icon button — square, right-rounded
                    let text_color = ui.visuals().text_color();
                    let themed_svg = apply_text_color(assets::COPY_SVG, text_color);
                    let uri = format!(
                        "bytes://vertex-topbar/device-code-copy-{:02x}{:02x}{:02x}.svg",
                        text_color.r(),
                        text_color.g(),
                        text_color.b()
                    );
                    let icon_size = (copy_btn_size - style::SPACE_MD * 2.0).clamp(12.0, 28.0);
                    let (btn_rect, btn_response) = ui.allocate_exact_size(
                        egui::vec2(copy_btn_size, code_row_height),
                        egui::Sense::click(),
                    );
                    let base_fill = ui.visuals().widgets.noninteractive.bg_fill;
                    let btn_fill = if btn_response.is_pointer_button_down_on() {
                        ui.visuals().widgets.active.bg_fill
                    } else if btn_response.hovered() {
                        ui.visuals().widgets.hovered.bg_fill
                    } else {
                        base_fill
                    };
                    let stroke_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
                    ui.painter().rect(
                        btn_rect,
                        btn_radius,
                        btn_fill,
                        egui::Stroke::new(1.0, stroke_color),
                        egui::StrokeKind::Inside,
                    );
                    // Paint icon centered in the button rect — paint_at bypasses the cursor
                    let icon_rect = egui::Rect::from_center_size(
                        btn_rect.center(),
                        egui::vec2(icon_size, icon_size),
                    );
                    egui::Image::from_bytes(uri, themed_svg).paint_at(ui, icon_rect);
                    if btn_response.clicked() {
                        ui.ctx().copy_text(prompt.user_code.clone());
                    }

                    // Code display box — sized to fit text, left-rounded right-flat
                    let (box_rect, _) = ui.allocate_exact_size(
                        egui::vec2(code_box_width, code_row_height),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect(
                        box_rect,
                        box_radius,
                        ui.visuals().widgets.noninteractive.bg_fill,
                        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                        egui::StrokeKind::Inside,
                    );
                    let inner = box_rect.shrink2(egui::vec2(style::SPACE_MD, 0.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let _ = text_ui.label(
                                ui,
                                "device_code_user_code",
                                prompt.user_code.as_str(),
                                &code_label_style,
                            );
                        });
                    });
                },
            );
        },
    );

    // Open in browser — full width
    let mut browser_button_style = button_style.clone();
    browser_button_style.min_size = egui::vec2(full_action_width, style::CONTROL_HEIGHT);
    if text_ui
        .button(
            ui,
            "device_code_open_browser",
            "Open Sign-in Window in Default Browser",
            &browser_button_style,
        )
        .clicked()
    {
        output.open_device_code_browser = true;
    }
}

fn render_qr_code(ui: &mut egui::Ui, text: &str, size: f32) {
    use qrcode::QrCode;
    use qrcode::types::Color;

    let Ok(code) = QrCode::new(text.as_bytes()) else {
        return;
    };

    let module_count = code.width();
    let colors = code.to_colors();

    let pixels: Vec<egui::Color32> = colors
        .iter()
        .map(|c| {
            if *c == Color::Dark {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            }
        })
        .collect();

    let color_image = egui::ColorImage {
        size: [module_count, module_count],
        pixels,
        source_size: egui::Vec2::new(module_count as f32, module_count as f32),
    };

    let texture_name = format!("qr_code_{}", text.len());
    let texture = ui
        .ctx()
        .load_texture(texture_name, color_image, egui::TextureOptions::NEAREST);

    let image = egui::Image::from_texture(egui::load::SizedTexture::from_handle(&texture))
        .fit_to_exact_size(egui::vec2(size, size))
        .corner_radius(egui::CornerRadius::same(4));
    ui.add(image);
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
                let redacted_name = privacy::redact_account_label(profile_ui.streamer_mode, name);
                let _ = text_ui.label(
                    ui,
                    "profile_popup_signed_in",
                    &format!("Signed in as {redacted_name}"),
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

            if profile_ui.device_code_prompt.is_none() {
                if let Some(message) = profile_ui.status_message {
                    let _ = text_ui.label(ui, "profile_popup_status", message, &muted_style);
                }
            }
        });

    if let Some(prompt) = profile_ui.device_code_prompt {
        render_device_code_section(
            ui,
            text_ui,
            prompt,
            output,
            full_action_width,
            &button_style,
        );
        return;
    }

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

                let mut refresh_button_style = button_style.clone();
                refresh_button_style.min_size = egui::vec2(78.0, style::CONTROL_HEIGHT);

                for account in profile_ui.accounts {
                    let label = if account.is_active && account.is_failed {
                        format!(
                            "{} (Failed)",
                            privacy::redact_account_label(
                                profile_ui.streamer_mode,
                                account.display_name.as_str()
                            )
                        )
                    } else if account.is_active {
                        format!(
                            "{} (Active)",
                            privacy::redact_account_label(
                                profile_ui.streamer_mode,
                                account.display_name.as_str()
                            )
                        )
                    } else {
                        privacy::redact_account_label(
                            profile_ui.streamer_mode,
                            account.display_name.as_str(),
                        )
                        .into_owned()
                    };

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), style::CONTROL_HEIGHT),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if account.is_failed
                                && ui
                                    .add_enabled_ui(!profile_ui.auth_busy, |ui| {
                                        text_ui.button(
                                            ui,
                                            ("profile_popup_account_refresh", &account.profile_id),
                                            "Refresh",
                                            &refresh_button_style,
                                        )
                                    })
                                    .inner
                                    .clicked()
                            {
                                output.refresh_account_id = Some(account.profile_id.clone());
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

                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let mut fill_style = list_button_style.clone();
                                    fill_style.min_size.x = ui.available_width().max(1.0);
                                    if text_ui
                                        .selectable_button(
                                            ui,
                                            ("profile_popup_account_select", &account.profile_id),
                                            &label,
                                            account.is_active,
                                            &fill_style,
                                        )
                                        .clicked()
                                    {
                                        output.select_account_id = Some(account.profile_id.clone());
                                    }
                                },
                            );
                        },
                    );
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

    if profile_ui.auth_busy {
        let mut pending_button_style = button_style.clone();
        pending_button_style.min_size = egui::vec2(full_action_width, style::CONTROL_HEIGHT);
        pending_button_style.stroke =
            egui::Stroke::new(1.4, ui.visuals().widgets.inactive.bg_stroke.color);
        ui.add_enabled_ui(false, |ui| {
            let _ = render_pending_text_button(
                ui,
                text_ui,
                "profile_popup_signing_in",
                pending_auth_label(profile_ui),
                &pending_button_style,
                true,
            );
        });
    } else {
        let half_width = (full_action_width - ui.spacing().item_spacing.x) / 2.0;
        let mut half_button_style = primary_button_style.clone();
        half_button_style.min_size = egui::vec2(half_width.max(1.0), style::CONTROL_HEIGHT);

        ui.allocate_ui_with_layout(
            egui::vec2(full_action_width, style::CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if text_ui
                    .button(
                        ui,
                        "profile_popup_signin_webview",
                        "Webview Sign-in",
                        &half_button_style,
                    )
                    .clicked()
                {
                    output.start_webview_sign_in = true;
                    egui::Popup::open_id(ui.ctx(), popup_id);
                }

                if text_ui
                    .button(
                        ui,
                        "profile_popup_signin_device_code",
                        "Device Code Sign-in",
                        &half_button_style,
                    )
                    .clicked()
                {
                    output.start_device_code_sign_in = true;
                    egui::Popup::open_id(ui.ctx(), popup_id);
                }
            },
        );
    }
}

fn pending_auth_label(profile_ui: ProfileUiModel<'_>) -> &'static str {
    if profile_ui.token_refresh_in_progress {
        "Refreshing session..."
    } else if profile_ui.sign_in_in_progress {
        "Signing in with Microsoft..."
    } else {
        "Authenticating..."
    }
}

fn render_pending_text_button(
    ui: &mut egui::Ui,
    text_ui: &mut TextUi,
    id_source: impl std::hash::Hash,
    label: &str,
    options: &ButtonOptions,
    show_spinner: bool,
) -> egui::Response {
    if !show_spinner {
        return text_ui.button(ui, id_source, label, options);
    }

    let desired_size = options.min_size;
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let label_id = ui.id().with("pending_auth_label").with(&id_source);
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(options.corner_radius),
        options.fill,
        options.stroke,
        egui::StrokeKind::Inside,
    );

    let inner_rect = rect.shrink2(options.padding);
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.spinner();
            ui.add_space(crate::ui::style::SPACE_XS);
            let mut label_style = LabelOptions {
                color: options.text_color,
                wrap: false,
                ..LabelOptions::default()
            };
            label_style.font_size = 14.0;
            label_style.line_height = 18.0;
            let _ = text_ui.label(ui, label_id, label, &label_style);
        });
    });

    response
}

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use egui::{
    self, Align, Context, CursorIcon, Layout, ResizeDirection, Sense, TopBottomPanel,
    ViewportCommand,
};
use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
use shared_lru::ThreadSafeLru;
use textui::{ButtonOptions, LabelOptions, TextUi};
use ui_foundation::{is_compact_width, popup_width};

use crate::{
    assets, privacy,
    ui::{
        components::{icon_button, image_textures},
        style,
    },
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
const PROFILE_POPUP_MIN_WIDTH_COMPACT: f32 = 220.0;
const RESIZE_GRAB_THICKNESS: f32 = 6.0;
const PROFILE_BUTTON_CORNER_RADIUS: u8 = 10;
const TOP_BAR_COMPACT_THRESHOLD: f32 = 720.0;
const ROUNDED_AVATAR_CACHE_MAX_ENTRIES: usize = 32;
const PROFILE_POPUP_FOCUS_FIRST_KEY: &str = "top_bar_profile_popup_focus_first";
const PROFILE_POPUP_OWNER_FOCUS_KEY: &str = "top_bar_profile_popup_owner_focus";

#[derive(Debug, Clone, Default)]
pub struct TopBarOutput {
    pub start_webview_sign_in: bool,
    pub start_device_code_sign_in: bool,
    pub open_device_code_browser: bool,
    pub cancel_device_code_sign_in: bool,
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
            let viewport_width = full_rect.width();
            let compact = is_compact_width(viewport_width, TOP_BAR_COMPACT_THRESHOLD);
            let profile_button_size =
                (TOP_BAR_HEIGHT - (PROFILE_BUTTON_VERTICAL_PADDING * 2.0)).max(1.0);
            let active_user_button_width = if compact {
                (viewport_width * 0.2).clamp(96.0, ACTIVE_USER_BUTTON_MIN_WIDTH)
            } else {
                ACTIVE_USER_BUTTON_MIN_WIDTH
            };
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
                    // Use a separate popup ID for device-code mode so the normal popup's
                    // cached area size is never inflated by the wider device-code layout.
                    let in_dc = profile_ui.device_code_prompt.is_some();
                    let normal_popup_id = ui.id().with("profile_selector_popup");
                    let dc_popup_id = ui.id().with("profile_selector_popup_dc");
                    let profile_popup_id = if in_dc { dc_popup_id } else { normal_popup_id };

                    // When device-code state changes, transfer open state to the new ID.
                    let prev_in_dc_key = ui.id().with("prev_in_dc");
                    let prev_in_dc = ui
                        .ctx()
                        .data_mut(|d| d.get_temp::<bool>(prev_in_dc_key))
                        .unwrap_or(in_dc);
                    ui.ctx().data_mut(|d| d.insert_temp(prev_in_dc_key, in_dc));
                    if prev_in_dc != in_dc {
                        let old_id = if prev_in_dc {
                            dc_popup_id
                        } else {
                            normal_popup_id
                        };
                        if egui::Popup::is_id_open(ui.ctx(), old_id) {
                            egui::Popup::open_id(ui.ctx(), profile_popup_id);
                        }
                    }
                    let popup_was_open = egui::Popup::is_id_open(ui.ctx(), profile_popup_id);

                    let _ = egui::Popup::menu(&profile_response)
                        .id(profile_popup_id)
                        .width(popup_width(
                            viewport_width,
                            if compact {
                                PROFILE_POPUP_MIN_WIDTH_COMPACT
                            } else {
                                PROFILE_POPUP_MIN_WIDTH
                            },
                            PROFILE_POPUP_MIN_WIDTH,
                            8.0,
                        ))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            if !popup_was_open {
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(
                                        egui::Id::new((
                                            PROFILE_POPUP_FOCUS_FIRST_KEY,
                                            profile_popup_id,
                                        )),
                                        true,
                                    );
                                });
                            }
                            render_profile_popup(
                                ui,
                                text_ui,
                                profile_ui,
                                &mut output,
                                profile_popup_id,
                                profile_response.id,
                                compact,
                            );
                        });
                    let popup_is_open = egui::Popup::is_id_open(ui.ctx(), profile_popup_id);
                    let should_restore_owner_focus = ui.ctx().data_mut(|data| {
                        let key = egui::Id::new((PROFILE_POPUP_OWNER_FOCUS_KEY, profile_popup_id));
                        let pending = data.get_temp::<bool>(key).unwrap_or(false);
                        if pending && !popup_is_open {
                            data.remove::<bool>(key);
                            true
                        } else {
                            false
                        }
                    });
                    if should_restore_owner_focus {
                        profile_response.request_focus();
                    }

                    if active_user_visible {
                        ui.add_space(ACTIVE_USER_TO_PROFILE_GAP);
                        if render_active_user_terminal_button(
                            ui,
                            text_ui,
                            profile_button_size,
                            active_user_button_width,
                            compact,
                        )
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
        let key = format!("bytes://vertex-profile/avatar-rounded-{avatar_hash:016x}.png");
        let rounded =
            rounded_profile_avatar_png(avatar_hash, avatar_png, PROFILE_BUTTON_CORNER_RADIUS);

        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(button_size, button_size), egui::Sense::click());
        let has_focus = response.has_focus();
        let fill = if profile_ui.auth_busy {
            ui.visuals().widgets.active.weak_bg_fill
        } else if response.is_pointer_button_down_on() {
            ui.visuals().widgets.active.weak_bg_fill
        } else if response.hovered() || has_focus {
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
            if has_focus {
                ui.visuals().selection.stroke
            } else {
                egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color)
            },
            egui::StrokeKind::Inside,
        );
        if has_focus {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                egui::CornerRadius::same(PROFILE_BUTTON_CORNER_RADIUS.saturating_add(2)),
                egui::Stroke::new(
                    (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                    ui.visuals().selection.stroke.color,
                ),
                egui::StrokeKind::Outside,
            );
        }
        if let image_textures::ManagedTextureStatus::Ready(texture) =
            image_textures::request_texture(ui.ctx(), key, rounded, egui::TextureOptions::LINEAR)
        {
            let icon = egui::Image::from_texture(&texture)
                .fit_to_exact_size(egui::vec2(button_size.max(1.0), button_size.max(1.0)));
            let _ = ui.put(rect, icon);
        }
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

fn rounded_profile_avatar_png(cache_key: u64, avatar_png: &[u8], radius: u8) -> Arc<[u8]> {
    static CACHE: OnceLock<ThreadSafeLru<u64, Arc<[u8]>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| ThreadSafeLru::new(usize::MAX));

    if let Some(bytes) = cache.write(|state| {
        state
            .touch(&cache_key)
            .map(|entry| Arc::clone(&entry.value))
    }) {
        return bytes;
    }

    let rounded = Arc::<[u8]>::from(
        round_avatar_png_bytes(avatar_png, radius).unwrap_or_else(|| avatar_png.to_vec()),
    );

    cache.write(|state| {
        state.insert_without_eviction(cache_key, Arc::clone(&rounded), rounded.len());
        while state.len() > ROUNDED_AVATAR_CACHE_MAX_ENTRIES {
            if state.pop_lru().is_none() {
                break;
            }
        }
    });

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
    button_width: f32,
    compact: bool,
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
        egui::vec2(button_width, button_height),
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
                    let _ = text_ui.label(
                        ui,
                        "topbar_user_active_label",
                        if compact { "active" } else { "user active" },
                        &label_style,
                    );
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
    let qr_url = prompt.verification_url();

    let instruction_text = "Scan this QR code to continue with device code on another device, or use browser sign-in on this device to avoid typing the code manually.";

    // Instructional text — wraps naturally, normal size
    let instruction_style = LabelOptions {
        color: ui.visuals().text_color(),
        wrap: true,
        ..LabelOptions::default()
    };
    let _ = text_ui.label(
        ui,
        "device_code_instruction",
        instruction_text,
        &instruction_style,
    );

    // QR code: pre-allocate exact rect so nothing can expand outward
    let qr_size = (full_action_width * 0.7).clamp(120.0, 220.0);
    ui.allocate_ui_with_layout(
        egui::vec2(full_action_width, qr_size),
        Layout::top_down(Align::Center),
        |ui| {
            render_qr_code(ui, &qr_url, qr_size);
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

    // Bottom row: Cancel (left half) + Open Browser (right half)
    let half = (full_action_width - style::SPACE_MD) / 2.0;
    ui.allocate_ui_with_layout(
        egui::vec2(full_action_width, style::CONTROL_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = style::SPACE_MD;
            let mut half_style = button_style.clone();
            half_style.min_size = egui::vec2(half, style::CONTROL_HEIGHT);
            if text_ui
                .button(ui, "device_code_cancel", "Cancel", &half_style)
                .clicked()
            {
                output.cancel_device_code_sign_in = true;
            }
            let open_browser_label = "Use Browser Sign-in";
            if text_ui
                .button(
                    ui,
                    "device_code_open_browser",
                    open_browser_label,
                    &half_style,
                )
                .clicked()
            {
                output.open_device_code_browser = true;
            }
        },
    );
}

fn render_qr_code(ui: &mut egui::Ui, text: &str, size: f32) {
    use qrcode::QrCode;
    use qrcode::types::Color as QrColor;
    use resvg::tiny_skia::{self, Paint, Transform};

    // Cache by text content — only rasterise once per unique URL
    let cache_key = ui.make_persistent_id(("qr_fancy_v1", text));
    let cached: Option<egui::TextureHandle> = ui.ctx().data_mut(|d| d.get_temp(cache_key));

    let texture = if let Some(t) = cached {
        t
    } else {
        let Ok(code) = QrCode::new(text.as_bytes()) else {
            return;
        };
        let module_count = code.width();
        let colors = code.to_colors();

        let dark = |row: i32, col: i32| -> bool {
            if row < 0 || col < 0 || row >= module_count as i32 || col >= module_count as i32 {
                return false;
            }
            colors[row as usize * module_count + col as usize] == QrColor::Dark
        };

        let quiet = 2usize; // quiet zone in modules
        let total_modules = module_count + quiet * 2;
        // Supersample 4× then downsample with Lanczos3 for clean anti-aliasing
        let ss: u32 = 4;
        let module_px: u32 = 10; // target pixels per module in the final texture
        let img_px = total_modules as u32 * module_px;
        let ss_module_px = module_px * ss;
        let ss_img_px = img_px * ss;
        let corner_r = ss_module_px as f32 * 0.40;

        let Some(mut pixmap) = tiny_skia::Pixmap::new(ss_img_px, ss_img_px) else {
            return;
        };
        pixmap.fill(tiny_skia::Color::WHITE);

        let mut paint = Paint::default();
        paint.set_color_rgba8(26, 26, 46, 255); // near-black with slight blue tint
        paint.anti_alias = true;

        for row in 0..module_count {
            for col in 0..module_count {
                if !dark(row as i32, col as i32) {
                    continue;
                }

                let x = (col + quiet) as f32 * ss_module_px as f32;
                let y = (row + quiet) as f32 * ss_module_px as f32;
                let s = ss_module_px as f32;

                let row = row as i32;
                let col = col as i32;
                let top = dark(row - 1, col);
                let bot = dark(row + 1, col);
                let left = dark(row, col - 1);
                let right = dark(row, col + 1);

                // A corner is only rounded when NEITHER neighbour along that corner is dark,
                // i.e. it is a true outer convex corner of the merged shape.
                let r_nw = if top || left { 0.0 } else { corner_r };
                let r_ne = if top || right { 0.0 } else { corner_r };
                let r_sw = if bot || left { 0.0 } else { corner_r };
                let r_se = if bot || right { 0.0 } else { corner_r };

                if let Some(path) = qr_rounded_rect(x, y, s, s, r_nw, r_ne, r_sw, r_se) {
                    pixmap.fill_path(
                        &path,
                        &paint,
                        tiny_skia::FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }

                // Concave (inner) corner rounding — carve a white arc into the dark shape
                // where two edge-neighbours are dark but their shared diagonal is light.
                // The white arc points INWARD (opposite quadrant to the notch direction).
                let cx = x;
                let cy = y;
                let cr = corner_r;
                if top && left && !dark(row - 1, col - 1) {
                    if let Some(p) = qr_concave_arc(cx, cy, cr, 0) {
                        pixmap.fill_path(
                            &p,
                            &paint,
                            tiny_skia::FillRule::Winding,
                            Transform::identity(),
                            None,
                        );
                    }
                }
                if top && right && !dark(row - 1, col + 1) {
                    if let Some(p) = qr_concave_arc(cx + s, cy, cr, 1) {
                        pixmap.fill_path(
                            &p,
                            &paint,
                            tiny_skia::FillRule::Winding,
                            Transform::identity(),
                            None,
                        );
                    }
                }
                if bot && left && !dark(row + 1, col - 1) {
                    if let Some(p) = qr_concave_arc(cx, cy + s, cr, 2) {
                        pixmap.fill_path(
                            &p,
                            &paint,
                            tiny_skia::FillRule::Winding,
                            Transform::identity(),
                            None,
                        );
                    }
                }
                if bot && right && !dark(row + 1, col + 1) {
                    if let Some(p) = qr_concave_arc(cx + s, cy + s, cr, 3) {
                        pixmap.fill_path(
                            &p,
                            &paint,
                            tiny_skia::FillRule::Winding,
                            Transform::identity(),
                            None,
                        );
                    }
                }
            }
        }

        // Downsample the supersampled pixmap to the target resolution
        let downscaled = image::imageops::resize(
            &image::RgbaImage::from_raw(ss_img_px, ss_img_px, pixmap.data().to_vec())
                .unwrap_or_default(),
            img_px,
            img_px,
            image::imageops::FilterType::Lanczos3,
        );
        let color_image = egui::ColorImage::from_rgba_premultiplied(
            [img_px as usize, img_px as usize],
            downscaled.as_raw(),
        );
        let t = ui.ctx().load_texture(
            format!("qr_fancy_{}", text.len()),
            color_image,
            egui::TextureOptions::LINEAR,
        );
        ui.ctx().data_mut(|d| d.insert_temp(cache_key, t.clone()));
        t
    };

    let resp = ui.add(
        egui::Image::from_texture(egui::load::SizedTexture::from_handle(&texture))
            .fit_to_exact_size(egui::vec2(size, size))
            .corner_radius(egui::CornerRadius::same(8)),
    );
    // Border around the QR
    ui.painter().rect_stroke(
        resp.rect,
        egui::CornerRadius::same(8),
        egui::Stroke::new(1.5, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Outside,
    );
}

/// Builds a tiny_skia path for a rectangle with independent per-corner radii.
fn qr_rounded_rect(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r_nw: f32,
    r_ne: f32,
    r_sw: f32,
    r_se: f32,
) -> Option<resvg::tiny_skia::Path> {
    use resvg::tiny_skia::PathBuilder;
    // Cubic bezier approximation constant for a quarter-circle
    const K: f32 = 0.5523;
    let mut pb = PathBuilder::new();

    pb.move_to(x + r_nw, y);

    // top edge → NE corner
    pb.line_to(x + w - r_ne, y);
    if r_ne > 0.0 {
        pb.cubic_to(
            x + w - r_ne * (1.0 - K),
            y,
            x + w,
            y + r_ne * (1.0 - K),
            x + w,
            y + r_ne,
        );
    } else {
        pb.line_to(x + w, y);
    }

    // right edge → SE corner
    pb.line_to(x + w, y + h - r_se);
    if r_se > 0.0 {
        pb.cubic_to(
            x + w,
            y + h - r_se * (1.0 - K),
            x + w - r_se * (1.0 - K),
            y + h,
            x + w - r_se,
            y + h,
        );
    } else {
        pb.line_to(x + w, y + h);
    }

    // bottom edge → SW corner
    pb.line_to(x + r_sw, y + h);
    if r_sw > 0.0 {
        pb.cubic_to(
            x + r_sw * (1.0 - K),
            y + h,
            x,
            y + h - r_sw * (1.0 - K),
            x,
            y + h - r_sw,
        );
    } else {
        pb.line_to(x, y + h);
    }

    // left edge → NW corner
    pb.line_to(x, y + r_nw);
    if r_nw > 0.0 {
        pb.cubic_to(
            x,
            y + r_nw * (1.0 - K),
            x + r_nw * (1.0 - K),
            y,
            x + r_nw,
            y,
        );
    }

    pb.close();
    pb.finish()
}

/// Expands the dark shape into the white notch at an inner corner.
/// Each path walks out along the two dark boundary lines then arcs back toward
/// the corner — ctrl points bow the arc *inward* so the new boundary is concave
/// from outside (smooth, not bumpy).
/// `quadrant`: 0=NW, 1=NE, 2=SW, 3=SE — which direction the notch faces.
fn qr_concave_arc(cx: f32, cy: f32, r: f32, quadrant: u8) -> Option<resvg::tiny_skia::Path> {
    use resvg::tiny_skia::PathBuilder;
    const K: f32 = 0.5523;
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy);
    match quadrant {
        0 => {
            // NW: up along boundary, arc left — ctrl points pull arc toward corner
            pb.line_to(cx, cy - r);
            pb.cubic_to(cx, cy - r + K * r, cx - r + K * r, cy, cx - r, cy);
        }
        1 => {
            // NE: up along boundary, arc right
            pb.line_to(cx, cy - r);
            pb.cubic_to(cx, cy - r + K * r, cx + r - K * r, cy, cx + r, cy);
        }
        2 => {
            // SW: down along boundary, arc left
            pb.line_to(cx, cy + r);
            pb.cubic_to(cx, cy + r - K * r, cx - r + K * r, cy, cx - r, cy);
        }
        _ => {
            // SE: down along boundary, arc right
            pb.line_to(cx, cy + r);
            pb.cubic_to(cx, cy + r - K * r, cx + r - K * r, cy, cx + r, cy);
        }
    }
    pb.close();
    pb.finish()
}

fn render_profile_popup(
    ui: &mut egui::Ui,
    text_ui: &mut TextUi,
    profile_ui: ProfileUiModel<'_>,
    output: &mut TopBarOutput,
    popup_id: egui::Id,
    _owner_id: egui::Id,
    compact: bool,
) {
    let focus_first = ui.ctx().data_mut(|data| {
        let key = egui::Id::new((PROFILE_POPUP_FOCUS_FIRST_KEY, popup_id));
        let value = data.get_temp::<bool>(key).unwrap_or(false);
        if value {
            data.remove::<bool>(key);
        }
        value
    });
    let mut first_focus_applied = false;
    let request_owner_focus = |ui: &egui::Ui| {
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new((PROFILE_POPUP_OWNER_FOCUS_KEY, popup_id)),
                true,
            );
        });
    };
    ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_MD, style::SPACE_MD);
    let full_action_width = if compact {
        ui.available_width().max(160.0)
    } else {
        ui.available_width().max(220.0)
    };

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
                            if account.is_failed {
                                let refresh_response = ui
                                    .add_enabled_ui(!profile_ui.auth_busy, |ui| {
                                        text_ui.button(
                                            ui,
                                            ("profile_popup_account_refresh", &account.profile_id),
                                            "Refresh",
                                            &refresh_button_style,
                                        )
                                    })
                                    .inner;
                                if focus_first && !first_focus_applied {
                                    refresh_response.request_focus();
                                    first_focus_applied = true;
                                }
                                if refresh_response.clicked() {
                                    output.refresh_account_id = Some(account.profile_id.clone());
                                    request_owner_focus(ui);
                                }
                            }

                            let remove_response = text_ui.button(
                                ui,
                                ("profile_popup_account_remove", &account.profile_id),
                                "Remove",
                                &remove_button_style,
                            );
                            if focus_first && !first_focus_applied {
                                remove_response.request_focus();
                                first_focus_applied = true;
                            }
                            if remove_response.clicked() {
                                output.remove_account_id = Some(account.profile_id.clone());
                                request_owner_focus(ui);
                            }

                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let mut fill_style = list_button_style.clone();
                                    fill_style.min_size.x = ui.available_width().max(1.0);
                                    let select_response = text_ui.selectable_button(
                                        ui,
                                        ("profile_popup_account_select", &account.profile_id),
                                        &label,
                                        account.is_active,
                                        &fill_style,
                                    );
                                    if focus_first && !first_focus_applied {
                                        select_response.request_focus();
                                        first_focus_applied = true;
                                    }
                                    if select_response.clicked() {
                                        output.select_account_id = Some(account.profile_id.clone());
                                        request_owner_focus(ui);
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
                let webview_response = text_ui.button(
                    ui,
                    "profile_popup_signin_webview",
                    "Webview Sign-in",
                    &half_button_style,
                );
                if focus_first && !first_focus_applied {
                    webview_response.request_focus();
                    first_focus_applied = true;
                }
                if webview_response.clicked() {
                    output.start_webview_sign_in = true;
                    egui::Popup::open_id(ui.ctx(), popup_id);
                }

                let device_code_response = text_ui.button(
                    ui,
                    "profile_popup_signin_device_code",
                    "Device Code Sign-in",
                    &half_button_style,
                );
                if focus_first && !first_focus_applied {
                    device_code_response.request_focus();
                    first_focus_applied = true;
                }
                if device_code_response.clicked() {
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

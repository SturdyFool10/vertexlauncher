use egui::{
    Area, Color32, Context, CornerRadius, CursorIcon, FontId, Id, Key, Order, Pos2, Rect, Sense,
    pos2, vec2,
};
use textui::TextUi;
use textui_egui::prelude::*;

use crate::ui::{motion, style};

#[derive(Clone, Debug)]
pub struct ContextMenuItem {
    pub action_id: String,
    pub label: String,
    pub icon_svg: Option<Vec<u8>>,
    pub danger: bool,
}

impl ContextMenuItem {
    pub fn new(action_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            action_id: action_id.into(),
            label: label.into(),
            icon_svg: None,
            danger: false,
        }
    }

    pub fn new_with_icon(
        action_id: impl Into<String>,
        label: impl Into<String>,
        icon_svg: impl Into<Vec<u8>>,
    ) -> Self {
        Self::new(action_id, label).with_icon(icon_svg)
    }

    pub fn with_icon(mut self, icon_svg: impl Into<Vec<u8>>) -> Self {
        self.icon_svg = Some(icon_svg.into());
        self
    }

    pub fn with_svg(self, icon_svg: impl Into<Vec<u8>>) -> Self {
        self.with_icon(icon_svg)
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

#[derive(Clone, Debug)]
pub struct ContextMenuRequest {
    pub source_id: Id,
    pub anchor_pos: Pos2,
    pub items: Vec<ContextMenuItem>,
}

impl ContextMenuRequest {
    pub fn new(source_id: Id, anchor_pos: Pos2, items: Vec<ContextMenuItem>) -> Self {
        Self {
            source_id,
            anchor_pos,
            items,
        }
    }
}

#[derive(Clone, Debug)]
struct ContextMenuInvocation {
    source_id: Id,
    action_id: String,
}

#[derive(Clone, Debug, Default)]
struct ContextMenuState {
    request: Option<ContextMenuRequest>,
    open: bool,
}

const STATE_ID: &str = "app_context_menu_state";
const INVOCATION_ID: &str = "app_context_menu_invocation";
const ANIM_ID: &str = "app_context_menu_anim";
const MENU_MIN_WIDTH: f32 = 140.0;
const MENU_MARGIN: f32 = 8.0;
const MENU_PADDING: f32 = 8.0;
const MENU_ITEM_HEIGHT: f32 = 30.0;
const MENU_SLIDE_DISTANCE: f32 = 14.0;
const MENU_COLLAPSED_HEIGHT: f32 = 2.0;
const MENU_MAX_HEIGHT: f32 = 420.0;
const MENU_MAX_HEIGHT_SCREEN_FRACTION: f32 = 0.55;
const SCROLLBAR_WIDTH_ALLOWANCE: f32 = 12.0;

fn linear_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - ((-2.0 * t + 2.0).powi(2) / 2.0)
    }
}

fn opaque(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 255)
}

fn resolved_anchor_pos(ctx: &Context, requested: Pos2) -> Pos2 {
    let content_rect = ctx.content_rect();
    let requested_ok = requested.x.is_finite()
        && requested.y.is_finite()
        && requested.x > content_rect.left() - 1.0
        && requested.y > content_rect.top() - 1.0;
    let fallback = ctx.input(|i| {
        i.pointer
            .latest_pos()
            .or(i.pointer.interact_pos())
            .or(i.pointer.press_origin())
    });
    let pos = if requested_ok {
        requested
    } else {
        fallback.unwrap_or(content_rect.center())
    };
    pos2(
        pos.x.clamp(
            content_rect.left() + MENU_MARGIN,
            content_rect.right() - MENU_MARGIN,
        ),
        pos.y.clamp(
            content_rect.top() + MENU_MARGIN,
            content_rect.bottom() - MENU_MARGIN,
        ),
    )
}

fn icon_slot_width(ctx: &Context, row_height: f32) -> f32 {
    let pixels_per_point = ctx.pixels_per_point();
    let scaled = row_height * pixels_per_point * 0.58;
    (scaled / pixels_per_point).clamp(14.0, 22.0)
}

fn menu_max_height(screen_rect: Rect) -> f32 {
    (screen_rect.height() * MENU_MAX_HEIGHT_SCREEN_FRACTION)
        .min(MENU_MAX_HEIGHT)
        .max(MENU_ITEM_HEIGHT + MENU_PADDING * 2.0)
}

pub fn request(ctx: &Context, request: ContextMenuRequest) {
    if request.items.is_empty() {
        close(ctx);
        return;
    }

    ctx.data_mut(|data| {
        data.insert_temp(
            Id::new(STATE_ID),
            ContextMenuState {
                request: Some(request),
                open: true,
            },
        );
    });
    ctx.request_repaint();
}

pub fn close(ctx: &Context) {
    let mut state = ctx
        .data_mut(|data| data.get_temp::<ContextMenuState>(Id::new(STATE_ID)))
        .unwrap_or_default();
    state.open = false;
    ctx.data_mut(|data| data.insert_temp(Id::new(STATE_ID), state));
    ctx.request_repaint();
}

pub fn take_invocation(ctx: &Context, source_id: Id) -> Option<String> {
    let invocation = ctx
        .data_mut(|data| data.get_temp::<Option<ContextMenuInvocation>>(Id::new(INVOCATION_ID)))
        .flatten();

    if let Some(invocation) = invocation {
        if invocation.source_id == source_id {
            ctx.data_mut(|data| {
                data.insert_temp(Id::new(INVOCATION_ID), None::<ContextMenuInvocation>);
            });
            return Some(invocation.action_id);
        }
    }

    None
}

pub fn show(ctx: &Context, text_ui: &mut TextUi) {
    let mut state = ctx
        .data_mut(|data| data.get_temp::<ContextMenuState>(Id::new(STATE_ID)))
        .unwrap_or_default();

    let target_open = state.open && state.request.is_some();
    let progress = motion::progress(ctx, Id::new(ANIM_ID), target_open);
    let eased = linear_in_out(progress);

    if (target_open && progress < 1.0) || (!target_open && progress > 0.0) {
        ctx.request_repaint();
    }

    let Some(request) = state.request.clone() else {
        return;
    };

    if !target_open && progress <= 0.001 {
        state.request = None;
        ctx.data_mut(|data| data.insert_temp(Id::new(STATE_ID), state));
        return;
    }

    let screen_rect = ctx.content_rect();
    let anchor_pos = resolved_anchor_pos(ctx, request.anchor_pos);
    let label_font = FontId::proportional(14.0);
    let visuals = ctx.global_style().visuals.clone();
    let normal_label_color = visuals.text_color();
    let danger_color = visuals.error_fg_color;

    let row_left_spacing = style::SPACE_XS;
    let row_gap_spacing = style::SPACE_XS;
    let row_right_spacing = style::SPACE_XS;

    let has_any_icon = request.items.iter().any(|item| item.icon_svg.is_some());
    let shared_icon_width = if has_any_icon {
        icon_slot_width(ctx, MENU_ITEM_HEIGHT)
    } else {
        0.0
    };

    let widest_label = ctx.fonts_mut(|fonts| {
        request
            .items
            .iter()
            .map(|item| {
                let color = if item.danger {
                    danger_color
                } else {
                    normal_label_color
                };
                fonts
                    .layout_no_wrap(item.label.clone(), label_font.clone(), color)
                    .size()
                    .x
            })
            .fold(0.0, f32::max)
    });

    let row_content_width = widest_label
        + row_left_spacing
        + row_right_spacing
        + if has_any_icon {
            row_gap_spacing + shared_icon_width
        } else {
            0.0
        };

    let visible_max_height =
        (screen_rect.height() - MENU_MARGIN * 2.0).max(MENU_ITEM_HEIGHT + MENU_PADDING * 2.0);
    let max_height = menu_max_height(screen_rect).min(visible_max_height);

    let full_content_height = request.items.len() as f32 * MENU_ITEM_HEIGHT;
    let needs_scroll = full_content_height + MENU_PADDING * 2.0 > max_height;
    let scrollbar_allowance = if needs_scroll {
        SCROLLBAR_WIDTH_ALLOWANCE
    } else {
        0.0
    };

    let desired_width = MENU_PADDING * 2.0 + row_content_width + scrollbar_allowance;
    let width = desired_width
        .max(MENU_MIN_WIDTH)
        .min((screen_rect.width() - MENU_MARGIN * 2.0).max(MENU_MIN_WIDTH));

    let final_height = (full_content_height + MENU_PADDING * 2.0).min(max_height);
    let current_height = MENU_COLLAPSED_HEIGHT + (final_height - MENU_COLLAPSED_HEIGHT) * eased;

    let left = anchor_pos.x.clamp(
        screen_rect.left() + MENU_MARGIN,
        screen_rect.right() - width - MENU_MARGIN,
    );
    let top = anchor_pos.y.clamp(
        screen_rect.top() + MENU_MARGIN,
        screen_rect.bottom() - final_height - MENU_MARGIN,
    );

    let frame_rect = Rect::from_min_size(pos2(left, top), vec2(width, current_height));
    let final_frame_rect = Rect::from_min_size(pos2(left, top), vec2(width, final_height));

    let mut close_due_to_outside_click = false;
    if target_open && ctx.input(|i| i.key_pressed(Key::Escape)) {
        close_due_to_outside_click = true;
    }
    if target_open && ctx.input(|i| i.pointer.primary_pressed()) {
        if let Some(press_origin) = ctx.input(|i| i.pointer.press_origin()) {
            if !final_frame_rect.contains(press_origin) {
                close_due_to_outside_click = true;
            }
        }
    }
    if close_due_to_outside_click {
        state.open = false;
        ctx.data_mut(|data| data.insert_temp(Id::new(STATE_ID), state.clone()));
    }

    let fill = opaque(visuals.panel_fill);
    let stroke = visuals.widgets.noninteractive.bg_stroke;
    let hover_fill = opaque(visuals.widgets.hovered.weak_bg_fill);
    let active_fill = opaque(visuals.widgets.active.weak_bg_fill);
    let danger_hover_fill =
        Color32::from_rgba_unmultiplied(danger_color.r(), danger_color.g(), danger_color.b(), 26);
    let danger_active_fill =
        Color32::from_rgba_unmultiplied(danger_color.r(), danger_color.g(), danger_color.b(), 44);
    let shadow_color = Color32::from_black_alpha((56.0 * eased) as u8);
    let corner_radius = CornerRadius::same(style::CORNER_RADIUS_MD);

    if target_open
        && ctx.input(|i| {
            i.pointer
                .hover_pos()
                .is_some_and(|pos| final_frame_rect.contains(pos))
        })
    {
        ctx.set_cursor_icon(CursorIcon::Default);
    }

    let area_id = Id::new((
        "app_context_menu_area",
        request.source_id,
        anchor_pos.x.to_bits(),
        anchor_pos.y.to_bits(),
        request.items.len(),
    ));

    Area::new(area_id)
        .order(Order::Foreground)
        .interactable(true)
        .movable(false)
        .fixed_pos(frame_rect.min)
        .show(ctx, |ui| {
            ui.set_min_size(frame_rect.size());

            let area_rect = Rect::from_min_size(ui.min_rect().min, frame_rect.size());
            let shadow_rect = area_rect.expand(6.0);
            ui.painter()
                .rect_filled(shadow_rect, corner_radius, shadow_color);
            ui.painter().rect_filled(area_rect, corner_radius, fill);
            ui.painter()
                .rect_stroke(area_rect, corner_radius, stroke, egui::StrokeKind::Outside);

            let inner_rect = area_rect.shrink(MENU_PADDING);
            let clip_rect = Rect::from_min_max(
                inner_rect.min,
                pos2(inner_rect.max.x, area_rect.max.y - MENU_PADDING),
            );
            let mut selected_action: Option<String> = None;
            let content_offset_y = (1.0 - eased) * MENU_SLIDE_DISTANCE;
            let builder = egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(inner_rect.min.x, inner_rect.min.y + content_offset_y),
                    vec2(inner_rect.width(), inner_rect.height()),
                ))
                .layout(*ui.layout());

            ui.scope_builder(builder, |ui| {
                let old_clip = ui.clip_rect();
                ui.set_clip_rect(old_clip.intersect(clip_rect));

                egui::ScrollArea::vertical()
                    .id_salt(("app_context_menu_scroll", request.source_id))
                    .auto_shrink([false, false])
                    .max_height((current_height - MENU_PADDING * 2.0).max(MENU_ITEM_HEIGHT))
                    .show(ui, |ui| {
                        ui.set_width(inner_rect.width());

                        for item in &request.items {
                            let (item_rect, response) = ui.allocate_exact_size(
                                vec2(inner_rect.width(), MENU_ITEM_HEIGHT),
                                Sense::click(),
                            );

                            let item_fill = if item.danger {
                                if response.is_pointer_button_down_on() {
                                    danger_active_fill
                                } else if response.hovered() {
                                    danger_hover_fill
                                } else {
                                    Color32::TRANSPARENT
                                }
                            } else if response.is_pointer_button_down_on() {
                                active_fill
                            } else if response.hovered() {
                                hover_fill
                            } else {
                                Color32::TRANSPARENT
                            };

                            ui.painter().rect_filled(
                                item_rect,
                                CornerRadius::same(style::CORNER_RADIUS_SM),
                                item_fill,
                            );

                            let item_color = if item.danger {
                                danger_color
                            } else {
                                normal_label_color
                            };

                            let text_left = item_rect.min.x + row_left_spacing;
                            let icon_right = item_rect.max.x - row_right_spacing;
                            let icon_left = if has_any_icon {
                                icon_right - shared_icon_width
                            } else {
                                icon_right
                            };

                            if let Some(icon_svg) = item.icon_svg.as_ref() {
                                let icon_rect = Rect::from_min_size(
                                    pos2(icon_left, item_rect.center().y - shared_icon_width * 0.5),
                                    vec2(shared_icon_width, shared_icon_width),
                                );
                                let icon = egui::Image::from_bytes(
                                    format!(
                                        "bytes://context-menu/{}-{:02x}{:02x}{:02x}.svg",
                                        item.action_id,
                                        item_color.r(),
                                        item_color.g(),
                                        item_color.b(),
                                    ),
                                    apply_color_to_svg(icon_svg, item_color),
                                )
                                .fit_to_exact_size(icon_rect.size());
                                let _ = ui.put(icon_rect, icon);
                            }

                            let label_rect = Rect::from_min_max(
                                pos2(text_left, item_rect.top()),
                                pos2(item_rect.right() - 10.0, item_rect.bottom()),
                            );
                            let label_style = LabelOptions {
                                font_size: 14.0,
                                line_height: 18.0,
                                color: item_color,
                                wrap: false,
                                ..style::body_strong(ui)
                            };
                            ui.scope_builder(egui::UiBuilder::new().max_rect(label_rect), |ui| {
                                ui.set_clip_rect(label_rect.intersect(ui.clip_rect()));
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let _ = text_ui.label(
                                            ui,
                                            ("context_menu_item_label", item.action_id.as_str()),
                                            item.label.as_str(),
                                            &label_style,
                                        );
                                    },
                                );
                            });

                            if response.hovered() {
                                ui.ctx().set_cursor_icon(CursorIcon::Default);
                            }

                            if response.clicked() {
                                selected_action = Some(item.action_id.clone());
                            }
                        }
                    });
            });

            if let Some(action_id) = selected_action {
                ctx.data_mut(|data| {
                    data.insert_temp(
                        Id::new(INVOCATION_ID),
                        Some(ContextMenuInvocation {
                            source_id: request.source_id,
                            action_id,
                        }),
                    );
                });
                state.open = false;
                ctx.data_mut(|data| data.insert_temp(Id::new(STATE_ID), state.clone()));
                ctx.request_repaint();
            }
        });

    ctx.data_mut(|data| data.insert_temp(Id::new(STATE_ID), state));
}

fn apply_color_to_svg(svg_bytes: &[u8], color: Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}

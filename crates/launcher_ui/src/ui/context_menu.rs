use egui::{
    pos2, vec2, Align2, Area, Color32, Context, CornerRadius, CursorIcon, FontId, Id, Key,
    Order, Pos2, Rect, Sense,
};

use crate::ui::{motion, style};

#[derive(Clone, Debug)]
pub struct ContextMenuItem {
    pub action_id: String,
    pub label: String,
}

impl ContextMenuItem {
    pub fn new(action_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            action_id: action_id.into(),
            label: label.into(),
        }
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
const MENU_MAX_WIDTH: f32 = 360.0;
const MENU_MARGIN: f32 = 8.0;
const MENU_PADDING: f32 = 8.0;
const MENU_ITEM_HEIGHT: f32 = 30.0;
const MENU_ITEM_HORIZONTAL_PADDING: f32 = 10.0;
const MENU_SLIDE_DISTANCE: f32 = 14.0;
const MENU_COLLAPSED_HEIGHT: f32 = 2.0;

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
        pos.x.clamp(content_rect.left() + MENU_MARGIN, content_rect.right() - MENU_MARGIN),
        pos.y.clamp(content_rect.top() + MENU_MARGIN, content_rect.bottom() - MENU_MARGIN),
    )
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
                data.insert_temp(Id::new(INVOCATION_ID), None::<ContextMenuInvocation>)
            });
            return Some(invocation.action_id);
        }
    }

    None
}

pub fn show(ctx: &Context) {
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
    let label_color = ctx.style().visuals.text_color();

    let widest_label = ctx.fonts_mut(|fonts| {
        request
            .items
            .iter()
            .map(|item| {
                fonts
                    .layout_no_wrap(item.label.clone(), label_font.clone(), label_color)
                    .size()
                    .x
            })
            .fold(0.0, f32::max)
    });

    let width = (widest_label + MENU_PADDING * 2.0 + MENU_ITEM_HORIZONTAL_PADDING * 2.0)
        .max(MENU_MIN_WIDTH)
        .min(MENU_MAX_WIDTH)
        .min((screen_rect.width() - MENU_MARGIN * 2.0).max(MENU_MIN_WIDTH));

    let available_below = (screen_rect.bottom() - anchor_pos.y - MENU_MARGIN)
        .max(MENU_ITEM_HEIGHT + MENU_PADDING * 2.0);
    let full_content_height = request.items.len() as f32 * MENU_ITEM_HEIGHT;
    let final_height = (full_content_height + MENU_PADDING * 2.0)
        .min(available_below)
        .min(screen_rect.height() - MENU_MARGIN * 2.0);

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

    let visuals = ctx.style().visuals.clone();
    let fill = opaque(visuals.panel_fill);
    let stroke = visuals.widgets.noninteractive.bg_stroke;
    let hover_fill = opaque(visuals.widgets.hovered.weak_bg_fill);
    let active_fill = opaque(visuals.widgets.active.weak_bg_fill);
    let shadow_color = Color32::from_black_alpha((56.0 * eased) as u8);
    let corner_radius = CornerRadius::same(style::CORNER_RADIUS_MD);

    if target_open
        && ctx.input(|i| i.pointer.hover_pos().is_some_and(|pos| final_frame_rect.contains(pos)))
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

                            let item_fill = if response.is_pointer_button_down_on() {
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

                            let text_pos = pos2(
                                item_rect.min.x + MENU_ITEM_HORIZONTAL_PADDING,
                                item_rect.center().y,
                            );

                            ui.painter().text(
                                text_pos,
                                Align2::LEFT_CENTER,
                                item.label.as_str(),
                                label_font.clone(),
                                label_color,
                            );

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

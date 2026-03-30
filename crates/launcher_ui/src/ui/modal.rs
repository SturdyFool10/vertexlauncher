use egui::{Color32, Context, CornerRadius, Frame, Id, Margin, Order, Rect, Stroke};

const MODAL_CORNER_RADIUS: u8 = 14;
const MODAL_INNER_MARGIN: i8 = 14;
const MODAL_SCRIM_ALPHA: u8 = 160;

pub fn show_scrim(ctx: &Context, id: impl std::hash::Hash, viewport_rect: Rect) {
    let area_id = Id::new((&id, "modal_scrim_area"));
    let blocker_id = Id::new((&id, "modal_scrim_blocker"));
    egui::Area::new(area_id)
        .order(Order::Foreground)
        .fixed_pos(viewport_rect.min)
        .interactable(true)
        .show(ctx, |ui| {
            let local_rect = Rect::from_min_size(egui::Pos2::ZERO, viewport_rect.size());
            let _ = ui.interact(local_rect, blocker_id, egui::Sense::click_and_drag());
            ui.painter().rect_filled(
                local_rect,
                CornerRadius::ZERO,
                Color32::from_rgba_premultiplied(0, 0, 0, MODAL_SCRIM_ALPHA),
            );
        });
}

pub fn window_frame(ctx: &Context) -> Frame {
    let base = ctx.style().visuals.window_fill;
    Frame::new()
        .fill(Color32::from_rgba_premultiplied(
            base.r(),
            base.g(),
            base.b(),
            255,
        ))
        .stroke(Stroke::new(
            1.0,
            ctx.style().visuals.widgets.hovered.bg_stroke.color,
        ))
        .corner_radius(CornerRadius::same(MODAL_CORNER_RADIUS))
        .inner_margin(Margin::same(MODAL_INNER_MARGIN))
}

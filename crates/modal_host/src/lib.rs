use egui::{Area, Color32, Context, CornerRadius, Frame, Id, Margin, Order, Rect, Stroke};

const MODAL_CORNER_RADIUS: u8 = 14;
const MODAL_INNER_MARGIN: i8 = 14;
const MODAL_SCRIM_ALPHA: u8 = 160;
const MODAL_HOST_STATE_ID: &str = "modal_host_state";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModalLayer {
    Base = 0,
    Elevated = 100,
    Blocking = 200,
    Critical = 300,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissBehavior {
    None,
    Escape,
    Scrim,
    EscapeAndScrim,
}

impl DismissBehavior {
    fn allows_escape(self) -> bool {
        matches!(self, Self::Escape | Self::EscapeAndScrim)
    }

    fn allows_scrim(self) -> bool {
        matches!(self, Self::Scrim | Self::EscapeAndScrim)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AxisSizing {
    viewport_fraction: f32,
    min: f32,
    max: f32,
}

impl AxisSizing {
    pub fn new(viewport_fraction: f32, min: f32, max: f32) -> Self {
        Self {
            viewport_fraction,
            min,
            max,
        }
    }

    fn resolve(self, available: f32) -> f32 {
        let target = available * self.viewport_fraction.clamp(0.0, 1.0);
        target
            .clamp(self.min.max(1.0), self.max.max(self.min))
            .min(available)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ModalLayout {
    pub width: AxisSizing,
    pub height: AxisSizing,
    pub anchor: egui::Align2,
    pub offset: egui::Vec2,
    pub viewport_margin: egui::Vec2,
    pub viewport_margin_fraction: egui::Vec2,
}

impl ModalLayout {
    pub fn centered(width: AxisSizing, height: AxisSizing) -> Self {
        Self {
            width,
            height,
            anchor: egui::Align2::CENTER_CENTER,
            offset: egui::Vec2::ZERO,
            viewport_margin: egui::Vec2::ZERO,
            viewport_margin_fraction: egui::Vec2::ZERO,
        }
    }

    pub fn with_viewport_margin(mut self, margin: egui::Vec2) -> Self {
        self.viewport_margin = margin;
        self
    }

    pub fn with_viewport_margin_fraction(mut self, margin_fraction: egui::Vec2) -> Self {
        self.viewport_margin_fraction = margin_fraction;
        self
    }

    fn resolve_rect(self, viewport_rect: Rect) -> Rect {
        let fractional_margin = egui::vec2(
            viewport_rect.width() * self.viewport_margin_fraction.x.clamp(0.0, 0.49),
            viewport_rect.height() * self.viewport_margin_fraction.y.clamp(0.0, 0.49),
        );
        let constrained = viewport_rect.shrink2(self.viewport_margin + fractional_margin);
        let size = egui::vec2(
            self.width.resolve(constrained.width().max(1.0)),
            self.height.resolve(constrained.height().max(1.0)),
        );
        let anchor_x = match self.anchor.x() {
            egui::Align::Min => constrained.left(),
            egui::Align::Center => constrained.center().x - size.x * 0.5,
            egui::Align::Max => constrained.right() - size.x,
        };
        let anchor_y = match self.anchor.y() {
            egui::Align::Min => constrained.top(),
            egui::Align::Center => constrained.center().y - size.y * 0.5,
            egui::Align::Max => constrained.bottom() - size.y,
        };
        Rect::from_min_size(
            egui::pos2(
                (anchor_x + self.offset.x).clamp(constrained.left(), constrained.right() - size.x),
                (anchor_y + self.offset.y).clamp(constrained.top(), constrained.bottom() - size.y),
            ),
            size,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ModalOptions {
    pub id: Id,
    pub layer: ModalLayer,
    pub dismiss_behavior: DismissBehavior,
    pub layout: ModalLayout,
    pub blocks_lower_layers: bool,
    pub draw_scrim: bool,
}

impl ModalOptions {
    pub fn new(id: Id, layout: ModalLayout) -> Self {
        Self {
            id,
            layer: ModalLayer::Base,
            dismiss_behavior: DismissBehavior::EscapeAndScrim,
            layout,
            blocks_lower_layers: true,
            draw_scrim: true,
        }
    }

    pub fn with_layer(mut self, layer: ModalLayer) -> Self {
        self.layer = layer;
        self
    }

    pub fn with_dismiss_behavior(mut self, dismiss_behavior: DismissBehavior) -> Self {
        self.dismiss_behavior = dismiss_behavior;
        self
    }

    pub fn with_scrim(mut self, draw_scrim: bool) -> Self {
        self.draw_scrim = draw_scrim;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ModalShowResponse<R> {
    pub inner: R,
    pub rect: Rect,
    pub close_requested: bool,
    pub is_top_modal: bool,
    pub stack_index: usize,
}

#[derive(Clone, Debug)]
struct ModalEntryState {
    id: Id,
    layer: ModalLayer,
    dismiss_behavior: DismissBehavior,
    blocks_lower_layers: bool,
    generation_seen: u64,
}

#[derive(Clone, Debug, Default)]
struct ModalHostState {
    generation: u64,
    stack: Vec<ModalEntryState>,
    close_requests: Vec<Id>,
}

#[derive(Clone, Copy, Debug)]
struct RegisteredModal {
    stack_index: usize,
    is_top_modal: bool,
}

pub fn begin_frame(ctx: &Context) {
    let mut state = ctx
        .data_mut(|data| data.get_temp::<ModalHostState>(Id::new(MODAL_HOST_STATE_ID)))
        .unwrap_or_default();
    state.generation = state.generation.saturating_add(1);
    ctx.data_mut(|data| data.insert_temp(Id::new(MODAL_HOST_STATE_ID), state));
}

pub fn end_frame(ctx: &Context) {
    let mut state = ctx
        .data_mut(|data| data.get_temp::<ModalHostState>(Id::new(MODAL_HOST_STATE_ID)))
        .unwrap_or_default();
    state
        .stack
        .retain(|entry| entry.generation_seen == state.generation);
    state
        .close_requests
        .retain(|id| state.stack.iter().any(|entry| entry.id == *id));
    ctx.data_mut(|data| data.insert_temp(Id::new(MODAL_HOST_STATE_ID), state));
}

pub fn close_top(ctx: &Context) -> bool {
    let mut handled = false;
    ctx.data_mut(|data| {
        let mut state = data
            .get_temp::<ModalHostState>(Id::new(MODAL_HOST_STATE_ID))
            .unwrap_or_default();
        let Some(top) = state.stack.last() else {
            data.insert_temp(Id::new(MODAL_HOST_STATE_ID), state);
            return;
        };
        if top.dismiss_behavior.allows_escape() {
            if !state.close_requests.contains(&top.id) {
                state.close_requests.push(top.id);
            }
            handled = true;
        }
        data.insert_temp(Id::new(MODAL_HOST_STATE_ID), state);
    });
    handled
}

pub fn show_window<R>(
    ctx: &Context,
    _title: &str,
    options: ModalOptions,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> ModalShowResponse<R> {
    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_rect = options.layout.resolve_rect(viewport_rect);
    let registration = register_modal(ctx, options);
    let mut close_requested = take_close_request(ctx, options.id);

    let order = order_for_modal(options.layer, registration.stack_index);
    if options.draw_scrim {
        let scrim_clicked = show_scrim_with_order(
            ctx,
            options.id.with("scrim"),
            viewport_rect,
            order,
            options.blocks_lower_layers,
        );
        if registration.is_top_modal && options.dismiss_behavior.allows_scrim() && scrim_clicked {
            close_requested = true;
        }
    }

    let mut inner = None;
    Area::new(options.id)
        .order(order)
        .fixed_pos(modal_rect.min)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_min_size(modal_rect.size());
            ui.set_max_size(modal_rect.size());
            window_frame(ctx).show(ui, |ui| {
                inner = Some(add_contents(ui));
            });
        });

    ModalShowResponse {
        inner: inner.expect("modal window should render exactly once"),
        rect: modal_rect,
        close_requested,
        is_top_modal: registration.is_top_modal,
        stack_index: registration.stack_index,
    }
}

pub fn show_area<R>(
    ctx: &Context,
    options: ModalOptions,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> ModalShowResponse<R> {
    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_rect = options.layout.resolve_rect(viewport_rect);
    let registration = register_modal(ctx, options);
    let mut close_requested = take_close_request(ctx, options.id);

    let order = order_for_modal(options.layer, registration.stack_index);
    if options.draw_scrim {
        let scrim_clicked = show_scrim_with_order(
            ctx,
            options.id.with("scrim"),
            viewport_rect,
            order,
            options.blocks_lower_layers,
        );
        if registration.is_top_modal && options.dismiss_behavior.allows_scrim() && scrim_clicked {
            close_requested = true;
        }
    }

    let mut inner = None;
    Area::new(options.id)
        .order(order)
        .fixed_pos(modal_rect.min)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_min_size(modal_rect.size());
            ui.set_max_size(modal_rect.size());
            window_frame(ctx).show(ui, |ui| {
                inner = Some(add_contents(ui));
            });
        });

    ModalShowResponse {
        inner: inner.expect("modal area should render exactly once"),
        rect: modal_rect,
        close_requested,
        is_top_modal: registration.is_top_modal,
        stack_index: registration.stack_index,
    }
}

pub fn show_scrim(ctx: &Context, id: impl std::hash::Hash, viewport_rect: Rect) {
    let _ = show_scrim_with_order(ctx, id, viewport_rect, Order::Foreground, true);
}

pub fn show_scrim_with_order(
    ctx: &Context,
    id: impl std::hash::Hash,
    viewport_rect: Rect,
    order: Order,
    interactable: bool,
) -> bool {
    let area_id = Id::new((&id, "modal_scrim_area"));
    let blocker_id = Id::new((&id, "modal_scrim_blocker"));
    let mut clicked = false;
    Area::new(area_id)
        .order(order)
        .fixed_pos(viewport_rect.min)
        .interactable(interactable)
        .show(ctx, |ui| {
            let local_rect = Rect::from_min_size(egui::Pos2::ZERO, viewport_rect.size());
            let response = ui.interact(local_rect, blocker_id, egui::Sense::click_and_drag());
            clicked = response.clicked();
            ui.painter().rect_filled(
                local_rect,
                CornerRadius::ZERO,
                Color32::from_rgba_premultiplied(0, 0, 0, MODAL_SCRIM_ALPHA),
            );
        });
    clicked
}

pub fn window_frame(ctx: &Context) -> Frame {
    let base = ctx.global_style().visuals.window_fill;
    Frame::new()
        .fill(Color32::from_rgba_premultiplied(
            base.r(),
            base.g(),
            base.b(),
            255,
        ))
        .stroke(Stroke::new(
            1.0,
            ctx.global_style().visuals.widgets.hovered.bg_stroke.color,
        ))
        .corner_radius(CornerRadius::same(MODAL_CORNER_RADIUS))
        .inner_margin(Margin::same(MODAL_INNER_MARGIN))
}

fn register_modal(ctx: &Context, options: ModalOptions) -> RegisteredModal {
    let mut registered = None;
    ctx.data_mut(|data| {
        let mut state = data
            .get_temp::<ModalHostState>(Id::new(MODAL_HOST_STATE_ID))
            .unwrap_or_default();

        let generation = state.generation;
        if let Some(entry) = state.stack.iter_mut().find(|entry| entry.id == options.id) {
            entry.layer = options.layer;
            entry.dismiss_behavior = options.dismiss_behavior;
            entry.blocks_lower_layers = options.blocks_lower_layers;
            entry.generation_seen = generation;
        } else {
            state.stack.push(ModalEntryState {
                id: options.id,
                layer: options.layer,
                dismiss_behavior: options.dismiss_behavior,
                blocks_lower_layers: options.blocks_lower_layers,
                generation_seen: generation,
            });
        }

        state.stack.sort_by_key(|entry| entry.layer);
        let stack_index = state
            .stack
            .iter()
            .position(|entry| entry.id == options.id)
            .unwrap_or_default();
        registered = Some(RegisteredModal {
            stack_index,
            is_top_modal: stack_index + 1 == state.stack.len(),
        });
        data.insert_temp(Id::new(MODAL_HOST_STATE_ID), state);
    });
    registered.expect("modal registration should succeed")
}

fn take_close_request(ctx: &Context, id: Id) -> bool {
    let mut requested = false;
    ctx.data_mut(|data| {
        let mut state = data
            .get_temp::<ModalHostState>(Id::new(MODAL_HOST_STATE_ID))
            .unwrap_or_default();
        if let Some(index) = state.close_requests.iter().position(|entry| *entry == id) {
            state.close_requests.remove(index);
            requested = true;
        }
        data.insert_temp(Id::new(MODAL_HOST_STATE_ID), state);
    });
    requested
}

fn order_for_modal(layer: ModalLayer, stack_index: usize) -> Order {
    match (layer, stack_index) {
        (ModalLayer::Blocking | ModalLayer::Critical, _) => Order::Tooltip,
        (_, index) if index > 0 => Order::Tooltip,
        _ => Order::Foreground,
    }
}

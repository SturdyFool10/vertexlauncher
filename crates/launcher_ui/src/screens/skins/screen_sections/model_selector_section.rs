use super::super::*;

pub(super) fn render_model_selector_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
) {
    let body = style::body(ui);
    let button_style =
        style::neutral_button_with_min_size(ui, egui::vec2(160.0, style::CONTROL_HEIGHT));

    ui.add_space(style::SPACE_SM);
    let mut model_button_style = button_style.clone();
    let model_button_gap = style::SPACE_XS;
    let half_width = ((ui.available_width() - model_button_gap) * 0.5).max(1.0);
    model_button_style.min_size = egui::vec2(half_width, style::CONTROL_HEIGHT);
    model_button_style.fill = ui.visuals().widgets.inactive.weak_bg_fill;
    model_button_style.fill_hovered = ui.visuals().widgets.hovered.bg_fill.gamma_multiply(1.05);
    model_button_style.fill_active = ui.visuals().selection.bg_fill.gamma_multiply(0.92);
    model_button_style.fill_selected = ui.visuals().selection.bg_fill.gamma_multiply(0.78);
    model_button_style.stroke = ui.visuals().widgets.hovered.bg_stroke;
    let _ = text_ui.label(ui, "skins_model_label", "Model:", &body);
    ui.add_space(style::SPACE_XS);
    let model_focus_request = take_model_focus_request(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(model_button_gap, style::SPACE_XS);
        let classic_response = text_ui.selectable_button(
            ui,
            "skins_model_classic",
            "Classic",
            state.pending_variant == MinecraftSkinVariant::Classic,
            &model_button_style,
        );
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new(CLASSIC_MODEL_BUTTON_ID_KEY),
                classic_response.id,
            )
        });
        if model_focus_request == Some(MinecraftSkinVariant::Classic) {
            classic_response.request_focus();
        }
        if classic_response.clicked() {
            state.pending_variant = MinecraftSkinVariant::Classic;
        }
        let slim_response = text_ui.selectable_button(
            ui,
            "skins_model_slim",
            "Slim (Alex)",
            state.pending_variant == MinecraftSkinVariant::Slim,
            &model_button_style,
        );
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new(SLIM_MODEL_BUTTON_ID_KEY), slim_response.id)
        });
        if model_focus_request == Some(MinecraftSkinVariant::Slim) {
            slim_response.request_focus();
        }
        if slim_response.clicked() {
            state.pending_variant = MinecraftSkinVariant::Slim;
        }
    });
}

fn take_model_focus_request(ctx: &egui::Context) -> Option<MinecraftSkinVariant> {
    ctx.data_mut(|data| {
        let key = egui::Id::new(FORCE_MODEL_FOCUS_ID);
        let value = data.get_temp::<MinecraftSkinVariant>(key);
        if value.is_some() {
            data.remove::<MinecraftSkinVariant>(key);
        }
        value
    })
}

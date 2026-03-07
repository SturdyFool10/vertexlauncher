use egui::Ui;

use crate::assets;
use crate::ui::components::icon_button;
use crate::ui::style;

use super::{ProfileShortcut, SidebarOutput};

pub fn render(
    ui: &mut Ui,
    profile_shortcuts: &[ProfileShortcut],
    output: &mut SidebarOutput,
    max_icon_width: f32,
) {
    if profile_shortcuts.is_empty() {
        return;
    }

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = style::SPACE_SM;
        for profile in profile_shortcuts {
            let response = icon_button::svg(
                ui,
                "user_profile",
                assets::USER_SVG,
                profile.name.as_str(),
                false,
                max_icon_width,
            );
            if response.clicked() {
                output.selected_profile_id = Some(profile.id.clone());
            }
        }
    });
}

use egui::Ui;

use crate::assets;
use crate::ui::components::icon_button;

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
        ui.spacing_mut().item_spacing.y = 6.0;
        for profile in profile_shortcuts {
            let response = ui
                .horizontal_centered(|ui| {
                    icon_button::svg(
                        ui,
                        "user_profile",
                        assets::USER_SVG,
                        profile.name.as_str(),
                        false,
                        max_icon_width,
                    )
                })
                .inner;
            if response.clicked() {
                output.selected_profile_id = Some(profile.id.clone());
            }
        }
    });
}

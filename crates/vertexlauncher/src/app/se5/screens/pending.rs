//! Screens that have not been rebuilt on SE5 yet.

use vertex_i18n::tr;
use launcher_ui::screens::AppScreen;
use vertex_shell::sturdy::ui::{TextStyle, TextWrap, Ui};

use super::super::kit::{self, Kit, TextRole};

pub fn render(ui: &mut Ui<'_>, kit: &mut Kit, screen: AppScreen) {
    let _scroll = kit.scroll(ui, "pending::scroll", kit::SPACE_LG, kit::SPACE_XL);
    kit.text(ui, TextRole::Title, &screen.label());
    kit::label(
        ui,
        &tr!("This screen has not been rebuilt for the new renderer yet. Start Vertex with --legacy-ui to use it in the \
         previous interface; your settings and instances are shared."),
        &TextStyle {
            wrap: TextWrap::Words,
            ..kit.style(TextRole::Muted)
        },
    );
}

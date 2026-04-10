use super::*;

#[path = "preview_panel/preview_canvas.rs"]
mod preview_canvas;
#[path = "preview_panel/preview_overlay_controls.rs"]
mod preview_overlay_controls;
use self::preview_canvas::render_preview_canvas;
use self::preview_overlay_controls::render_preview_overlay_controls;

/// Renders the interactive 3D skin preview panel and returns whether the elytra toggle
/// button was clicked.
///
/// The preview uses the current `SkinManagerState` as-is. Missing skin textures are
/// handled by rendering a placeholder message instead of the model.
///
/// This function does not panic.
pub(super) fn render_preview_panel(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
) -> bool {
    let (rect, _) = render_preview_canvas(ui, text_ui, state);
    render_preview_overlay_controls(ui, text_ui, state, rect)
}

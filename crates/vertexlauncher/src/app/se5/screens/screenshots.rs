//! Screenshot viewer dialog: one screenshot at full size with copy, open-folder and delete.

use std::path::Path;

use launcher_ui::app_commands::{self, AppCommand};
use vertex_shell::sturdy::ui::{Direction, ElementDesc, Sizing, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::AppState;
use super::super::images::ImageSource;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

/// Returns false once the viewer should close.
pub fn viewer(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, path: &Path) -> bool {
    let viewport = ui.viewport_size();
    let max_width = (viewport.x - 160.0).clamp(320.0, 1400.0);
    let max_height = (viewport.y - 260.0).clamp(200.0, 900.0);
    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fit,
        ..ModalStyle::default()
    };
    let key = format!("screenshot::{}", path.display());
    let picture = kit
        .images
        .get(&mut ui.engine(), &key, || ImageSource::File(path.to_path_buf()), 0);
    let mut close = false;
    let mut delete = false;
    let controller = (state.gamepad.viewer_pan, state.gamepad.viewer_zoom);
    let displayed_time = state.config.displayed_time().clone();
    let viewer = &mut state.viewer;
    let response = kit::modal(ui, "screenshot-viewer", &style, true, |ui| {
        let _column = ui.element(
            &ElementDesc::new()
                .width(Sizing::Fit)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_MD),
        );
        match picture {
            Some(picture) => {
                super::viewer::picture(
                    ui,
                    kit,
                    viewer,
                    controller,
                    "screenshot-viewer::image",
                    &key,
                    path,
                    &displayed_time,
                    picture,
                    max_width,
                    max_height,
                );
            }
            None if kit.images.failed(&key) => {
                kit.text(ui, TextRole::Danger, "This screenshot could not be opened.")
            }
            None => kit.text(ui, TextRole::Muted, "Loading…"),
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        kit.text(ui, TextRole::Small, &name);
        let _row = kit.row(ui, kit::SPACE_SM);
        if kit.button(
            ui,
            "screenshot-viewer::copy",
            "Copy",
            ButtonKind::Neutral,
            true,
        ) {
            app_commands::push(AppCommand::CopyImage(path.to_path_buf()));
        }
        if kit.button(
            ui,
            "screenshot-viewer::folder",
            "Show in folder",
            ButtonKind::Neutral,
            true,
        ) && let Some(parent) = path.parent()
            && let Err(err) = launcher_ui::desktop::open_in_file_manager(parent)
        {
            launcher_ui::notification::error!("screenshots", "Failed to open the folder: {}", err);
        }
        kit.fill(ui);
        if kit.button(
            ui,
            "screenshot-viewer::delete",
            "Delete",
            ButtonKind::Danger,
            true,
        ) {
            delete = true;
        }
        if kit.button(
            ui,
            "screenshot-viewer::close",
            "Close",
            ButtonKind::Neutral,
            true,
        ) {
            close = true;
        }
    });
    if delete {
        let path = path.to_path_buf();
        launcher_runtime::spawn_blocking_detached(move || match std::fs::remove_file(&path) {
            Ok(()) => {
                launcher_ui::notification::info!("screenshots", "Deleted screenshot from disk.")
            }
            Err(err) => launcher_ui::notification::error!(
                "screenshots",
                "Failed to delete screenshot: {}",
                err
            ),
        });
        close = true;
    }
    let _ = state;
    !((response.dismissed || kit.back_pressed()) || close)
}

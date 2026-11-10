//! A zoomable, pannable picture for the screenshot viewers: the wheel zooms, dragging pans, and a controller's right
//! stick and triggers do the same.

use std::path::Path;

use config::DisplayedTime;
use vertex_shell::sturdy::glam::Vec2;
use vertex_shell::sturdy::ui::{
    AttachPoint, Border, Clip, CornerRadius, Direction, ElementDesc, FloatTarget, Floating, Padding, Sizing, Ui,
};

use super::super::images::Picture;
use super::super::kit::{self, Kit, TextRole};

const MIN_ZOOM: f32 = 1.0;
const MAX_ZOOM: f32 = 8.0;
const SCROLL_SENSITIVITY: f32 = 0.0015;
/// Pixels one wheel notch counts as (the engine reports notches).
const WHEEL_NOTCH_PIXELS: f32 = 40.0;

/// Zoom and pan of the picture being viewed. `pan` is the offset of the view centre from the picture centre, in
/// fractions of the picture, and stays inside what the zoom can show.
#[derive(Default)]
pub struct ViewerState {
    key: String,
    zoom: f32,
    pan: Vec2,
    last_pointer: Vec2,
    dragging: bool,
    /// When the picture on show was taken, in Unix seconds; read once per picture.
    taken: Option<i64>,
}

impl ViewerState {
    /// Starts over when a different picture is shown.
    fn show(&mut self, key: &str, path: &Path) {
        if self.key != key {
            self.key = key.to_owned();
            self.taken = config::screenshot_taken_at(path);
            self.zoom = MIN_ZOOM;
            self.pan = Vec2::ZERO;
            self.dragging = false;
        }
    }

    fn clamp_pan(&mut self) {
        let visible = 1.0 / self.zoom.max(MIN_ZOOM);
        let limit = (1.0 - visible) * 0.5;
        self.pan = self.pan.clamp(Vec2::splat(-limit), Vec2::splat(limit));
    }
}

/// Draws `picture` scaled to fit `max_width` x `max_height`, zoomed and panned by `view`. `controller` is the right
/// stick pan and trigger zoom. `path` is the screenshot's file, for the "Taken at" card the `time` settings describe.
#[allow(clippy::too_many_arguments)]
pub fn picture(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    view: &mut ViewerState,
    controller: (Vec2, f32),
    id: &str,
    key: &str,
    path: &Path,
    time: &DisplayedTime,
    picture: Picture,
    max_width: f32,
    max_height: f32,
) {
    view.show(key, path);
    let fit = (max_width / picture.width as f32).min(max_height / picture.height as f32);
    let size = Vec2::new(picture.width as f32 * fit, picture.height as f32 * fit);
    let dt = kit.delta_seconds.clamp(1.0 / 240.0, 0.05);

    let hovered = ui.hovered(id);
    if hovered {
        let wheel = ui.engine().input().wheel_delta().y * WHEEL_NOTCH_PIXELS;
        if wheel != 0.0 {
            let scale = (1.0 + wheel * SCROLL_SENSITIVITY).clamp(0.7, 1.3);
            view.zoom = (view.zoom * scale).clamp(MIN_ZOOM, MAX_ZOOM);
            view.clamp_pan();
        }
    }
    let pointer = ui.pointer_position();
    if ui.pointer_down() && (hovered || view.dragging) {
        if view.dragging && view.zoom > MIN_ZOOM {
            let visible = 1.0 / view.zoom.max(MIN_ZOOM);
            let delta = pointer - view.last_pointer;
            view.pan.x -= delta.x / size.x.max(1.0) * visible;
            view.pan.y -= delta.y / size.y.max(1.0) * visible;
            view.clamp_pan();
        }
        view.dragging = true;
    } else {
        view.dragging = false;
    }
    view.last_pointer = pointer;
    let (pan, zoom) = controller;
    if zoom.abs() > 0.05 {
        let scale = (1.0 + zoom * 1.8 * dt).clamp(0.7, 1.3);
        view.zoom = (view.zoom * scale).clamp(MIN_ZOOM, MAX_ZOOM);
        view.clamp_pan();
    }
    if view.zoom > MIN_ZOOM && (pan.x.abs() > 0.05 || pan.y.abs() > 0.05) {
        let visible = 1.0 / view.zoom.max(MIN_ZOOM);
        view.pan += pan * (1.35 * 0.2 * dt * visible);
        view.clamp_pan();
    }
    if view.zoom > MIN_ZOOM + 0.001
        || view.dragging
        || zoom.abs() > 0.05
        || pan.length_squared() > 0.0
    {
    }

    // The visible window is a fixed box; the picture, enlarged by the zoom, slides behind it.
    let _window = ui.element(
        &ElementDesc::new()
            .id(id.into())
            .width(Sizing::Fixed(size.x))
            .height(Sizing::Fixed(size.y))
            .corner_radius(CornerRadius::all(8.0))
            .clip(Clip::both()),
    );
    let zoomed = size * view.zoom;
    let offset = size * 0.5 - (Vec2::splat(0.5) + view.pan) * zoomed;
    ui.image(
        &ElementDesc::new()
            .id(format!("{id}::picture"))
            .width(Sizing::Fixed(zoomed.x))
            .height(Sizing::Fixed(zoomed.y))
            .floating(Some(Floating {
                element_attach_point: AttachPoint::LeftTop,
                parent_attach_point: AttachPoint::LeftTop,
                offset,
                // Floating layers are ordered by z-index across the whole UI; the viewer sits in a modal (z 2000), so
                // the picture must be above that or it is drawn behind the modal's own background.
                z_index: 2001,
                capture_pointer: false,
                clip_to_target: true,
                ..Floating::new(FloatTarget::Parent)
            })),
        picture.texture,
    );
    if time.show_screenshot_info
        && let Some(taken) = view.taken
    {
        taken_card(ui, kit, time, taken, "viewer::taken".to_owned(), FloatTarget::Parent, 2002);
    }
}

/// The small card over the picture's bottom-left corner: when it was taken, and how long ago that was.
fn taken_card(
    ui: &mut Ui<'_>,
    kit: &Kit,
    time: &DisplayedTime,
    taken: i64,
    id: String,
    target: FloatTarget,
    z_index: i16,
) {
    let _card = ui.element(
        &ElementDesc::new()
            .id(id)
            .width(Sizing::Fit)
            .height(Sizing::Fit)
            .direction(Direction::TopToBottom)
            .child_gap(2)
            .padding(Padding::all(8))
            .background(kit.palette.bg_dark.with_w(0.85))
            .corner_radius(CornerRadius::all(8.0))
            .border(Border::all(kit.palette.border_muted, 1))
            .floating(Some(Floating {
                element_attach_point: AttachPoint::LeftBottom,
                parent_attach_point: AttachPoint::LeftBottom,
                offset: Vec2::new(12.0, -12.0),
                z_index,
                capture_pointer: false,
                clip_to_target: true,
                ..Floating::new(target)
            })),
    );
    kit.text(ui, TextRole::Small, &time.taken_at(taken));
    if time.show_relative {
        kit.text(ui, TextRole::Small, &time.time_ago(taken, config::unix_now()));
    }
}

/// What a gallery tile's hover buttons asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileAction {
    Copy,
    Delete,
}

/// Copy and delete buttons over the top-right corner of the gallery tile `tile_id`, shown while the pointer is on
/// the tile. Call it right after drawing the tile.
pub fn tile_actions(ui: &mut Ui<'_>, kit: &mut Kit, tile_id: &str) -> Option<TileAction> {
    use super::super::icons::Icon;
    use super::super::kit::TextRole;
    let copy_id = format!("{tile_id}::copy");
    let delete_id = format!("{tile_id}::delete");
    let active = ui.hovered(tile_id) || ui.hovered(&copy_id) || ui.hovered(&delete_id);
    if !active {
        return None;
    }
    {
        let _buttons = ui.element(
            &ElementDesc::new()
                .id(format!("{tile_id}::actions"))
                .width(Sizing::Fit)
                .height(Sizing::Fit)
                .direction(vertex_shell::sturdy::ui::Direction::LeftToRight)
                .child_gap(6)
                .floating(Some(Floating {
                    element_attach_point: AttachPoint::RightTop,
                    parent_attach_point: AttachPoint::RightTop,
                    offset: Vec2::new(-8.0, 8.0),
                    z_index: 5,
                    // Not capturing: a capturing float swallows the mouse wheel, so the gallery stopped scrolling
                    // while the pointer was over a button. A press on a button is not also a press on its tile
                    // because the callers act on the button's result and only look at the tile when there was none.
                    capture_pointer: false,
                    // Clipped like the tile: when the tile is partly scrolled out of the gallery, the buttons are cut
                    // off with it instead of drawing over what is around the gallery.
                    clip_to_target: true,
                    ..Floating::new(vertex_shell::sturdy::ui::FloatTarget::Element(
                        tile_id.to_owned(),
                    ))
                })),
        );
        for (id, icon) in [(&copy_id, Icon::Copy), (&delete_id, Icon::Trash)] {
            let hovered = ui.hovered(id);
            {
                let _button = ui.element(
                    &ElementDesc::new()
                        .id(id.clone())
                        .width(Sizing::Fixed(30.0))
                        .height(Sizing::Fixed(30.0))
                        .align(vertex_shell::sturdy::ui::ChildAlignment::center())
                        .background(if hovered {
                            kit.palette.bg_light
                        } else {
                            kit.palette.bg_dark.with_w(0.85)
                        })
                        .corner_radius(CornerRadius::all(8.0)),
                );
                kit.icon(
                    ui,
                    icon,
                    16.0,
                    if icon == Icon::Trash {
                        kit.palette.danger
                    } else {
                        kit.palette.text
                    },
                );
            }
        }
    }
    kit.tooltip(ui, &copy_id, "Copy image to clipboard");
    kit.tooltip(ui, &delete_id, "Delete screenshot");
    let _ = TextRole::Small;
    if kit.clicked_plain(ui, &copy_id) {
        Some(TileAction::Copy)
    } else if kit.clicked_plain(ui, &delete_id) {
        Some(TileAction::Delete)
    } else {
        None
    }
}

/// The "Taken at" card over the bottom-left corner of the gallery tile `tile_id` while the pointer is on it (or on its
/// buttons). Call it right after drawing the tile.
pub fn tile_taken(ui: &mut Ui<'_>, kit: &Kit, tile_id: &str, path: &Path, time: &DisplayedTime) {
    if !time.show_screenshot_info {
        return;
    }
    let hovered = ui.hovered(tile_id)
        || ui.hovered(&format!("{tile_id}::copy"))
        || ui.hovered(&format!("{tile_id}::delete"));
    if !hovered {
        return;
    }
    if let Some(taken) = config::screenshot_taken_at(path) {
        taken_card(
            ui,
            kit,
            time,
            taken,
            format!("{tile_id}::taken"),
            FloatTarget::Element(tile_id.to_owned()),
            5,
        );
    }
}

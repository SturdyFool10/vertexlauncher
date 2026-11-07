//! Skins: the active account's skin, model and cape, with a live 3D preview. The preview is drawn by the launcher's
//! rasteriser into an engine texture that is updated in place every frame; everything else is ordinary UI.

use crate::app::se5::file_dialog::{Dialog as PickerDialog, Pick};
use launcher_ui::screens::skins_api::{
    CapeView, MotionMode, PreviewSettings, SkinVariant, SkinsSession,
};
use vertex_shell::sturdy::assets::{
    Asset, RendererTextureHandle, TextureColorSpace, TextureKind, TexturePixelFormat,
};
use vertex_shell::sturdy::glam::Vec2;
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, AttachPoint, Border, ChildAlignment, Clip, CornerRadius, Direction,
    ElementDesc, FloatTarget, Floating, Padding, Sizing, TextStyle, TextWrap, Ui,
};

use super::super::AppState;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

const PREVIEW_HEIGHT: f32 = 460.0;
/// The model stands in the middle of the canvas, so only this much of its width is drawn (centred): the rest is empty
/// and drawing it on the CPU every frame is what made the preview slow.
const MAX_PREVIEW_WIDTH: u32 = 600;
const CANVAS_ID: &str = "skins::canvas";
const PICK_SLOT: &str = "skins::pick";
const CAPE_TILE_WIDTH: f32 = 140.0;
const CAPE_TILE_HEIGHT: f32 = 186.0;

struct PreviewTexture {
    asset: Asset,
    handle: RendererTextureHandle,
    width: u32,
    height: u32,
}

/// Per-screen UI state; the profile and preview state live in the `SkinsSession`.
#[derive(Default)]
pub struct SkinsView {
    texture: Option<PreviewTexture>,
    /// The screen was drawn last frame, so entering it again counts as opening it.
    shown: bool,
    pointer_was_down: bool,
    dragging: bool,
    last_pointer_x: f32,
    /// The preview generation last uploaded.
    uploaded: u64,
    /// Preview textures replaced by one of a new size that the engine would not release yet (a frame still using
    /// them), to be released later rather than forgotten.
    retired: Vec<Asset>,
    /// How many preview textures have been created, so each has its own name.
    created: u64,
}

impl SkinsView {
    /// Frees the preview texture after leaving the screen.
    pub fn hidden(&mut self, ui: &mut Ui<'_>) {
        if self.shown {
            self.shown = false;
            self.dragging = false;
            let assets = self.texture.take().map(|texture| texture.asset);
            for asset in assets.into_iter().chain(std::mem::take(&mut self.retired)) {
                let _ = ui.engine().assets().unload(asset);
            }
        }
    }
}

fn preview_settings(config: &config::Config) -> PreviewSettings {
    PreviewSettings {
        aa_mode: config.skin_preview_aa_mode(),
        msaa_samples: config.skin_preview_msaa_samples().max(1) as u32,
        texel_aa_mode: config.skin_preview_texel_aa_mode(),
        motion_blur_enabled: config.skin_preview_motion_blur_enabled(),
        motion_blur_amount: config.skin_preview_motion_blur_amount(),
        motion_blur_shutter_frames: config.skin_preview_motion_blur_shutter_frames(),
        motion_blur_sample_count: config.skin_preview_motion_blur_sample_count(),
        layers_3d_enabled: config.skin_preview_3d_layers_enabled(),
        expressions_enabled: config.skin_preview_fresh_format_enabled(),
    }
}

pub fn render(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let opened = !state.skins_view.shown;
    state.skins_view.shown = true;
    let settings = preview_settings(&state.config);
    let auth =
        state
            .auth
            .active_launch_context()
            .map(|context| launcher_ui::screens::LaunchAuthContext {
                account_key: context.account_key,
                player_name: context.player_name,
                player_uuid: context.player_uuid,
                access_token: context.access_token,
                xuid: context.xuid,
                user_type: context.user_type,
            });
    let busy = state
        .skins
        .tick(auth.as_ref(), opened, &settings);
    if busy {
    }
    if let Some(path) = state
        .file_dialogs
        .take(PICK_SLOT)
        .and_then(|paths| paths.into_iter().next())
    {
        state.skins.load_skin_path(path);
    }

    let _scroll = kit.scroll(ui, "skins::scroll", kit::SPACE_MD, kit::SPACE_XL);
    kit.text(ui, TextRole::Title, "Skins");

    let Some((id, name)) = state
        .skins
        .account()
        .map(|(id, name)| (id.to_owned(), name.to_owned()))
    else {
        kit.text(
            ui,
            TextRole::Muted,
            "Sign in with a Minecraft account to manage skins and capes.",
        );
        return;
    };
    let streamer = state.streamer_mode();
    kit.account_label(
        ui,
        TextRole::Body,
        &format!(
            "Active account: {}",
            launcher_ui::account_text::mention(&id, &name)
        ),
        streamer,
    );

    let AppState {
        skins,
        skins_view,
        file_dialogs,
        ..
    } = state;
    preview(ui, kit, skins, skins_view, &settings);

    kit.gap(ui, f32::from(kit::SPACE_SM));
    kit.text(ui, TextRole::Heading, "Skin Image");
    drop_zone(ui, kit, skins, file_dialogs);

    kit.gap(ui, f32::from(kit::SPACE_XS));
    model_selector(ui, kit, skins);

    kit.gap(ui, f32::from(kit::SPACE_SM));
    kit.text(ui, TextRole::Heading, "Cape");
    cape_grid(ui, kit, skins);

    kit.gap(ui, f32::from(kit::SPACE_SM));
    let label = if skins.saving() { "Saving…" } else { "Save" };
    if kit.button_sized(
        ui,
        "skins::save",
        label,
        ButtonKind::Primary,
        skins.can_save(),
        Sizing::Grow,
    ) {
        skins.save();
    }
}

fn content_width(ui: &Ui<'_>) -> f32 {
    kit::content_width(ui)
}

// -- Preview ----------------------------------------------------------------------------------------------------------

fn preview(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    skins: &mut SkinsSession,
    view: &mut SkinsView,
    settings: &PreviewSettings,
) {
    let now = kit::uptime();
    // From the scroll area, never from the canvas: the canvas holds the picture, so sizing the picture by it would
    // keep the layout at the largest size it has ever had.
    let width = content_width(ui);
    // Whole 8-pixel steps, so dragging the window edge does not reallocate the texture every frame.
    let (frame_w, frame_h) = (
        ((width / 8.0).floor() as u32 * 8).clamp(200, MAX_PREVIEW_WIDTH),
        PREVIEW_HEIGHT as u32,
    );

    // Camera: drag to turn the model; it keeps turning with the drag's speed and eases to a stop.
    // The engine only learns of a release that happens over the window, so a button let go elsewhere would leave the
    // drag running. Treat the cursor leaving the window as the end of the drag.
    let pointer_down = ui.pointer_down() && !(view.dragging && cursor_outside_window(ui));
    let pointer_x = ui.pointer_position().x;
    let over_controls = ui.hovered("skins::motion") || ui.hovered("skins::elytra");
    // A press also counts when the engine never saw the previous release (it was outside the window).
    let pressed = !view.pointer_was_down || ui.pointer_state().pressed_this_frame;
    if pointer_down && pressed && ui.hovered(CANVAS_ID) && !over_controls {
        view.dragging = true;
        skins.drag_started(now);
    }
    if view.dragging {
        if pointer_down {
            skins.dragged(pointer_x - view.last_pointer_x);
        } else {
            view.dragging = false;
            skins.drag_stopped(now);
        }
    }
    // The raw state, so a release the engine missed does not look like a fresh press when the cursor comes back.
    view.pointer_was_down = ui.pointer_down();
    view.last_pointer_x = pointer_x;

    let frame = skins.render_preview(frame_w, frame_h, now, settings);
    if let Some(frame) = &frame {
        if view.uploaded != skins.preview_generation() || view.texture.is_none() {
            upload(ui, view, frame);
            view.uploaded = skins.preview_generation();
        }
        // The model turns continuously, so keep drawing while it is on screen.
    }

    let _canvas = ui.element(
        &ElementDesc::new()
            .id(CANVAS_ID.into())
            .width(Sizing::Grow)
            .height(Sizing::Fixed(PREVIEW_HEIGHT))
            .align(ChildAlignment::center())
            .background(kit.palette.bg_light)
            .corner_radius(CornerRadius::all(8.0))
            .border(Border::all(kit.palette.border_muted, 1))
            .clip(Clip::both()),
    );
    match (&frame, &view.texture) {
        (Some(_), Some(texture)) => ui.image(
            // Floating, so the picture takes no part in the layout and the canvas always follows the window.
            &ElementDesc::new()
                .width(Sizing::Fixed(texture.width as f32))
                .height(Sizing::Fixed(texture.height as f32))
                .floating(Some(Floating {
                    element_attach_point: AttachPoint::CenterCenter,
                    parent_attach_point: AttachPoint::CenterCenter,
                    z_index: 0,
                    capture_pointer: false,
                    clip_to_target: true,
                    ..Floating::new(FloatTarget::Parent)
                })),
            texture.handle,
        ),
        _ => kit.text(ui, TextRole::Muted, "No skin loaded"),
    }

    // Controls floating over the bottom-left corner.
    let _controls = ui.element(
        &ElementDesc::new()
            .id("skins::controls".into())
            .width(Sizing::Fit)
            .height(Sizing::Fit)
            .direction(Direction::TopToBottom)
            .child_gap(8)
            .floating(Some(Floating {
                element_attach_point: AttachPoint::LeftBottom,
                parent_attach_point: AttachPoint::LeftBottom,
                offset: Vec2::new(14.0, -14.0),
                z_index: 10,
                // Not capturing: a capturing float swallows the mouse wheel, so scrolling stuck while the cursor was
                // over a button. The canvas ignores presses that start on these buttons (`over_controls`).
                capture_pointer: false,
                // Clipped by the page like everything else, so the buttons do not draw over the window's edge or the
                // title bar when the page is scrolled.
                clip_to_target: true,
                ..Floating::new(FloatTarget::Element(CANVAS_ID.to_owned()))
            })),
    );
    let motion = match skins.motion_mode() {
        MotionMode::Idle => "Motion: Idle",
        MotionMode::Walk => "Motion: Walk",
    };
    if kit.button_sized(
        ui,
        "skins::motion",
        motion,
        ButtonKind::Neutral,
        true,
        Sizing::Fit,
    ) {
        skins.toggle_motion_mode();
    }
    let elytra = if skins.show_elytra() {
        "Elytra: On"
    } else {
        "Elytra: Off"
    };
    if kit.button_sized(
        ui,
        "skins::elytra",
        elytra,
        ButtonKind::Neutral,
        true,
        Sizing::Fit,
    ) {
        skins.toggle_elytra();
    }
}

/// Whether the cursor is outside the window. False when the platform cannot say.
fn cursor_outside_window(ui: &mut Ui<'_>) -> bool {
    let mut engine = ui.engine();
    let Some(mut window) = engine.primary_window_control() else {
        return false;
    };
    let (Some(origin), Some(size), Some(cursor)) = (
        window.position(),
        window.size(),
        window.global_cursor_position(),
    ) else {
        return false;
    };
    let local = cursor - origin;
    local.x < 0 || local.y < 0 || local.x >= size.x as i32 || local.y >= size.y as i32
}

/// Puts `frame` on the preview texture, creating it (or replacing it when the size changed) as needed.
fn upload(ui: &mut Ui<'_>, view: &mut SkinsView, frame: &image::RgbaImage) {
    let (width, height) = (frame.width(), frame.height());
    let mut engine = ui.engine();
    if let Some(texture) = &view.texture
        && (texture.width, texture.height) == (width, height)
    {
        if engine
            .assets()
            .update_texture(texture.asset, frame.as_raw())
            .is_err()
        {
            tracing::debug!(target: "vertexlauncher/se5/skins", "Could not update the preview texture.");
        }
        return;
    }
    let mut assets = engine.assets();
    // Release what could not be released earlier, then the texture being replaced; whatever the engine still holds
    // is kept and tried again on the next upload.
    let mut pending = std::mem::take(&mut view.retired);
    pending.extend(view.texture.take().map(|old| old.asset));
    for asset in pending {
        if assets.unload(asset).is_err() && assets.contains(asset) {
            view.retired.push(asset);
        }
    }
    // A name of its own: resizing quickly replaces the texture again and again, and a new texture with the name of
    // one the engine has not finished releasing can be refused, which left the canvas without a picture.
    view.created += 1;
    let label = format!("skins/preview/{}", view.created);
    // Per-frame textures are created uncompressed and without mips, as `update_texture` requires.
    let created = assets
        .create_texture_sampled(
            frame.as_raw(),
            width,
            height,
            TexturePixelFormat::Rgba8,
            TextureColorSpace::Srgb,
            TextureKind::ColorAlpha,
            &label,
            false,
            false,
            true,
        )
        .and_then(|asset| {
            assets.texture_handle(asset).map(|handle| PreviewTexture {
                asset,
                handle,
                width,
                height,
            })
        });
    match created {
        Ok(texture) => view.texture = Some(texture),
        Err(err) => {
            tracing::warn!(target: "vertexlauncher/se5/skins", "Could not create the preview texture: {err:?}")
        }
    }
}

// -- Skin image -------------------------------------------------------------------------------------------------------

fn drop_zone(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    skins: &mut SkinsSession,
    file_dialogs: &mut crate::app::se5::file_dialog::FileDialogs,
) {
    let zone_id = "skins::dropzone";
    let hovered = ui.hovered(zone_id);
    {
        let _zone = ui.element(
            &ElementDesc::new()
                .id(zone_id.into())
                .width(Sizing::Grow)
                .height(Sizing::Fixed(150.0))
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_XS)
                .align(ChildAlignment::center())
                .background(if hovered {
                    kit.palette.bg_light.with_w(0.9)
                } else {
                    kit.palette.bg_light.with_w(0.6)
                })
                .corner_radius(CornerRadius::all(14.0))
                .border(Border::all(
                    if hovered {
                        kit.palette.primary
                    } else {
                        kit.palette.border_muted
                    },
                    2,
                )),
        );
        kit.text(ui, TextRole::Heading, "Drag Skin Image here");
        kit.text(ui, TextRole::Muted, "or");
        let picking = skins.picking();
        if kit.button_sized(
            ui,
            "skins::choose",
            "Choose Skin Image",
            ButtonKind::Neutral,
            !picking,
            Sizing::Fixed(220.0),
        ) {
            open_picker(file_dialogs);
        }
    }
    if kit.clicked_plain(ui, zone_id) && !ui.hovered("skins::choose") && !skins.picking() {
        open_picker(file_dialogs);
    }
    if skins.picking() {
        kit.text(
            ui,
            TextRole::Muted,
            "Loading selected skin in the background...",
        );
        kit.progress(ui, "skins::loading", None, kit::uptime());
    }
    if let Some(path) = skins.pending_skin_path() {
        kit::label(
            ui,
            &path.to_string_lossy(),
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
    }
}

fn open_picker(file_dialogs: &mut crate::app::se5::file_dialog::FileDialogs) {
    file_dialogs.open(
        PICK_SLOT,
        Pick::File,
        PickerDialog::new()
            .title("Select Minecraft Skin")
            .filter("PNG", &["png"]),
    );
}

// -- Model ----------------------------------------------------------------------------------------------------------------

fn model_selector(ui: &mut Ui<'_>, kit: &mut Kit, skins: &mut SkinsSession) {
    kit.text(ui, TextRole::Body, "Model:");
    let _row = kit.row(ui, kit::SPACE_XS);
    for (variant, label, id) in [
        (SkinVariant::Classic, "Classic", "skins::model-classic"),
        (SkinVariant::Slim, "Slim (Alex)", "skins::model-slim"),
    ] {
        let kind = if skins.variant() == variant {
            ButtonKind::Primary
        } else {
            ButtonKind::Neutral
        };
        if kit.button_sized(ui, id, label, kind, true, Sizing::Grow) {
            skins.set_variant(variant);
        }
    }
}

// -- Capes ----------------------------------------------------------------------------------------------------------------

fn cape_grid(ui: &mut Ui<'_>, kit: &mut Kit, skins: &mut SkinsSession) {
    let capes = skins.capes();
    let width = content_width(ui);
    let gap = f32::from(kit::SPACE_MD);
    let columns = (((width + gap) / (CAPE_TILE_WIDTH + gap)).floor() as usize).max(1);
    let total = capes.len() + 1;
    let selected = skins.selected_cape().map(str::to_owned);
    let mut choice: Option<Option<String>> = None;
    let mut start = 0;
    while start < total {
        // Each line of capes is centred as one group.
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_MD)
                .align(ChildAlignment::new(AlignX::Center, AlignY::Center))
                .clip(Clip::horizontal()),
        );
        for index in start..(start + columns).min(total) {
            if index == 0 {
                if cape_tile(
                    ui,
                    kit,
                    "skins::cape-none",
                    "No Cape",
                    None,
                    selected.is_none(),
                ) {
                    choice = Some(None);
                }
            } else {
                let cape: &CapeView = &capes[index - 1];
                let is_selected = selected.as_deref() == Some(cape.id.as_str());
                if cape_tile(
                    ui,
                    kit,
                    &format!("skins::cape::{}", cape.id),
                    &cape.label,
                    Some(cape),
                    is_selected,
                ) {
                    choice = Some(Some(cape.id.clone()));
                }
            }
        }
        start += columns;
    }
    if let Some(choice) = choice {
        skins.select_cape(choice);
    }
}

fn cape_tile(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    label: &str,
    cape: Option<&CapeView>,
    selected: bool,
) -> bool {
    let hovered = ui.hovered(id);
    let fill = if selected {
        kit.palette.primary.with_w(0.28)
    } else if hovered {
        kit.palette.bg_light.with_w(0.95)
    } else {
        kit.palette.bg_light.with_w(0.7)
    };
    {
        let _tile = ui.element(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Fixed(CAPE_TILE_WIDTH))
                .height(Sizing::Fixed(CAPE_TILE_HEIGHT))
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_SM)
                .padding(Padding::all(12))
                .align(ChildAlignment::new(AlignX::Center, AlignY::Top))
                .background(fill)
                .corner_radius(CornerRadius::all(10.0))
                .border(Border::all(
                    if selected {
                        kit.palette.primary
                    } else {
                        kit.palette.border_muted
                    },
                    if selected { 2 } else { 1 },
                )),
        );
        {
            // The picture box; "No Cape" is an empty outlined box, a cape shows its outer face.
            let _preview = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(112.0))
                    .align(ChildAlignment::center())
                    .border(Border::all(kit.palette.text_muted.with_w(0.6), 1))
                    .corner_radius(CornerRadius::all(6.0)),
            );
            if let Some(face) = cape.and_then(|cape| cape.face_png.clone()) {
                let key = format!("{id}::face::{}", face.len());
                kit.image(
                    ui,
                    &format!("{id}::image"),
                    &key,
                    || super::super::images::ImageSource::Bytes(face),
                    64.0,
                    100.0,
                    4.0,
                );
            }
        }
        kit.text_clipped(ui, TextRole::Body, label, CAPE_TILE_WIDTH - 24.0);
    }
    kit.clicked(ui, id)
}

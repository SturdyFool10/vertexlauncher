//! Skins: the active account's skin, model and cape, with a live 3D preview. The preview is drawn by the engine on the
//! GPU every frame (`skin_gpu`) into an offscreen target that is shown as an image. Everything else is ordinary UI.

use vertex_i18n::tr;
use crate::app::se5::file_dialog::{Dialog as PickerDialog, Pick};
use launcher_ui::screens::skins_api::{
    CapeView, MotionMode, PreviewSettings, SkinVariant, SkinsSession,
};
use vertex_shell::sturdy::assets::RendererTextureHandle;
use vertex_shell::sturdy::glam::Vec2;
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, AttachPoint, Border, ChildAlignment, Clip, CornerRadius, Direction,
    ElementDesc, FloatTarget, Floating, Padding, Sizing, TextAlign, TextStyle, TextWrap, Ui,
};

use super::super::AppState;
use super::super::icons::Icon;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

const PREVIEW_HEIGHT: f32 = 460.0;
/// The model stands in the middle of the canvas, so only this much of its width is drawn (centred).
const MAX_PREVIEW_WIDTH: u32 = 600;
const CANVAS_ID: &str = "skins::canvas";
const PICK_SLOT: &str = "skins::pick";
const CAPE_TILE_WIDTH: f32 = 140.0;
/// A cape tile is at least this tall; a long name wraps onto more lines and makes it (and its whole row) taller.
const CAPE_TILE_MIN_HEIGHT: f32 = 186.0;
const CAPE_TILE_PADDING: u16 = 12;

/// Per-screen UI state; the profile and preview state live in the `SkinsSession`.
#[derive(Default)]
pub struct SkinsView {
    /// The screen was drawn last frame, so entering it again counts as opening it.
    shown: bool,
    pointer_was_down: bool,
    dragging: bool,
    last_pointer_x: f32,
    /// The GPU preview, made on first use and dropped when the screen is left.
    gpu: Option<skin_gpu::PreviewRenderer>,
    /// The GPU preview could not start (or failed): the screen says why instead of showing a picture, and it is tried again
    /// after a pause that grows with each failure in a row (a graphics hiccup, a device switch, a full disk must not cost the
    /// preview for the rest of the session).
    gpu_failure: Option<GpuFailure>,
    /// Failures in a row since the preview last drew; sets the length of the pause.
    gpu_failures_in_a_row: u32,
    /// Physical pixels per layout unit of the display the window is on (0 = not asked yet), and when it was asked.
    display_scale: f32,
    display_scale_checked: f64,
    /// The window position and size `display_scale` was worked out for.
    display_scale_key: Option<(i32, i32, u32, u32)>,
}

/// Why the GPU preview is not showing, and when it will be tried again.
struct GpuFailure {
    message: String,
    retry_at: f64,
}

impl GpuFailure {
    fn new(message: String, now: f64, failures_in_a_row: u32) -> Self {
        // 1 s, 2 s, 4 s ... up to half a minute.
        let pause = (1_u32 << failures_in_a_row.saturating_sub(1).min(5)).min(30);
        Self { message, retry_at: now + f64::from(pause) }
    }
}

impl SkinsView {
    /// Frees the preview texture after leaving the screen.
    pub fn hidden(&mut self, ui: &mut Ui<'_>) {
        if self.shown {
            self.shown = false;
            self.dragging = false;
            // Coming back is a fresh start: no waiting out a pause that belonged to the last visit.
            self.gpu_failure = None;
            self.gpu_failures_in_a_row = 0;
            if let Some(mut renderer) = self.gpu.take() {
                renderer.destroy(&mut ui.engine());
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
    // "Lazy Account Refresh": the skin service needs a fresh session, so the active account (only it) is renewed first.
    // Nothing below runs, and the screen does not count as opened, until that is done.
    if !state.auth.ensure_active_fresh() {
        kit.text(ui, TextRole::Title, &tr!("Skins"));
        kit.text(ui, TextRole::Muted, &tr!("Refreshing your account..."));
        return;
    }
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
    kit.text(ui, TextRole::Title, &tr!("Skins"));

    let Some((id, name)) = state
        .skins
        .account()
        .map(|(id, name)| (id.to_owned(), name.to_owned()))
    else {
        kit.text(
            ui,
            TextRole::Muted,
            &tr!("Sign in with a Minecraft account to manage skins and capes."),
        );
        return;
    };
    let streamer = state.streamer_mode();
    kit.account_label(
        ui,
        TextRole::Body,
        &tr!("Active account: {account}", account = launcher_ui::account_text::mention(&id, &name)),
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
    kit.text(ui, TextRole::Heading, &tr!("Skin Image"));
    drop_zone(ui, kit, skins, file_dialogs);

    kit.gap(ui, f32::from(kit::SPACE_XS));
    model_selector(ui, kit, skins);

    // Players with no capes are not reminded of it: the whole section stays out.
    let cape_count = skins.capes().len();
    if cape_count > 0 {
        kit.gap(ui, f32::from(kit::SPACE_SM));
        // "Cape" for one; "Capes (3)" for several, so the player sees how many they own.
        let heading = vertex_i18n::trn!(cape_count, "Cape", "Capes ({count})");
        kit.text(ui, TextRole::Heading, &heading);
        cape_grid(ui, kit, skins);
    }

    kit.gap(ui, f32::from(kit::SPACE_SM));
    let label = if skins.saving() { tr!("Saving…") } else { tr!("Save") };
    if kit.button_sized(
        ui,
        "skins::save",
        &label,
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

    let scale = display_scale(ui, view, now);

    // Camera: drag to turn the model; it keeps turning with the drag's speed and eases to a stop.
    // The engine only learns of a release that happens over the window, so a button let go elsewhere would leave the
    // drag running. Treat the cursor leaving the window as the end of the drag.
    let pointer_down = ui.pointer_down() && !(view.dragging && cursor_outside_window(ui));
    let pointer_x = ui.pointer_position().x;
    let over_controls = ui.hovered("skins::motion") || ui.hovered("skins::elytra") || ui.hovered("skins::refresh");
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

    // The picture to show and its size, or `None` while there is no skin (or no GPU preview).
    let outcome = gpu_picture(ui, skins, view, settings, now, frame_w, frame_h, scale);
    let picture: Option<(RendererTextureHandle, u32, u32)> =
        match outcome {
            // Drawn at the layout size whatever resolution the picture was rendered at.
            GpuPicture::Shown(handle) => Some((handle, frame_w, frame_h)),
            GpuPicture::NoSkin | GpuPicture::Unavailable => None,
        };
    let unavailable = view.gpu_failure.as_ref().map(|failure| {
        tr!("The skin preview could not be drawn ({message}). Trying again in {seconds:.0}s.", message = failure.message, seconds = (failure.retry_at - now).max(0.0).ceil()).into_owned()
    });

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
    match picture {
        Some((handle, width, height)) => ui.image(
            // Floating, so the picture takes no part in the layout and the canvas always follows the window.
            &ElementDesc::new()
                .width(Sizing::Fixed(width as f32))
                .height(Sizing::Fixed(height as f32))
                .floating(Some(Floating {
                    element_attach_point: AttachPoint::CenterCenter,
                    parent_attach_point: AttachPoint::CenterCenter,
                    z_index: 0,
                    capture_pointer: false,
                    clip_to_target: true,
                    ..Floating::new(FloatTarget::Parent)
                })),
            handle,
        ),
        None if unavailable.is_some() => kit.text(
            ui,
            TextRole::Muted,
            unavailable.as_deref().unwrap_or_default(),
        ),
        None => kit.text(ui, TextRole::Muted, &tr!("No skin loaded")),
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
        MotionMode::Idle => tr!("Motion: Idle"),
        MotionMode::Walk => tr!("Motion: Walk"),
    };
    if kit.button_sized(
        ui,
        "skins::motion",
        &motion,
        ButtonKind::Neutral,
        true,
        Sizing::Fit,
    ) {
        skins.toggle_motion_mode();
    }
    let elytra = if skins.show_elytra() {
        &tr!("Elytra: On")
    } else {
        &tr!("Elytra: Off")
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
    drop(_controls);
    refresh_button(ui, kit, skins);
}

/// A large refresh button over the viewer's top-right corner: fetches the skin, model and capes again.
fn refresh_button(ui: &mut Ui<'_>, kit: &mut Kit, skins: &mut SkinsSession) {
    const ID: &str = "skins::refresh";
    let busy = skins.refreshing();
    let hovered = ui.hovered(ID) && !busy;
    {
        let _button = ui.element(
            &ElementDesc::new()
                .id(ID.into())
                .width(Sizing::Fixed(48.0))
                .height(Sizing::Fixed(48.0))
                .align(ChildAlignment::center())
                .background(if hovered { kit.palette.hovered_fill() } else { kit.palette.bg_dark.with_w(0.85) })
                .corner_radius(CornerRadius::all(12.0))
                .border(Border::all(kit.palette.border_muted, 1))
                .floating(Some(Floating {
                    element_attach_point: AttachPoint::RightTop,
                    parent_attach_point: AttachPoint::RightTop,
                    offset: Vec2::new(-14.0, 14.0),
                    z_index: 10,
                    // Like the other viewer controls: not capturing, so the mouse wheel still scrolls the page over it.
                    capture_pointer: false,
                    clip_to_target: true,
                    ..Floating::new(FloatTarget::Element(CANVAS_ID.to_owned()))
                })),
        );
        let color = if busy { kit.palette.text_muted.with_w(0.5) } else { kit.palette.text };
        kit.icon(ui, Icon::Refresh, 26.0, color);
    }
    kit.tooltip(ui, ID, &tr!("Refresh skin and capes"));
    if !busy && kit.clicked_plain(ui, ID) {
        skins.refresh();
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

/// Physical pixels per layout unit of the display the window is on, from the display list. The list is only read when
/// the window has moved or been resized since the last time (and every half minute): enumerating displays is not free,
/// and the answer changes only when the window changes monitor. 1 when it cannot be told.
fn display_scale(ui: &mut Ui<'_>, view: &mut SkinsView, now: f64) -> f32 {
    let mut engine = ui.engine();
    let Some(mut window) = engine.primary_window_control() else {
        return if view.display_scale > 0.0 { view.display_scale } else { 1.0 };
    };
    let placement = window.position().zip(window.size());
    let key = placement.map(|(position, size)| (position.x, position.y, size.x, size.y));
    // Asked again only when the window moved or changed size (the only ways its display changes): the enumeration it takes is a
    // long frame, and doing it on a timer showed up as a periodic stall.
    if view.display_scale > 0.0 && key == view.display_scale_key {
        return view.display_scale;
    }
    view.display_scale_checked = now;
    view.display_scale_key = key;
    let center = placement.map(|(position, size)| (position.x + size.x as i32 / 2, position.y + size.y as i32 / 2));
    let displays = window.displays();
    let containing = center.and_then(|(x, y)| {
        displays.iter().find(|display| {
            let bounds = &display.bounds;
            x >= bounds.x && y >= bounds.y && x < bounds.x + bounds.width as i32 && y < bounds.y + bounds.height as i32
        })
    });
    let scale = containing
        .or_else(|| displays.iter().find(|display| display.primary))
        .map_or(1.0, |display| display.content_scale);
    view.display_scale = if scale.is_finite() { scale.clamp(1.0, 4.0) } else { 1.0 };
    view.display_scale
}

enum GpuPicture {
    Shown(RendererTextureHandle),
    NoSkin,
    /// The GPU renderer is not running.
    Unavailable,
}

/// Draws the preview on the GPU for this frame, creating the renderer on first use.
#[allow(clippy::too_many_arguments)]
fn gpu_picture(
    ui: &mut Ui<'_>,
    skins: &mut SkinsSession,
    view: &mut SkinsView,
    settings: &PreviewSettings,
    now: f64,
    width: u32,
    height: u32,
    scale: f32,
) -> GpuPicture {
    if let Some(failure) = &view.gpu_failure {
        if now < failure.retry_at {
            return GpuPicture::Unavailable;
        }
        // The pause is over: try again from scratch.
        view.gpu_failure = None;
    }
    if view.gpu.is_none() {
        match skin_gpu::PreviewRenderer::new(&mut ui.engine()) {
            Ok(renderer) => {
                view.gpu = Some(renderer);
                // A new renderer has none of the geometry the session believes it handed out.
                skins.gpu_resend_all();
            }
            Err(err) => {
                view.gpu_failures_in_a_row = view.gpu_failures_in_a_row.saturating_add(1);
                tracing::warn!(
                    target: "vertexlauncher/se5/skins",
                    "The GPU skin preview could not start ({err}); no preview is shown (failure {} in a row, trying again soon).",
                    view.gpu_failures_in_a_row
                );
                view.gpu_failure = Some(GpuFailure::new(tr!("could not start: {err}", err = err).into_owned(), now, view.gpu_failures_in_a_row));
                return GpuPicture::Unavailable;
            }
        }
    }
    let Some(frame) = skins.gpu_frame(now, settings) else {
        return GpuPicture::NoSkin;
    };
    let Some(renderer) = view.gpu.as_mut() else {
        return GpuPicture::Unavailable;
    };
    let request = skin_gpu::RenderRequest {
        frame: &frame,
        // Rendered at the display's real resolution (layout units are logical, so a scaled display shows the picture
        // larger than `width` x `height` pixels), so the anti-aliasing works on the pixels that reach the screen.
        size: vertex_shell::sturdy::glam::UVec2::new(
            ((width as f32 * scale).round() as u32).clamp(16, 4096),
            ((height as f32 * scale).round() as u32).clamp(16, 4096),
        ),
        settings: skin_gpu::RenderSettings {
            aa_mode: settings.aa_mode,
            msaa_samples: settings.msaa_samples,
            texel_boundary_aa: settings.texel_aa_mode == config::SkinPreviewTexelAaMode::TexelBoundary,
        },
    };
    let rendered = renderer.render(&mut ui.engine(), &request);
    match rendered {
        Ok(missing_geometry) => {
            if missing_geometry {
                skins.gpu_resend_all();
            }
        }
        Err(err) => {
            return gpu_failed(ui, view, skins, now, tr!("failed: {err}", err = err).into_owned());
        }
    }
    match renderer.texture(&mut ui.engine()) {
        Some((handle, _size)) => {
            view.gpu_failures_in_a_row = 0;
            GpuPicture::Shown(handle)
        }
        // Drawn without error, yet the engine has no picture for the target (it lost it): the same as a failure, with its own
        // wording, rather than a blank canvas.
        None => gpu_failed(ui, view, skins, now, tr!("the engine no longer has the preview's picture").into_owned()),
    }
}

/// The preview failed: its engine objects are released, the failure is recorded for the screen to show, and drawing is tried
/// again after a pause (see [`GpuFailure`]).
fn gpu_failed(ui: &mut Ui<'_>, view: &mut SkinsView, skins: &mut SkinsSession, now: f64, message: String) -> GpuPicture {
    view.gpu_failures_in_a_row = view.gpu_failures_in_a_row.saturating_add(1);
    tracing::warn!(
        target: "vertexlauncher/se5/skins",
        "The GPU skin preview {message} (failure {} in a row); no preview is shown until it is tried again.",
        view.gpu_failures_in_a_row
    );
    if let Some(mut renderer) = view.gpu.take() {
        renderer.destroy(&mut ui.engine());
    }
    // A renderer made afterwards has none of the geometry the session believes it handed out.
    skins.gpu_resend_all();
    view.gpu_failure = Some(GpuFailure::new(message, now, view.gpu_failures_in_a_row));
    GpuPicture::Unavailable
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
        kit.text(ui, TextRole::Heading, &tr!("Drag Skin Image here"));
        kit.text(ui, TextRole::Muted, &tr!("or"));
        let picking = skins.picking();
        if kit.button_sized(
            ui,
            "skins::choose",
            &tr!("Choose Skin Image"),
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
            &tr!("Loading selected skin in the background..."),
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
            .title(tr!("Select Minecraft Skin"))
            .filter(tr!("PNG"), &["png"]),
    );
}

// -- Model ----------------------------------------------------------------------------------------------------------------

fn model_selector(ui: &mut Ui<'_>, kit: &mut Kit, skins: &mut SkinsSession) {
    kit.text(ui, TextRole::Body, &tr!("Model:"));
    let _row = kit.row(ui, kit::SPACE_XS);
    for (variant, label, id) in [
        (SkinVariant::Classic, tr!("Classic"), "skins::model-classic"),
        (SkinVariant::Slim, tr!("Slim (Alex)"), "skins::model-slim"),
    ] {
        let kind = if skins.variant() == variant {
            ButtonKind::Primary
        } else {
            ButtonKind::Neutral
        };
        if kit.button_sized(ui, id, &label, kind, true, Sizing::Grow) {
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
    // "Random" only makes sense when there is a choice to make.
    let has_random = capes.len() >= 2;
    // One cape a second under the dice, drawn by the engine into a single reused target.
    let random_picture = if has_random { kit.cape_dice.texture(&mut ui.engine(), &capes) } else { None };
    let total = capes.len() + 1 + usize::from(has_random);
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
                    &tr!("No Cape"),
                    None,
                    None,
                    None,
                    selected.is_none(),
                ) {
                    choice = Some(None);
                }
            } else if has_random && index == 1 {
                if cape_tile(
                    ui,
                    kit,
                    "skins::cape-random",
                    &tr!("Random"),
                    None,
                    Some(Icon::Dice),
                    random_picture,
                    false,
                ) {
                    choice = Some(random_cape(&capes, selected.as_deref()));
                }
            } else {
                // Capes come after "No Cape" and, when it is there, "Random".
                let cape: &CapeView = &capes[index - 1 - usize::from(has_random)];
                let is_selected = selected.as_deref() == Some(cape.id.as_str());
                if cape_tile(
                    ui,
                    kit,
                    &format!("skins::cape::{}", cape.id),
                    &cape.label,
                    Some(cape),
                    None,
                    None,
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

/// A random cape other than the one already worn.
fn random_cape(capes: &[CapeView], current: Option<&str>) -> Option<String> {
    let others: Vec<&CapeView> = capes.iter().filter(|cape| Some(cape.id.as_str()) != current).collect();
    if others.is_empty() {
        return current.map(str::to_owned);
    }
    // No `rand` dependency for one pick: the clock's nanoseconds are random enough for choosing a cape.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos() as usize);
    Some(others[nanos % others.len()].id.clone())
}

fn cape_tile(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    label: &str,
    cape: Option<&CapeView>,
    // Drawn in the picture box when there is no picture of the cape: the dice on the "Random" tile.
    icon: Option<Icon>,
    // A ready-made picture for the box (the "Random" tile's changing cape under the dice).
    picture: Option<vertex_shell::sturdy::assets::RendererTextureHandle>,
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
                .height(Sizing::Grow)
                .direction(Direction::LeftToRight)
                .padding(Padding::all(CAPE_TILE_PADDING))
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
        // Holds the tile open to its minimum height; the tile grows past it when the name wraps, and `Grow` makes
        // every tile in a row as tall as the tallest.
        drop(
            ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed(0.0))
                    .height(Sizing::Fixed(
                        CAPE_TILE_MIN_HEIGHT - 2.0 * f32::from(CAPE_TILE_PADDING),
                    )),
            ),
        );
        let _column = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_SM)
                .align(ChildAlignment::new(AlignX::Center, AlignY::Top)),
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
            match (picture, icon) {
                (Some(texture), _) => ui.image(
                    &ElementDesc::new()
                        .id(format!("{id}::picture"))
                        .width(Sizing::Fixed(64.0))
                        .height(Sizing::Fixed(100.0))
                        .corner_radius(CornerRadius::all(4.0)),
                    texture,
                ),
                (None, Some(icon)) => kit.icon(ui, icon, 40.0, kit.palette.text_muted),
                (None, None) => {}
            }
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
        let style = TextStyle {
            align: TextAlign::Center,
            ..kit.style(TextRole::Body)
        };
        kit::wrapped_label(
            ui,
            label,
            &style,
            CAPE_TILE_WIDTH - 2.0 * f32::from(CAPE_TILE_PADDING),
        );
    }
    kit.clicked(ui, id)
}

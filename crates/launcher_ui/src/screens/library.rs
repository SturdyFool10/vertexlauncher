use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

#[path = "library_runtime.rs"]
mod library_runtime;
#[path = "library/library_output.rs"]
mod library_output;
#[path = "library/library_state.rs"]
mod library_state;
#[path = "library/library_tile_metrics.rs"]
mod library_tile_metrics;
#[path = "library/runtime_action.rs"]
mod runtime_action;
#[path = "library/thumbnail_cache_entry.rs"]
mod thumbnail_cache_entry;

use config::Config;
use egui::{TextureOptions, Ui};
use installation::{
    is_instance_running, is_instance_running_for_account, normalize_path_key,
    running_instance_for_account, stop_running_instance_for_account,
};
use instances::{InstanceRecord, InstanceStore, instance_root_path};
use textui::TextUi;
use textui_egui::prelude::*;
use ui_foundation::{
    DialogPreset, UiMetrics, danger_button, dialog_options, secondary_button, show_dialog,
};

use crate::app::tokio_runtime;
use crate::ui::components::{image_memory::load_image_path_for_memory, image_textures};
use crate::{assets, desktop, install_activity, notification, ui::style};
use self::library_output::LibraryOutput;
use self::library_state::LibraryState;
use self::library_tile_metrics::LibraryTileMetrics;
use self::runtime_action::RuntimeAction;
use self::thumbnail_cache_entry::ThumbnailCacheEntry;
use library_runtime::{
    LibraryLaunchIdentity, LibraryRuntimeState as RuntimeWorkflowState,
    poll_delete_instance_results, poll_runtime_actions, request_instance_delete,
    request_runtime_launch,
};

use super::{
    AppScreen, LaunchAuthContext, QuickLaunchCommandMode, build_quick_launch_command,
    build_quick_launch_steam_options, peek_launch_intent, selected_quick_launch_user,
};
use crate::ui::instance_context_menu::{self, InstanceContextAction};

const TILE_DELETE_BUTTON_HEIGHT: f32 = style::CONTROL_HEIGHT;
const LIBRARY_RUNTIME_LAUNCH_TASK_KIND: &str = "library runtime launch";
const LIBRARY_GRID_COMPACT_THRESHOLD: f32 = 760.0;
const LIBRARY_THUMBNAIL_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const LIBRARY_THUMBNAIL_CACHE_STALE_FRAMES: u64 = 900;

fn library_runtime_state_id() -> egui::Id {
    egui::Id::new("library_runtime_state")
}

pub fn purge_inactive_state(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(library_runtime_state_id(), LibraryState::default()));
}

/// Requests that the library delete-confirmation flow open for the given instance.
///
/// This routes deletion requests from outside the library screen, such as the
/// sidebar context menu, back through the same confirmation and async-delete
/// machinery used by the library itself.
pub fn request_delete_instance(ctx: &egui::Context, instance_id: impl Into<String>) {
    let state_id = library_runtime_state_id();
    let instance_id = instance_id.into();
    ctx.data_mut(|data| {
        let mut state = data.get_temp::<LibraryState>(state_id).unwrap_or_default();
        state.runtime.delete_target_instance_id = Some(instance_id);
        state.runtime.delete_error = None;
        data.insert_temp(state_id, state);
    });
    ctx.request_repaint();
}

pub(super) fn handle_escape(ctx: &egui::Context) -> bool {
    let state_id = library_runtime_state_id();
    let mut handled = false;
    ctx.data_mut(|data| {
        let Some(mut state) = data.get_temp::<LibraryState>(state_id) else {
            return;
        };
        if state.runtime.delete_target_instance_id.is_some() && !state.runtime.delete_in_flight {
            state.runtime.delete_target_instance_id = None;
            state.runtime.delete_error = None;
            data.insert_temp(state_id, state);
            handled = true;
        }
    });
    handled
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    active_account_owns_minecraft: bool,
    _streamer_mode: bool,
    instances: &mut InstanceStore,
    installations_root: &Path,
    config: &mut Config,
    account_avatars_by_key: &HashMap<String, Vec<u8>>,
) -> LibraryOutput {
    let mut output = LibraryOutput::default();
    let state_id = library_runtime_state_id();
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp::<LibraryState>(state_id))
        .unwrap_or_default();
    state.thumbnail_cache_frame_index = state.thumbnail_cache_frame_index.saturating_add(1);
    trim_library_thumbnail_cache(ui.ctx(), &mut state);
    let pending_launch_intent = peek_launch_intent(ui.ctx());
    poll_runtime_actions(&mut state.runtime, config, instances);
    poll_delete_instance_results(&mut state.runtime, instances);
    poll_thumbnail_results(ui.ctx(), &mut state);
    if !state.runtime.pending_launches.is_empty()
        || state.runtime.delete_in_flight
        || !state.thumbnail_in_flight.is_empty()
        || install_activity::snapshot().is_some()
    {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }

    if instances.instances.is_empty() {
        let _ = text_ui.label(
            ui,
            "library_empty_profiles",
            "No instances created yet.",
            &style::muted(ui),
        );
        return output;
    }

    let tiles_height = ui.available_height().max(1.0);
    let launch_identity = LibraryLaunchIdentity {
        account: active_launch_auth
            .map(|auth| auth.account_key.clone())
            .or_else(|| {
                active_username
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            }),
        display_name: active_launch_auth
            .map(|auth| auth.player_name.clone())
            .or_else(|| {
                active_username
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            }),
        player_uuid: active_launch_auth.map(|auth| auth.player_uuid.clone()),
        access_token: active_launch_auth.and_then(|auth| auth.access_token.clone()),
        xuid: active_launch_auth.and_then(|auth| auth.xuid.clone()),
        user_type: active_launch_auth.map(|auth| auth.user_type.clone()),
    };
    egui::ScrollArea::both()
        .id_salt("library_instance_tiles_scroll")
        .auto_shrink([false, false])
        .max_height(tiles_height)
        .show(ui, |ui| {
            let (_, column_count, tile_metrics) = LibraryTileMetrics::from_ui(ui);
            ui.add_space(style::SPACE_MD);
            for row in instances.instances.chunks(column_count.max(1)) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_XL, style::SPACE_XL);
                    for instance in row {
                        let instance_root = instance_root_path(installations_root, instance);
                        let instance_running = is_instance_running(instance_root.as_path());
                        let runtime_running_for_active_account =
                            launch_identity.account.as_deref().is_some_and(|account| {
                                is_instance_running_for_account(instance_root.as_path(), account)
                            });
                        let running_account_key = if runtime_running_for_active_account {
                            launch_identity
                                .player_uuid
                                .clone()
                                .or_else(|| launch_identity.account.clone())
                                .map(|value| value.to_ascii_lowercase())
                        } else {
                            None
                        };
                        let running_avatar = running_account_key
                            .as_deref()
                            .and_then(|key| account_avatars_by_key.get(key))
                            .map(Vec::as_slice);
                        let instance_root_key = normalize_path_key(instance_root.as_path());
                        let account_running_root = launch_identity
                            .account
                            .as_deref()
                            .and_then(running_instance_for_account);
                        let launch_disabled_for_account = !runtime_running_for_active_account
                            && account_running_root.as_deref().is_some_and(|running_root| {
                                running_root != instance_root_key.as_str()
                            });
                        let launch_disabled_for_missing_ownership =
                            !runtime_running_for_active_account && !active_account_owns_minecraft;
                        let launch_disabled =
                            launch_disabled_for_account || launch_disabled_for_missing_ownership;
                        let launch_in_flight = state
                            .runtime
                            .pending_launches
                            .contains(instance.id.as_str());
                        let install_in_flight =
                            install_activity::is_instance_installing(instance.id.as_str());
                        let delete_disabled =
                            instance_running || launch_in_flight || install_in_flight;

                        let action = render_instance_tile(
                            ui,
                            &mut state,
                            text_ui,
                            instance,
                            runtime_running_for_active_account,
                            launch_disabled,
                            launch_in_flight,
                            install_in_flight,
                            launch_disabled_for_account,
                            launch_disabled_for_missing_ownership,
                            running_avatar,
                            delete_disabled,
                            selected_instance_id == Some(instance.id.as_str()),
                            tile_metrics,
                        );
                        if matches!(
                            action,
                            RuntimeAction::LaunchRequested | RuntimeAction::StopRequested
                        ) {
                            output.selected_instance_id = Some(instance.id.clone());
                        }
                        match action {
                            RuntimeAction::None => {}
                            RuntimeAction::StopRequested => {
                                let stopped =
                                    launch_identity.account.as_deref().is_some_and(|account| {
                                        stop_running_instance_for_account(
                                            instance_root.as_path(),
                                            account,
                                        )
                                    });
                                state.runtime.status_by_instance.insert(
                                    instance.id.clone(),
                                    if stopped {
                                        "Stopped instance runtime.".to_owned()
                                    } else {
                                        "Instance runtime was not running for this account."
                                            .to_owned()
                                    },
                                );
                            }
                            RuntimeAction::LaunchRequested => {
                                let requested = request_runtime_launch(
                                    &mut state.runtime,
                                    instance,
                                    instance_root.clone(),
                                    config,
                                    launch_identity.display_name.clone(),
                                    launch_identity.player_uuid.clone(),
                                    launch_identity.access_token.clone(),
                                    launch_identity.xuid.clone(),
                                    launch_identity.user_type.clone(),
                                    launch_identity.account.clone(),
                                    None,
                                    None,
                                );
                                if !requested {
                                    state.runtime.status_by_instance.insert(
                                        instance.id.clone(),
                                        "Launch is already in progress.".to_owned(),
                                    );
                                }
                            }
                            RuntimeAction::DeleteRequested => {
                                state.runtime.delete_target_instance_id = Some(instance.id.clone());
                                state.runtime.delete_error = None;
                            }
                            RuntimeAction::OpenFolderRequested => {
                                if let Err(err) =
                                    desktop::open_in_file_manager(instance_root.as_path())
                                {
                                    state.runtime.status_by_instance.insert(
                                        instance.id.clone(),
                                        format!("Failed to open folder: {err}"),
                                    );
                                }
                            }
                            RuntimeAction::CopyCommandRequested => {
                                copy_instance_launch_command(
                                    ui.ctx(),
                                    instance.id.as_str(),
                                    active_username,
                                    active_launch_auth,
                                );
                            }
                            RuntimeAction::CopySteamOptionsRequested => {
                                copy_instance_steam_launch_options(
                                    ui.ctx(),
                                    instance.id.as_str(),
                                    active_username,
                                    active_launch_auth,
                                );
                            }
                            RuntimeAction::OpenInstanceRequested => {
                                output.selected_instance_id = Some(instance.id.clone());
                                output.requested_screen = Some(AppScreen::Instance);
                            }
                        }

                        if let Some(intent) = pending_launch_intent
                            .as_ref()
                            .filter(|intent| intent.instance_id == instance.id)
                            .filter(|intent| {
                                state.runtime.last_handled_launch_intent_nonce != Some(intent.nonce)
                            })
                        {
                            state.runtime.last_handled_launch_intent_nonce = Some(intent.nonce);
                            output.selected_instance_id = Some(instance.id.clone());
                            if install_in_flight {
                                state.runtime.status_by_instance.insert(
                                    instance.id.clone(),
                                    "Wait for installation to finish before launching.".to_owned(),
                                );
                            } else if launch_disabled {
                                state.runtime.status_by_instance.insert(
                                    instance.id.clone(),
                                    if launch_disabled_for_account {
                                        "Selected account is already running another instance."
                                            .to_owned()
                                    } else if launch_disabled_for_missing_ownership {
                                        "Sign in with an account that owns Minecraft to launch."
                                            .to_owned()
                                    } else {
                                        "Launch is currently unavailable.".to_owned()
                                    },
                                );
                            } else {
                                let requested = request_runtime_launch(
                                    &mut state.runtime,
                                    instance,
                                    instance_root.clone(),
                                    config,
                                    launch_identity.display_name.clone(),
                                    launch_identity.player_uuid.clone(),
                                    launch_identity.access_token.clone(),
                                    launch_identity.xuid.clone(),
                                    launch_identity.user_type.clone(),
                                    launch_identity.account.clone(),
                                    intent.quick_play_singleplayer.clone(),
                                    intent.quick_play_multiplayer.clone(),
                                );
                                if !requested {
                                    state.runtime.status_by_instance.insert(
                                        instance.id.clone(),
                                        "Launch is already in progress.".to_owned(),
                                    );
                                }
                            }
                        }
                    }
                });
                ui.add_space(style::SPACE_MD);
            }
        });
    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
    output
}

fn render_instance_tile(
    ui: &mut Ui,
    state: &mut LibraryState,
    text_ui: &mut TextUi,
    instance: &InstanceRecord,
    runtime_running_for_active_account: bool,
    launch_disabled: bool,
    launch_in_flight: bool,
    install_in_flight: bool,
    launch_disabled_for_account: bool,
    launch_disabled_for_missing_ownership: bool,
    running_avatar_png: Option<&[u8]>,
    delete_disabled: bool,
    _selected: bool,
    tile_metrics: LibraryTileMetrics,
) -> RuntimeAction {
    let tile_fill = ui.visuals().widgets.noninteractive.bg_fill;
    let tile_stroke = ui.visuals().widgets.noninteractive.bg_stroke;

    let frame = egui::Frame::new()
        .fill(tile_fill)
        .stroke(tile_stroke)
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(style::SPACE_XL as i8));

    let mut action = RuntimeAction::None;

    let frame_response = frame
        .show(ui, |ui| {
            ui.set_min_width(tile_metrics.tile_width);
            ui.set_max_width(tile_metrics.tile_width);
            ui.set_min_height(tile_metrics.tile_height);
            ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_SM, style::SPACE_SM);
            ui.vertical(|ui| {
                let tile_start_y = ui.cursor().min.y;
                render_instance_thumbnail(ui, state, instance, tile_metrics);

                let mut name_style = style::heading(ui, 22.0, 28.0);
                name_style.wrap = true;
                render_scroll_text_block(
                    ui,
                    ("library_instance_name", instance.id.as_str()),
                    text_ui,
                    instance.name.as_str(),
                    &name_style,
                    tile_metrics.name_scroll_height,
                );
                let detail_style = style::body(ui);

                let _ = text_ui.label(
                    ui,
                    ("library_instance_version", instance.id.as_str()),
                    &format!("Version: {}", instance.game_version),
                    &detail_style,
                );
                let _ = text_ui.label(
                    ui,
                    ("library_instance_modloader", instance.id.as_str()),
                    &format!("Modloader: {}", instance.modloader),
                    &detail_style,
                );

                let (description, muted) = if let Some(description) = instance
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    (description, false)
                } else {
                    ("No description provided.", true)
                };
                let description_style = if muted {
                    style::muted(ui)
                } else {
                    style::body(ui)
                };
                render_scroll_text_block(
                    ui,
                    ("library_instance_description", instance.id.as_str()),
                    text_ui,
                    description,
                    &description_style,
                    tile_metrics.description_scroll_height,
                );

                let play_button_height = style::CONTROL_HEIGHT_LG;
                let consumed_height = ui.cursor().min.y - tile_start_y;
                let remaining_height = (tile_metrics.tile_height
                    - consumed_height
                    - play_button_height
                    - style::SPACE_SM
                    - TILE_DELETE_BUTTON_HEIGHT)
                    .max(0.0);
                if remaining_height > 0.0 {
                    ui.add_space(remaining_height);
                }

                let button_response = render_runtime_action_button(
                    ui,
                    instance.id.as_str(),
                    runtime_running_for_active_account,
                    launch_disabled,
                    launch_in_flight,
                    install_in_flight,
                    running_avatar_png,
                );
                if button_response.clicked() {
                    if runtime_running_for_active_account {
                        action = RuntimeAction::StopRequested;
                    } else if launch_disabled || launch_in_flight || install_in_flight {
                        action = RuntimeAction::None;
                    } else {
                        action = RuntimeAction::LaunchRequested;
                    }
                }
                ui.add_space(style::SPACE_SM);
                let delete_response =
                    render_delete_instance_button(ui, instance.id.as_str(), delete_disabled);
                if delete_response.clicked() && !delete_disabled {
                    action = RuntimeAction::DeleteRequested;
                }
                if delete_disabled {
                    let reason = if install_in_flight {
                        "Wait for installation to finish before deleting this instance."
                    } else if launch_in_flight {
                        "Wait for launch preparation to finish before deleting this instance."
                    } else {
                        "Stop the running instance before deleting its folder."
                    };
                    let _ = delete_response.on_hover_text(reason);
                }

                let muted_style = style::muted(ui);
                if launch_disabled_for_account {
                    let _ = text_ui.label(
                        ui,
                        ("library_instance_account_locked", instance.id.as_str()),
                        "This account is already running another instance.",
                        &muted_style,
                    );
                }
                if launch_disabled_for_missing_ownership {
                    let _ = text_ui.label(
                        ui,
                        ("library_instance_account_ownership", instance.id.as_str()),
                        "Sign in with an account that owns Minecraft to launch.",
                        &muted_style,
                    );
                }
            });
        })
        .response;

    let context_id = ui.make_persistent_id(("library_instance_context", instance.id.as_str()));
    if frame_response.secondary_clicked() {
        let anchor = frame_response
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_latest_pos())
            .unwrap_or(frame_response.rect.left_bottom());
        instance_context_menu::request_for_instance(ui.ctx(), context_id, anchor, true);
    }

    if let Some(action_id) = instance_context_menu::take(ui.ctx(), context_id) {
        action = match action_id {
            InstanceContextAction::OpenInstance => RuntimeAction::OpenInstanceRequested,
            InstanceContextAction::OpenFolder => RuntimeAction::OpenFolderRequested,
            InstanceContextAction::CopyLaunchCommand => RuntimeAction::CopyCommandRequested,
            InstanceContextAction::CopySteamLaunchOptions => {
                RuntimeAction::CopySteamOptionsRequested
            }
            InstanceContextAction::Delete => RuntimeAction::DeleteRequested,
        };
    }

    action
}

fn render_scroll_text_block(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + Copy,
    text_ui: &mut TextUi,
    text: &str,
    text_style: &LabelOptions,
    height: f32,
) {
    let block = egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(6));
    block.show(ui, |ui| {
        let available_width = ui.available_width().max(1.0);
        let inner_height = (height - 12.0).max(1.0);
        ui.allocate_ui_with_layout(
            egui::vec2(available_width, inner_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt((id_source, "scroll"))
                    .auto_shrink([false, false])
                    .max_height(inner_height)
                    .show(ui, |ui| {
                        let _ = text_ui.label(ui, (id_source, "text"), text, text_style);
                    });
            },
        );
    });
}

fn render_runtime_action_button(
    ui: &mut Ui,
    instance_id: &str,
    runtime_running_for_active_account: bool,
    launch_disabled: bool,
    launch_in_flight: bool,
    install_in_flight: bool,
    running_avatar_png: Option<&[u8]>,
) -> egui::Response {
    let desired_size = egui::vec2(ui.available_width().max(1.0), style::CONTROL_HEIGHT_LG);
    let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let response = ui.interact(
        rect,
        ui.make_persistent_id(("library_runtime_action_button", instance_id)),
        egui::Sense::click(),
    );
    let has_focus = response.has_focus();
    let (fill_base, stroke, text_color) = if runtime_running_for_active_account {
        let error = ui.visuals().error_fg_color;
        (
            egui::Color32::from_rgba_premultiplied(error.r(), error.g(), error.b(), 36),
            egui::Stroke::new(1.0, error),
            error,
        )
    } else if launch_in_flight || install_in_flight {
        (
            ui.visuals().widgets.noninteractive.bg_fill,
            ui.visuals().widgets.noninteractive.bg_stroke,
            ui.visuals().weak_text_color(),
        )
    } else if launch_disabled {
        (
            ui.visuals().widgets.noninteractive.bg_fill,
            ui.visuals().widgets.noninteractive.bg_stroke,
            ui.visuals().weak_text_color(),
        )
    } else {
        (
            ui.visuals().selection.bg_fill,
            ui.visuals().selection.stroke,
            ui.visuals().widgets.active.fg_stroke.color,
        )
    };
    let fill = if response.is_pointer_button_down_on() {
        fill_base.gamma_multiply(0.9)
    } else if response.hovered() || has_focus {
        fill_base.gamma_multiply(1.1)
    } else {
        fill_base
    };

    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        if has_focus {
            ui.visuals().selection.stroke
        } else {
            stroke
        },
        egui::StrokeKind::Inside,
    );
    if has_focus {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            egui::CornerRadius::same(10),
            egui::Stroke::new(
                (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }

    let inner_rect = rect.shrink2(egui::vec2(8.0, 4.0));
    if runtime_running_for_active_account {
        let avatar_size = (inner_rect.height() - 2.0).clamp(14.0, 22.0);
        let avatar_rect =
            egui::Rect::from_min_size(inner_rect.min, egui::vec2(avatar_size, avatar_size));
        render_running_user_avatar(ui, instance_id, avatar_rect, running_avatar_png);

        let label_rect = egui::Rect::from_min_max(
            egui::pos2(
                (avatar_rect.max.x + 8.0).min(inner_rect.max.x),
                inner_rect.min.y,
            ),
            inner_rect.max,
        );
        let icon_size = (label_rect.height() - 4.0).clamp(12.0, 18.0);
        let stop_icon_rect =
            egui::Rect::from_center_size(label_rect.center(), egui::vec2(icon_size, icon_size));
        let stop_icon_color = egui::Color32::WHITE;
        let stop_icon = egui::Image::from_bytes(
            format!(
                "bytes://library/stop-icon-v3/{instance_id}-{:02x}{:02x}{:02x}.svg",
                stop_icon_color.r(),
                stop_icon_color.g(),
                stop_icon_color.b()
            ),
            apply_color_to_svg(assets::STOP_SVG, stop_icon_color),
        )
        .fit_to_exact_size(egui::vec2(icon_size, icon_size));
        let _ = ui.put(stop_icon_rect, stop_icon);
    } else {
        let icon_size = (inner_rect.height() - 4.0).clamp(12.0, 18.0);
        let icon_rect =
            egui::Rect::from_center_size(inner_rect.center(), egui::vec2(icon_size, icon_size));
        if launch_in_flight || install_in_flight {
            let spinner_radius = (icon_size * 0.5).max(6.0);
            ui.painter().circle_stroke(
                icon_rect.center(),
                spinner_radius,
                egui::Stroke::new(1.5, text_color),
            );
            return response;
        }
        let play_icon = egui::Image::from_bytes(
            format!(
                "bytes://library/play-icon-v3/{instance_id}-{:02x}{:02x}{:02x}.svg",
                text_color.r(),
                text_color.g(),
                text_color.b()
            ),
            apply_color_to_svg(assets::PLAY_SVG, text_color),
        )
        .fit_to_exact_size(egui::vec2(icon_size, icon_size));
        let _ = ui.put(icon_rect, play_icon);
    }

    response
}

fn render_delete_instance_button(ui: &mut Ui, instance_id: &str, disabled: bool) -> egui::Response {
    let desired_size = egui::vec2(ui.available_width().max(1.0), TILE_DELETE_BUTTON_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let response = ui.interact(
        rect,
        ui.make_persistent_id(("library_delete_instance_button", instance_id)),
        egui::Sense::click(),
    );
    let has_focus = response.has_focus();
    let danger = ui.visuals().error_fg_color;
    let stroke_color = if disabled {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    } else {
        danger
    };
    let fill = if disabled {
        ui.visuals().widgets.noninteractive.bg_fill
    } else if response.is_pointer_button_down_on() {
        danger.gamma_multiply(0.88)
    } else if response.hovered() || has_focus {
        danger
    } else {
        ui.visuals().widgets.inactive.bg_fill.gamma_multiply(0.18)
    };
    let text_color = if disabled {
        ui.visuals().weak_text_color()
    } else if response.hovered() || response.is_pointer_button_down_on() {
        egui::Color32::WHITE
    } else {
        danger
    };

    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        if has_focus {
            ui.visuals().selection.stroke
        } else {
            egui::Stroke::new(1.0, stroke_color)
        },
        egui::StrokeKind::Inside,
    );
    if has_focus {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            egui::CornerRadius::same(10),
            egui::Stroke::new(
                (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Delete Instance",
        egui::FontId::proportional(15.0),
        text_color,
    );

    let _ = instance_id;
    response
}

fn render_running_user_avatar(
    ui: &mut Ui,
    instance_id: &str,
    rect: egui::Rect,
    avatar_png: Option<&[u8]>,
) {
    if let Some(bytes) = avatar_png {
        let mut hasher = DefaultHasher::new();
        instance_id.hash(&mut hasher);
        bytes.hash(&mut hasher);
        let key = format!("bytes://library/runtime-avatar/{}", hasher.finish());
        if let image_textures::ManagedTextureStatus::Ready(texture) =
            image_textures::request_texture(
                ui.ctx(),
                key,
                Arc::<[u8]>::from(bytes.to_vec().into_boxed_slice()),
                TextureOptions::LINEAR,
            )
        {
            let image = egui::Image::from_texture(&texture).fit_to_exact_size(rect.size());
            let _ = ui.put(rect, image);
        }
        return;
    }

    let fallback = egui::Image::from_bytes(
        format!("bytes://library/runtime-avatar-fallback/{instance_id}.svg"),
        apply_color_to_svg(assets::USER_SVG, ui.visuals().text_color()),
    )
    .fit_to_exact_size(rect.size());
    let _ = ui.put(rect, fallback);
}

pub fn render_global_overlays(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instances: &mut InstanceStore,
    installations_root: &Path,
) {
    let state_id = library_runtime_state_id();
    let mut state = ctx
        .data_mut(|data| data.get_temp::<LibraryState>(state_id))
        .unwrap_or_default();

    poll_delete_instance_results(&mut state.runtime, instances);
    if state.runtime.delete_in_flight {
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    render_delete_instance_modal(ctx, text_ui, &mut state, instances, installations_root);

    ctx.data_mut(|data| data.insert_temp(state_id, state));
}

fn render_delete_instance_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut LibraryState,
    instances: &mut InstanceStore,
    installations_root: &Path,
) {
    let Some(instance_id) = state.runtime.delete_target_instance_id.clone() else {
        return;
    };
    let Some(instance) = instances.find(instance_id.as_str()).cloned() else {
        state.runtime.delete_target_instance_id = None;
        state.runtime.delete_error = None;
        return;
    };

    let instance_root = instance_root_path(installations_root, &instance);
    let instance_running = is_instance_running(instance_root.as_path());
    let danger = ctx.style().visuals.error_fg_color;
    let request_cancel_focus =
        modal_default_focus_requested(ctx, ("library_delete_instance_modal", instance.id.as_str()));
    let response = show_dialog(
        ctx,
        dialog_options("library_delete_instance_modal", DialogPreset::Confirm),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_MD, style::SPACE_MD);

            let heading_style = style::heading_color(ui, 28.0, 32.0, danger);
            let body_style = style::body(ui);
            let muted_style = style::muted(ui);

            let _ = text_ui.label(
                ui,
                ("library_delete_heading", instance.id.as_str()),
                "Delete Instance Folder?",
                &heading_style,
            );
            let _ = text_ui.label(
                ui,
                ("library_delete_body", instance.id.as_str()),
                &format!(
                    "Delete the whole folder for \"{}\". This permanently removes installed content and personal files, including worlds.",
                    instance.name
                ),
                &body_style,
            );
            let _ = text_ui.label(
                ui,
                ("library_delete_path", instance.id.as_str()),
                &format!("Folder: {}", instance_root.display()),
                &muted_style,
            );

            if instance_running {
                let _ = text_ui.label(
                    ui,
                    ("library_delete_running", instance.id.as_str()),
                    "Stop the running instance before deleting its folder.",
                    &LabelOptions {
                        color: danger,
                        ..style::body(ui)
                    },
                );
            }

            if let Some(error) = state.runtime.delete_error.as_deref() {
                let _ = text_ui.label(
                    ui,
                    ("library_delete_error", instance.id.as_str()),
                    error,
                    &LabelOptions {
                        color: danger,
                        ..style::body(ui)
                    },
                );
            }

            ui.add_space(style::SPACE_MD);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let delete_clicked = ui
                    .add_enabled_ui(!instance_running && !state.runtime.delete_in_flight, |ui| {
                        text_ui.button(
                            ui,
                            ("library_delete_confirm", instance.id.as_str()),
                            "Delete Folder",
                            &danger_button(ui, egui::vec2(140.0, style::CONTROL_HEIGHT)),
                        )
                    })
                    .inner
                    .clicked();
                let cancel_clicked = text_ui.button(
                    ui,
                    ("library_delete_cancel", instance.id.as_str()),
                    "Cancel",
                    &secondary_button(ui, egui::vec2(96.0, style::CONTROL_HEIGHT)),
                );
                if request_cancel_focus {
                    cancel_clicked.request_focus();
                }
                let cancel_clicked = cancel_clicked.clicked();

                if state.runtime.delete_in_flight {
                    ui.add_space(style::SPACE_SM);
                    ui.spinner();
                }

                if cancel_clicked && !state.runtime.delete_in_flight {
                    state.runtime.delete_target_instance_id = None;
                    state.runtime.delete_error = None;
                }

                if delete_clicked {
                    request_instance_delete(
                        &mut state.runtime,
                        instance.clone(),
                        installations_root.to_path_buf(),
                    );
                }
            });
        },
    );
    if response.close_requested && !state.runtime.delete_in_flight {
        state.runtime.delete_target_instance_id = None;
        state.runtime.delete_error = None;
    }
}

fn modal_default_focus_requested(ctx: &egui::Context, id_source: impl Hash) -> bool {
    let key = egui::Id::new(("modal_default_focus_frame", id_source));
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| {
        let last_seen = data.get_temp::<u64>(key);
        data.insert_temp(key, frame);
        !matches!(last_seen, Some(previous) if previous.saturating_add(1) >= frame)
    })
}

fn apply_color_to_svg(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}

fn copy_instance_launch_command(
    ctx: &egui::Context,
    instance_id: &str,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) {
    let Some(user) = selected_quick_launch_user(active_username, active_launch_auth) else {
        notification::warn!(
            "library/quick_launch",
            "Sign in before copying an instance command line."
        );
        return;
    };
    let command = build_quick_launch_command(
        QuickLaunchCommandMode::Pack,
        instance_id,
        user.as_str(),
        None,
        None,
    );
    ctx.copy_text(command);
    notification::info!(
        "library/quick_launch",
        "Copied instance command line to clipboard."
    );
}

fn copy_instance_steam_launch_options(
    ctx: &egui::Context,
    instance_id: &str,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) {
    let Some(user) = selected_quick_launch_user(active_username, active_launch_auth) else {
        notification::warn!(
            "library/quick_launch",
            "Sign in before copying Steam launch options."
        );
        return;
    };
    let options = build_quick_launch_steam_options(
        QuickLaunchCommandMode::Pack,
        instance_id,
        user.as_str(),
        None,
        None,
    );
    ctx.copy_text(options);
    notification::info!(
        "library/quick_launch",
        "Copied Steam launch options to clipboard."
    );
}

fn ensure_thumbnail_channel(state: &mut LibraryState) {
    if state.thumbnail_results_tx.is_some() && state.thumbnail_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, Option<Arc<[u8]>>)>();
    state.thumbnail_results_tx = Some(tx);
    state.thumbnail_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn thumbnail_cache_key(instance_id: &str, path: &Path) -> String {
    format!("{instance_id}\n{}", path.display())
}

fn thumbnail_uri(instance_id: &str, path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    instance_id.hash(&mut hasher);
    path.hash(&mut hasher);
    format!(
        "bytes://library/instance-thumbnail/{:016x}",
        hasher.finish()
    )
}

fn request_instance_thumbnail(state: &mut LibraryState, instance_id: &str, path: &Path) {
    let key = thumbnail_cache_key(instance_id, path);
    if state.thumbnail_in_flight.contains(key.as_str()) {
        return;
    }

    ensure_thumbnail_channel(state);
    let Some(tx) = state.thumbnail_results_tx.as_ref().cloned() else {
        return;
    };
    state.thumbnail_in_flight.insert(key.clone());
    let path = path.to_path_buf();
    tokio_runtime::spawn_detached(async move {
        let bytes = load_image_path_for_memory(path.clone()).await.ok();
        if let Err(err) = tx.send((key.clone(), bytes)) {
            tracing::error!(
                target: "vertexlauncher/library",
                thumbnail_key = %key,
                path = %path.display(),
                error = %err,
                "Failed to deliver library thumbnail result."
            );
        }
    });
}

fn poll_thumbnail_results(ctx: &egui::Context, state: &mut LibraryState) {
    let Some(rx) = state.thumbnail_results_rx.as_ref() else {
        return;
    };

    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    match rx.lock() {
        Ok(receiver) => loop {
            match receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    should_reset_channel = true;
                    break;
                }
            }
        },
        Err(_) => should_reset_channel = true,
    }

    if should_reset_channel {
        state.thumbnail_results_tx = None;
        state.thumbnail_results_rx = None;
        state.thumbnail_in_flight.clear();
    }

    for (key, bytes) in updates {
        state.thumbnail_in_flight.remove(key.as_str());
        state.thumbnail_cache.insert(
            key,
            ThumbnailCacheEntry {
                approx_bytes: bytes.as_ref().map_or(0, |bytes| bytes.len()),
                bytes,
                last_touched_frame: state.thumbnail_cache_frame_index,
            },
        );
    }
    trim_library_thumbnail_cache(ctx, state);
}

fn trim_library_thumbnail_cache(_ctx: &egui::Context, state: &mut LibraryState) {
    let stale_before = state
        .thumbnail_cache_frame_index
        .saturating_sub(LIBRARY_THUMBNAIL_CACHE_STALE_FRAMES);
    state.thumbnail_cache.retain(|key, entry| {
        let keep = state.thumbnail_in_flight.contains(key.as_str())
            || entry.last_touched_frame >= stale_before;
        if !keep {
            let Some((instance_id, path)) = key.split_once('\n') else {
                return keep;
            };
            image_textures::evict_source_key(thumbnail_uri(instance_id, Path::new(path)).as_str());
        }
        keep
    });

    let mut total_bytes = state
        .thumbnail_cache
        .values()
        .map(|entry| entry.approx_bytes)
        .sum::<usize>();
    if total_bytes <= LIBRARY_THUMBNAIL_CACHE_MAX_BYTES {
        return;
    }

    let mut eviction_order = state
        .thumbnail_cache
        .iter()
        .filter(|(key, _)| !state.thumbnail_in_flight.contains(key.as_str()))
        .map(|(key, entry)| (key.clone(), entry.last_touched_frame, entry.approx_bytes))
        .collect::<Vec<_>>();
    eviction_order.sort_by_key(|(_, last_touched_frame, _)| *last_touched_frame);

    for (key, _, approx_bytes) in eviction_order {
        if total_bytes <= LIBRARY_THUMBNAIL_CACHE_MAX_BYTES {
            break;
        }
        if state.thumbnail_cache.remove(key.as_str()).is_some() {
            if let Some((instance_id, path)) = key.split_once('\n') {
                image_textures::evict_source_key(
                    thumbnail_uri(instance_id, Path::new(path)).as_str(),
                );
            }
            total_bytes = total_bytes.saturating_sub(approx_bytes);
        }
    }
}

fn render_instance_thumbnail(
    ui: &mut Ui,
    state: &mut LibraryState,
    instance: &InstanceRecord,
    tile_metrics: LibraryTileMetrics,
) {
    let thumbnail_width = ui.available_width().max(120.0);
    let thumbnail_size = egui::vec2(thumbnail_width, tile_metrics.thumbnail_height);
    let centered_thumbnail_size = egui::vec2(
        thumbnail_width.min(tile_metrics.centered_thumbnail_width),
        tile_metrics.thumbnail_height,
    );

    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(6));

    frame.show(ui, |ui| {
        if let Some(path) = instance
            .thumbnail_path
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
        {
            let key = thumbnail_cache_key(instance.id.as_str(), path);
            match state.thumbnail_cache.get_mut(&key) {
                Some(entry) => {
                    entry.last_touched_frame = state.thumbnail_cache_frame_index;
                    if let Some(bytes) = entry.bytes.clone() {
                        let uri = thumbnail_uri(instance.id.as_str(), path);
                        let (rect, _) =
                            ui.allocate_exact_size(thumbnail_size, egui::Sense::hover());
                        let image_rect =
                            egui::Rect::from_center_size(rect.center(), centered_thumbnail_size);
                        if let image_textures::ManagedTextureStatus::Ready(texture) =
                            image_textures::request_texture(
                                ui.ctx(),
                                uri,
                                bytes,
                                TextureOptions::LINEAR,
                            )
                        {
                            ui.put(
                                image_rect,
                                egui::Image::from_texture(&texture)
                                    .fit_to_exact_size(centered_thumbnail_size),
                            );
                        }
                        return;
                    }
                }
                None => request_instance_thumbnail(state, instance.id.as_str(), path),
            }
        }

        let placeholder_dim = (tile_metrics.thumbnail_height * 0.28).clamp(32.0, 48.0);
        let placeholder_size = egui::vec2(placeholder_dim, placeholder_dim);
        let placeholder = egui::Image::from_bytes(
            format!(
                "bytes://library/instance-thumbnail-default/{}.svg",
                instance.id
            ),
            apply_color_to_svg(assets::LIBRARY_SVG, ui.visuals().text_color()),
        )
        .fit_to_exact_size(placeholder_size);
        let (rect, _) = ui.allocate_exact_size(thumbnail_size, egui::Sense::hover());
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(6),
            ui.visuals().faint_bg_color,
        );
        let icon_rect = egui::Rect::from_center_size(rect.center(), placeholder_size);
        ui.put(icon_rect, placeholder);
    });
}

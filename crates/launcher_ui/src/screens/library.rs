use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use config::{Config, JavaRuntimeVersion};
use egui::Ui;
use installation::{
    DownloadPolicy, LaunchRequest, LaunchResult, ensure_game_files, ensure_openjdk_runtime,
    is_instance_running_for_account, launch_instance, running_instance_for_account,
    stop_running_instance_for_account,
};
use instances::{InstanceRecord, InstanceStore, instance_root_path};
use textui::{LabelOptions, TextUi};

use crate::app::tokio_runtime;
use crate::{assets, ui::style};

use super::{AppScreen, LaunchAuthContext};

const TILE_WIDTH: f32 = 300.0;
const TILE_HEIGHT: f32 = 430.0;
const TILE_THUMBNAIL_HEIGHT: f32 = 150.0;
const TILE_NAME_SCROLL_HEIGHT: f32 = 58.0;
const TILE_DESCRIPTION_SCROLL_HEIGHT: f32 = 96.0;

#[derive(Debug, Default, Clone)]
pub struct LibraryOutput {
    pub selected_instance_id: Option<String>,
    pub requested_screen: Option<AppScreen>,
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    active_account_owns_minecraft: bool,
    instances: &mut InstanceStore,
    installations_root: &Path,
    config: &mut Config,
    account_avatars_by_key: &HashMap<String, Vec<u8>>,
) -> LibraryOutput {
    let mut output = LibraryOutput::default();
    let state_id = ui.make_persistent_id("library_runtime_state");
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp::<LibraryRuntimeState>(state_id))
        .unwrap_or_default();
    poll_runtime_actions(&mut state, config);
    if !state.pending_launches.is_empty() {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }

    if instances.instances.is_empty() {
        let _ = text_ui.label(
            ui,
            "library_empty_profiles",
            "No instances created yet.",
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        return output;
    }

    egui::ScrollArea::vertical()
        .id_salt("library_instance_tiles_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_MD, style::SPACE_MD);
                for instance in &instances.instances {
                    let instance_root = instance_root_path(installations_root, instance);
                    let launch_account = active_launch_auth
                        .map(|auth| auth.account_key.clone())
                        .or_else(|| {
                            active_username
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(str::to_owned)
                        });
                    let launch_display_name = active_launch_auth
                        .map(|auth| auth.player_name.clone())
                        .or_else(|| {
                            active_username
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(str::to_owned)
                        });
                    let launch_player_uuid =
                        active_launch_auth.map(|auth| auth.player_uuid.clone());
                    let launch_access_token =
                        active_launch_auth.map(|auth| auth.access_token.clone());
                    let launch_xuid = active_launch_auth.and_then(|auth| auth.xuid.clone());
                    let launch_user_type = active_launch_auth.map(|auth| auth.user_type.clone());
                    let runtime_running_for_active_account =
                        launch_account.as_deref().is_some_and(|account| {
                            is_instance_running_for_account(instance_root.as_path(), account)
                        });
                    let running_account_key = if runtime_running_for_active_account {
                        launch_player_uuid
                            .clone()
                            .or_else(|| launch_account.clone())
                            .map(|value| value.to_ascii_lowercase())
                    } else {
                        None
                    };
                    let running_avatar = running_account_key
                        .as_deref()
                        .and_then(|key| account_avatars_by_key.get(key))
                        .map(Vec::as_slice);
                    let instance_root_key = std::fs::canonicalize(instance_root.as_path())
                        .unwrap_or_else(|_| instance_root.clone())
                        .display()
                        .to_string();
                    let account_running_root = launch_account
                        .as_deref()
                        .and_then(running_instance_for_account);
                    let launch_disabled_for_account = !runtime_running_for_active_account
                        && account_running_root
                            .as_deref()
                            .is_some_and(|running_root| running_root != instance_root_key.as_str());
                    let launch_disabled_for_missing_ownership =
                        !runtime_running_for_active_account && !active_account_owns_minecraft;
                    let launch_disabled =
                        launch_disabled_for_account || launch_disabled_for_missing_ownership;
                    let launch_in_flight = state.pending_launches.contains(instance.id.as_str());

                    let action = render_instance_tile(
                        ui,
                        text_ui,
                        instance,
                        runtime_running_for_active_account,
                        launch_disabled,
                        launch_in_flight,
                        launch_disabled_for_account,
                        launch_disabled_for_missing_ownership,
                        running_avatar,
                        selected_instance_id == Some(instance.id.as_str()),
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
                            let stopped = launch_account.as_deref().is_some_and(|account| {
                                stop_running_instance_for_account(instance_root.as_path(), account)
                            });
                            state.status_by_instance.insert(
                                instance.id.clone(),
                                if stopped {
                                    "Stopped instance runtime.".to_owned()
                                } else {
                                    "Instance runtime was not running for this account.".to_owned()
                                },
                            );
                        }
                        RuntimeAction::LaunchRequested => {
                            let requested = request_runtime_launch(
                                &mut state,
                                instance,
                                instance_root.clone(),
                                config,
                                launch_display_name.clone(),
                                launch_player_uuid.clone(),
                                launch_access_token.clone(),
                                launch_xuid.clone(),
                                launch_user_type.clone(),
                                launch_account.clone(),
                            );
                            if !requested {
                                state.status_by_instance.insert(
                                    instance.id.clone(),
                                    "Launch is already in progress.".to_owned(),
                                );
                            }
                        }
                    }
                }
            });
        });
    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
    output
}

fn render_instance_tile(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance: &InstanceRecord,
    runtime_running_for_active_account: bool,
    launch_disabled: bool,
    launch_in_flight: bool,
    launch_disabled_for_account: bool,
    launch_disabled_for_missing_ownership: bool,
    running_avatar_png: Option<&[u8]>,
    selected: bool,
) -> RuntimeAction {
    let tile_fill = if selected {
        ui.visuals().selection.bg_fill.gamma_multiply(0.22)
    } else {
        ui.visuals().widgets.noninteractive.bg_fill
    };
    let tile_stroke = if selected {
        ui.visuals().selection.stroke
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };

    let frame = egui::Frame::new()
        .fill(tile_fill)
        .stroke(tile_stroke)
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(10));

    let mut action = RuntimeAction::None;
    frame.show(ui, |ui| {
        ui.set_min_width(TILE_WIDTH);
        ui.set_max_width(TILE_WIDTH);
        ui.set_min_height(TILE_HEIGHT);
        ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_SM, style::SPACE_SM);
        ui.vertical(|ui| {
            render_instance_thumbnail(ui, instance);

            let name_style = LabelOptions {
                font_size: 22.0,
                line_height: 28.0,
                weight: 700,
                color: ui.visuals().text_color(),
                wrap: true,
                ..LabelOptions::default()
            };
            render_scroll_text_block(
                ui,
                ("library_instance_name", instance.id.as_str()),
                text_ui,
                instance.name.as_str(),
                &name_style,
                TILE_NAME_SCROLL_HEIGHT,
            );

            let _ = text_ui.label(
                ui,
                ("library_instance_version", instance.id.as_str()),
                &format!("Version: {}", instance.game_version),
                &LabelOptions {
                    color: ui.visuals().text_color(),
                    wrap: true,
                    ..LabelOptions::default()
                },
            );
            let _ = text_ui.label(
                ui,
                ("library_instance_modloader", instance.id.as_str()),
                &format!("Modloader: {}", instance.modloader),
                &LabelOptions {
                    color: ui.visuals().text_color(),
                    wrap: true,
                    ..LabelOptions::default()
                },
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
            let description_style = LabelOptions {
                color: if muted {
                    ui.visuals().weak_text_color()
                } else {
                    ui.visuals().text_color()
                },
                wrap: true,
                ..LabelOptions::default()
            };
            render_scroll_text_block(
                ui,
                ("library_instance_description", instance.id.as_str()),
                text_ui,
                description,
                &description_style,
                TILE_DESCRIPTION_SCROLL_HEIGHT,
            );

            let play_button_height = style::CONTROL_HEIGHT_LG;
            let remaining_height = (ui.available_height() - play_button_height).max(0.0);
            if remaining_height > 0.0 {
                ui.add_space(remaining_height);
            }

            let button_response = render_runtime_action_button(
                ui,
                instance.id.as_str(),
                runtime_running_for_active_account,
                launch_disabled,
                launch_in_flight,
                running_avatar_png,
            );
            if button_response.clicked() {
                if runtime_running_for_active_account {
                    action = RuntimeAction::StopRequested;
                } else if launch_disabled || launch_in_flight {
                    action = RuntimeAction::None;
                } else {
                    action = RuntimeAction::LaunchRequested;
                }
            }

            let mut muted_style = LabelOptions::default();
            muted_style.color = ui.visuals().weak_text_color();
            muted_style.wrap = true;
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
    });
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
    running_avatar_png: Option<&[u8]>,
) -> egui::Response {
    let desired_size = egui::vec2(ui.available_width().max(1.0), style::CONTROL_HEIGHT_LG);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    let (fill_base, stroke, text_color) = if runtime_running_for_active_account {
        let error = ui.visuals().error_fg_color;
        (
            egui::Color32::from_rgba_premultiplied(error.r(), error.g(), error.b(), 36),
            egui::Stroke::new(1.0, error),
            error,
        )
    } else if launch_in_flight {
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
    } else if response.hovered() {
        fill_base.gamma_multiply(1.1)
    } else {
        fill_base
    };

    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        stroke,
        egui::StrokeKind::Inside,
    );

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
        if launch_in_flight {
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
        let image = egui::Image::from_bytes(
            format!("bytes://library/runtime-avatar/{}", hasher.finish()),
            bytes.to_vec(),
        )
        .fit_to_exact_size(rect.size());
        let _ = ui.put(rect, image);
        return;
    }

    let fallback = egui::Image::from_bytes(
        format!("bytes://library/runtime-avatar-fallback/{instance_id}.svg"),
        apply_color_to_svg(assets::USER_SVG, ui.visuals().text_color()),
    )
    .fit_to_exact_size(rect.size());
    let _ = ui.put(rect, fallback);
}

fn apply_color_to_svg(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeAction {
    None,
    LaunchRequested,
    StopRequested,
}

#[derive(Debug, Clone, Default)]
struct LibraryRuntimeState {
    results_tx: Option<mpsc::Sender<RuntimeLaunchResult>>,
    results_rx: Option<Arc<Mutex<mpsc::Receiver<RuntimeLaunchResult>>>>,
    pending_launches: HashSet<String>,
    status_by_instance: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct RuntimeLaunchResult {
    instance_id: String,
    result: Result<RuntimeLaunchOutcome, String>,
}

#[derive(Debug, Clone)]
struct RuntimeLaunchOutcome {
    launch: LaunchResult,
    downloaded_files: u32,
    resolved_modloader_version: Option<String>,
    configured_java: Option<(u8, String)>,
}

fn ensure_result_channel(state: &mut LibraryRuntimeState) {
    if state.results_tx.is_some() && state.results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<RuntimeLaunchResult>();
    state.results_tx = Some(tx);
    state.results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_runtime_launch(
    state: &mut LibraryRuntimeState,
    instance: &InstanceRecord,
    instance_root: PathBuf,
    config: &Config,
    player_name: Option<String>,
    player_uuid: Option<String>,
    access_token: Option<String>,
    xuid: Option<String>,
    user_type: Option<String>,
    launch_account_name: Option<String>,
) -> bool {
    if state.pending_launches.contains(instance.id.as_str()) {
        return false;
    }

    let game_version = instance.game_version.trim().to_owned();
    if game_version.is_empty() {
        state.status_by_instance.insert(
            instance.id.clone(),
            "Cannot launch: choose a Minecraft game version first.".to_owned(),
        );
        return false;
    }

    ensure_result_channel(state);
    let Some(tx) = state.results_tx.as_ref().cloned() else {
        return false;
    };

    let instance_id = instance.id.clone();
    state.pending_launches.insert(instance_id.clone());
    state.status_by_instance.insert(
        instance_id.clone(),
        format!("Preparing Minecraft {}...", game_version),
    );

    let modloader = instance.modloader.trim().to_owned();
    let modloader_version = normalize_optional(instance.modloader_version.as_str());
    let required_java_major = effective_required_java_major(config, game_version.as_str());
    let java_executable = choose_java_executable(config, instance, required_java_major);
    let download_policy = DownloadPolicy {
        max_concurrent_downloads: config.download_max_concurrent().max(1),
        max_download_bps: config.parsed_download_speed_limit_bps(),
    };
    let max_memory_mib = instance
        .max_memory_mib
        .unwrap_or(config.default_instance_max_memory_mib());
    let extra_jvm_args = instance
        .cli_args
        .as_deref()
        .and_then(normalize_optional)
        .or_else(|| normalize_optional(config.default_instance_cli_args()));

    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            let mut configured_java = None;
            let java_path = if let Some(path) = java_executable
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .filter(|value| Path::new(value).exists())
                .map(str::to_owned)
            {
                path
            } else if let Some(runtime_major) = required_java_major {
                let installed = ensure_openjdk_runtime(runtime_major).map_err(|err| {
                    format!("failed to auto-install OpenJDK {runtime_major}: {err}")
                })?;
                let installed = installed.display().to_string();
                configured_java = Some((runtime_major, installed.clone()));
                installed
            } else {
                "java".to_owned()
            };

            let setup = ensure_game_files(
                instance_root.as_path(),
                game_version.as_str(),
                modloader.as_str(),
                modloader_version.as_deref(),
                Some(java_path.as_str()),
                &download_policy,
                None,
            )
            .map_err(|err| err.to_string())?;

            let launch_request = LaunchRequest {
                instance_root: instance_root.clone(),
                game_version: game_version.clone(),
                modloader: modloader.clone(),
                modloader_version: modloader_version.clone(),
                account_key: launch_account_name.clone(),
                java_executable: Some(java_path),
                max_memory_mib,
                extra_jvm_args: extra_jvm_args.clone(),
                player_name: player_name.clone().or(launch_account_name.clone()),
                player_uuid: player_uuid.clone(),
                auth_access_token: access_token.clone(),
                auth_xuid: xuid.clone(),
                auth_user_type: user_type.clone(),
            };
            let launch = launch_instance(&launch_request).map_err(|err| err.to_string())?;
            Ok(RuntimeLaunchOutcome {
                launch,
                downloaded_files: setup.downloaded_files,
                resolved_modloader_version: setup.resolved_modloader_version,
                configured_java,
            })
        })
        .await
        .map_err(|err| format!("runtime launch task join error: {err}"))
        .and_then(|inner| inner);

        let _ = tx.send(RuntimeLaunchResult {
            instance_id,
            result,
        });
    });
    true
}

fn poll_runtime_actions(state: &mut LibraryRuntimeState, config: &mut Config) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.results_rx.as_ref() {
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
    }

    if should_reset_channel {
        state.results_tx = None;
        state.results_rx = None;
    }

    for update in updates {
        state.pending_launches.remove(update.instance_id.as_str());
        match update.result {
            Ok(outcome) => {
                if let Some((runtime_major, path)) = outcome.configured_java
                    && let Some(runtime) = java_runtime_from_major(runtime_major)
                {
                    config.set_java_runtime_path(runtime, Some(path));
                }
                state.status_by_instance.insert(
                    update.instance_id,
                    format!(
                        "Launched (pid {}, profile {}, {} file(s), loader {}).",
                        outcome.launch.pid,
                        outcome.launch.profile_id,
                        outcome.downloaded_files,
                        outcome
                            .resolved_modloader_version
                            .as_deref()
                            .unwrap_or("n/a"),
                    ),
                );
            }
            Err(err) => {
                state
                    .status_by_instance
                    .insert(update.instance_id, format!("Launch failed: {err}"));
            }
        }
    }
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn choose_java_executable(
    config: &Config,
    instance: &InstanceRecord,
    required_java_major: Option<u8>,
) -> Option<String> {
    if instance.java_override_enabled
        && let Some(override_major) = instance.java_override_runtime_major
        && let Some(runtime) = java_runtime_from_major(override_major)
        && let Some(path) = config.java_runtime_path(runtime)
    {
        let trimmed = path.trim();
        if !trimmed.is_empty() && Path::new(trimmed).exists() {
            return Some(trimmed.to_owned());
        }
    }

    if let Some(runtime_major) = required_java_major
        && let Some(runtime) = java_runtime_from_major(runtime_major)
        && let Some(path) = config.java_runtime_path(runtime)
    {
        let trimmed = path.trim();
        if !trimmed.is_empty() && Path::new(trimmed).exists() {
            return Some(trimmed.to_owned());
        }
    }
    None
}

fn required_java_major(game_version: &str) -> Option<u8> {
    let mut parts = game_version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or(0);

    if major != 1 {
        return Some(21);
    }
    if minor <= 16 {
        return Some(8);
    }
    if minor == 17 {
        return Some(16);
    }
    if minor >= 21 {
        return u8::try_from(minor).ok();
    }
    if minor > 20 || (minor == 20 && patch >= 5) {
        return Some(21);
    }
    Some(17)
}

fn effective_required_java_major(config: &Config, game_version: &str) -> Option<u8> {
    let required = required_java_major(game_version)?;
    if config.force_java_21_minimum() && required < 21 {
        Some(21)
    } else {
        Some(required)
    }
}

fn java_runtime_from_major(major: u8) -> Option<JavaRuntimeVersion> {
    match major {
        8 => Some(JavaRuntimeVersion::Java8),
        16 => Some(JavaRuntimeVersion::Java16),
        17 => Some(JavaRuntimeVersion::Java17),
        21 => Some(JavaRuntimeVersion::Java21),
        _ => None,
    }
}

fn render_instance_thumbnail(ui: &mut Ui, instance: &InstanceRecord) {
    let thumbnail_width = (TILE_WIDTH - 20.0).max(120.0);
    let thumbnail_size = egui::vec2(thumbnail_width, TILE_THUMBNAIL_HEIGHT);

    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(6));

    frame.show(ui, |ui| {
        if let Some(path) = instance
            .thumbnail_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && let Ok(bytes) = std::fs::read(path)
        {
            let mut hasher = DefaultHasher::new();
            instance.id.hash(&mut hasher);
            path.hash(&mut hasher);
            let uri = format!(
                "bytes://library/instance-thumbnail/{:016x}",
                hasher.finish()
            );
            ui.add(egui::Image::from_bytes(uri, bytes).fit_to_exact_size(thumbnail_size));
            return;
        }

        let placeholder_size = egui::vec2(42.0, 42.0);
        let placeholder = egui::Image::from_bytes(
            format!(
                "bytes://library/instance-thumbnail-default/{}.svg",
                instance.id
            ),
            assets::LIBRARY_SVG,
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

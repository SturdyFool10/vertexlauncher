//! Library: every instance as a card with launch/stop, open folder and delete, plus the shared launch helpers the
//! sidebar menu and the instance screen use. Launching goes through `launcher_ui`'s library runtime, the same code
//! the egui library uses.

use std::path::PathBuf;

use installation::{
    is_instance_running, is_instance_running_for_account, normalize_path_key,
    running_instance_for_account, stop_running_instance_for_account,
};
use instances::{InstanceRecord, instance_root_path};
use launcher_ui::{desktop, notification};
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, Border, ChildAlignment, CornerRadius, Direction, ElementDesc, Padding,
    Sizing, TextStyle, TextWrap, Ui,
};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::icons::Icon;
use super::super::kit::{self, ButtonKind, Kit, TextRole};
use super::super::{AppState, Dialog};

/// Who launches: the active account's credentials.
struct Identity {
    account: Option<String>,
    display_name: Option<String>,
    player_uuid: Option<String>,
    access_token: Option<String>,
    xuid: Option<String>,
    user_type: Option<String>,
}

fn identity(state: &AppState) -> Identity {
    let auth = state.auth.active_launch_context();
    Identity {
        account: auth
            .as_ref()
            .map(|a| a.account_key.clone())
            .or_else(|| state.auth.display_name().map(str::to_owned)),
        display_name: auth
            .as_ref()
            .map(|a| a.player_name.clone())
            .or_else(|| state.auth.display_name().map(str::to_owned)),
        player_uuid: auth.as_ref().map(|a| a.player_uuid.clone()),
        access_token: auth.as_ref().and_then(|a| a.access_token.clone()),
        xuid: auth.as_ref().and_then(|a| a.xuid.clone()),
        user_type: auth.as_ref().map(|a| a.user_type.clone()),
    }
}

/// Whether and why an instance can be launched or stopped right now.
pub struct LaunchStatus {
    pub running_for_account: bool,
    pub running_any: bool,
    pub launching: bool,
    pub installing: bool,
    /// Why launch is unavailable, if it is.
    pub blocked: Option<&'static str>,
}

pub fn launch_status(state: &AppState, instance: &InstanceRecord) -> LaunchStatus {
    let root = instance_root_path(state.installations_root().as_path(), instance);
    let identity = identity(state);
    let running_for_account = identity
        .account
        .as_deref()
        .is_some_and(|account| is_instance_running_for_account(root.as_path(), account));
    let other_running = identity
        .account
        .as_deref()
        .and_then(running_instance_for_account)
        .is_some_and(|running| running != normalize_path_key(root.as_path()));
    let blocked = if running_for_account {
        None
    } else if state.auth.active_launch_context().is_none() {
        Some("Sign in with an account that owns Minecraft to launch.")
    } else if state.auth.token_refresh_in_progress() {
        Some("Waiting for account token refresh...")
    } else if other_running {
        Some("This account is already running another instance.")
    } else {
        None
    };
    LaunchStatus {
        running_for_account,
        running_any: is_instance_running(root.as_path()),
        launching: state
            .runtime
            .pending_launches
            .contains(instance.id.as_str()),
        installing: launcher_ui::install_activity::is_instance_installing(instance.id.as_str()),
        blocked,
    }
}

/// A launch that is waiting for the active account to be renewed first ("Lazy Account Refresh").
pub struct DeferredLaunch {
    instance_id: String,
    quick_play_singleplayer: Option<String>,
    quick_play_multiplayer: Option<String>,
    /// The account the launch was asked for; if the active account changes meanwhile the launch is dropped.
    account: Option<String>,
}

/// Runs a launch that was waiting for the account renewal, once the account is ready.
pub fn poll_deferred_launch(state: &mut AppState) {
    let Some(deferred) = state.deferred_launch.take() else {
        return;
    };
    if state.auth.active_account_key() != deferred.account {
        notification::warn!("library", "The active account changed, so the launch was cancelled.");
        return;
    }
    if !state.auth.ensure_active_fresh() {
        state.deferred_launch = Some(deferred);
        return;
    }
    launch_with(
        state,
        &deferred.instance_id,
        deferred.quick_play_singleplayer,
        deferred.quick_play_multiplayer,
    );
}

/// Launches `instance_id` with the active account (no-op with a status message when that is not possible).
pub fn launch(state: &mut AppState, instance_id: &str) {
    launch_with(state, instance_id, None, None);
}

/// Launches `instance_id`, optionally straight into a singleplayer world or onto a multiplayer server.
pub fn launch_with(
    state: &mut AppState,
    instance_id: &str,
    quick_play_singleplayer: Option<String>,
    quick_play_multiplayer: Option<String>,
) {
    let Some(instance) = state.instances.find(instance_id).cloned() else {
        return;
    };
    // With "Lazy Account Refresh" the active account is renewed now, before the launch uses its token. Only that one
    // account; the launch goes ahead from `poll_deferred_launch` once it is done.
    if !state.auth.ensure_active_fresh() {
        if state.deferred_launch.is_none() {
            notification::info!("library", "Refreshing your account before launching...");
        }
        state.deferred_launch = Some(DeferredLaunch {
            instance_id: instance_id.to_owned(),
            quick_play_singleplayer,
            quick_play_multiplayer,
            account: state.auth.active_account_key(),
        });
        return;
    }
    let status = launch_status(state, &instance);
    if let Some(reason) = status.blocked {
        notification::warn!("library", "{}", reason);
        return;
    }
    let root = instance_root_path(state.installations_root().as_path(), &instance);
    let identity = identity(state);
    let requested = launcher_ui::screens::request_library_runtime_launch(
        &mut state.runtime,
        &instance,
        root,
        &state.config,
        identity.display_name,
        identity.player_uuid,
        identity.access_token,
        identity.xuid,
        identity.user_type,
        identity.account,
        quick_play_singleplayer,
        quick_play_multiplayer,
    );
    if !requested {
        state.runtime.status_by_instance.insert(
            instance.id.clone(),
            "Launch is already in progress.".to_owned(),
        );
    }
}

pub fn stop(state: &mut AppState, instance_id: &str) {
    let Some(instance) = state.instances.find(instance_id).cloned() else {
        return;
    };
    let root = instance_root_path(state.installations_root().as_path(), &instance);
    let stopped = identity(state)
        .account
        .as_deref()
        .is_some_and(|account| stop_running_instance_for_account(root.as_path(), account));
    state.runtime.status_by_instance.insert(
        instance.id.clone(),
        if stopped {
            "Stopped instance runtime.".to_owned()
        } else {
            "Instance runtime was not running for this account.".to_owned()
        },
    );
}

pub fn open_folder(state: &mut AppState, instance_id: &str) {
    let Some(root) = instance_root(state, instance_id) else {
        return;
    };
    if let Err(err) = desktop::open_in_file_manager(root.as_path()) {
        notification::error!("library", "Failed to open instance folder: {}", err);
    }
}

pub fn instance_root(state: &AppState, instance_id: &str) -> Option<PathBuf> {
    let instance = state.instances.find(instance_id)?;
    Some(instance_root_path(
        state.installations_root().as_path(),
        instance,
    ))
}

/// The instance's thumbnail, or a tinted square with its initial when it has none.
pub fn thumbnail(ui: &mut Ui<'_>, kit: &mut Kit, instance: &InstanceRecord, id: &str, size: f32) {
    use super::super::images::ImageSource;
    if let Some(path) = instance.thumbnail_path.clone()
        && !kit.images.failed(&format!("thumb::{}", path.display()))
    {
        let key = format!("thumb::{}", path.display());
        kit.image(ui, id, &key, || ImageSource::File(path), size, size, 12.0);
        return;
    }
    let initial = instance
        .name
        .chars()
        .find(|ch| ch.is_alphanumeric())
        .map(|ch| ch.to_uppercase().to_string())
        .unwrap_or_default();
    let _tile = ui.element(
        &ElementDesc::new()
            .id(id.into())
            .width(Sizing::Fixed(size))
            .height(Sizing::Fixed(size))
            .align(ChildAlignment::center())
            .background(kit.palette.primary.with_w(0.3))
            .corner_radius(CornerRadius::all(12.0)),
    );
    kit.text(ui, TextRole::Heading, &initial);
}

pub fn loader_label(instance: &InstanceRecord) -> String {
    let loader = instance.modloader.trim();
    let version = instance.modloader_version.trim();
    match (loader, version) {
        ("" | "Vanilla", _) => format!("Minecraft {}", instance.game_version),
        (loader, "") => format!("{loader} · Minecraft {}", instance.game_version),
        (loader, version) => format!("{loader} {version} · Minecraft {}", instance.game_version),
    }
}

/// Launch/stop button for one instance. Returns nothing; acts directly on state.
pub fn launch_button(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    instance: &InstanceRecord,
    id: &str,
) {
    let status = launch_status(state, instance);
    if status.running_for_account {
        if kit.button(ui, id, "Stop", ButtonKind::Danger, true) {
            stop(state, &instance.id);
        }
    } else {
        let label = if status.launching {
            "Launching…"
        } else if status.installing {
            "Installing…"
        } else {
            "Launch"
        };
        let enabled = status.blocked.is_none() && !status.launching && !status.installing;
        if kit.button_icon_right(
            ui,
            id,
            Icon::Play,
            label,
            ButtonKind::Primary,
            enabled,
            Sizing::Fit,
        ) {
            launch(state, &instance.id);
        }
        if let Some(reason) = status.blocked {
            kit.tooltip(ui, id, reason);
        }
    }
}

pub fn render(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let _scroll = kit.scroll(ui, "library::scroll", kit::SPACE_LG, kit::SPACE_XL as u16);
    {
        let _header = kit.row(ui, kit::SPACE_SM);
        kit.text(ui, TextRole::Title, "Library");
        kit.fill(ui);
        if kit.button(ui, "library::import", "Import", ButtonKind::Neutral, true) {
            state.open_import_dialog();
        }
        if kit.button(
            ui,
            "library::create",
            "Create instance",
            ButtonKind::Primary,
            true,
        ) {
            state.dialog = Some(Dialog::CreateInstance);
        }
    }
    if state.instances.instances.is_empty() {
        kit.text(ui, TextRole::Muted, "No instances created yet.");
        return;
    }

    let viewport = ui
        .element_bounds("library::scroll")
        .map_or(900.0, |(_, size)| size.x);
    let columns = ((viewport - 48.0) / 300.0).floor().max(1.0) as usize;
    let instances = state.instances.instances.clone();
    for row in instances.chunks(columns) {
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_LG),
        );
        for instance in row {
            card(ui, kit, state, instance);
        }
        // Keep the last row's cards the same width as full rows.
        for _ in row.len()..columns {
            drop(
                ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Grow)
                        .height(Sizing::Fixed(1.0)),
                ),
            );
        }
    }
}

fn card(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, instance: &InstanceRecord) {
    let id = format!("library::card::{}", instance.id);
    let hovered = ui.hovered(&id);
    let status = launch_status(state, instance);
    let border = if status.running_for_account {
        kit.palette.success
    } else if hovered {
        kit.palette.border
    } else {
        kit.palette.border_muted
    };
    {
        let _card = ui.element(
            &ElementDesc::new()
                .id(id.clone())
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_SM)
                .padding(Padding::all(kit::SPACE_LG))
                .background(if hovered {
                    kit.palette.bg_light
                } else {
                    kit.palette.bg
                })
                .corner_radius(CornerRadius::all(kit::CORNER))
                .border(Border::all(border, 1)),
        );
        {
            let _head = kit.row(ui, kit::SPACE_MD);
            thumbnail(ui, kit, instance, &format!("{id}::thumb"), 56.0);
            let _titles = kit.column(ui, kit::SPACE_XS);
            kit.text_clipped(ui, TextRole::Heading, &instance.name, 190.0);
            kit.text_clipped(ui, TextRole::Muted, &loader_label(instance), 190.0);
        }
        if let Some(message) = state.runtime.status_by_instance.get(&instance.id) {
            kit::label(
                ui,
                message,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Small)
                },
            );
        }
        if status.installing
            && let Some(activity) =
                launcher_ui::install_activity::snapshot().filter(|a| a.instance_id == instance.id)
        {
            let fraction = (activity.total_files > 0)
                .then(|| activity.downloaded_files as f32 / activity.total_files as f32);
            kit.progress(
                ui,
                &format!("{id}::install"),
                fraction,
                state.started.elapsed().as_secs_f64(),
            );
        }
        {
            let _actions = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::LeftToRight)
                    .child_gap(kit::SPACE_SM)
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
            );
            launch_button(ui, kit, state, instance, &format!("{id}::launch"));
            kit.fill(ui);
            if kit.button(
                ui,
                &format!("{id}::folder"),
                "Folder",
                ButtonKind::Neutral,
                true,
            ) {
                open_folder(state, &instance.id);
            }
            // The folder cannot be deleted while something is using it; hovering says why.
            let delete_reason = if status.installing {
                Some("Wait for installation to finish before deleting this instance.")
            } else if status.launching {
                Some("Wait for launch preparation to finish before deleting this instance.")
            } else if status.running_any {
                Some("Stop the running instance before deleting its folder.")
            } else {
                None
            };
            let delete_id = format!("{id}::delete");
            if kit.button(
                ui,
                &delete_id,
                "Delete",
                ButtonKind::Danger,
                delete_reason.is_none(),
            ) {
                state.dialog = Some(Dialog::ConfirmDelete(instance.id.clone()));
            }
            kit.tooltip(
                ui,
                &delete_id,
                delete_reason.unwrap_or("Delete This Instance"),
            );
        }
        if let Some(reason) = status.blocked {
            kit.text(ui, TextRole::Muted, reason);
        }
    }
    // A click on the card itself (not one of its buttons) opens the instance.
    let on_button = [
        format!("{id}::launch"),
        format!("{id}::folder"),
        format!("{id}::delete"),
    ]
    .iter()
    .any(|button| ui.hovered(button));
    if kit.clicked(ui, &id) && !on_button {
        state.open_instance(&instance.id);
    }
}

/// Confirms deleting an instance and its folder. Returns false once the dialog should close.
pub fn confirm_delete(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    instance_id: &str,
) -> bool {
    let Some(instance) = state.instances.find(instance_id).cloned() else {
        return false;
    };
    let deleting = state.runtime.delete_in_flight;
    let root = instance_root_path(state.installations_root().as_path(), &instance);
    let running = is_instance_running(root.as_path());
    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(520.0),
        ..ModalStyle::default()
    };
    let mut close = false;
    let mut delete = false;
    let error = state.runtime.delete_error.clone();
    let response = kit::modal(ui, "library::delete", &style, !deleting, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text_colored(ui, "Delete Instance Folder?", kit.palette.danger, 1.3);
        kit::label(
            ui,
            &format!(
                "Delete the whole folder for \"{}\". This permanently removes installed content and personal files, including worlds.",
                instance.name
            ),
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Body)
            },
        );
        kit::label(
            ui,
            &format!("Folder: {}", root.display()),
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
        if running {
            kit.text(
                ui,
                TextRole::Danger,
                "Stop the running instance before deleting its folder.",
            );
        }
        if let Some(error) = &error {
            kit::label(
                ui,
                error,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
        }
        if deleting {
            kit.progress(ui, "library::delete::progress", None, kit::uptime());
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        // Cancel is the default action, so a stray Enter never deletes anything.
        if kit.button(
            ui,
            "library::delete::cancel",
            "Cancel",
            ButtonKind::Primary,
            !deleting,
        ) {
            close = true;
        }
        if kit.button(
            ui,
            "library::delete::confirm",
            "Delete Folder",
            ButtonKind::Danger,
            !running && !deleting,
        ) {
            delete = true;
        }
    });
    if delete {
        let root = state.installations_root();
        launcher_ui::screens::request_library_instance_delete(&mut state.runtime, instance, root);
    }
    if close || ((response.dismissed || kit.back_pressed()) && !deleting) {
        state.runtime.delete_error = None;
        return false;
    }
    true
}

/// Puts the quick-launch command line (or Steam launch options) for `instance_id` on the clipboard.
pub fn copy_quick_launch(ui: &mut Ui<'_>, state: &mut AppState, instance_id: &str, steam: bool) {
    use launcher_ui::screens::{
        QuickLaunchCommandMode, build_quick_launch_command, build_quick_launch_steam_options,
    };
    let user = state
        .auth
        .active_launch_context()
        .map(|context| context.player_uuid)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| state.auth.display_name().map(str::to_owned));
    let Some(user) = user else {
        launcher_ui::notification::warn!(
            "instance/quick_launch",
            "{}",
            if steam {
                "Sign in before copying Steam launch options."
            } else {
                "Sign in before copying an instance command line."
            }
        );
        return;
    };
    let text = if steam {
        build_quick_launch_steam_options(
            QuickLaunchCommandMode::Pack,
            instance_id,
            &user,
            None,
            None,
        )
    } else {
        build_quick_launch_command(QuickLaunchCommandMode::Pack, instance_id, &user, None, None)
    };
    let _ = ui
        .engine()
        .primary_window_control()
        .map(|mut window| window.set_clipboard_text(&text));
    launcher_ui::notification::info!(
        "instance/quick_launch",
        "{}",
        if steam {
            "Copied Steam launch options to clipboard."
        } else {
            "Copied instance command line to clipboard."
        }
    );
}

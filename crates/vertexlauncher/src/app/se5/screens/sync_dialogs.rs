//! Home's sync dialogs: which instances a multiplayer server is kept in, and sharing a world between instances.

use vertex_i18n::tr;
use std::collections::BTreeSet;

use instance_sync::compare_game_versions;
use instances::ServerSyncScope;
use launcher_ui::screens::home_api::{EntryKind, EntryView};
use launcher_ui::sync_runner::{
    self, SyncRunConfig, WorldSyncReceiver, WorldSyncRequest, WorldSyncResult,
};
use vertex_shell::sturdy::ui::{Clip, Direction, ElementDesc, Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::kit::{self, ButtonKind, Kit, TextRole};
use super::super::{AppState, Dialog};

fn modal_style(kit: &Kit, ui: &Ui<'_>, width: f32) -> ModalStyle {
    ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(width.min(ui.viewport_size().x * 0.9)),
        ..ModalStyle::default()
    }
}

fn instance_list(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    height: f32,
    body: impl FnOnce(&mut Ui<'_>, &mut Kit),
) {
    let _scroll = ui.element(
        &ElementDesc::new()
            .id(id.into())
            .width(Sizing::Grow)
            .height(Sizing::Fixed(height))
            .direction(Direction::TopToBottom)
            .child_gap(kit::SPACE_XS)
            .clip(Clip::vertical()),
    );
    kit.scrollbar(ui, id);
    body(ui, kit);
}

fn wrapped(kit: &Kit, role: TextRole) -> TextStyle {
    TextStyle {
        wrap: TextWrap::Words,
        ..kit.style(role)
    }
}

// -- Server ---------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeChoice {
    FollowDefault,
    ThisInstanceOnly,
    AllInstances,
    Selected,
}

pub struct ServerSyncState {
    address: String,
    name: String,
    choice: ScopeChoice,
    selected: BTreeSet<String>,
}

/// Opens the sync options dialog for a server entry, starting from its saved scope.
pub fn open_server(state: &mut AppState, entry: &EntryView) {
    if entry.kind != EntryKind::Server {
        return;
    }
    let Some(primary) = entry.targets.first() else {
        return;
    };
    let scope = state.instances.server_sync_scope(&primary.target);
    let selected = match &scope {
        ServerSyncScope::Selected(ids) => ids.clone(),
        // Start a new selection with the instance the server was found in.
        _ => BTreeSet::from([primary.instance_id.clone()]),
    };
    let choice = match scope {
        ServerSyncScope::FollowDefault => ScopeChoice::FollowDefault,
        ServerSyncScope::ThisInstanceOnly => ScopeChoice::ThisInstanceOnly,
        ServerSyncScope::AllInstances => ScopeChoice::AllInstances,
        ServerSyncScope::Selected(_) => ScopeChoice::Selected,
    };
    state.server_sync = Some(ServerSyncState {
        address: primary.target.clone(),
        name: entry.title.clone(),
        choice,
        selected,
    });
    state.dialog = Some(Dialog::ServerSync);
}

/// Returns false once the dialog should close.
pub fn server_dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) -> bool {
    let AppState {
        server_sync,
        instances,
        config,
        ..
    } = state;
    let Some(sync) = server_sync.as_mut() else {
        return false;
    };
    let style = modal_style(kit, ui, 620.0);
    let mut save = false;
    let mut cancel = false;
    let response = kit::modal(ui, "server-sync", &style, true, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, &tr!("Server sync options"));
        kit.text(
            ui,
            TextRole::Strong,
            &format!("{} ({})", sync.name, sync.address),
        );
        kit::label(
            ui,
            &tr!("Choose which instances get this server in their server list. Entries that already exist are never changed or removed."),
            &wrapped(kit, TextRole::Muted),
        );
        if !config.sync_servers_enabled() {
            kit::label(
                ui,
                &tr!("Server sync is turned off. Enable it in Settings → Instance Sync for these options to take effect."),
                &wrapped(kit, TextRole::Warning),
            );
        }
        let default_label = if config.sync_servers_to_all_instances_by_default() {
            tr!("Follow launcher default (currently: every instance)")
        } else {
            tr!("Follow launcher default (currently: only where it already exists)")
        };
        for (choice, label) in [
            (ScopeChoice::FollowDefault, default_label),
            (
                ScopeChoice::ThisInstanceOnly,
                tr!("Only where it already exists"),
            ),
            (ScopeChoice::AllInstances, tr!("Every instance")),
            (ScopeChoice::Selected, tr!("Selected instances")),
        ] {
            if kit.radio(
                ui,
                &format!("server-sync::choice::{choice:?}"),
                &label,
                sync.choice == choice,
            ) {
                sync.choice = choice;
            }
        }
        if sync.choice == ScopeChoice::Selected {
            instance_list(ui, kit, "server-sync::instances", 260.0, |ui, kit| {
                for instance in &instances.instances {
                    let label = tr!("{name} (Minecraft {game_version})", name = instance.name, game_version = instance.game_version).into_owned();
                    let checked = sync.selected.contains(&instance.id);
                    if let Some(next) = kit.checkbox(
                        ui,
                        &format!("server-sync::instance::{}", instance.id),
                        &label,
                        checked,
                    ) {
                        if next {
                            sync.selected.insert(instance.id.clone());
                        } else {
                            sync.selected.remove(&instance.id);
                        }
                    }
                }
            });
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        cancel = kit.button(
            ui,
            "server-sync::cancel",
            &tr!("Cancel"),
            ButtonKind::Neutral,
            true,
        );
        save = kit.button(ui, "server-sync::save", &tr!("Save"), ButtonKind::Primary, true);
    });
    if save {
        let scope = match sync.choice {
            ScopeChoice::FollowDefault => ServerSyncScope::FollowDefault,
            ScopeChoice::ThisInstanceOnly => ServerSyncScope::ThisInstanceOnly,
            ScopeChoice::AllInstances => ServerSyncScope::AllInstances,
            ScopeChoice::Selected => ServerSyncScope::Selected(sync.selected.clone()),
        };
        instances.set_server_sync_scope(&sync.address, scope);
        // Apply right away instead of waiting for the next launch or exit.
        sync_runner::spawn(instances.clone(), SyncRunConfig::from_config(config));
    }
    let done = save || cancel || (response.dismissed || kit.back_pressed());
    if done {
        *server_sync = None;
        kit.forget("server-sync");
    }
    !done
}

// -- World ----------------------------------------------------------------------------------------------------------

pub struct WorldSyncState {
    /// Instance the world was picked from; it always stays a member.
    instance_id: String,
    folder: String,
    world_name: String,
    world_version: Option<String>,
    selected: BTreeSet<String>,
    pending: Option<WorldSyncReceiver>,
    /// `(is_error, text)` from the last operation.
    message: Option<(bool, String)>,
    warnings: Vec<String>,
}

/// Opens the sync dialog for a world entry.
pub fn open_world(state: &mut AppState, entry: &EntryView) {
    if entry.kind != EntryKind::World {
        return;
    }
    let Some(primary) = entry.targets.first() else {
        return;
    };
    let selected = match state
        .instances
        .synced_world_for(&primary.instance_id, &primary.target)
    {
        Some(synced) => synced.members.keys().cloned().collect(),
        None => BTreeSet::from([primary.instance_id.clone()]),
    };
    state.world_sync = Some(WorldSyncState {
        instance_id: primary.instance_id.clone(),
        folder: primary.target.clone(),
        world_name: entry.title.clone(),
        world_version: entry.world_version.clone(),
        selected,
        pending: None,
        message: None,
        warnings: Vec::new(),
    });
    state.dialog = Some(Dialog::WorldSync);
}

fn poll_world(
    sync: &mut WorldSyncState,
    instances: &mut instances::InstanceStore,
    home: &mut launcher_ui::screens::home_api::HomeSession,
) {
    let Some(receiver) = sync.pending.clone() else {
        return;
    };
    let result = receiver.lock().ok().and_then(|rx| rx.try_recv().ok());
    let Some(WorldSyncResult {
        synced_worlds,
        outcome,
    }) = result
    else {
        return;
    };
    sync.pending = None;
    match outcome {
        Ok(outcome) => {
            instances.synced_worlds = synced_worlds;
            sync.warnings = outcome.warnings;
            sync.message = Some((false, tr!("Done.").into_owned()));
            // Reflect what actually ended up shared, e.g. members skipped for conflicts.
            sync.selected = match instances.synced_world_for(&sync.instance_id, &sync.folder) {
                Some(synced) => synced.members.keys().cloned().collect(),
                None => BTreeSet::from([sync.instance_id.clone()]),
            };
            home.request_rescan();
        }
        Err(err) => sync.message = Some((true, err)),
    }
}

/// Returns false once the dialog should close.
pub fn world_dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) -> bool {
    let AppState {
        world_sync,
        instances,
        config,
        home,
        ..
    } = state;
    let Some(sync) = world_sync.as_mut() else {
        return false;
    };
    poll_world(sync, instances, home);
    let synced_id = instances
        .synced_world_for(&sync.instance_id, &sync.folder)
        .map(|world| world.id.clone());
    let busy = sync.pending.is_some();
    let mut request: Option<WorldSyncRequest> = None;
    let mut close = false;
    let style = modal_style(kit, ui, 640.0);
    let response = kit::modal(ui, "world-sync", &style, !busy, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, &tr!("Sync world"));
        kit.text(ui, TextRole::Strong, &sync.world_name);
        let help = if synced_id.is_some() {
            &tr!("This world is stored once and linked into each instance below, so progress made in any of them appears in all of them. Removing an instance gives it its own copy.")
        } else {
            &tr!("The world moves to Vertex's shared worlds folder and each instance you pick gets a link to it (or a copy that refreshes at launch and exit where links aren't possible). Progress made in any of them appears in all of them. Close the game in every picked instance first.")
        };
        kit::label(ui, help, &wrapped(kit, TextRole::Muted));
        instance_list(ui, kit, "world-sync::instances", 280.0, |ui, kit| {
            for instance in &instances.instances {
                let label = tr!("{name} (Minecraft {game_version})", name = instance.name, game_version = instance.game_version).into_owned();
                let checked = sync.selected.contains(&instance.id);
                if let Some(next) = kit.checkbox(
                    ui,
                    &format!("world-sync::instance::{}", instance.id),
                    &label,
                    checked,
                ) && !busy
                {
                    if next {
                        sync.selected.insert(instance.id.clone());
                    } else {
                        sync.selected.remove(&instance.id);
                    }
                }
            }
        });
        if synced_id.is_none() {
            // The instance the world lives in is always part of the group.
            sync.selected.insert(sync.instance_id.clone());
        }
        if let Some(world_version) = sync.world_version.as_deref() {
            for instance in &instances.instances {
                if sync.selected.contains(&instance.id)
                    && compare_game_versions(&instance.game_version, world_version)
                        == Some(std::cmp::Ordering::Less)
                {
                    kit::label(
                        ui,
                        &tr!("{name} runs Minecraft {game_version}, older than this world ({world_version}). Opening it there can corrupt the world.", name = instance.name, game_version = instance.game_version, world_version = world_version),
                        &wrapped(kit, TextRole::Warning),
                    );
                }
            }
        }
        for warning in &sync.warnings {
            kit::label(ui, warning, &wrapped(kit, TextRole::Warning));
        }
        if busy {
            kit.text(
                ui,
                TextRole::Muted,
                &tr!("Working… moving or copying a large world can take a while."),
            );
            kit.progress(ui, "world-sync::progress", None, kit::uptime());
        } else if let Some((is_error, message)) = &sync.message {
            kit::label(
                ui,
                message,
                &wrapped(
                    kit,
                    if *is_error {
                        TextRole::Danger
                    } else {
                        TextRole::Muted
                    },
                ),
            );
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        close = kit.button(ui, "world-sync::close", &tr!("Close"), ButtonKind::Neutral, true);
        if let Some(world_id) = synced_id.clone() {
            if kit.button(
                ui,
                "world-sync::stop",
                &tr!("Stop syncing"),
                ButtonKind::Danger,
                !busy,
            ) {
                request = Some(WorldSyncRequest::Disable {
                    world_id: world_id.clone(),
                });
            }
            if kit.button(
                ui,
                "world-sync::apply",
                &tr!("Apply changes"),
                ButtonKind::Primary,
                !busy,
            ) {
                request = Some(WorldSyncRequest::SetMembers {
                    world_id,
                    members: sync.selected.clone(),
                });
            }
        } else if sync.selected.len() >= 2
            && kit.button(
                ui,
                "world-sync::start",
                &tr!("Start syncing"),
                ButtonKind::Primary,
                !busy,
            )
        {
            request = Some(WorldSyncRequest::Enable {
                source_instance_id: sync.instance_id.clone(),
                folder: sync.folder.clone(),
                members: sync.selected.clone(),
            });
        }
    });
    if let Some(request) = request {
        sync.message = None;
        sync.warnings.clear();
        sync.pending = Some(sync_runner::spawn_world_sync(
            instances.clone(),
            &SyncRunConfig::from_config(config),
            request,
        ));
    }
    let done = (close || (response.dismissed || kit.back_pressed())) && sync.pending.is_none();
    if done {
        *world_sync = None;
        kit.forget("world-sync");
    }
    !done
}

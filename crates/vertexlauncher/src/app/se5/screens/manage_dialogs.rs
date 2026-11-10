//! Home's manage dialog for a world or server card: rename or edit it, delete it, or move or copy it to another instance.
//! Sharing one between instances stays in `sync_dialogs`.

use vertex_i18n::tr;
use std::collections::BTreeSet;

use instance_sync::{ManageRequest, compare_game_versions};
use launcher_ui::screens::home_api::{EntryKind, EntryView, LaunchTarget};
use launcher_ui::sync_runner::{self, ManageReceiver, ManageResult, SyncRunConfig};
use vertex_shell::sturdy::ui::{Clip, Direction, ElementDesc, Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::kit::{self, ButtonKind, Kit, TextRole};
use super::super::{AppState, Dialog};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManageMode {
    /// Rename a world, or edit a server's name and address.
    Edit,
    Delete,
    Move,
    Copy,
}

pub struct ManageState {
    mode: ManageMode,
    kind: EntryKind,
    title: String,
    world_version: Option<String>,
    /// Every instance the entry is in.
    targets: Vec<LaunchTarget>,
    /// Edit and delete apply to these instances.
    selected: BTreeSet<String>,
    /// Move and copy take the entry from this instance.
    source: String,
    dest: Option<String>,
    name: String,
    address: String,
    pending: Option<ManageReceiver>,
    /// `(is_error, text)` from the last operation.
    message: Option<(bool, String)>,
    done: bool,
}

/// Opens the manage dialog for `entry`.
pub fn open(state: &mut AppState, entry: &EntryView, mode: ManageMode) {
    let Some(primary) = entry.targets.first() else {
        return;
    };
    state.manage = Some(ManageState {
        mode,
        kind: entry.kind,
        title: entry.title.clone(),
        world_version: entry.world_version.clone(),
        targets: entry.targets.clone(),
        selected: entry.targets.iter().map(|target| target.instance_id.clone()).collect(),
        source: primary.instance_id.clone(),
        dest: None,
        name: entry.title.clone(),
        address: if entry.kind == EntryKind::Server { primary.target.clone() } else { String::new() },
        pending: None,
        message: None,
        done: false,
    });
    state.dialog = Some(Dialog::ManageEntry);
}

fn modal_style(kit: &Kit, ui: &Ui<'_>) -> ModalStyle {
    ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(560.0_f32.min(ui.viewport_size().x * 0.9)),
        ..ModalStyle::default()
    }
}

fn wrapped(kit: &Kit, role: TextRole) -> TextStyle {
    TextStyle { wrap: TextWrap::Words, ..kit.style(role) }
}

fn list(ui: &mut Ui<'_>, kit: &mut Kit, id: &str, height: f32, body: impl FnOnce(&mut Ui<'_>, &mut Kit)) {
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

fn poll(manage: &mut ManageState, state_scopes: &mut std::collections::BTreeMap<String, instances::ServerSyncScope>, home: &mut launcher_ui::screens::home_api::HomeSession) {
    let Some(receiver) = manage.pending.clone() else {
        return;
    };
    let result = receiver.lock().ok().and_then(|rx| rx.try_recv().ok());
    let Some(ManageResult { server_sync_scopes, outcome }) = result else {
        return;
    };
    manage.pending = None;
    *state_scopes = server_sync_scopes;
    match outcome {
        Ok(done) => {
            manage.message = Some((false, done.message));
            manage.done = true;
        }
        Err(err) => manage.message = Some((true, err)),
    }
    // The card shows what is on disk.
    home.request_rescan();
}

fn target_of<'a>(manage: &'a ManageState, instance_id: &str) -> Option<&'a LaunchTarget> {
    manage.targets.iter().find(|target| target.instance_id == instance_id)
}

fn requests(manage: &ManageState) -> Vec<ManageRequest> {
    let is_world = manage.kind == EntryKind::World;
    match manage.mode {
        ManageMode::Edit | ManageMode::Delete => manage
            .targets
            .iter()
            .filter(|target| manage.selected.contains(&target.instance_id))
            .map(|target| {
                let ids = BTreeSet::from([target.instance_id.clone()]);
                match (manage.mode, is_world) {
                    (ManageMode::Edit, true) => ManageRequest::RenameWorld {
                        instance_ids: ids,
                        folder: target.target.clone(),
                        new_name: manage.name.clone(),
                    },
                    (ManageMode::Edit, false) => ManageRequest::EditServer {
                        instance_ids: ids,
                        address: target.target.clone(),
                        new_name: manage.name.clone(),
                        new_address: manage.address.clone(),
                    },
                    (_, true) => ManageRequest::DeleteWorld { instance_ids: ids, folder: target.target.clone() },
                    (_, false) => ManageRequest::DeleteServer { instance_ids: ids, address: target.target.clone() },
                }
            })
            .collect(),
        ManageMode::Move | ManageMode::Copy => {
            let (Some(target), Some(dest)) = (target_of(manage, &manage.source), manage.dest.clone()) else {
                return Vec::new();
            };
            let remove_source = manage.mode == ManageMode::Move;
            vec![if is_world {
                ManageRequest::TransferWorld {
                    source_id: manage.source.clone(),
                    folder: target.target.clone(),
                    dest_id: dest,
                    remove_source,
                }
            } else {
                ManageRequest::TransferServer {
                    source_id: manage.source.clone(),
                    address: target.target.clone(),
                    dest_id: dest,
                    remove_source,
                }
            }]
        }
    }
}

/// Returns false once the dialog should close.
pub fn dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) -> bool {
    let AppState { manage, instances, config, home, .. } = state;
    let Some(manage) = manage.as_mut() else {
        return false;
    };
    poll(manage, &mut instances.server_sync_scopes, home);
    let busy = manage.pending.is_some();
    let is_world = manage.kind == EntryKind::World;
    let noun = if is_world { "world" } else { "server" };
    let mut run: Option<Vec<ManageRequest>> = None;
    let mut close = false;
    let style = modal_style(kit, ui);
    let response = kit::modal(ui, "manage-entry", &style, !busy, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        let heading = match (manage.mode, is_world) {
            (ManageMode::Edit, true) => tr!("Rename world").into_owned(),
            (ManageMode::Edit, false) => tr!("Edit server").into_owned(),
            (ManageMode::Delete, _) => tr!("Delete {noun}", noun = noun).into_owned(),
            (ManageMode::Move, _) => tr!("Move {noun} to another instance", noun = noun).into_owned(),
            (ManageMode::Copy, _) => tr!("Copy {noun} to another instance", noun = noun).into_owned(),
        };
        kit.text(ui, TextRole::Heading, &heading);
        kit.text(ui, TextRole::Strong, &manage.title);

        match manage.mode {
            ManageMode::Edit => {
                kit.text(ui, TextRole::Muted, &tr!("Name"));
                kit.text_field(ui, "manage::name", &mut manage.name, &tr!("Name"), false, Sizing::Grow);
                if is_world {
                    kit::label(
                        ui,
                        &tr!("This is the name the game lists the world under; its folder keeps its name."),
                        &wrapped(kit, TextRole::Muted),
                    );
                } else {
                    kit.text(ui, TextRole::Muted, &tr!("Address"));
                    kit.text_field(ui, "manage::address", &mut manage.address, &tr!("Address"), false, Sizing::Grow);
                }
            }
            ManageMode::Delete => {
                let text = if is_world {
                    &tr!("This permanently deletes the world's folder from the instances below. It cannot be undone.")
                } else {
                    &tr!("This removes the server from the instances below, and stops sync from adding it back to them.")
                };
                kit::label(ui, text, &wrapped(kit, TextRole::Warning));
            }
            ManageMode::Move | ManageMode::Copy => {}
        }

        if matches!(manage.mode, ManageMode::Edit | ManageMode::Delete) {
            if manage.targets.len() > 1 {
                kit.text(ui, TextRole::Muted, &tr!("Apply to"));
                list(ui, kit, "manage::targets", 160.0, |ui, kit| {
                    for target in &manage.targets {
                        let checked = manage.selected.contains(&target.instance_id);
                        if let Some(next) = kit.checkbox(
                            ui,
                            &format!("manage::target::{}", target.instance_id),
                            &target.instance_name,
                            checked,
                        ) && !busy
                        {
                            if next {
                                manage.selected.insert(target.instance_id.clone());
                            } else {
                                manage.selected.remove(&target.instance_id);
                            }
                        }
                    }
                });
            }
        } else {
            if manage.targets.len() > 1 {
                kit.text(ui, TextRole::Muted, &tr!("From"));
                for target in &manage.targets {
                    if kit.radio(
                        ui,
                        &format!("manage::from::{}", target.instance_id),
                        &target.instance_name,
                        manage.source == target.instance_id,
                    ) && !busy
                    {
                        manage.source = target.instance_id.clone();
                        if manage.dest.as_deref() == Some(target.instance_id.as_str()) {
                            manage.dest = None;
                        }
                    }
                }
            }
            kit.text(ui, TextRole::Muted, &tr!("To"));
            list(ui, kit, "manage::dest", 240.0, |ui, kit| {
                for instance in &instances.instances {
                    if instance.id == manage.source {
                        continue;
                    }
                    let label = tr!("{name} (Minecraft {game_version})", name = instance.name, game_version = instance.game_version).into_owned();
                    if kit.radio(
                        ui,
                        &format!("manage::dest::{}", instance.id),
                        &label,
                        manage.dest.as_deref() == Some(instance.id.as_str()),
                    ) && !busy
                    {
                        manage.dest = Some(instance.id.clone());
                    }
                }
            });
            if let (Some(world_version), Some(dest)) = (manage.world_version.as_deref(), manage.dest.as_deref())
                && let Some(instance) = instances.find(dest)
                && compare_game_versions(&instance.game_version, world_version) == Some(std::cmp::Ordering::Less)
            {
                kit::label(
                    ui,
                    &tr!("{name} runs Minecraft {game_version}, older than this world ({world_version}). Opening it there can corrupt the world.", name = instance.name, game_version = instance.game_version, world_version = world_version),
                    &wrapped(kit, TextRole::Warning),
                );
            }
            if is_world && manage.mode == ManageMode::Copy {
                kit::label(
                    ui,
                    &tr!("To keep a world in step between instances instead of making a separate copy, use \"Sync world…\"."),
                    &wrapped(kit, TextRole::Muted),
                );
            }
        }

        if busy {
            kit.text(ui, TextRole::Muted, &tr!("Working… copying a large world can take a while."));
            kit.progress(ui, "manage::progress", None, kit::uptime());
        } else if let Some((is_error, message)) = &manage.message {
            kit::label(
                ui,
                message,
                &wrapped(kit, if *is_error { TextRole::Danger } else { TextRole::Muted }),
            );
        }

        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        close = kit.button(
            ui,
            "manage::close",
            &if manage.done { tr!("Close") } else { tr!("Cancel") },
            ButtonKind::Neutral,
            !busy,
        );
        if !manage.done {
            let (label, kind, ready) = match manage.mode {
                ManageMode::Edit => (
                    tr!("Save"),
                    ButtonKind::Primary,
                    !manage.selected.is_empty()
                        && !manage.name.trim().is_empty()
                        && (is_world || !manage.address.trim().is_empty()),
                ),
                ManageMode::Delete => (tr!("Delete"), ButtonKind::Danger, !manage.selected.is_empty()),
                ManageMode::Move => (tr!("Move"), ButtonKind::Primary, manage.dest.is_some()),
                ManageMode::Copy => (tr!("Copy"), ButtonKind::Primary, manage.dest.is_some()),
            };
            if kit.button(ui, "manage::go", &label, kind, ready && !busy) {
                run = Some(requests(manage));
            }
        }
    });
    if let Some(requests) = run.filter(|requests| !requests.is_empty()) {
        manage.message = None;
        manage.pending = Some(sync_runner::spawn_manage(
            instances.clone(),
            &SyncRunConfig::from_config(config),
            requests,
        ));
    }
    let finished = (close || response.dismissed || kit.back_pressed()) && manage.pending.is_none();
    if finished {
        state.manage = None;
        kit.forget("manage-entry");
    }
    !finished
}

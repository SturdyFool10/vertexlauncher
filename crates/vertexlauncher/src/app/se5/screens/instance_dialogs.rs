//! Instance dialogs: settings (metadata, versions, runtime overrides, maintenance), exporting a pack/patch or a
//! server zip, moving the instance, and the screenshot viewer. They edit the instance's `InstanceSession`, shared
//! with the egui screen, and save through the same code.

use vertex_i18n::tr;
use std::path::PathBuf;

use crate::app::se5::file_dialog::{Dialog as PickerDialog, Pick};
use launcher_ui::screens::instance_api::{self as api, InstanceSession};
use vertex_shell::sturdy::ui::{Clip, Direction, ElementDesc, Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;
use vtmpack::{VtmpackCompressionMode, VtmpackProviderMode};

use super::super::AppState;
use super::super::images::ImageSource;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

fn modal_style(kit: &Kit, width: f32) -> ModalStyle {
    ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(width),
        ..ModalStyle::default()
    }
}

/// A scrolling column inside a dialog, so long forms fit small windows.
fn scrolled(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    max_height: f32,
    body: impl FnOnce(&mut Ui<'_>, &mut Kit),
) {
    let height = (ui.viewport_size().y - 220.0).clamp(240.0, max_height);
    let _scroll = ui.element(
        &ElementDesc::new()
            .id(id.into())
            .width(Sizing::Grow)
            .height(Sizing::Fixed(height))
            .direction(Direction::TopToBottom)
            .child_gap(kit::SPACE_MD)
            .clip(Clip::vertical()),
    );
    kit.scrollbar(ui, id);
    body(ui, kit);
}

// -- Settings -------------------------------------------------------------------------------------------------------

/// Returns false once the dialog should close.
pub fn settings(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, id: &str) -> bool {
    let Some(instance) = state.instances.find(id).cloned() else {
        return false;
    };
    state.ensure_session(&instance);
    let root = instances::instance_root_path(state.installations_root().as_path(), &instance);
    let config = state.config.clone();
    let limits = api::settings::override_limits(&config);
    let thumb_picked = state
        .file_dialogs
        .take("instance-settings::thumbnail")
        .and_then(|paths| paths.into_iter().next());

    let mut close = false;
    let mut changed = false;
    let mut open_export = None;
    let mut open_move = false;
    let style = modal_style(kit, 720.0);
    let AppState {
        sessions,
        instances,
        file_dialogs,
        ..
    } = state;
    let Some(session) = sessions.get_mut(id) else {
        return false;
    };
    if let Some(path) = thumb_picked {
        session.state.thumbnail_input = path;
    }
    let controls = api::settings::version_controls(&mut session.state);
    let mut pick_thumbnail = false;
    let response = kit::modal(ui, "instance-settings", &style, true, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, &tr!("Instance Settings"));
        kit.text(ui, TextRole::Muted, &tr!("Manage this profile's metadata, version stack, runtime overrides, and maintenance actions."));
        if let Some(status) = &session.state.status_message {
            kit::label(
                ui,
                status,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Small)
                },
            );
        }
        scrolled(ui, kit, "instance-settings::scroll", 620.0, |ui, kit| {
            // Metadata.
            kit.text(ui, TextRole::Heading, &tr!("Metadata & Versions"));
            kit.text_setting(
                ui,
                "instance-settings::name",
                &tr!("Name"),
                Some(&tr!("Display name shown in the sidebar.")),
                &mut session.state.name_input,
                &tr!("Name"),
                false,
            );
            kit.text_setting(
                ui,
                "instance-settings::description",
                &tr!("Description (optional)"),
                Some(&tr!("Optional note shown in library instance tiles.")),
                &mut session.state.description_input,
                "",
                false,
            );
            let mut thumbnail = session.state.thumbnail_input.to_string_lossy().into_owned();
            kit.setting_row(
                ui,
                "instance-settings::thumbnail-row",
                &tr!("Thumbnail path (optional)"),
                Some(&tr!("Local image path for this instance.")),
                |kit, ui| {
                    if kit.text_field(
                        ui,
                        "instance-settings::thumbnail",
                        &mut thumbnail,
                        &tr!("Path to an image"),
                        false,
                        Sizing::Fixed(260.0),
                    ) {
                        session.state.thumbnail_input = if thumbnail.trim().is_empty() {
                            PathBuf::new()
                        } else {
                            PathBuf::from(thumbnail.trim())
                        };
                    }
                    if kit.button(
                        ui,
                        "instance-settings::thumbnail-pick",
                        &tr!("Choose…"),
                        ButtonKind::Neutral,
                        true,
                    ) {
                        pick_thumbnail = true;
                    }
                },
            );

            {
                let _row = kit.row(ui, kit::SPACE_SM);
                if kit.button(
                    ui,
                    "instance-settings::refresh-versions",
                    &tr!("Refresh version list"),
                    ButtonKind::Neutral,
                    true,
                ) {
                    api::settings::refresh_versions(&mut session.state, &config);
                }
                if kit.button(
                    ui,
                    "instance-settings::detect",
                    &tr!("Auto-detect installed versions"),
                    ButtonKind::Neutral,
                    true,
                ) {
                    changed |=
                        api::settings::detect_versions(&mut session.state, instances, id, &root);
                }
            }
            if controls.catalog_loading {
                kit.text(ui, TextRole::Muted, &tr!("Fetching version catalog..."));
            }
            if let Some(error) = &controls.catalog_error {
                kit::label(
                    ui,
                    error,
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(TextRole::Danger)
                    },
                );
            }
            if controls.game_versions.is_empty() {
                kit.text(ui, TextRole::Muted, &tr!("No game versions available yet."));
            } else {
                kit.setting_row(
                    ui,
                    "instance-settings::game-version-row",
                    &tr!("Minecraft game version"),
                    Some(&tr!("Pick from available Minecraft versions.")),
                    |kit, ui| {
                        if let Some(index) = kit.dropdown(
                            ui,
                            "instance-settings::game-version",
                            controls.game_version,
                            &controls.game_versions,
                            320.0,
                        ) {
                            api::settings::set_game_version(&mut session.state, index);
                        }
                    },
                );
            }
            let labels: Vec<String> = controls
                .loaders
                .iter()
                .map(|choice| {
                    if choice.unavailable.is_some() {
                        tr!("{label} (unavailable)", label = choice.label).into_owned()
                    } else {
                        choice.label.to_owned()
                    }
                })
                .collect();
            kit.setting_row(
                ui,
                "instance-settings::loader-row",
                &tr!("Modloader"),
                Some(&tr!("Loaders marked unavailable do not support the selected Minecraft version.")),
                |kit, ui| {
                    if let Some(index) = kit.dropdown(
                        ui,
                        "instance-settings::loader",
                        controls.loader,
                        &labels,
                        220.0,
                    ) && controls
                        .loaders
                        .get(index)
                        .is_some_and(|choice| choice.unavailable.is_none())
                    {
                        api::settings::set_loader(&mut session.state, index);
                    }
                },
            );
            if let Some(reason) = controls
                .loaders
                .get(controls.loader)
                .and_then(|choice| choice.unavailable.as_deref())
            {
                kit.text(ui, TextRole::Danger, reason);
            }
            if controls.custom_loader {
                kit.text_setting(
                    ui,
                    "instance-settings::custom-loader",
                    &tr!("Custom modloader id"),
                    Some(&tr!("Use any custom modloader name.")),
                    &mut session.state.custom_modloader,
                    "",
                    false,
                );
            }
            if controls.loader != 0 {
                if controls.loader_versions_loading {
                    kit.text(ui, TextRole::Muted, &tr!("Fetching modloader versions..."));
                }
                if let Some((status, is_error)) = &controls.loader_status {
                    kit.text(
                        ui,
                        if *is_error {
                            TextRole::Danger
                        } else {
                            TextRole::Muted
                        },
                        status,
                    );
                }
                if controls.loader_versions.is_empty() {
                    if !controls.custom_loader {
                        kit.text(ui, TextRole::Muted, &tr!("No cataloged modloader versions were found for this Minecraft version."));
                    }
                } else {
                    kit.setting_row(
                        ui,
                        "instance-settings::loader-version-row",
                        &tr!("Modloader version"),
                        Some(
                            &tr!("Cataloged by loader+Minecraft compatibility and cached once per day."),
                        ),
                        |kit, ui| {
                            if let Some(index) = kit.dropdown(
                                ui,
                                "instance-settings::loader-version",
                                controls.loader_version,
                                &controls.loader_versions,
                                320.0,
                            ) {
                                api::settings::set_loader_version(
                                    &mut session.state,
                                    &controls,
                                    index,
                                );
                            }
                        },
                    );
                }
                if !controls.custom_loader
                    && kit.button(
                        ui,
                        "instance-settings::refresh-loader",
                        &tr!("Refresh modloader versions"),
                        ButtonKind::Neutral,
                        !controls.loader_versions_loading,
                    )
                {
                    api::settings::refresh_loader_versions(&mut session.state);
                }
            }
            if let Some(error) = &controls.validation_error {
                kit::label(
                    ui,
                    error,
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(TextRole::Danger)
                    },
                );
            }
            {
                let _row = kit.row(ui, kit::SPACE_SM);
                if kit.button(
                    ui,
                    "instance-settings::save-versions",
                    &tr!("Save metadata & versions"),
                    ButtonKind::Primary,
                    controls.validation_error.is_none(),
                ) {
                    changed |= api::settings::save_versions(&mut session.state, id, instances);
                }
                let can_reinstall = controls.validation_error.is_none()
                    && !session.state.runtime_prepare_in_flight
                    && !session.state.running;
                if kit.button(
                    ui,
                    "instance-settings::reinstall",
                    &tr!("Reinstall Profile"),
                    ButtonKind::Danger,
                    can_reinstall,
                ) {
                    changed |= api::settings::reinstall(&mut session.state, id, instances, &config);
                }
            }

            // Runtime overrides.
            kit.separator(ui);
            kit.text(ui, TextRole::Heading, &tr!("Runtime Overrides"));
            kit.text(
                ui,
                TextRole::Muted,
                &tr!("Per-instance overrides for memory, JVM arguments, and Java runtime selection."),
            );
            kit.toggle_setting(
                ui,
                "instance-settings::memory-override",
                &tr!("Override max memory for this instance"),
                Some(&tr!("When disabled, launcher instance default memory is used.")),
                &mut session.state.memory_override_enabled,
            );
            if session.state.memory_override_enabled {
                let max = limits
                    .memory_max_mib
                    .max(config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN + 1);
                let value = session
                    .state
                    .memory_override_mib
                    .clamp(config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, max)
                    as f64;
                if let Some(next) = kit.slider_setting(
                    ui,
                    "instance-settings::memory",
                    &tr!("Max memory allocation (MiB)"),
                    Some(&tr!("Per-instance memory limit.")),
                    value,
                    config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN as f64,
                    max as f64,
                    config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP as f64,
                    |value| tr!("{value:.0} MiB", value = value).into_owned(),
                ) {
                    session.state.memory_override_mib = next as u128;
                }
            }
            kit.text_setting(
                ui,
                "instance-settings::jvm-args",
                &tr!("JVM args override (optional)"),
                Some(&tr!("Leave blank to use launcher instance default JVM args.")),
                &mut session.state.cli_args_input,
                "",
                false,
            );
            kit.text(ui, TextRole::Body, &tr!("Environment variables (optional)"));
            kit.text(ui, TextRole::Small, &tr!("One KEY=value per line. Blank lines and # comments are ignored. Values are not written to launch logs."));
            kit.text_area(
                ui,
                "instance-settings::env",
                &mut session.state.env_vars_input,
                &tr!("VK_INSTANCE_LAYERS=\nMESA_VK_WSI_PRESENT_MODE=mailbox"),
                4,
            );
            kit.toggle_setting(
                ui,
                "instance-settings::rpc-mod",
                &tr!("This modpack has a rich presence mod"),
                Some(&tr!("When enabled, Vertex clears its own Discord Rich Presence for this instance after launch so the mod inside Minecraft can take over.")),
                &mut session.state.discord_rich_presence_mod_installed,
            );
            kit.toggle_setting(
                ui,
                "instance-settings::java-override",
                &tr!("Override Java runtime for this instance"),
                Some(&tr!("When enabled, this instance will use the selected configured global Java path.")),
                &mut session.state.java_override_enabled,
            );
            if session.state.java_override_enabled {
                if limits.java_options.is_empty() {
                    kit.text(ui, TextRole::Danger, &tr!("No configured global Java paths found. Add at least one Java path in Settings first."));
                } else {
                    if session
                        .state
                        .java_override_runtime_major
                        .is_none_or(|major| !limits.java_options.iter().any(|(m, _)| *m == major))
                    {
                        session.state.java_override_runtime_major =
                            limits.java_options.first().map(|(major, _)| *major);
                    }
                    let labels: Vec<String> = limits
                        .java_options
                        .iter()
                        .map(|(_, label)| label.clone())
                        .collect();
                    let current = limits
                        .java_options
                        .iter()
                        .position(|(major, _)| {
                            Some(*major) == session.state.java_override_runtime_major
                        })
                        .unwrap_or(0);
                    kit.setting_row(
                        ui,
                        "instance-settings::java-row",
                        &tr!("Java path override"),
                        Some(&tr!("Select which configured Java path this instance should use.")),
                        |kit, ui| {
                            if let Some(index) =
                                kit.dropdown(ui, "instance-settings::java", current, &labels, 360.0)
                            {
                                session.state.java_override_runtime_major =
                                    limits.java_options.get(index).map(|(major, _)| *major);
                            }
                        },
                    );
                }
            }
            // Platform-specific launch compatibility.
            let platform = if cfg!(target_os = "linux") {
                Some((
                    tr!("Linux"),
                    tr!("Linux-specific launch compatibility settings for this instance."),
                ))
            } else if cfg!(target_os = "windows") {
                Some((
                    tr!("Windows"),
                    tr!("Reserved for Windows-specific instance settings."),
                ))
            } else if cfg!(target_os = "macos") {
                Some((tr!("macOS"), tr!("Reserved for macOS-specific instance settings.")))
            } else {
                None
            };
            if let Some((heading, description)) = platform {
                kit.separator(ui);
                kit.text(ui, TextRole::Heading, &heading);
                kit.text(ui, TextRole::Muted, &description);
            }
            if cfg!(target_os = "linux") {
                kit.toggle_setting(
                    ui,
                    "instance-settings::opengl",
                    &tr!("Set Linux OpenGL Driver"),
                    Some(&tr!("Linux-only. When enabled, this instance uses the Zink toggle below instead of the launcher-wide Linux OpenGL driver behavior. When disabled, this instance falls back to the launcher-wide Linux graphics settings. Versions using Vulkan directly should ignore it.")),
                    &mut session.state.linux_set_opengl_driver,
                );
                let zink_enabled = session.state.linux_set_opengl_driver;
                kit.toggle_setting_ex(
                    ui,
                    "instance-settings::zink",
                    &tr!("Use Zink Driver (Experimental)"),
                    Some(&tr!("Linux-only. Experimental. When the setting above is enabled, this toggle decides whether this instance forces Mesa Zink so OpenGL runs over Vulkan. When the setting above is disabled, the launcher-wide Zink setting is used instead. Versions using Vulkan directly should ignore it.")),
                    &mut session.state.linux_use_zink_driver,
                    zink_enabled,
                );
            }

            // Maintenance.
            kit.separator(ui);
            kit.text(ui, TextRole::Heading, &tr!("Maintenance & Actions"));
            kit.text(
                ui,
                TextRole::Muted,
                &tr!("Open the instance folder and commit any metadata or runtime changes."),
            );
            if kit.button(
                ui,
                "instance-settings::open-folder",
                &tr!("Open Instance Folder"),
                ButtonKind::Neutral,
                true,
            ) {
                api::settings::open_folder_action(&mut session.state, instances, &config, id);
            }
            if kit.button(
                ui,
                "instance-settings::export-pack",
                &tr!("Export .vtmpack..."),
                ButtonKind::Neutral,
                true,
            ) {
                open_export = Some(ExportKind::Pack);
            }
            if kit.button(
                ui,
                "instance-settings::export-patch",
                &tr!("Generate Patch..."),
                ButtonKind::Neutral,
                true,
            ) {
                open_export = Some(ExportKind::Patch);
            }
            if kit.button(
                ui,
                "instance-settings::export-server",
                &tr!("Auto-generate server zip..."),
                ButtonKind::Neutral,
                true,
            ) {
                open_export = Some(ExportKind::Server);
            }
            if kit.button(
                ui,
                "instance-settings::move",
                &tr!("Move Instance..."),
                ButtonKind::Neutral,
                true,
            ) {
                open_move = true;
            }
        });
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        if kit.button(
            ui,
            "instance-settings::save",
            &tr!("Save instance settings"),
            ButtonKind::Neutral,
            true,
        ) {
            changed |= api::settings::save_overrides(&mut session.state, id, instances, &limits);
        }
        if kit.button(
            ui,
            "instance-settings::done",
            &tr!("Done"),
            ButtonKind::Primary,
            true,
        ) {
            close = true;
        }
    });
    if pick_thumbnail {
        file_dialogs.open(
            "instance-settings::thumbnail",
            Pick::File,
            PickerDialog::new()
                .title(tr!("Choose thumbnail"))
                .filter(tr!("Images"), &["png", "jpg", "jpeg", "webp", "bmp", "gif"]),
        );
    }
    // Override edits save themselves shortly after they stop.
    let (autosaved, _) =
        api::settings::autosave_overrides(&mut session.state, id, instances, &limits);
    let _ = (autosaved, changed);
    if limits.memory_pending {
    }
    let done = (response.dismissed || kit.back_pressed()) || close;
    if done {
        api::settings::flush_overrides(&mut session.state, id, instances, &config);
        kit.forget("instance-settings::");
    }
    if let Some(kind) = open_export {
        state.open_export(id, kind);
        return true;
    }
    if open_move {
        state.open_move(id);
        return true;
    }
    !done
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    Pack,
    Patch,
    Server,
}

// -- Export ---------------------------------------------------------------------------------------------------------

/// The .vtmpack / patch export dialog. Returns false once it should close.
pub fn export_pack(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, id: &str) -> bool {
    let Some(instance) = state.instances.find(id).cloned() else {
        return false;
    };
    state.ensure_session(&instance);
    let root = instances::instance_root_path(state.installations_root().as_path(), &instance);
    let base_slot = "vtmpack::base";
    let patch_out = "vtmpack::patch-out";
    let pack_out = "vtmpack::pack-out";
    let patch_mode = state
        .sessions
        .get(id)
        .is_some_and(|session| session.state.export_vtmpack_patch_mode);

    // Chained pickers: pack export asks for the output; a patch asks for the base first, then the output.
    let base = state
        .file_dialogs
        .take(base_slot)
        .and_then(|paths| paths.into_iter().next());
    let patch_target = state
        .file_dialogs
        .take(patch_out)
        .and_then(|paths| paths.into_iter().next());
    let pack_target = state
        .file_dialogs
        .take(pack_out)
        .and_then(|paths| paths.into_iter().next());
    if let Some(base) = base {
        if let Some(session) = state.sessions.get_mut(id) {
            session.state.export_patch_base_path = Some(base);
        }
        state.file_dialogs.open(
            patch_out,
            Pick::SaveFile,
            PickerDialog::new()
                .title(tr!("Generate Patch"))
                .file_name(api::default_vtmpatch_name(&instance.name))
                .filter(tr!("Vertex Modpack Patch"), &[vtmpack::VTMPATCH_EXTENSION]),
        );
    }
    if let Some(session) = state.sessions.get_mut(id) {
        if let Some(output) = patch_target
            && let Some(base) = session.state.export_patch_base_path.take()
        {
            api::start_vtmpatch_export(
                &mut session.state,
                instance.name.clone(),
                root.clone(),
                base,
                output,
            );
        }
        if let Some(output) = pack_target {
            api::start_vtmpack_export(&mut session.state, &instance, root.clone(), output);
        }
    }

    let style = modal_style(kit, 640.0);
    let mut cancel = false;
    let mut choose = false;
    let AppState { sessions, .. } = state;
    let Some(session) = sessions.get_mut(id) else {
        return false;
    };
    api::sync_vtmpack_options(&mut session.state, &root);
    let (entries, loaded, root_exists) = api::root_entries(&mut session.state, false, &root);
    let in_flight = session.state.export_vtmpack_in_flight;
    let response = kit::modal(ui, "export-vtmpack", &style, !in_flight, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        if in_flight {
            let progress = session.state.export_vtmpack_latest_progress.as_ref();
            kit.text(ui, TextRole::Heading, &tr!("Export in progress"));
            kit.text(
                ui,
                TextRole::Body,
                progress.map_or(&tr!("Starting export..."), |p| p.message.as_str()),
            );
            kit.text(
                ui,
                TextRole::Muted,
                &progress
                    .map(|p| {
                        tr!("{done} of {total_steps} steps complete", done = p.completed_steps.min(p.total_steps), total_steps = p.total_steps).into_owned()
                    })
                    .unwrap_or_else(|| tr!("Preparing export task...").into_owned()),
            );
            if let Some(path) = &session.state.export_vtmpack_output_path {
                kit.text(
                    ui,
                    TextRole::Small,
                    &tr!("Destination: {path}", path = path.display()),
                );
            }
            let fraction = progress.and_then(|p| {
                (p.total_steps > 0)
                    .then(|| (p.completed_steps as f32 / p.total_steps as f32).clamp(0.0, 1.0))
            });
            kit.progress(ui, "export-vtmpack::progress", fraction, kit::uptime());
            return;
        }
        kit.text(
            ui,
            TextRole::Heading,
            &if patch_mode {
                tr!("Generate .vtmpatch")
            } else {
                tr!("Export .vtmpack")
            },
        );
        kit::label(
            ui,
            &if patch_mode {
                tr!("Choose what to include in the patch. After clicking Choose files, you will be prompted for the base .vtmpack to compare against, then where to save the generated .vtmpatch.")
            } else {
                tr!("Choose whether the exported pack may reference CurseForge metadata directly, then select which top-level files and folders from the Minecraft root should be bundled into the pack.")
            },
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
        for mode in [
            VtmpackProviderMode::IncludeCurseForge,
            VtmpackProviderMode::ExcludeCurseForge,
        ] {
            if kit.radio(
                ui,
                &format!("export-vtmpack::provider::{}", mode.label()),
                &vertex_i18n::t(mode.label()),
                session.state.export_vtmpack_options.provider_mode == mode,
            ) {
                session.state.export_vtmpack_options.provider_mode = mode;
            }
        }
        kit.text(
            ui,
            TextRole::Small,
            &match session.state.export_vtmpack_options.provider_mode {
                VtmpackProviderMode::IncludeCurseForge => tr!("Managed CurseForge entries stay downloadable in the pack manifest."),
                VtmpackProviderMode::ExcludeCurseForge => {
                    tr!("CurseForge metadata is removed from the export. CurseForge-managed files are bundled into the pack unless they already use Modrinth as the selected source.")
                }
            },
        );
        kit.text(ui, TextRole::Heading, &tr!("Compression"));
        for mode in [
            VtmpackCompressionMode::Standard,
            VtmpackCompressionMode::Extreme,
        ] {
            if kit.radio(
                ui,
                &format!("export-vtmpack::compression::{}", mode.label()),
                &vertex_i18n::t(mode.label()),
                session.state.export_vtmpack_options.compression_mode == mode,
            ) {
                session.state.export_vtmpack_options.compression_mode = mode;
            }
        }
        kit.text(
            ui,
            TextRole::Small,
            &match session.state.export_vtmpack_options.compression_mode {
                VtmpackCompressionMode::Standard => tr!("Uses the standard XZ compression preset for a balanced export."),
                VtmpackCompressionMode::Extreme => {
                    tr!("Uses the maximum XZ preset with the extreme flag. Exports take longer and may use more memory, but prioritize the smallest archive.")
                }
            },
        );
        kit.text(
            ui,
            TextRole::Heading,
            &tr!("Include top-level entries from the Minecraft root"),
        );
        kit.text(ui, TextRole::Small, &tr!("Defaults to mods, resourcepacks, shaderpacks, and config. You can also include any other top-level files or folders found in the instance root."));
        if loaded && !root_exists {
            kit.text(
                ui,
                TextRole::Danger,
                &tr!("Instance root directory not found: {root}", root = root.display()),
            );
        } else if !loaded {
            kit.text(ui, TextRole::Muted, &tr!("Reading instance files..."));
        } else if entries.is_empty() {
            kit.text(
                ui,
                TextRole::Muted,
                &tr!("No files or folders found in the instance root."),
            );
        } else {
            scrolled(ui, kit, "export-vtmpack::entries", 300.0, |ui, kit| {
                for (name, is_dir) in &entries {
                    let checked = session
                        .state
                        .export_vtmpack_options
                        .included_root_entries
                        .entry(name.clone())
                        .or_insert_with(|| api::default_vtmpack_selection(name));
                    let label = if *is_dir {
                        format!("{name}/")
                    } else {
                        name.clone()
                    };
                    if let Some(next) = kit.checkbox(
                        ui,
                        &format!("export-vtmpack::entry::{name}"),
                        &label,
                        *checked,
                    ) {
                        *checked = next;
                    }
                }
            });
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        if kit.button(
            ui,
            "export-vtmpack::cancel",
            &tr!("Cancel"),
            ButtonKind::Neutral,
            true,
        ) {
            cancel = true;
        }
        if kit.button(
            ui,
            "export-vtmpack::choose",
            &if patch_mode {
                tr!("Choose files")
            } else {
                tr!("Choose file")
            },
            ButtonKind::Primary,
            true,
        ) {
            choose = true;
        }
    });
    if choose {
        if patch_mode {
            state.file_dialogs.open(
                base_slot,
                Pick::File,
                PickerDialog::new()
                    .title(tr!("Choose Base Modpack"))
                    .filter(tr!("Vertex Modpack"), &[vtmpack::VTMPACK_EXTENSION]),
            );
        } else {
            state.file_dialogs.open(
                pack_out,
                Pick::SaveFile,
                PickerDialog::new()
                    .title(tr!("Export Modpack"))
                    .file_name(api::default_vtmpack_name(&instance.name))
                    .filter(tr!("Vertex Modpack"), &[vtmpack::VTMPACK_EXTENSION]),
            );
        }
    }
    let running = state
        .sessions
        .get(id)
        .is_some_and(|session| session.state.export_vtmpack_in_flight);
    if ((response.dismissed || kit.back_pressed()) || cancel)
        && !running
        && let Some(session) = state.sessions.get_mut(id)
    {
        session.state.show_export_vtmpack_modal = false;
    }
    // The dialog stays up while an export runs; the result handler clears the flag when it finishes.
    let open = state
        .sessions
        .get(id)
        .is_some_and(|session| session.state.show_export_vtmpack_modal)
        || running;
    if !open {
        kit.forget("export-vtmpack::");
    }
    open
}

/// The server zip export dialog. Returns false once it should close.
pub fn export_server(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, id: &str) -> bool {
    let Some(instance) = state.instances.find(id).cloned() else {
        return false;
    };
    state.ensure_session(&instance);
    let root = instances::instance_root_path(state.installations_root().as_path(), &instance);
    let config = state.config.clone();
    let style = modal_style(kit, 640.0);
    let mut cancel = false;
    let mut start = false;
    let AppState { sessions, .. } = state;
    let Some(session) = sessions.get_mut(id) else {
        return false;
    };
    api::sync_server_options(&mut session.state, &root);
    let (entries, loaded, root_exists) = api::root_entries(&mut session.state, true, &root);
    let in_flight = session.state.export_server_in_flight;
    let response = kit::modal(ui, "export-server", &style, !in_flight, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        if in_flight {
            let progress = session.state.export_server_latest_progress.as_ref();
            kit.text(ui, TextRole::Heading, &tr!("Building server zip"));
            kit.text(
                ui,
                TextRole::Body,
                progress.map_or(&tr!("Starting export..."), |p| p.message.as_str()),
            );
            kit.text(
                ui,
                TextRole::Muted,
                &progress
                    .map(|p| {
                        tr!("{done} of {total_steps} steps complete", done = p.completed_steps.min(p.total_steps), total_steps = p.total_steps).into_owned()
                    })
                    .unwrap_or_else(|| tr!("Preparing export task...").into_owned()),
            );
            if let Some(path) = &session.state.export_server_output_path {
                kit.text(
                    ui,
                    TextRole::Small,
                    &tr!("Destination: {path}", path = path.display()),
                );
            }
            let fraction = progress.and_then(|p| {
                (p.total_steps > 0)
                    .then(|| (p.completed_steps as f32 / p.total_steps as f32).clamp(0.0, 1.0))
            });
            kit.progress(ui, "export-server::progress", fraction, kit::uptime());
            return;
        }
        kit.text(ui, TextRole::Heading, &tr!("Auto-generate server zip"));
        kit::label(
            ui,
            &tr!("Builds a portable server package in your Downloads folder using this instance's files. CurseForge-managed mods that cannot be resolved on Modrinth by hash are listed as unknowns in the report."),
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
        kit.text(
            ui,
            TextRole::Heading,
            &tr!("Include top-level entries from the Minecraft root"),
        );
        if loaded && !root_exists {
            kit.text(
                ui,
                TextRole::Danger,
                &tr!("Instance root directory not found: {root}", root = root.display()),
            );
        } else if !loaded {
            kit.text(ui, TextRole::Muted, &tr!("Reading instance files..."));
        } else {
            scrolled(ui, kit, "export-server::entries", 320.0, |ui, kit| {
                for (name, is_dir) in &entries {
                    let checked = session
                        .state
                        .export_server_included_root_entries
                        .entry(name.clone())
                        .or_insert(false);
                    let label = if *is_dir {
                        format!("{name}/")
                    } else {
                        name.clone()
                    };
                    if let Some(next) = kit.checkbox(
                        ui,
                        &format!("export-server::entry::{name}"),
                        &label,
                        *checked,
                    ) {
                        *checked = next;
                    }
                }
            });
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        if kit.button(
            ui,
            "export-server::cancel",
            &tr!("Cancel"),
            ButtonKind::Neutral,
            true,
        ) {
            cancel = true;
        }
        if kit.button(
            ui,
            "export-server::start",
            &tr!("Build zip in Downloads"),
            ButtonKind::Primary,
            true,
        ) {
            start = true;
        }
    });
    if start {
        api::start_server_export(&mut session.state, &instance, root.clone(), &config);
    }
    if ((response.dismissed || kit.back_pressed()) || cancel)
        && !session.state.export_server_in_flight
    {
        session.state.show_export_server_modal = false;
    }
    // The dialog stays up while an export runs; the result handler clears the flag when it finishes.
    let open = session.state.show_export_server_modal || session.state.export_server_in_flight;
    if !open {
        kit.forget("export-server::");
    }
    open
}

// -- Move -----------------------------------------------------------------------------------------------------------

/// The move-instance dialog. Returns false once it should close.
pub fn move_instance(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, id: &str) -> bool {
    let Some(instance) = state.instances.find(id).cloned() else {
        return false;
    };
    state.ensure_session(&instance);
    let root = instances::instance_root_path(state.installations_root().as_path(), &instance);
    let picked = state
        .file_dialogs
        .take("move::destination")
        .and_then(|paths| paths.into_iter().next());
    let style = modal_style(kit, 600.0);
    let mut cancel = false;
    let mut start = None;
    let mut pick = false;
    let AppState { sessions, .. } = state;
    let Some(session) = sessions.get_mut(id) else {
        return false;
    };
    if let Some(path) = picked {
        session.state.move_instance_dest_input = path.display().to_string();
        revalidate(session);
    }
    let in_flight = session.state.move_instance_in_flight;
    let completion = session.state.move_instance_completion_message.clone();
    let failed = session.state.move_instance_completion_failed;
    let response = kit::modal(ui, "move-instance", &style, !in_flight, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(
            ui,
            TextRole::Heading,
            &if in_flight {
                tr!("Moving Instance")
            } else if completion.is_some() {
                if failed {
                    tr!("Move Failed")
                } else {
                    tr!("Move Complete")
                }
            } else {
                tr!("Move Instance")
            },
        );
        if in_flight {
            kit.text(ui, TextRole::Muted, &tr!("Vertex is moving this instance now."));
            if let Some(progress) = &session.state.move_instance_latest_progress {
                let fraction = (progress.total_bytes > 0).then(|| {
                    (progress.bytes_done as f32 / progress.total_bytes as f32).clamp(0.0, 1.0)
                });
                kit.progress(ui, "move::progress", fraction, kit::uptime());
                kit.text(
                    ui,
                    TextRole::Small,
                    &tr!("{moved} of {total} · {files_done} of {total_files} files", moved = api::move_bytes_label(progress.bytes_done), total = api::move_bytes_label(progress.total_bytes), files_done = progress.files_done, total_files = progress.total_files),
                );
                for file in progress.active_files.iter().take(4) {
                    kit.text_clipped(ui, TextRole::Small, file, 520.0);
                }
            } else {
                kit.progress(ui, "move::progress", None, kit::uptime());
            }
            return;
        }
        if let Some(message) = &completion {
            kit::label(
                ui,
                message,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(if failed {
                        TextRole::Danger
                    } else {
                        TextRole::Body
                    })
                },
            );
            let _row = kit.row(ui, kit::SPACE_SM);
            kit.fill(ui);
            if kit.button(ui, "move::close", &tr!("Close"), ButtonKind::Primary, true) {
                cancel = true;
            }
            return;
        }
        kit.text(ui, TextRole::Muted, &tr!("Choose a destination folder. It can be an empty existing folder or a new path that will be created."));
        let mut text = session.state.move_instance_dest_input.clone();
        {
            let _row = kit.row(ui, kit::SPACE_SM);
            if kit.text_field(
                ui,
                "move::dest",
                &mut text,
                &tr!("Choose an empty folder or enter a new destination path"),
                false,
                Sizing::Grow,
            ) {
                session.state.move_instance_dest_input = text;
                revalidate(session);
            }
            if kit.button(ui, "move::browse", &tr!("Browse..."), ButtonKind::Neutral, true) {
                pick = true;
            }
        }
        if let Some(error) = &session.state.move_instance_dest_error {
            kit::label(
                ui,
                error,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        if kit.button(ui, "move::cancel", &tr!("Cancel"), ButtonKind::Neutral, true) {
            cancel = true;
        }
        if kit.button(
            ui,
            "move::start",
            &tr!("Move Instance"),
            ButtonKind::Primary,
            session.state.move_instance_dest_valid,
        ) {
            start = api::validate_move_destination(&session.state.move_instance_dest_input).ok();
        }
    });
    if pick {
        state.file_dialogs.open(
            "move::destination",
            Pick::Folder,
            PickerDialog::new().title(tr!("Choose Destination Folder")),
        );
    }
    if let Some(destination) = start
        && let Some(session) = state.sessions.get_mut(id)
    {
        api::start_move(&mut session.state, root, destination);
    }
    let done = ((response.dismissed || kit.back_pressed()) || cancel)
        && !state
            .sessions
            .get(id)
            .is_some_and(|session| session.state.move_instance_in_flight);
    if done {
        if let Some(session) = state.sessions.get_mut(id) {
            session.state.move_instance_completion_message = None;
            session.state.move_instance_completion_failed = false;
        }
        kit.forget("move::");
    }
    !done
}

fn revalidate(session: &mut InstanceSession) {
    match api::validate_move_destination(&session.state.move_instance_dest_input) {
        Ok(_) => {
            session.state.move_instance_dest_valid = true;
            session.state.move_instance_dest_error = None;
        }
        Err(err) => {
            session.state.move_instance_dest_valid = false;
            session.state.move_instance_dest_error = Some(err);
        }
    }
}

// -- Screenshot viewer ----------------------------------------------------------------------------------------------

/// The instance screenshot viewer: the picture at full size with copy, folder and (confirmed) delete. Returns false
/// once it should close.
pub fn screenshot(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    id: &str,
    path: &std::path::Path,
) -> bool {
    let viewport = ui.viewport_size();
    let max_width = (viewport.x - 160.0).clamp(320.0, 1500.0);
    let max_height = (viewport.y - 280.0).clamp(200.0, 900.0);
    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fit,
        ..ModalStyle::default()
    };
    let key = format!("viewer::{}", path.display());
    let picture = kit
        .images
        .get(&mut ui.engine(), &key, || ImageSource::File(path.to_path_buf()), 0);
    let confirm_id = format!("viewer-confirm::{}", path.display());
    let confirming = state.confirming_screenshot_delete.as_deref() == Some(confirm_id.as_str());
    let deleting = state
        .sessions
        .get(id)
        .is_some_and(|session| session.state.delete_screenshot_in_flight);
    let mut close = false;
    let mut copy = false;
    let mut delete = false;
    let mut ask = false;
    let mut cancel_confirm = false;
    let controller = (state.gamepad.viewer_pan, state.gamepad.viewer_zoom);
    let displayed_time = state.config.displayed_time().clone();
    let viewer = &mut state.viewer;
    let response = kit::modal(ui, "instance-screenshot", &style, !deleting, |ui| {
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
                    "instance-screenshot::image",
                    &key,
                    path,
                    &displayed_time,
                    picture,
                    max_width,
                    max_height,
                );
            }
            None if kit.images.failed(&key) => {
                kit.text(ui, TextRole::Danger, &tr!("This screenshot could not be opened."))
            }
            None => kit.text(ui, TextRole::Muted, &tr!("Loading…")),
        }
        kit.text(
            ui,
            TextRole::Small,
            &path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        if confirming {
            kit::label(
                ui,
                &tr!("Delete this screenshot from disk? This permanently removes it."),
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
            let _row = kit.row(ui, kit::SPACE_SM);
            kit.fill(ui);
            if kit.button(
                ui,
                "instance-screenshot::no",
                &tr!("Cancel"),
                ButtonKind::Neutral,
                !deleting,
            ) {
                cancel_confirm = true;
            }
            if kit.button(
                ui,
                "instance-screenshot::yes",
                &if deleting { tr!("Deleting…") } else { tr!("Delete") },
                ButtonKind::Danger,
                !deleting,
            ) {
                delete = true;
            }
        } else {
            let _row = kit.row(ui, kit::SPACE_SM);
            if kit.button(
                ui,
                "instance-screenshot::copy",
                &tr!("Copy"),
                ButtonKind::Neutral,
                true,
            ) {
                copy = true;
            }
            if kit.button(
                ui,
                "instance-screenshot::folder",
                &tr!("Show in folder"),
                ButtonKind::Neutral,
                true,
            ) && let Some(parent) = path.parent()
                && let Err(err) = launcher_ui::desktop::open_in_file_manager(parent)
            {
                launcher_ui::notification::error!("screenshots", "Failed to open the folder: {err}", err = err);
            }
            kit.fill(ui);
            if kit.button(
                ui,
                "instance-screenshot::delete",
                &tr!("Delete"),
                ButtonKind::Danger,
                true,
            ) {
                ask = true;
            }
            if kit.button(
                ui,
                "instance-screenshot::close",
                &tr!("Close"),
                ButtonKind::Neutral,
                true,
            ) {
                close = true;
            }
        }
    });
    if copy {
        launcher_ui::app_commands::push(launcher_ui::app_commands::AppCommand::CopyImage(
            path.to_path_buf(),
        ));
    }
    if ask {
        state.confirming_screenshot_delete = Some(confirm_id.clone());
    }
    if cancel_confirm {
        state.confirming_screenshot_delete = None;
    }
    if delete && let Some(session) = state.sessions.get_mut(id) {
        api::request_screenshot_delete(&mut session.state, &api::key_of(path));
        state.screenshot_delete_requested = true;
    }
    // Once the delete finishes the file is gone from the session's list; close with it.
    // The file is probed once, when the delete finishes, rather than every frame.
    let finished = state.screenshot_delete_requested
        && state
            .sessions
            .get(id)
            .is_some_and(|session| !session.state.delete_screenshot_in_flight);
    if finished {
        state.screenshot_delete_requested = false;
    }
    let gone = finished && confirming && !path.exists();
    if close || gone || ((response.dismissed || kit.back_pressed()) && !deleting) {
        state.confirming_screenshot_delete = None;
        return false;
    }
    true
}

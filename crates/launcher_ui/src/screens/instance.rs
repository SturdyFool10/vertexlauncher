use config::{
    Config, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
    JavaRuntimeVersion,
};
use content_resolver::{
    InstalledContentFile, InstalledContentHashCache, InstalledContentKind,
    InstalledContentResolver, ResolveInstalledContentRequest, ResolvedInstalledContent,
};
use egui::Ui;
use installation::{
    DownloadPolicy, InstallProgress, InstallProgressCallback, InstallStage, LaunchRequest,
    LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry, VersionCatalog,
    display_user_path, ensure_game_files, ensure_openjdk_runtime, fetch_loader_versions_for_game,
    fetch_version_catalog_with_refresh, is_instance_running_for_account, launch_instance,
    normalize_path_key, running_instance_for_account, stop_running_instance_for_account,
};
use instances::{
    InstanceStore, record_instance_launch_usage, set_instance_settings, set_instance_versions,
};
use managed_content::{InstalledContentIdentity, load_managed_content_identities};
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use textui::{ButtonOptions, LabelOptions, TextUi, TooltipOptions};
use vtmpack::{
    VTMPACK_EXTENSION, VtmpackInstanceMetadata, VtmpackProviderMode, default_vtmpack_file_name,
    default_vtmpack_root_entry_selected, enforce_vtmpack_extension, export_instance_as_vtmpack,
    list_exportable_root_entries, sync_vtmpack_export_options,
};

use crate::app::tokio_runtime;
use crate::desktop;
use crate::screens::{AppScreen, LaunchAuthContext};
use crate::ui::{
    components::{icon_button, remote_tiled_image, settings_widgets, text_helpers},
    modal, style,
};
use crate::{assets, console, install_activity, notification, privacy};

mod content;
mod content_lookup_result;
mod installed_content_cache;
mod installed_entry_render_result;
mod instance_screen_output;
mod instance_screen_state;
mod runtime;
mod runtime_prepare_operation;
mod runtime_prepare_outcome;

use content::{poll_content_lookup_results, render_installed_content_section};
use content_lookup_result::ContentLookupResult;
use installed_content_cache::InstalledContentCache;
use installed_entry_render_result::InstalledEntryRenderResult;
pub use instance_screen_output::InstanceScreenOutput;
use instance_screen_state::InstanceScreenState;
use runtime::*;
use runtime_prepare_operation::RuntimePrepareOperation;
use runtime_prepare_outcome::RuntimePrepareOutcome;

const RESERVED_SYSTEM_MEMORY_MIB: u128 = 4 * 1024;
const FALLBACK_TOTAL_MEMORY_MIB: u128 = 20 * 1024;
const MODLOADER_OPTIONS: [&str; 6] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt", "Custom"];
const CUSTOM_MODLOADER_INDEX: usize = MODLOADER_OPTIONS.len() - 1;
const INSTALLED_CONTENT_SCROLLBAR_RESERVE: f32 = 18.0;
const INSTALLED_CONTENT_PAGE_SIZES: [usize; 4] = [10, 25, 50, 100];

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    active_account_owns_minecraft: bool,
    streamer_mode: bool,
    instances: &mut InstanceStore,
    config: &mut Config,
    account_avatars_by_key: &HashMap<String, Vec<u8>>,
) -> InstanceScreenOutput {
    let mut output = InstanceScreenOutput::default();
    let text_color = ui.visuals().text_color();
    let heading_style = LabelOptions {
        font_size: 30.0,
        line_height: 34.0,
        weight: 700,
        color: text_color,
        wrap: false,
        ..LabelOptions::default()
    };
    let body_style = LabelOptions {
        color: text_color,
        wrap: true,
        ..LabelOptions::default()
    };
    let mut muted_style = body_style.clone();
    muted_style.color = ui.visuals().weak_text_color();

    let Some(instance_id) = selected_instance_id else {
        let _ = text_ui.label(
            ui,
            "instance_screen_empty_heading",
            "Instance",
            &heading_style,
        );
        ui.add_space(8.0);
        let _ = text_ui.label(
            ui,
            "instance_screen_empty_body",
            "Select an instance from the left sidebar or click + to create one.",
            &body_style,
        );
        return output;
    };

    let Some(instance_snapshot) = instances.find(instance_id).cloned() else {
        let _ = text_ui.label(
            ui,
            "instance_screen_missing_heading",
            "Instance",
            &heading_style,
        );
        ui.add_space(8.0);
        let _ = text_ui.label(
            ui,
            "instance_screen_missing_body",
            "Selected instance no longer exists.",
            &body_style,
        );
        return output;
    };

    let state_id = ui.make_persistent_id(("instance_screen_state", instance_id));
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<InstanceScreenState>(state_id))
        .unwrap_or_else(|| InstanceScreenState::from_instance(&instance_snapshot, config));

    poll_background_tasks(&mut state, config, instances, instance_id);
    sync_version_catalog(&mut state, config.include_snapshots_and_betas(), false);
    if state.version_catalog_in_flight
        || !state.modloader_versions_in_flight.is_empty()
        || state.runtime_prepare_in_flight
    {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let selected_game_version_for_loader = selected_game_version(&state).to_owned();
    ensure_selected_modloader_is_supported(&mut state, selected_game_version_for_loader.as_str());

    let installations_root = std::path::PathBuf::from(config.minecraft_installations_root());
    let instance_root_path = instances::instance_root_path(&installations_root, &instance_snapshot);

    let _ = text_ui.label(
        ui,
        ("instance_screen_root", instance_id),
        &format!("Root: {}", instance_root_path.display()),
        &muted_style,
    );
    ui.add_space(12.0);

    let selected_game_version_for_runtime = selected_game_version(&state).to_owned();
    let external_activity =
        install_activity::snapshot().filter(|activity| activity.instance_id == state.name_input);
    let external_install_active = external_activity
        .as_ref()
        .is_some_and(|activity| !matches!(activity.stage, InstallStage::Complete));
    render_runtime_row(
        ui,
        text_ui,
        &mut state,
        instance_id,
        instance_root_path.as_path(),
        selected_game_version_for_runtime.as_str(),
        config,
        external_install_active,
        active_username,
        active_launch_auth,
        active_account_owns_minecraft,
        streamer_mode,
        account_avatars_by_key,
    );
    render_install_feedback(
        ui,
        text_ui,
        instance_id,
        state.runtime_latest_progress.as_ref(),
        external_activity.as_ref(),
        state.runtime_prepare_in_flight,
    );
    ui.add_space(10.0);
    output.instances_changed |= render_instance_settings_modal(
        ui.ctx(),
        text_ui,
        instance_id,
        &mut state,
        instances,
        config,
    );
    render_export_vtmpack_modal(
        ui.ctx(),
        text_ui,
        instance_id,
        &mut state,
        instances,
        config,
    );

    render_installed_content_section(
        ui,
        text_ui,
        instance_id,
        instance_root_path.as_path(),
        &mut state,
        &mut output,
    );

    ui.ctx().data_mut(|d| d.insert_temp(state_id, state));
    output
}

fn render_instance_settings_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &mut InstanceStore,
    config: &mut Config,
) -> bool {
    if !state.show_settings_modal {
        return false;
    }

    let mut instances_changed = false;
    let mut open = state.show_settings_modal;
    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_width = (viewport_rect.width() * 0.92).max(1.0);
    let modal_height = (viewport_rect.height() * 0.92).max(1.0);
    let modal_pos_x = (viewport_rect.center().x - modal_width * 0.5)
        .clamp(viewport_rect.left(), viewport_rect.right() - modal_width);
    let modal_pos_y = (viewport_rect.center().y - modal_height * 0.5)
        .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_height);
    let modal_pos = egui::pos2(modal_pos_x, modal_pos_y);
    let modal_size = egui::vec2(modal_width, modal_height);
    let mut close_requested = false;
    modal::show_scrim(
        ctx,
        ("instance_settings_modal_scrim", instance_id),
        viewport_rect,
    );

    egui::Window::new("Instance Settings")
        .id(egui::Id::new(("instance_settings_modal", instance_id)))
        .order(egui::Order::Foreground)
        .open(&mut open)
        .fixed_pos(modal_pos)
        .fixed_size(modal_size)
        .collapsible(false)
        .title_bar(false)
        .resizable(false)
        .movable(false)
        .hscroll(false)
        .vscroll(false)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            let text_color = ui.visuals().text_color();
            let mut muted_style = LabelOptions::default();
            muted_style.color = ui.visuals().weak_text_color();
            muted_style.wrap = true;
            let section_style = LabelOptions {
                font_size: 22.0,
                line_height: 26.0,
                weight: 700,
                color: text_color,
                wrap: false,
                ..LabelOptions::default()
            };
            let body_style = LabelOptions {
                color: text_color,
                wrap: true,
                ..LabelOptions::default()
            };
            let action_button_style = ButtonOptions {
                min_size: egui::vec2(220.0, 34.0),
                text_color: ui.visuals().widgets.active.fg_stroke.color,
                fill: ui.visuals().selection.bg_fill,
                fill_hovered: ui.visuals().selection.bg_fill.gamma_multiply(1.1),
                fill_active: ui.visuals().selection.bg_fill.gamma_multiply(0.9),
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().selection.stroke,
                ..ButtonOptions::default()
            };
            let refresh_style = ButtonOptions {
                min_size: egui::vec2(190.0, 30.0),
                text_color: ui.visuals().text_color(),
                fill: ui.visuals().widgets.inactive.bg_fill,
                fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                fill_active: ui.visuals().widgets.active.bg_fill,
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().widgets.inactive.bg_stroke,
                ..ButtonOptions::default()
            };
            let reinstall_button_style = ButtonOptions {
                min_size: egui::vec2(220.0, 34.0),
                text_color: ui.visuals().text_color(),
                fill: ui.visuals().widgets.inactive.bg_fill,
                fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                fill_active: ui.visuals().widgets.active.bg_fill,
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().widgets.inactive.bg_stroke,
                ..ButtonOptions::default()
            };

            egui::ScrollArea::vertical()
                .id_salt(("instance_settings_modal_scroll", instance_id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_modal_heading", instance_id),
                        "Instance Settings",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_modal_description", instance_id),
                        "Manage this profile's metadata, version stack, runtime overrides, and maintenance actions.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_versions_heading", instance_id),
                        "Metadata & Versions",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_versions_description", instance_id),
                        "Display info, Minecraft version, and modloader selection for this instance.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_name_input", instance_id),
                        "Name",
                        Some("Display name shown in the sidebar."),
                        &mut state.name_input,
                    );
                    ui.add_space(6.0);
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_description_input", instance_id),
                        "Description (optional)",
                        Some("Optional note shown in library instance tiles."),
                        &mut state.description_input,
                    );
                    ui.add_space(6.0);

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_thumbnail_input", instance_id),
                        "Thumbnail path (optional)",
                        Some("Local image path for this instance."),
                        &mut state.thumbnail_input,
                    );
                    ui.add_space(6.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_refresh_versions", instance_id),
                            "Refresh version list",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        sync_version_catalog(state, config.include_snapshots_and_betas(), true);
                        state.modloader_versions_cache.clear();
                        state.modloader_versions_status = None;
                        state.modloader_versions_status_key = None;
                    }
                    if state.version_catalog_in_flight {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let _ = text_ui.label(
                                ui,
                                ("instance_versions_loading", instance_id),
                                "Fetching version catalog...",
                                &muted_style,
                            );
                        });
                    }

                    if let Some(catalog_error) = state.version_catalog_error.as_deref() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_version_catalog_error", instance_id),
                            catalog_error,
                            &LabelOptions {
                                color: ui.visuals().error_fg_color,
                                wrap: true,
                                ..LabelOptions::default()
                            },
                        );
                    }

                    let version_labels: Vec<String> = state
                        .available_game_versions
                        .iter()
                        .map(MinecraftVersionEntry::display_label)
                        .collect();
                    let version_refs: Vec<&str> =
                        version_labels.iter().map(String::as_str).collect();
                    if !version_refs.is_empty() {
                        let mut selected_index = state
                            .selected_game_version_index
                            .min(version_refs.len().saturating_sub(1));
                        let response = settings_widgets::dropdown_row(
                            text_ui,
                            ui,
                            ("instance_game_version_dropdown", instance_id),
                            "Minecraft game version",
                            Some("Pick from available Minecraft versions."),
                            &mut selected_index,
                            &version_refs,
                        );
                        if response.changed() {
                            state.selected_game_version_index = selected_index;
                            if let Some(version) = state.available_game_versions.get(selected_index)
                            {
                                state.game_version_input = version.id.clone();
                            }
                        }
                    } else {
                        let _ = text_ui.label(
                            ui,
                            ("instance_game_version_empty", instance_id),
                            "No game versions available yet.",
                            &muted_style,
                        );
                    }
                    ui.add_space(6.0);

                    let selected_game_version_for_loader = selected_game_version(state).to_owned();
                    ensure_selected_modloader_is_supported(
                        state,
                        selected_game_version_for_loader.as_str(),
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_modloader_label", instance_id),
                        "Modloader",
                        &body_style,
                    );
                    ui.add_space(4.0);
                    render_modloader_selector(
                        ui,
                        text_ui,
                        state,
                        instance_id,
                        selected_game_version_for_loader.as_str(),
                    );
                    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
                        ui.add_space(6.0);
                        let _ = settings_widgets::full_width_text_input_row(
                            text_ui,
                            ui,
                            ("instance_custom_modloader_input", instance_id),
                            "Custom modloader id",
                            Some("Use any custom modloader name."),
                            &mut state.custom_modloader,
                        );
                    }
                    ui.add_space(6.0);

                    let selected_modloader_label = selected_modloader_value(state);
                    let modloader_versions_key = modloader_versions_cache_key(
                        selected_modloader_label.as_str(),
                        selected_game_version_for_loader.as_str(),
                    );
                    let available_modloader_versions =
                        selected_modloader_versions(state, selected_game_version_for_loader.as_str())
                            .to_vec();
                    if state.selected_modloader == 0 {
                        state.modloader_version_input.clear();
                    } else {
                        let mut resolved_modloader_versions = available_modloader_versions;
                        let should_fetch_remote = state.selected_modloader != CUSTOM_MODLOADER_INDEX
                            && resolved_modloader_versions.is_empty();
                        if should_fetch_remote {
                            if let Some(cached) =
                                state.modloader_versions_cache.get(&modloader_versions_key)
                            {
                                resolved_modloader_versions = cached.clone();
                            } else {
                                request_modloader_versions(
                                    state,
                                    selected_modloader_label.as_str(),
                                    selected_game_version_for_loader.as_str(),
                                    false,
                                );
                            }
                        }

                        let in_flight = state
                            .modloader_versions_in_flight
                            .contains(&modloader_versions_key);
                        if in_flight {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                let _ = text_ui.label(
                                    ui,
                                    ("instance_modloader_versions_fetching", instance_id),
                                    "Fetching modloader versions...",
                                    &muted_style,
                                );
                            });
                        }

                        if state.modloader_versions_status_key.as_deref()
                            == Some(modloader_versions_key.as_str())
                            && let Some(status) = state.modloader_versions_status.as_deref()
                        {
                            let is_error = status.starts_with("Failed");
                            let _ = text_ui.label(
                                ui,
                                ("instance_modloader_versions_status", instance_id),
                                status,
                                &LabelOptions {
                                    color: if is_error {
                                        ui.visuals().error_fg_color
                                    } else {
                                        ui.visuals().weak_text_color()
                                    },
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
                            );
                        }

                        let mut modloader_version_options: Vec<String> =
                            Vec::with_capacity(resolved_modloader_versions.len() + 1);
                        modloader_version_options.push("Latest available".to_owned());
                        modloader_version_options.extend(resolved_modloader_versions.iter().cloned());
                        let option_refs: Vec<&str> = modloader_version_options
                            .iter()
                            .map(String::as_str)
                            .collect();
                        let current_modloader_version = state.modloader_version_input.trim().to_owned();
                        let mut selected_index = if current_modloader_version.is_empty() {
                            0
                        } else {
                            modloader_version_options
                                .iter()
                                .position(|entry| entry == &current_modloader_version)
                                .unwrap_or(0)
                        };
                        if !current_modloader_version.is_empty() && selected_index == 0 {
                            state.modloader_version_input.clear();
                        }
                        if settings_widgets::full_width_dropdown_row(
                            text_ui,
                            ui,
                            ("instance_modloader_version_dropdown", instance_id),
                            "Modloader version",
                            Some("Cataloged by loader+Minecraft compatibility and cached once per day. Pick Latest available for automatic selection."),
                            &mut selected_index,
                            &option_refs,
                        )
                        .changed()
                        {
                            if selected_index == 0 {
                                state.modloader_version_input.clear();
                            } else if let Some(selected) = modloader_version_options.get(selected_index) {
                                state.modloader_version_input = selected.clone();
                            }
                        }

                        if state.selected_modloader != CUSTOM_MODLOADER_INDEX {
                            let refresh_clicked = ui
                                .add_enabled_ui(!in_flight, |ui| {
                                    text_ui.button(
                                        ui,
                                        ("instance_modloader_versions_refresh", instance_id),
                                        "Refresh modloader versions",
                                        &refresh_style,
                                    )
                                })
                                .inner
                                .clicked();
                            if refresh_clicked {
                                request_modloader_versions(
                                    state,
                                    selected_modloader_label.as_str(),
                                    selected_game_version_for_loader.as_str(),
                                    true,
                                );
                            }
                        }

                        if resolved_modloader_versions.is_empty()
                            && state.selected_modloader != CUSTOM_MODLOADER_INDEX
                        {
                            let _ = text_ui.label(
                                ui,
                                ("instance_modloader_versions_unavailable", instance_id),
                                "No cataloged modloader versions were found for this Minecraft version.",
                                &muted_style,
                            );
                        }
                    }

                    ui.add_space(8.0);

                    let trimmed_name = state.name_input.trim();
                    let requested_modloader = selected_modloader_value(state);
                    let requested_game_version = state.game_version_input.trim().to_owned();
                    let validation_error = if trimmed_name.is_empty() {
                        Some("Name cannot be empty.".to_owned())
                    } else if requested_game_version.is_empty() {
                        Some("Minecraft game version cannot be empty.".to_owned())
                    } else if requested_modloader.trim().is_empty() {
                        Some("Modloader cannot be empty.".to_owned())
                    } else if support_catalog_ready(state)
                        && !state
                            .loader_support
                            .supports_loader(
                                requested_modloader.as_str(),
                                requested_game_version.as_str(),
                            )
                        && state.selected_modloader != CUSTOM_MODLOADER_INDEX
                    {
                        Some(format!(
                            "{} is not available for Minecraft {}.",
                            requested_modloader, requested_game_version
                        ))
                    } else {
                        resolve_modloader_version_for_settings(
                            state,
                            requested_modloader.as_str(),
                            requested_game_version.as_str(),
                        )
                        .err()
                    };
                    let can_save_versions = validation_error.is_none();
                    if let Some(error) = validation_error.as_deref() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_save_versions_validation_error", instance_id),
                            error,
                            &LabelOptions {
                                color: ui.visuals().error_fg_color,
                                wrap: true,
                                ..LabelOptions::default()
                            },
                        );
                        ui.add_space(6.0);
                    }

                    let save_versions_clicked = ui
                        .add_enabled_ui(can_save_versions, |ui| {
                            text_ui.button(
                                ui,
                                ("instance_save_versions", instance_id),
                                "Save metadata & versions",
                                &action_button_style,
                            )
                        })
                        .inner
                        .clicked();
                    let reinstall_enabled =
                        can_save_versions && !state.runtime_prepare_in_flight && !state.running;
                    let reinstall_clicked = ui
                        .add_enabled_ui(reinstall_enabled, |ui| {
                            text_ui.button(
                                ui,
                                ("instance_reinstall_profile", instance_id),
                                "Reinstall Profile",
                                &reinstall_button_style,
                            )
                        })
                        .inner
                        .clicked();
                    if save_versions_clicked {
                        match save_instance_metadata_and_versions(state, instance_id, instances) {
                            Ok(()) => {
                                instances_changed = true;
                                if let Some(saved) = instances.find(instance_id) {
                                    tracing::info!(
                                        target: "vertexlauncher/ui/instance",
                                        instance_id = %instance_id,
                                        saved_modloader = %saved.modloader,
                                        saved_game_version = %saved.game_version,
                                        saved_modloader_version = %saved.modloader_version,
                                        "Saved instance metadata and versions."
                                    );
                                }
                                state.status_message =
                                    Some("Saved metadata and version settings.".to_owned());
                            }
                            Err(err) => {
                                tracing::warn!(
                                    target: "vertexlauncher/ui/instance",
                                    instance_id = %instance_id,
                                    error = %err,
                                    "Failed to save instance metadata and versions."
                                );
                                state.status_message = Some(err);
                            }
                        }
                    }
                    if reinstall_clicked {
                        match save_instance_metadata_and_versions(state, instance_id, instances) {
                            Ok(()) => {
                                instances_changed = true;
                                let game_version = state.game_version_input.trim().to_owned();
                                let modloader = selected_modloader_value(state);
                                if let Some(saved_instance) = instances.find(instance_id).cloned() {
                                    let modloader_version = normalize_optional(
                                        saved_instance.modloader_version.as_str(),
                                    );
                                    let installations_root =
                                        PathBuf::from(config.minecraft_installations_root());
                                    let instance_root = instances::instance_root_path(
                                        &installations_root,
                                        &saved_instance,
                                    );
                                    request_runtime_prepare(
                                        state,
                                        RuntimePrepareOperation::ReinstallProfile,
                                        instance_root,
                                        game_version.clone(),
                                        modloader.clone(),
                                        modloader_version,
                                        effective_required_java_major(
                                            config,
                                            game_version.as_str(),
                                        ),
                                        choose_java_executable(
                                            config,
                                            state.java_override_enabled,
                                            state.java_override_runtime_major,
                                            effective_required_java_major(
                                                config,
                                                game_version.as_str(),
                                            ),
                                        ),
                                        config.download_max_concurrent(),
                                        config.parsed_download_speed_limit_bps(),
                                        config.default_instance_max_memory_mib(),
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                    );
                                } else {
                                    state.status_message =
                                        Some("Instance was removed before reinstall.".to_owned());
                                }
                            }
                            Err(err) => {
                                state.status_message = Some(err);
                            }
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_heading", instance_id),
                        "Runtime Overrides",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_runtime_overrides_description", instance_id),
                        "Per-instance overrides for memory, JVM arguments, and Java runtime selection.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = settings_widgets::toggle_row(
                        text_ui,
                        ui,
                        "Override max memory for this instance",
                        Some("When disabled, launcher instance default memory is used."),
                        &mut state.memory_override_enabled,
                    );
                    ui.add_space(6.0);

                    let memory_slider_max = memory_slider_max_mib();
                    if state.memory_override_enabled {
                        let mut memory_mib = state
                            .memory_override_mib
                            .clamp(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, memory_slider_max);
                        let response = settings_widgets::u128_slider_with_input_row(
                            text_ui,
                            ui,
                            ("instance_memory_override", instance_id),
                            "Max memory allocation (MiB)",
                            Some("Per-instance memory limit."),
                            &mut memory_mib,
                            INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
                            memory_slider_max,
                            INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
                        );
                        if response.changed() {
                            state.memory_override_mib = memory_mib;
                        }
                        ui.add_space(6.0);
                    }

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_cli_args_override", instance_id),
                        "JVM args override (optional)",
                        Some("Leave blank to use launcher instance default JVM args."),
                        &mut state.cli_args_input,
                    );
                    ui.add_space(8.0);

                    let _ = settings_widgets::toggle_row(
                        text_ui,
                        ui,
                        "Override Java runtime for this instance",
                        Some("When enabled, this instance will use the selected configured global Java path."),
                        &mut state.java_override_enabled,
                    );
                    ui.add_space(6.0);

                    let java_options = configured_java_path_options(config);
                    if state.java_override_enabled {
                        if java_options.is_empty() {
                            let _ = text_ui.label(
                                ui,
                                ("instance_java_override_no_options", instance_id),
                                "No configured global Java paths found. Add at least one Java path in Settings first.",
                                &LabelOptions {
                                    color: ui.visuals().error_fg_color,
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
                            );
                        } else {
                            if state
                                .java_override_runtime_major
                                .is_none_or(|major| !java_options.iter().any(|(m, _)| *m == major))
                            {
                                state.java_override_runtime_major = java_options.first().map(|(major, _)| *major);
                            }
                            let option_labels: Vec<&str> =
                                java_options.iter().map(|(_, label)| label.as_str()).collect();
                            let mut selected_index = java_options
                                .iter()
                                .position(|(major, _)| Some(*major) == state.java_override_runtime_major)
                                .unwrap_or(0);
                            if settings_widgets::full_width_dropdown_row(
                                text_ui,
                                ui,
                                ("instance_java_override_runtime", instance_id),
                                "Java path override",
                                Some("Select which configured Java path this instance should use."),
                                &mut selected_index,
                                &option_labels,
                            )
                            .changed()
                            {
                                state.java_override_runtime_major =
                                    java_options.get(selected_index).map(|(major, _)| *major);
                            }
                        }
                    }
                    ui.add_space(8.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_save_settings", instance_id),
                            "Save instance settings",
                            &action_button_style,
                        )
                        .clicked()
                    {
                        let java_override_runtime_major = if state.java_override_enabled {
                            if java_options.is_empty() {
                                state.status_message = Some(
                                    "Cannot save Java override: configure at least one global Java path in Settings."
                                        .to_owned(),
                                );
                                None
                            } else {
                                let selected = state.java_override_runtime_major.and_then(|major| {
                                    java_options
                                        .iter()
                                        .find_map(|(candidate, _)| (*candidate == major).then_some(major))
                                });
                                selected.or_else(|| java_options.first().map(|(major, _)| *major))
                            }
                        } else {
                            None
                        };
                        if !state.java_override_enabled || java_override_runtime_major.is_some() {
                            let memory_override = if state.memory_override_enabled {
                                Some(
                                    state
                                        .memory_override_mib
                                        .clamp(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, memory_slider_max),
                                )
                            } else {
                                None
                            };
                            let cli_override = normalize_optional(state.cli_args_input.as_str());
                            match set_instance_settings(
                                instances,
                                instance_id,
                                memory_override,
                                cli_override,
                                state.java_override_enabled,
                                java_override_runtime_major,
                            ) {
                                Ok(()) => {
                                    instances_changed = true;
                                    state.status_message = Some("Saved instance settings.".to_owned());
                                }
                                Err(err) => state.status_message = Some(err.to_string()),
                            }
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_actions_heading", instance_id),
                        "Maintenance & Actions",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_actions_description", instance_id),
                        "Open the instance folder and commit any metadata or runtime changes.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_open_folder", instance_id),
                            "Open Instance Folder",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        if let Some(instance) = instances.find(instance_id) {
                            let installations_root =
                                PathBuf::from(config.minecraft_installations_root());
                            let instance_root =
                                instances::instance_root_path(&installations_root, instance);
                            match desktop::open_in_file_manager(instance_root.as_path()) {
                                Ok(()) => {
                                    state.status_message = Some(format!(
                                        "Opened instance folder: {}",
                                        instance_root.display()
                                    ));
                                }
                                Err(err) => {
                                    state.status_message =
                                        Some(format!("Failed to open instance folder: {err}"));
                                }
                            }
                        } else {
                            state.status_message =
                                Some("Instance was removed before opening its folder.".to_owned());
                        }
                    }
                    ui.add_space(6.0);
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_vtmpack", instance_id),
                            "Export .vtmpack...",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        state.show_export_vtmpack_modal = true;
                    }
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if text_ui
                            .button(
                                ui,
                                ("instance_settings_close", instance_id),
                                "Done",
                                &action_button_style,
                            )
                            .clicked()
                        {
                            close_requested = true;
                        }
                    });
                });
        });

    if close_requested {
        open = false;
    }
    state.show_settings_modal = open;
    instances_changed
}

fn render_export_vtmpack_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &InstanceStore,
    config: &Config,
) {
    if !state.show_export_vtmpack_modal {
        return;
    }

    let mut open = state.show_export_vtmpack_modal;
    let mut close_requested = false;
    let viewport_rect = ctx.input(|i| i.content_rect());
    let installations_root = PathBuf::from(config.minecraft_installations_root());
    let instance_root = instances
        .find(instance_id)
        .map(|instance| instances::instance_root_path(&installations_root, instance));
    if let Some(instance_root) = instance_root.as_deref() {
        sync_vtmpack_export_options(instance_root, &mut state.export_vtmpack_options);
    }
    let modal_width = viewport_rect.width().min(560.0).max(320.0);
    let modal_height = viewport_rect.height().min(520.0).max(300.0);
    let modal_pos = egui::pos2(
        (viewport_rect.center().x - modal_width * 0.5)
            .clamp(viewport_rect.left(), viewport_rect.right() - modal_width),
        (viewport_rect.center().y - modal_height * 0.5)
            .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_height),
    );
    modal::show_scrim(
        ctx,
        ("instance_export_vtmpack_modal_scrim", instance_id),
        viewport_rect,
    );

    let mut export_requested = false;
    egui::Window::new("Export .vtmpack")
        .id(egui::Id::new(("instance_export_vtmpack_modal", instance_id)))
        .order(egui::Order::Foreground)
        .open(&mut open)
        .fixed_pos(modal_pos)
        .fixed_size(egui::vec2(modal_width, modal_height))
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .hscroll(false)
        .vscroll(true)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            let title_style = LabelOptions {
                font_size: 26.0,
                line_height: 30.0,
                weight: 700,
                color: ui.visuals().text_color(),
                wrap: false,
                ..LabelOptions::default()
            };
            let body_style = LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            };
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_title", instance_id),
                "Export .vtmpack",
                &title_style,
            );
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_body", instance_id),
                "Choose whether the exported pack may reference CurseForge metadata directly, then select which top-level files and folders from the Minecraft root should be bundled into the pack.",
                &body_style,
            );
            ui.add_space(12.0);

            for provider_mode in [
                VtmpackProviderMode::IncludeCurseForge,
                VtmpackProviderMode::ExcludeCurseForge,
            ] {
                let selected = state.export_vtmpack_options.provider_mode == provider_mode;
                if ui.radio(selected, provider_mode.label()).clicked() {
                    state.export_vtmpack_options.provider_mode = provider_mode;
                }
            }

            ui.add_space(12.0);
            let explanation = match state.export_vtmpack_options.provider_mode {
                VtmpackProviderMode::IncludeCurseForge => {
                    "Managed CurseForge entries stay downloadable in the pack manifest."
                }
                VtmpackProviderMode::ExcludeCurseForge => {
                    "CurseForge metadata is removed from the export. CurseForge-managed files are bundled into the pack unless they already use Modrinth as the selected source."
                }
            };
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_explanation", instance_id),
                explanation,
                &body_style,
            );

            ui.add_space(16.0);
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_include_label", instance_id),
                "Include top-level entries from the Minecraft root",
                &LabelOptions {
                    font_size: 18.0,
                    line_height: 22.0,
                    weight: 600,
                    color: ui.visuals().text_color(),
                    wrap: false,
                    ..LabelOptions::default()
                },
            );
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_include_help", instance_id),
                "Defaults to mods, resourcepacks, shaderpacks, and config. You can also include any other top-level files or folders found in the instance root.",
                &body_style,
            );
            ui.add_space(8.0);

            if let Some(instance_root) = instance_root.as_deref() {
                let entries = list_exportable_root_entries(instance_root);
                egui::ScrollArea::vertical()
                    .id_salt(("instance_export_vtmpack_entries_scroll", instance_id))
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for entry in entries {
                            let checked = state
                                .export_vtmpack_options
                                .included_root_entries
                                .entry(entry.clone())
                                .or_insert_with(|| default_vtmpack_root_entry_selected(&entry));
                            let label = if instance_root.join(entry.as_str()).is_dir() {
                                format!("{entry}/")
                            } else {
                                entry.clone()
                            };
                            ui.checkbox(checked, label);
                        }
                    });
            } else {
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_missing_instance", instance_id),
                    "Instance root is unavailable, so folder selection cannot be shown.",
                    &body_style,
                );
            }

            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if text_ui
                    .button(
                        ui,
                        ("instance_export_vtmpack_cancel", instance_id),
                        "Cancel",
                        &ButtonOptions::default(),
                    )
                    .clicked()
                {
                    close_requested = true;
                }
                if text_ui
                    .button(
                        ui,
                        ("instance_export_vtmpack_confirm", instance_id),
                        "Choose file",
                        &ButtonOptions::default(),
                    )
                    .clicked()
                {
                    export_requested = true;
                }
            });
        });

    if close_requested {
        open = false;
    }

    if export_requested {
        open = false;
        if let Some(instance) = instances.find(instance_id) {
            let instance_root = instances::instance_root_path(&installations_root, instance);
            let default_file_name = default_vtmpack_file_name(instance.name.as_str());
            let selected_output = rfd::FileDialog::new()
                .set_title("Export Modpack")
                .set_file_name(default_file_name.as_str())
                .add_filter("Vertex Modpack", &[VTMPACK_EXTENSION])
                .save_file();

            if let Some(selected_path) = selected_output {
                let output_path = enforce_vtmpack_extension(selected_path);
                let pack_instance = VtmpackInstanceMetadata {
                    id: instance.id.clone(),
                    name: instance.name.clone(),
                    game_version: instance.game_version.clone(),
                    modloader: instance.modloader.clone(),
                    modloader_version: instance.modloader_version.clone(),
                };
                match export_instance_as_vtmpack(
                    &pack_instance,
                    instance_root.as_path(),
                    output_path.as_path(),
                    &state.export_vtmpack_options,
                ) {
                    Ok(stats) => {
                        state.status_message = Some(format!(
                            "Exported {} ({} bundled mods, {} config files, {} additional files) to {}",
                            instance.name,
                            stats.bundled_mod_files,
                            stats.config_files,
                            stats.additional_files,
                            output_path.display()
                        ));
                    }
                    Err(err) => {
                        state.status_message = Some(format!("Failed to export .vtmpack: {err}"));
                    }
                }
            }
        } else {
            state.status_message = Some("Instance was removed before export.".to_owned());
        }
    }

    state.show_export_vtmpack_modal = open;
}

fn apply_color_to_svg(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}

fn ensure_selected_modloader_is_supported(state: &mut InstanceScreenState, game_version: &str) {
    if !support_catalog_ready(state) {
        return;
    }
    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
        return;
    }

    let selected_label = MODLOADER_OPTIONS
        .get(state.selected_modloader)
        .copied()
        .unwrap_or(MODLOADER_OPTIONS[0]);
    let entered_modloader_version = state.modloader_version_input.trim();
    if entered_modloader_version.is_empty()
        || is_latest_modloader_version_alias(entered_modloader_version)
    {
        return;
    }
    let Some(known_versions) = state
        .loader_versions
        .versions_for_loader(selected_label, game_version)
    else {
        return;
    };
    if known_versions
        .iter()
        .any(|version| version.eq_ignore_ascii_case(entered_modloader_version))
    {
        return;
    }

    tracing::warn!(
        target: "vertexlauncher/ui/instance",
        selected_modloader = %selected_label,
        game_version = %game_version,
        selected_modloader_version = %entered_modloader_version,
        "Selected modloader version is not currently marked compatible for this game version; keeping user selection."
    );
}

fn support_catalog_ready(state: &InstanceScreenState) -> bool {
    state.version_catalog_include_snapshots.is_some() && state.version_catalog_error.is_none()
}

fn memory_slider_max_mib() -> u128 {
    static CACHED: OnceLock<u128> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let total_mib = detect_total_memory_mib().unwrap_or(FALLBACK_TOTAL_MEMORY_MIB);
        total_mib
            .saturating_sub(RESERVED_SYSTEM_MEMORY_MIB)
            .max(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN)
    })
}

#[cfg(target_os = "linux")]
fn detect_total_memory_mib() -> Option<u128> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = "/proc/meminfo", context = "detect total memory");
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u128>().ok()?;
    Some(kib / 1024)
}

#[cfg(target_os = "windows")]
fn detect_total_memory_mib() -> Option<u128> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..unsafe { std::mem::zeroed() }
    };

    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return None;
    }

    Some((status.ullTotalPhys as u128) / (1024 * 1024))
}

#[cfg(target_os = "macos")]
fn detect_total_memory_mib() -> Option<u128> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let bytes = String::from_utf8(output.stdout).ok()?;
    let bytes = bytes.trim().parse::<u128>().ok()?;
    Some(bytes / (1024 * 1024))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn detect_total_memory_mib() -> Option<u128> {
    None
}

use config::{
    Config, ConfigFormat, JavaRuntimeVersion, LoadConfigResult, create_default_config, load_config,
    save_config,
};
use eframe::{self, egui, egui_wgpu::wgpu};
use egui::CentralPanel;
use installation::{
    DownloadPolicy, InstallProgress, InstallProgressCallback, InstallStage, display_user_path,
    ensure_game_files_async, ensure_openjdk_runtime_async, normalize_path_key,
    running_instance_for_account, running_instance_roots, take_finished_instance_processes,
};
use instances::{
    InstanceRecord, InstanceStore, create_instance, delete_instance, instance_root_path,
    load_store, save_store as save_instance_store,
};
use launcher_runtime as tokio_runtime;
use launcher_ui::ui::svg_aa;
use launcher_ui::{
    console, install_activity, notification, screens, ui,
    ui::instance_context_menu::InstanceContextAction, window_effects,
};
use std::{
    any::Any,
    collections::HashMap,
    fs,
    io::{Read, Write},
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use textui::TextUi;
use textui_egui as textui_adapter;
use textui_egui::prelude::*;
use ui_foundation::{DialogPreset, dialog_options, secondary_button, show_dialog};

use self::auth_state::{AuthState, REPAINT_INTERVAL};
use self::config_format_modal::ModalAction;
use self::discord_presence::DiscordPresenceManager;
use self::fonts::FontController;

mod app_core;
mod app_icon;
mod app_metadata;
mod appearance;
mod auth_state;
mod cli;
mod config_format_modal;
mod create_instance_modal;
mod discord_presence;
mod discover_flow;
mod fonts;
mod gamepad;
mod gamepad_calibration_modal;
mod import_flow;
mod import_instance_modal;
mod native_options;
mod persistence;
mod platform;
mod single_instance;
mod startup;
mod system_browser_sign_in;
mod taskbar_progress;
mod tracing_setup;
mod webview_runtime;
mod webview_sign_in;

pub use app_core::{RunError, init_tracing};
pub use single_instance::{SingleInstanceError, acquire_single_instance};
pub use startup::{maybe_run_cli_command, maybe_run_webview_helper, run};

use self::{
    app_core::*, appearance::*, discover_flow::*, import_flow::*, persistence::*, startup::*,
};

use instances::{InstanceStore, instance_root_path, save_store as save_instance_store};
use launcher_runtime as tokio_runtime;
use launcher_ui::screens;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod app_icon;
mod app_metadata;
mod appearance;
mod better_blur;
mod auth_state;
mod cli;
mod discord_presence;
mod discover_flow;
mod fonts;
mod gamepad;
mod gamepad_calibration_modal;
mod import_flow;
mod import_instance_modal;
mod obs_detection;
mod platform;
mod se5;
mod single_instance;
mod startup;
mod system_browser_sign_in;
mod tracing_setup;
mod webview_runtime;
mod webview_sign_in;

use self::import_flow::*;
pub use single_instance::{SingleInstanceError, acquire_single_instance};
pub(self) use startup::InitialInstanceInstallResult;
pub use startup::{
    RunError, init_tracing, maybe_run_cli_command, maybe_run_webview_helper, run_se5,
};

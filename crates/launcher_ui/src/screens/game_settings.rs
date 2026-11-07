//! In-game settings editor for an instance's `options.txt`.
//!
//! The dialog shows a live view of the file: edits are written a moment after you stop
//! typing (on top of whatever is on disk at that time), and changes made by the game or by
//! settings sync appear in the dialog as soon as they hit the disk.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use config::Config;
use instance_sync::{PackStatus, pack_status, scan_packs};
use instances::{InstanceRecord, InstanceStore, instance_root_path};
use mc_options::packs::PackSupport;
use mc_options::schema::{self, Display, Kind, OptionDef};
use mc_options::translate::format_for_def;
use mc_options::{LiveOptions, Poll, Ver, keys, parse_game_version, value};

use crate::sync_runner::{self, SyncRunConfig};

#[path = "game_settings/api.rs"]
pub mod api;
#[path = "game_settings/audio_devices.rs"]
mod audio_devices;
#[path = "game_settings/keybind_row.rs"]
mod keybind_row;
#[path = "game_settings/option_rows.rs"]
mod option_rows;
#[path = "game_settings/packs_tab.rs"]
mod packs_tab;
#[path = "game_settings/search.rs"]
mod search;

use self::keybind_row::*;
use self::option_rows::*;
use self::packs_tab::*;
use self::search::*;


/// Edits are written once they have been idle this long.
const SAVE_DELAY: Duration = Duration::from_millis(250);
/// How often to look for outside changes while the dialog is open.





/// A resource pack seen in this instance or another syncing instance.
#[derive(Clone)]
struct PackRow {
    name: String,
    support: Option<PackSupport>,
    present_here: bool,
    /// Names of other instances that have it.
    elsewhere: Vec<String>,
}



/// Opens the live view of `options.txt`. If the game hasn't recorded its data version there yet
/// (fresh instance), records it now, as if that version of the game had written the file. The
/// value is never shown in the UI.
fn open_live_options(
    path: std::path::PathBuf,
    instance_root: &Path,
    game_version: &str,
) -> LiveOptions {
    let mut live = LiveOptions::open(path);
    if live.get("version").is_none()
        && let Some(world_version) = mc_options::data_version::read_world_version(
            &mc_options::data_version::client_jar_path(instance_root, game_version),
        )
    {
        live.edit("version", Some(&world_version.to_string()));
    }
    live
}



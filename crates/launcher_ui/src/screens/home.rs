use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use config::Config;
use flate2::read::GzDecoder;
use instances::{InstanceStore, instance_root_path, set_server_favorite, set_world_favorite};
use launcher_runtime as tokio_runtime;



#[path = "home_nbt.rs"]
mod home_nbt;
use home_nbt::{parse_servers_dat, parse_world_metadata};

#[path = "home/home_api.rs"]
pub mod api;
#[path = "home/home_activity_result_channel.rs"]
mod home_activity_result_channel;
#[path = "home/home_activity_scan_instance.rs"]
mod home_activity_scan_instance;
#[path = "home/home_activity_scan_request.rs"]
mod home_activity_scan_request;
#[path = "home/home_activity_scan_result.rs"]
mod home_activity_scan_result;
#[path = "home/home_entry_ref.rs"]
mod home_entry_ref;
#[path = "home/home_output.rs"]
mod home_output;
#[path = "home/home_presence_section.rs"]
mod home_presence_section;
#[path = "home/home_state.rs"]
mod home_state;
#[path = "home/home_tab.rs"]
mod home_tab;
#[path = "home/home_ui_metrics.rs"]
mod home_ui_metrics;
#[path = "home/world_entry.rs"]
mod world_entry;

#[path = "home/activity_scan.rs"]
mod activity_scan;
#[path = "home/quick_launch_copy.rs"]
mod quick_launch_copy;
#[path = "home/server_row.rs"]
mod server_row;
#[path = "home/world_row.rs"]
mod world_row;

use self::activity_scan::*;
use self::server_row::*;
use self::world_row::*;

const HOME_SCAN_INTERVAL: Duration = Duration::from_secs(3);
const SERVER_PING_REFRESH_INTERVAL: Duration = Duration::from_secs(20);
const SERVER_PINGS_PER_SCAN: usize = 3;










const SCREENSHOT_SCAN_INTERVAL: Duration = Duration::from_secs(10);
const SCREENSHOT_PAGE_SIZE: usize = 30;









#[path = "home_grouping.rs"]
mod home_grouping;
#[path = "home_screenshots.rs"]
mod home_screenshots;
#[path = "home_server_ping.rs"]
mod home_server_ping;
#[path = "home_support.rs"]
mod home_support;
#[path = "home_thumbnails.rs"]
mod home_thumbnails;
#[path = "screenshot_common.rs"]
pub(in crate::screens) mod screenshot_common;

use self::home_activity_result_channel::HomeActivityResultChannel;
use self::home_activity_scan_instance::HomeActivityScanInstance;
use self::home_activity_scan_request::HomeActivityScanRequest;
use self::home_activity_scan_result::HomeActivityScanResult;
use self::home_grouping::EntryInstance;
pub use self::home_presence_section::HomePresenceSection;
use self::home_screenshots::ScreenshotCandidate;
use self::home_screenshots::ScreenshotEntry;
use self::home_screenshots::ScreenshotViewerState;
use self::home_screenshots::poll_delete_screenshot_results;
use self::home_screenshots::poll_screenshot_results;
use self::home_screenshots::{load_next_screenshot_page, poll_screenshot_candidates};
use self::home_screenshots::refresh_screenshot_state;
use self::home_server_ping::ServerEntry;
use self::home_server_ping::ServerPingSnapshot;
use self::home_server_ping::collect_servers_from_request;
use self::home_server_ping::normalize_server_address;
use self::home_server_ping::poll_server_ping_results;
use self::home_server_ping::queue_server_pings;
use self::home_server_ping::retain_known_server_pings;
use self::home_server_ping::server_meta_line;
use self::home_state::HomeState;
use self::home_support::{
    current_time_millis, format_time_ago, modified_millis,
};
use self::home_tab::HomeTab;
use self::home_thumbnails::HomeThumbnailState;
use self::world_entry::WorldEntry;
use crate::notification;





















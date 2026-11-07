//! Notifies the user when a new screenshot shows up in any instance while the launcher is running.
//!
//! Minecraft writes screenshots to `<instance root>/screenshots`, the same folders the Home and
//! Instance galleries scan. Those galleries only scan while visible, so this polls independently.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use config::Config;
use instances::{InstanceStore, instance_root_path};
use launcher_runtime as tokio_runtime;

use crate::account_text::AccountMention;
use crate::notification::{self, NotificationAction, Severity};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

struct DirListing {
    dir: PathBuf,
    files: Vec<PathBuf>,
}

struct WatchState {
    tx: mpsc::Sender<Vec<DirListing>>,
    rx: mpsc::Receiver<Vec<DirListing>>,
    in_flight: bool,
    last_poll: Option<Instant>,
    /// Directories already listed once. A directory's first listing is treated as a baseline so
    /// existing screenshots (or a freshly imported instance) don't raise notifications.
    seeded_dirs: HashSet<PathBuf>,
    known_files: HashSet<PathBuf>,
}

fn state() -> &'static Mutex<WatchState> {
    static STATE: OnceLock<Mutex<WatchState>> = OnceLock::new();
    STATE.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        Mutex::new(WatchState {
            tx,
            rx,
            in_flight: false,
            last_poll: None,
            seeded_dirs: HashSet::new(),
            known_files: HashSet::new(),
        })
    })
}

/// Call once per frame. Cheap: the directory listing runs on a blocking worker every few seconds.
///
/// `player` should be the account that is playing right now, or `None` when no game is
/// running (e.g. the file was copied in by hand), in which case the notification stays generic.
/// The account is mentioned in the notification, so streamer mode hides it when it is drawn.
pub fn poll(
    instances: &InstanceStore,
    config: &Config,
    player: Option<AccountMention<'_>>,
) {
    let Ok(mut state) = state().lock() else {
        return;
    };

    while let Ok(listings) = state.rx.try_recv() {
        state.in_flight = false;
        for listing in listings {
            let baseline = state.seeded_dirs.insert(listing.dir.clone());
            for file in listing.files {
                if state.known_files.insert(file.clone()) && !baseline {
                    // Screenshots arriving close together merge into one notification
                    // ("Steve took three screenshots!"); its buttons act on the newest one.
                    let player_key = player.map(|account| account.id.to_owned());
                    let player_token = player.map(|account| account.token());
                    let stack_key = format!("screenshots:{}", player_key.as_deref().unwrap_or(""));
                    let item = file.to_string_lossy().into_owned();
                    notification::NotificationBuilder::new(Severity::Info, "screenshots", "")
                        .actions(screenshot_actions(file))
                        .stack(stack_key, vec![item], move |shots| {
                            screenshot_message(player_token.as_deref(), shots.len())
                        })
                        .emit();
                }
            }
        }
    }

    let due = state
        .last_poll
        .is_none_or(|last| last.elapsed() >= POLL_INTERVAL);
    if due && !state.in_flight {
        state.in_flight = true;
        state.last_poll = Some(Instant::now());
        let installations_root = config.minecraft_installations_root_path().to_path_buf();
        let targets: Vec<PathBuf> = instances
            .instances
            .iter()
            .map(|instance| {
                instance_root_path(installations_root.as_path(), instance).join("screenshots")
            })
            .collect();
        let tx = state.tx.clone();
        tokio_runtime::spawn_detached(async move {
            let listings = tokio_runtime::spawn_blocking(move || {
                targets
                    .into_iter()
                    .map(|dir| DirListing {
                        files: list_screenshots(dir.as_path()),
                        dir,
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let _ = tx.send(listings);
        });
    }
}

fn list_screenshots(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(OsStr::to_str).is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
                    )
                })
        })
        .collect()
}

/// `player` is an account mention (see [`crate::account_text`]).
fn screenshot_message(player: Option<&str>, count: usize) -> String {
    use notification::natural_language::a_or_count;
    match player {
        Some(name) => format!("{name} took {}!", a_or_count(count, "screenshot")),
        None => format!("We detected {}!", a_or_count(count, "new screenshot")),
    }
}

fn screenshot_actions(path: PathBuf) -> Vec<NotificationAction> {
    let view_path = path.clone();
    let copy_path = path.clone();
    let remove_path = path;
    vec![
        NotificationAction::new("View", move || {
            crate::app_commands::push(crate::app_commands::AppCommand::ViewScreenshot(view_path.clone()));
        }),
        NotificationAction::new("Copy", move || {
            crate::app_commands::push(crate::app_commands::AppCommand::CopyImage(copy_path.clone()));
        }),
        NotificationAction::new("Remove", move || {
            let path = remove_path.clone();
            tokio_runtime::spawn_detached(async move {
                match launcher_runtime::fs::remove_file(path.as_path()).await {
                    Ok(()) => notification::info!("screenshots", "Deleted screenshot from disk."),
                    Err(err) => {
                        tracing::warn!(
                            target: "vertexlauncher/io",
                            op = "remove_file",
                            path = %path.display(),
                            error = %err,
                            context = "delete screenshot from notification"
                        );
                        notification::error!("screenshots", "Failed to delete screenshot: {}", err);
                    }
                }
            });
        })
        .danger(),
    ]
}

#[cfg(test)]
mod tests {
    use super::screenshot_message;

    #[test]
    fn screenshot_messages_follow_the_count() {
        assert_eq!(
            screenshot_message(Some("Steve"), 1),
            "Steve took a screenshot!"
        );
        assert_eq!(
            screenshot_message(Some("Steve"), 3),
            "Steve took three screenshots!"
        );
        assert_eq!(screenshot_message(None, 1), "We detected a new screenshot!");
        assert_eq!(
            screenshot_message(None, 12),
            "We detected several new screenshots!"
        );
    }
}

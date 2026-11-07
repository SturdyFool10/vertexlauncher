//! Cached, asynchronously refreshed listing of an instance root's top-level entries.
//!
//! The export dialogs show these as checkboxes and redraw every frame; listing the folder
//! and stat-ing each entry per frame would put filesystem calls on the UI thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use launcher_runtime::WorkerChannel;

use crate::app::tokio_runtime;

/// How long a listing is trusted before it is refreshed in the background.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RootEntry {
    pub(super) name: String,
    pub(super) is_dir: bool,
}

/// The latest known listing.
#[derive(Clone, Debug, Default)]
pub(super) struct RootListing {
    /// `false` once a scan found the instance root missing.
    pub(super) root_exists: bool,
    pub(super) entries: Arc<[RootEntry]>,
    /// `false` until the first scan finishes, so the UI can show "Loading" instead of "empty".
    pub(super) loaded: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RootEntries {
    root: Option<PathBuf>,
    listing: RootListing,
    scanned_at: Option<Instant>,
    in_flight: bool,
    results: WorkerChannel<(PathBuf, RootListing)>,
}

fn scan(root: &Path) -> RootListing {
    if !root.is_dir() {
        return RootListing::default();
    }
    let entries: Vec<RootEntry> = vtmpack::list_exportable_root_entries(root)
        .into_iter()
        .map(|name| RootEntry {
            is_dir: root.join(&name).is_dir(),
            name,
        })
        .collect();
    RootListing {
        root_exists: true,
        entries: entries.into(),
        loaded: true,
    }
}

impl RootEntries {
    /// The current listing for `root`, starting a background refresh when it is stale or the root changed. Never touches the filesystem on the calling thread.
    pub(super) fn poll(&mut self, root: &Path) -> RootListing {
        for (scanned_root, listing) in self.results.drain().items {
            self.in_flight = false;
            if scanned_root == root {
                self.listing = listing;
                self.scanned_at = Some(Instant::now());
            }
        }
        if self.root.as_deref() != Some(root) {
            // A different instance: don't show the previous one's files while scanning.
            self.root = Some(root.to_path_buf());
            self.listing = RootListing::default();
            self.scanned_at = None;
        }
        let stale = self
            .scanned_at
            .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL);
        if stale && !self.in_flight {
            self.in_flight = true;
            let tx = self.results.sender();
            let root = root.to_path_buf();
            tokio_runtime::spawn_blocking_detached(move || {
                let listing = scan(&root);
                let _ = tx.send((root, listing));
            });
        }
        self.listing.clone()
    }


}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_lists_files_and_folders_and_flags_a_missing_root() {
        let dir = std::env::temp_dir().join(format!("vertex-root-entries-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("options.txt"), "a:1").unwrap();

        let listing = scan(&dir);
        assert!(listing.root_exists && listing.loaded);
        let find = |name: &str| listing.entries.iter().find(|e| e.name == name).cloned();
        assert_eq!(find("mods").map(|e| e.is_dir), Some(true));
        assert_eq!(find("options.txt").map(|e| e.is_dir), Some(false));

        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!scan(&dir).root_exists);
    }
}

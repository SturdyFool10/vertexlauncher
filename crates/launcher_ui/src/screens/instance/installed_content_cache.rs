use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use content_resolver::{InstalledContentFile, InstalledContentKind};
use managed_content::InstalledContentIdentity;

#[derive(Clone, Debug)]
pub(super) struct InstalledContentScanResult {
    pub(super) generation: u64,
    pub(super) kind: InstalledContentKind,
    pub(super) managed_identities: HashMap<String, InstalledContentIdentity>,
    pub(super) files: Arc<[InstalledContentFile]>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct InstalledContentCache {
    pub(super) managed_identities: Option<HashMap<String, InstalledContentIdentity>>,
    pub(super) files_by_tab: HashMap<InstalledContentKind, Arc<[InstalledContentFile]>>,
    pub(super) scan_generation: u64,
    pub(super) scans_in_flight: HashSet<InstalledContentKind>,
    /// Tabs whose file list is out of date: it keeps showing until a fresh scan replaces it, so the list does not blink
    /// out (and the page scroll with it) while rescanning.
    pub(super) stale_tabs: HashSet<InstalledContentKind>,
    pub(super) scan_results: launcher_runtime::WorkerChannel<InstalledContentScanResult>,
}

impl InstalledContentCache {
    /// Marks every file list out of date without dropping it; each is rescanned the next time it is asked for.
    pub(super) fn mark_stale(&mut self) {
        self.managed_identities = None;
        self.stale_tabs.extend(self.files_by_tab.keys().copied());
        self.scans_in_flight.clear();
        self.scan_generation = self.scan_generation.saturating_add(1);
    }

    pub(super) fn clear(&mut self) {
        self.managed_identities = None;
        self.files_by_tab.clear();
        self.stale_tabs.clear();
        self.scans_in_flight.clear();
        self.scan_generation = self.scan_generation.saturating_add(1);
    }
}

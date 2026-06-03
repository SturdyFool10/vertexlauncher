use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, mpsc};

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
    pub(super) scan_results_tx: Option<mpsc::Sender<InstalledContentScanResult>>,
    pub(super) scan_results_rx: Option<Arc<Mutex<mpsc::Receiver<InstalledContentScanResult>>>>,
}

impl InstalledContentCache {
    pub(super) fn clear(&mut self) {
        self.managed_identities = None;
        self.files_by_tab.clear();
        self.scans_in_flight.clear();
        self.scan_generation = self.scan_generation.saturating_add(1);
    }
}

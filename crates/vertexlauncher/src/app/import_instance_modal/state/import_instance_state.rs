use super::*;

#[derive(Default)]
pub struct ImportInstanceState {
    pub source_mode_index: usize,
    pub package_path: PathBuf,
    pub launcher_path: PathBuf,
    pub launcher_kind_index: usize,
    pub instance_name: String,
    pub error: Option<String>,
    pub(crate) preview_in_flight: bool,
    pub(crate) preview_request_serial: u64,
    pub(crate) preview_results_tx: Option<mpsc::Sender<(u64, Result<ImportPreview, String>)>>,
    pub(crate) preview_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(u64, Result<ImportPreview, String>)>>>>,
    pub import_in_flight: bool,
    pub import_latest_progress: Option<ImportProgress>,
    pub import_progress_tx: Option<mpsc::Sender<ImportProgress>>,
    pub import_progress_rx: Option<Arc<Mutex<mpsc::Receiver<ImportProgress>>>>,
    pub import_results_tx: Option<mpsc::Sender<ImportTaskResult>>,
    pub import_results_rx: Option<Arc<Mutex<mpsc::Receiver<ImportTaskResult>>>>,
    pub(crate) preview: Option<ImportPreview>,
}

impl ImportInstanceState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

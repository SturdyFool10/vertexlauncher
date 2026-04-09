use super::*;

pub(crate) struct HomeActivityResultChannel {
    pub(crate) tx: mpsc::Sender<HomeActivityScanResult>,
    pub(crate) rx: mpsc::Receiver<HomeActivityScanResult>,
}

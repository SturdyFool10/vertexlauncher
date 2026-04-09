use super::*;

pub(super) struct ScreenshotResultChannel {
    pub(super) tx: mpsc::Sender<ScreenshotScanMessage>,
    pub(super) rx: mpsc::Receiver<ScreenshotScanMessage>,
}

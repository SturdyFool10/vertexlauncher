use super::*;

pub(super) struct ServerPingResultChannel {
    pub(super) tx: mpsc::Sender<ServerPingResult>,
    pub(super) rx: mpsc::Receiver<ServerPingResult>,
}

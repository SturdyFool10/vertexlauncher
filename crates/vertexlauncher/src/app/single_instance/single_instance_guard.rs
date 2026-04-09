use super::*;

pub struct SingleInstanceGuard {
    pub(super) endpoint: SocketAddrV4,
    pub(super) stop_requested: Arc<AtomicBool>,
    pub(super) completion_rx: Option<mpsc::Receiver<()>>,
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::SeqCst);
        let _ = send_probe(self.endpoint, HELLO_MESSAGE);
        if let Some(completion_rx) = self.completion_rx.take() {
            let _ = completion_rx.recv_timeout(PROBE_TIMEOUT);
        }
    }
}

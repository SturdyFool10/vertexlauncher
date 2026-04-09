use super::*;

pub(crate) struct AsyncRasterState {
    pub(crate) tx: Option<mpsc::Sender<AsyncRasterWorkerMessage>>,
    pub(crate) rx: Option<mpsc::Receiver<AsyncRasterResponse>>,
    pub(crate) pending: FxHashSet<u64>,
    pub(crate) cache: ThreadSafeLru<u64, AsyncRasterCacheEntry>,
}

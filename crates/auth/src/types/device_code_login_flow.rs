use std::sync::mpsc::{self, Receiver};

use crate::types::LoginEvent;

/// Handle for polling device-code login events from background runtime tasks.
#[derive(Debug)]
pub struct DeviceCodeLoginFlow {
    pub(crate) receiver: Receiver<LoginEvent>,
    pub(crate) finished: bool,
}

impl DeviceCodeLoginFlow {
    /// Drains currently available login events without blocking.
    ///
    /// Returns a vector of all events that have been queued since the last call to this method.
    /// The `finished` flag is set to true once a `Completed` or `Failed` event is received.
    pub fn poll_events(&mut self) -> Vec<LoginEvent> {
        let mut out = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(event) => {
                    if matches!(event, LoginEvent::Completed(_) | LoginEvent::Failed(_)) {
                        self.finished = true;
                    }
                    out.push(event);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finished = true;
                    break;
                }
            }
        }
        out
    }

    /// Returns `true` once the flow has completed or failed.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

//! Things background code or a notification button asks the front end to do, without knowing which UI toolkit is
//! running. The front end drains the queue once per frame.

use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppCommand {
    /// Show a screenshot in the screenshot viewer.
    ViewScreenshot(PathBuf),
    /// Put an image file on the clipboard.
    CopyImage(PathBuf),
    /// Put text on the clipboard.
    CopyText(String),
}

static QUEUE: Mutex<Vec<AppCommand>> = Mutex::new(Vec::new());

/// Queues `command` and wakes the UI so it is handled promptly.
pub fn push(command: AppCommand) {
    if let Ok(mut queue) = QUEUE.lock() {
        queue.push(command);
    }
}

/// Takes every queued command, oldest first.
pub fn drain() -> Vec<AppCommand> {
    QUEUE.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default()
}

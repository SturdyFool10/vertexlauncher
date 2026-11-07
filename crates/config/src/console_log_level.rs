use serde::{Deserialize, Serialize};

/// The least severe log line the console shows. Lines below it are still kept (the console remembers its recent history),
/// so lowering this brings back what was hidden.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleLogLevel {
    /// Errors only.
    Error,
    /// Warnings and errors.
    Warn,
    /// Normal operation, warnings and errors.
    Info,
    /// Adds diagnostic detail.
    Debug,
    /// Everything, including the engine's progress and timing lines.
    Trace,
}

impl ConsoleLogLevel {
    /// Every level, most severe first.
    pub const ALL: [ConsoleLogLevel; 5] = [
        ConsoleLogLevel::Error,
        ConsoleLogLevel::Warn,
        ConsoleLogLevel::Info,
        ConsoleLogLevel::Debug,
        ConsoleLogLevel::Trace,
    ];

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            ConsoleLogLevel::Error => "Error",
            ConsoleLogLevel::Warn => "Warning",
            ConsoleLogLevel::Info => "Info",
            ConsoleLogLevel::Debug => "Debug",
            ConsoleLogLevel::Trace => "Trace",
        }
    }
}

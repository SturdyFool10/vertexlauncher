#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

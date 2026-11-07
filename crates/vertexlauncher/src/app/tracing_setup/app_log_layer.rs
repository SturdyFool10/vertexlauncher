use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicU8, Ordering};

use tracing_subscriber::layer::{Context as LayerContext, Layer};

use crate::app::tracing_setup::{
    SharedLogWriter, current_date_time_parts, format_module_path, message_visitor::MessageVisitor,
    should_omit_module_path, write_log_line,
};

#[derive(Clone)]
pub(super) struct AppLogLayer {
    pub(super) writer: SharedLogWriter,
    /// Whether stdout is a terminal that should receive ANSI-colored lines.
    color_stdout: bool,
    /// Whether stderr is a terminal that should receive ANSI-colored lines.
    color_stderr: bool,
}

impl AppLogLayer {
    pub(super) fn new(writer: SharedLogWriter) -> Self {
        let allow_color = std::env::var_os("NO_COLOR").is_none();
        Self {
            writer,
            color_stdout: allow_color && std::io::stdout().is_terminal(),
            color_stderr: allow_color && std::io::stderr().is_terminal(),
        }
    }
}

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

fn level_color(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::ERROR => "\x1b[1;31m",
        tracing::Level::WARN => "\x1b[33m",
        tracing::Level::INFO => "\x1b[32m",
        tracing::Level::DEBUG => "\x1b[34m",
        tracing::Level::TRACE => "\x1b[35m",
    }
}

impl<S> Layer<S> for AppLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
        let meta = event.metadata();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let (date, time) = current_date_time_parts();
        let level = meta.level().as_str();
        let module_path = format_module_path(meta.target(), meta.file());
        let message = if visitor.message.is_empty() {
            visitor.fields
        } else if visitor.fields.is_empty() {
            visitor.message
        } else {
            format!("{} {}", visitor.message, visitor.fields)
        };
        let line = if should_omit_module_path(meta.target(), &module_path) {
            format!("[{date}][{time}][{level}]: {message}")
        } else {
            format!("[{date}][{time}][{level}][{module_path}]: {message}")
        };

        // Always emit; both writes are no-ops when the process has no console attached.
        // Errors go to stderr, everything less severe to stdout.
        let is_error = *meta.level() == tracing::Level::ERROR;
        let color = if is_error {
            self.color_stderr
        } else {
            self.color_stdout
        };
        let out = if color {
            let color = level_color(meta.level());
            let module = if should_omit_module_path(meta.target(), &module_path) {
                String::new()
            } else {
                format!("{DIM}[{module_path}]{RESET}")
            };
            format!("{DIM}[{date}][{time}]{RESET}{color}[{level}]{RESET}{module}: {message}")
        } else {
            line.clone()
        };
        if is_error {
            let _ = writeln!(std::io::stderr().lock(), "{out}");
        } else {
            let _ = writeln!(std::io::stdout().lock(), "{out}");
        }

        write_log_line(&self.writer, &line);
    }
}

/// Sends every event the console is set to remember into it. This is separate from [`AppLogLayer`] (which writes the log
/// file and the terminal at the `RUST_LOG` level) because the console keeps more than it shows: events down to the
/// capture level (see [`set_console_capture_level`]) are stored, and the console screen decides which to display, so a
/// line hidden at the current level is still there when the level is lowered.
#[derive(Clone, Copy, Default)]
pub(super) struct ConsoleLayer;

static CAPTURE_LEVEL: AtomicU8 = AtomicU8::new(level_rank(tracing::Level::DEBUG));

const fn level_rank(level: tracing::Level) -> u8 {
    match level {
        tracing::Level::ERROR => 0,
        tracing::Level::WARN => 1,
        tracing::Level::INFO => 2,
        tracing::Level::DEBUG => 3,
        tracing::Level::TRACE => 4,
    }
}

/// The most verbose level the console stores. Events more verbose than this are never formatted.
pub fn set_console_capture_level(level: tracing::Level) {
    CAPTURE_LEVEL.store(level_rank(level), Ordering::Relaxed);
    // A filter that looks at the event's static metadata has its answer cached per call site; ask again.
    tracing::callsite::rebuild_interest_cache();
}

/// Whether the console stores an event with this metadata.
pub(super) fn console_captures(meta: &tracing::Metadata<'_>) -> bool {
    level_rank(*meta.level()) <= CAPTURE_LEVEL.load(Ordering::Relaxed)
}

impl<S> Layer<S> for ConsoleLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
        let meta = event.metadata();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let (date, time) = current_date_time_parts();
        let level = meta.level().as_str();
        let module_path = format_module_path(meta.target(), meta.file());
        let message = if visitor.message.is_empty() {
            visitor.fields
        } else if visitor.fields.is_empty() {
            visitor.message
        } else {
            format!("{} {}", visitor.message, visitor.fields)
        };
        let line = if should_omit_module_path(meta.target(), &module_path) {
            format!("[{date}][{time}][{level}]: {message}")
        } else {
            format!("[{date}][{time}][{level}][{module_path}]: {message}")
        };
        launcher_ui::console::push_line(line);
    }
}

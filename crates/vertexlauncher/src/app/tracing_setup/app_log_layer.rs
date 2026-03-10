use std::{
    io,
    sync::{Arc, Mutex},
};

use tracing_subscriber::layer::{Context as LayerContext, Layer};

use crate::app::tracing_setup::{
    current_date_time_parts, format_module_path, message_visitor::MessageVisitor,
    should_omit_module_path,
};

#[derive(Clone)]
pub(super) struct AppLogLayer {
    pub(super) writer: Arc<Mutex<Box<dyn io::Write + Send>>>,
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
        } else {
            visitor.message
        };
        let line = if should_omit_module_path(meta.target(), &module_path) {
            format!("[{date}][{time}][{level}]: {message}")
        } else {
            format!("[{date}][{time}][{level}][{module_path}]: {message}")
        };

        launcher_ui::console::push_line(line.clone());
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writeln!(writer, "{line}");
        }
    }
}

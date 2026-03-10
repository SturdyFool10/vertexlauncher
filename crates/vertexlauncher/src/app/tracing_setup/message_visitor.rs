#[derive(Default)]
pub(super) struct MessageVisitor {
    pub(super) message: String,
    pub(super) fields: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message = rendered.trim_matches('"').to_owned();
            return;
        }
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        self.fields.push_str(field.name());
        self.fields.push('=');
        self.fields.push_str(rendered.trim_matches('"'));
    }
}

#[derive(Default)]
pub(super) struct MessageVisitor {
    pub(super) message: String,
    pub(super) fields: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        let sanitized = sanitize_field_value(field.name(), &rendered);
        if field.name() == "message" {
            self.message = sanitized.trim_matches('"').to_owned();
            return;
        }
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        self.fields.push_str(field.name());
        self.fields.push('=');
        self.fields.push_str(sanitized.trim_matches('"'));
    }
}

fn sanitize_field_value(field_name: &str, rendered: &str) -> String {
    let normalized = field_name.to_ascii_lowercase();
    let sensitive = [
        "token",
        "secret",
        "password",
        "cookie",
        "authorization",
        "verifier",
        "code",
        "session",
        "display_name",
        "player_name",
        "player_uuid",
        "xuid",
        "account_key",
        "profile_id",
    ];
    if sensitive.iter().any(|needle| normalized.contains(needle)) {
        "\"[redacted]\"".to_owned()
    } else {
        rendered.to_owned()
    }
}

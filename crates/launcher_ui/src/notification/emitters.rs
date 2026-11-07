use super::*;

use crate::{account_text, privacy};

/// Redacts (in streamer mode) and sanitizes one notification text field for logs and display.
///
/// Account mentions (see [`crate::account_text`]) pass through untouched: only the text around
/// them is sanitized, so the sanitizer never sees (or masks) an account id, and whitespace
/// next to a mention is kept.
pub(super) fn sanitize_field(streamer_mode: bool, value: String) -> String {
    account_text::map_text(&value, |part| {
        let body = part.trim();
        if body.is_empty() {
            return part.to_owned();
        }
        let leading = &part[..part.len() - part.trim_start().len()];
        let trailing = &part[part.trim_end().len()..];
        let sanitized =
            privacy::sanitize_text_for_log(&privacy::redact_sensitive_text(streamer_mode, body));
        format!("{leading}{sanitized}{trailing}")
    })
}

/// A message as plain text for the log: mentions resolved the way the user would see them.
fn log_text(message: &str, streamer_mode: bool) -> String {
    account_text::plain(message, streamer_mode).into_owned()
}

pub fn emit(severity: Severity, source: impl Into<String>, message: impl Into<String>) {
    let streamer_mode = streamer_mode_enabled();
    let source = sanitize_field(streamer_mode, source.into());
    let message = sanitize_field(streamer_mode, message.into());
    match severity {
        Severity::Log => {
            tracing::debug!(target: "notification", source = %source, message = %log_text(&message, streamer_mode))
        }
        Severity::Info => {
            tracing::info!(target: "notification", source = %source, message = %log_text(&message, streamer_mode))
        }
        Severity::Warn => {
            tracing::warn!(target: "notification", source = %source, message = %log_text(&message, streamer_mode))
        }
        Severity::Error => {
            tracing::error!(target: "notification", source = %source, message = %log_text(&message, streamer_mode))
        }
    }
    if let Err(err) = center().tx.send(Notification {
        severity,
        source,
        message,
        progress: None,
        spinner: false,
        replace_key: None,
        actions: Vec::new(),
        icon: None,
        stack: None,
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            error = %err,
            "Failed to enqueue notification."
        );
    }
}

/// Like [`emit`], but with buttons attached to the popup.
pub fn emit_with_actions(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    actions: Vec<NotificationAction>,
) {
    NotificationBuilder::new(severity, source, message)
        .actions(actions)
        .emit();
}

/// Like [`emit`], but shows `icon_svg` in place of the severity icon.
pub fn emit_with_icon(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    icon_svg: &'static [u8],
) {
    NotificationBuilder::new(severity, source, message)
        .icon(icon_svg)
        .emit();
}

/// Builds a notification with optional buttons, icon and stacking.
///
/// ```ignore
/// NotificationBuilder::new(Severity::Info, "skins", "")
///     .stack("new_capes:Steve", vec!["Migrator".into()], |capes| capes_message(capes))
///     .emit();
/// ```
pub struct NotificationBuilder {
    severity: Severity,
    source: String,
    message: String,
    actions: Vec<NotificationAction>,
    icon: Option<&'static [u8]>,
    stack: Option<StackSpec>,
    spinner: bool,
}

impl NotificationBuilder {
    pub fn new(severity: Severity, source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity,
            source: source.into(),
            message: message.into(),
            actions: Vec::new(),
            icon: None,
            stack: None,
            spinner: false,
        }
    }

    pub fn actions(mut self, actions: Vec<NotificationAction>) -> Self {
        self.actions = actions;
        self
    }

    pub fn icon(mut self, icon_svg: &'static [u8]) -> Self {
        self.icon = Some(icon_svg);
        self
    }

    /// Merges this notification into any showing one with the same `key`. `render` turns the
    /// combined `items` into the message, and the message given to [`Self::new`] is replaced by
    /// `render(&items)`. Buttons on a merged popup belong to the most recent arrival.
    pub fn stack(
        mut self,
        key: impl Into<String>,
        items: Vec<String>,
        render: impl Fn(&[String]) -> String + Send + Sync + 'static,
    ) -> Self {
        self.message = render(&items);
        self.stack = Some(StackSpec {
            key: key.into(),
            items,
            render: Arc::new(render),
            remove: false,
        });
        self
    }

    /// Shows a spinner beside the message, for work that is still in progress.
    pub fn spinner(mut self) -> Self {
        self.spinner = true;
        self
    }

    pub fn emit(self) {
        let streamer_mode = streamer_mode_enabled();
        let source = sanitize_field(streamer_mode, self.source);
        let message = sanitize_field(streamer_mode, self.message);
        tracing::info!(target: "notification", source = %source, message = %log_text(&message, streamer_mode));
        if let Err(err) = center().tx.send(Notification {
            severity: self.severity,
            source,
            message,
            progress: None,
            spinner: self.spinner,
            replace_key: None,
            actions: self.actions,
            icon: self.icon,
            stack: self.stack,
            when: Instant::now(),
        }) {
            tracing::error!(
                target: "notification",
                error = %err,
                "Failed to enqueue notification."
            );
        }
    }
}

/// Takes `items` back out of the showing notification stacked under `key`, re-rendering its
/// message, and dismissing it once no items are left. Does nothing if no such notification is
/// showing. Use it when part of what a stacked notification describes stops being true, such as
/// one of several "renewing..." spinners finishing.
pub fn retract_from_stack(key: impl Into<String>, items: Vec<String>) {
    if let Err(err) = center().tx.send(Notification {
        severity: Severity::Log,
        source: String::new(),
        message: String::new(),
        progress: None,
        spinner: false,
        replace_key: None,
        actions: Vec::new(),
        icon: None,
        stack: Some(StackSpec {
            key: key.into(),
            items,
            render: Arc::new(|_| String::new()),
            remove: true,
        }),
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            error = %err,
            "Failed to enqueue stack retraction."
        );
    }
}

pub fn emit_progress(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    progress: f32,
) {
    let streamer_mode = streamer_mode_enabled();
    let source = sanitize_field(streamer_mode, source.into());
    let message = sanitize_field(streamer_mode, message.into());
    let progress = progress.clamp(0.0, 1.0);
    tracing::info!(
        target: "notification",
        source = %source,
        message = %log_text(&message, streamer_mode),
        progress,
        "notification progress update"
    );
    if let Err(err) = center().tx.send(Notification {
        severity,
        source: source.clone(),
        message,
        progress: Some(progress),
        spinner: false,
        replace_key: Some(source.clone()),
        actions: Vec::new(),
        icon: None,
        stack: None,
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            source = %source,
            progress,
            error = %err,
            "Failed to enqueue progress notification."
        );
    }
}

pub fn emit_spinner(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    replace_key: impl Into<String>,
) {
    let streamer_mode = streamer_mode_enabled();
    let source = sanitize_field(streamer_mode, source.into());
    let message = sanitize_field(streamer_mode, message.into());
    let replace_key = sanitize_field(streamer_mode, replace_key.into());
    tracing::info!(
        target: "notification",
        source = %source,
        message = %log_text(&message, streamer_mode),
        replace_key = %replace_key,
        spinner = true,
        "notification spinner update"
    );
    if let Err(err) = center().tx.send(Notification {
        severity,
        source,
        message,
        progress: None,
        spinner: true,
        replace_key: Some(replace_key),
        actions: Vec::new(),
        icon: None,
        stack: None,
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            error = %err,
            "Failed to enqueue spinner notification."
        );
    }
}

pub fn emit_replace(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    replace_key: impl Into<String>,
) {
    let streamer_mode = streamer_mode_enabled();
    let source = sanitize_field(streamer_mode, source.into());
    let message = sanitize_field(streamer_mode, message.into());
    let replace_key = sanitize_field(streamer_mode, replace_key.into());
    tracing::info!(
        target: "notification",
        source = %source,
        message = %log_text(&message, streamer_mode),
        replace_key = %replace_key,
        "notification replacement update"
    );
    if let Err(err) = center().tx.send(Notification {
        severity,
        source,
        message,
        progress: None,
        spinner: false,
        replace_key: Some(replace_key),
        actions: Vec::new(),
        icon: None,
        stack: None,
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            error = %err,
            "Failed to enqueue replacement notification."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_text::{Segment, mention, segments};

    const ID: &str = "069a79f4-44e9-4726-a5be-fca90e38aaf5";

    #[test]
    fn sanitizing_keeps_mentions_and_the_spaces_around_them() {
        for streamer_mode in [false, true] {
            let message = format!("Renewing login token for {} now", mention(ID, "Steve"));
            let sanitized = sanitize_field(streamer_mode, message.clone());
            assert_eq!(sanitized, message, "streamer_mode={streamer_mode}");
        }
    }

    #[test]
    fn account_ids_in_mentions_are_not_masked_but_loose_ids_are() {
        let message = format!("{} had profile {ID}", mention(ID, "Steve"));
        let sanitized = sanitize_field(true, message);
        let parts = segments(&sanitized);
        assert!(
            parts.contains(&Segment::Account {
                id: ID,
                name: "Steve"
            }),
            "the mention keeps its id: {parts:?}"
        );
        let shown = account_text::plain(&sanitized, true).into_owned();
        assert!(!shown.contains(ID), "the loose id is masked: {shown}");
    }

    #[test]
    fn logs_never_contain_a_name_in_streamer_mode() {
        let message = format!("{} took a screenshot!", mention(ID, "Steve"));
        assert_eq!(
            log_text(&message, true),
            "Hidden Account took a screenshot!"
        );
        assert_eq!(log_text(&message, false), "Steve took a screenshot!");
    }

    #[test]
    fn plain_messages_sanitize_like_before() {
        assert_eq!(
            sanitize_field(false, "hello  world".to_owned()),
            "hello  world"
        );
    }
}

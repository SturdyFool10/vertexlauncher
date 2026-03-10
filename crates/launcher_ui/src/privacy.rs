use std::borrow::Cow;

pub const REDACTED_ACCOUNT_LABEL: &str = "Hidden Account";

pub fn redact_account_label<'a>(streamer_mode: bool, label: &'a str) -> Cow<'a, str> {
    if streamer_mode {
        Cow::Borrowed(REDACTED_ACCOUNT_LABEL)
    } else {
        Cow::Borrowed(label)
    }
}

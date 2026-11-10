use vertex_i18n::tr;
/// Formats a user-facing authentication error for a skin-profile mutation.
///
/// `operation` should be a short verb phrase such as `"upload skin"` or `"set cape"`.
/// The returned string is suitable for notifications and logs. This function does not panic.
pub(super) fn format_auth_error(operation: &str, err: &auth::AuthError) -> String {
    let message = err.to_string();
    if message.contains("HTTP status 401") {
        return tr!("Failed to {operation}: {message}. Minecraft auth token may be expired. Sign out and sign back in, then retry.", operation = operation, message = message).into_owned();
    }
    tr!("Failed to {operation}: {message}", operation = operation, message = message).into_owned()
}

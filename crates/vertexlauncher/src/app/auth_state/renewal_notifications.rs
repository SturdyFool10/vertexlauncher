use super::*;

use launcher_ui::account_text::mention;
use launcher_ui::notification::natural_language::join_natural;
use vertex_i18n::{tr, trn};
use launcher_ui::notification::{NotificationBuilder, Severity, retract_from_stack};

const RENEWING_KEY: &str = "login-renewing";
const RENEWED_KEY: &str = "login-renewed";
const RENEWAL_FAILED_KEY: &str = "login-renewal-failed";
/// Separates an account name from its error inside one stacked failure item.
const FAILURE_FIELD_SEPARATOR: char = '\u{1f}';
const NOTIFICATION_SOURCE: &str = "Login Renewal";
const UNKNOWN_ERROR: &str = vertex_i18n::src!("unknown error");

/// Shows renewal progress and results. Accounts renewing or finishing around the same time share
/// one notification each for "renewing", "renewed" and "failed", worded for however many there
/// are, instead of one popup per account.
pub(super) fn emit_cached_account_renewal_notification(event: CachedAccountRenewalEvent) {
    // Accounts are written as mentions, so the notification hides (and highlights) them whenever
    // streamer mode is on at the time it is drawn.
    match event {
        CachedAccountRenewalEvent::Started {
            profile_id,
            display_name,
        } => {
            let name = mention(&profile_id, &display_name);
            NotificationBuilder::new(Severity::Info, NOTIFICATION_SOURCE, String::new())
                .spinner()
                .stack(RENEWING_KEY, vec![name], renewing_message)
                .emit();
        }
        CachedAccountRenewalEvent::Succeeded {
            profile_id,
            display_name,
        } => {
            let name = mention(&profile_id, &display_name);
            retract_from_stack(RENEWING_KEY, vec![name.clone()]);
            NotificationBuilder::new(Severity::Info, NOTIFICATION_SOURCE, String::new())
                .stack(RENEWED_KEY, vec![name], renewed_message)
                .emit();
        }
        CachedAccountRenewalEvent::Failed {
            profile_id,
            display_name,
            error,
        } => {
            let name = mention(&profile_id, &display_name);
            tracing::warn!(
                target: "vertexlauncher/auth/renew",
                profile_fingerprint = %webview_sign_in::fingerprint_for_log(&profile_id),
                error = %webview_sign_in::sanitize_message_for_log(&error),
                "Auto-renew login notification reported an error."
            );
            retract_from_stack(RENEWING_KEY, vec![name.clone()]);
            NotificationBuilder::new(Severity::Error, NOTIFICATION_SOURCE, String::new())
                .stack(
                    RENEWAL_FAILED_KEY,
                    vec![format!("{name}{FAILURE_FIELD_SEPARATOR}{error}")],
                    failure_message,
                )
                .emit();
        }
    }
}

fn renewing_message(names: &[String]) -> String {
    trn!(
        names.len(),
        "Renewing login token for {accounts}",
        "Renewing login tokens for {accounts}",
        accounts = join_natural(names)
    )
}

fn renewed_message(names: &[String]) -> String {
    trn!(
        names.len(),
        "Login token for {accounts} renewed successfully, you are now ready to play!",
        "Login tokens for {accounts} renewed successfully, you are now ready to play!",
        accounts = join_natural(names)
    )
}

/// Each item is `name<separator>error`. One failure names its error; several that share an error
/// say it once, and several with different errors list each account with its own.
fn failure_message(items: &[String]) -> String {
    let failures: Vec<(&str, &str)> = items
        .iter()
        .map(|item| {
            item.split_once(FAILURE_FIELD_SEPARATOR)
                .unwrap_or((item.as_str(), UNKNOWN_ERROR))
        })
        .collect();
    let names: Vec<String> = failures
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    match failures.as_slice() {
        [] => String::new(),
        [(name, error)] => tr!("Error in attempting to renew login for {name}: {error}", name = name, error = vertex_i18n::t(error)).into_owned(),
        [(_, first_error), rest @ ..] if rest.iter().all(|(_, error)| error == first_error) => {
            tr!("Errors in attempting to renew login for {accounts}: {error}", accounts = join_natural(&names), error = vertex_i18n::t(first_error)).into_owned()
        }
        _ => {
            let described: Vec<String> = failures
                .iter()
                .map(|(name, error)| tr!("{name} ({error})", name = name, error = vertex_i18n::t(error)).into_owned())
                .collect();
            tr!("Errors in attempting to renew login for {accounts}", accounts = join_natural(&described)).into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn failure(name: &str, error: &str) -> String {
        format!("{name}{FAILURE_FIELD_SEPARATOR}{error}")
    }

    #[test]
    fn renewing_and_renewed_messages_follow_the_count() {
        assert_eq!(
            renewing_message(&names(&["Steve"])),
            "Renewing login token for Steve"
        );
        assert_eq!(
            renewing_message(&names(&["Steve", "Alex"])),
            "Renewing login tokens for Steve and Alex"
        );
        assert_eq!(
            renewed_message(&names(&["Steve", "Alex", "Zoe"])),
            "Login tokens for Steve, Alex, and Zoe renewed successfully, you are now ready to play!"
        );
    }

    #[test]
    fn mentioned_accounts_read_as_names_or_hide_in_streamer_mode() {
        use launcher_ui::account_text::{mention, plain};
        let steve = mention("id-steve", "Steve");
        let alex = mention("id-alex", "Alex");

        let renewing = renewing_message(&[steve.clone(), alex.clone()]);
        assert_eq!(
            plain(&renewing, false),
            "Renewing login tokens for Steve and Alex"
        );
        assert_eq!(
            plain(&renewing, true),
            "Renewing login tokens for Hidden Account and Hidden Account"
        );

        let failed = failure_message(&[failure(&steve, "offline")]);
        assert_eq!(
            plain(&failed, false),
            "Error in attempting to renew login for Steve: offline"
        );
        let shown = plain(&failed, true);
        assert!(!shown.contains("Steve"), "{shown}");
    }

    #[test]
    fn failure_messages_share_a_common_error_or_list_each() {
        assert_eq!(
            failure_message(&[failure("Steve", "offline")]),
            "Error in attempting to renew login for Steve: offline"
        );
        assert_eq!(
            failure_message(&[failure("Steve", "offline"), failure("Alex", "offline")]),
            "Errors in attempting to renew login for Steve and Alex: offline"
        );
        assert_eq!(
            failure_message(&[failure("Steve", "offline"), failure("Alex", "expired")]),
            "Errors in attempting to renew login for Steve (offline) and Alex (expired)"
        );
    }
}

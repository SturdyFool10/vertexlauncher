use super::*;

pub(super) enum RenewalResult {
    Bulk {
        result: Result<CachedAccountsState, String>,
        failed_account_errors: HashMap<String, String>,
        succeeded_profile_ids: HashSet<String>,
    },
    Single {
        profile_id: String,
        result: Result<CachedAccountsState, String>,
    },
}

use super::*;

#[path = "content_browser_state/active_content_download.rs"]
mod active_content_download;
#[path = "content_browser_state/browser_content_type.rs"]
mod browser_content_type;
#[path = "content_browser_state/browser_loader.rs"]
mod browser_loader;
#[path = "content_browser_state/browser_project_entry.rs"]
mod browser_project_entry;
#[path = "content_browser_state/browser_search_request.rs"]
mod browser_search_request;
#[path = "content_browser_state/browser_search_snapshot.rs"]
mod browser_search_snapshot;
#[path = "content_browser_state/browser_version_entry.rs"]
mod browser_version_entry;
#[path = "content_browser_state/bulk_content_update.rs"]
mod bulk_content_update;
#[path = "content_browser_state/content_browser_page.rs"]
mod content_browser_page;
#[path = "content_browser_state/content_browser_state.rs"]
mod content_browser_state;
#[path = "content_browser_state/content_detail_tab.rs"]
mod content_detail_tab;
#[path = "content_browser_state/content_download_outcome.rs"]
mod content_download_outcome;
#[path = "content_browser_state/content_install_request.rs"]
mod content_install_request;
#[path = "content_browser_state/content_scope.rs"]
mod content_scope;
#[path = "content_browser_state/deferred_content_cleanup.rs"]
mod deferred_content_cleanup;
#[path = "content_browser_state/detail_versions_result.rs"]
mod detail_versions_result;
#[path = "content_browser_state/mod_sort_mode.rs"]
mod mod_sort_mode;
#[path = "content_browser_state/queued_content_download.rs"]
mod queued_content_download;
#[path = "content_browser_state/search_update.rs"]
mod search_update;
#[path = "content_browser_state/version_row_action.rs"]
mod version_row_action;

pub(super) use self::active_content_download::ActiveContentDownload;
pub(super) use self::browser_content_type::BrowserContentType;
pub(super) use self::browser_loader::BrowserLoader;
pub(super) use self::browser_project_entry::BrowserProjectEntry;
pub(super) use self::browser_search_request::BrowserSearchRequest;
pub(super) use self::browser_search_snapshot::BrowserSearchSnapshot;
pub(super) use self::browser_version_entry::BrowserVersionEntry;
pub(crate) use self::bulk_content_update::BulkContentUpdate;
pub(super) use self::content_browser_page::ContentBrowserPage;
pub use self::content_browser_state::ContentBrowserState;
pub(super) use self::content_detail_tab::ContentDetailTab;
pub(super) use self::content_download_outcome::ContentDownloadOutcome;
pub(super) use self::content_install_request::ContentInstallRequest;
pub(super) use self::content_scope::ContentScope;
pub(super) use self::deferred_content_cleanup::DeferredContentCleanup;
pub(super) use self::detail_versions_result::DetailVersionsResult;
pub(super) use self::mod_sort_mode::ModSortMode;
pub(super) use self::queued_content_download::QueuedContentDownload;
pub(super) use self::search_update::SearchUpdate;
pub(super) use self::version_row_action::VersionRowAction;

pub(super) fn trim_content_browser_search_cache(state: &mut ContentBrowserState) {
    if state.search_cache.len() <= CONTENT_BROWSER_SEARCH_CACHE_MAX_ENTRIES {
        return;
    }
    let active_request = state.active_search_request.clone();
    state.search_cache.retain(|request, _| {
        active_request
            .as_ref()
            .is_some_and(|active| active == request)
            || request.page >= state.current_page.saturating_sub(2)
    });
    if state.search_cache.len() <= CONTENT_BROWSER_SEARCH_CACHE_MAX_ENTRIES {
        return;
    }
    let mut requests = state.search_cache.keys().cloned().collect::<Vec<_>>();
    requests.sort_by_key(|request| request.page);
    for request in requests {
        if state.search_cache.len() <= CONTENT_BROWSER_SEARCH_CACHE_MAX_ENTRIES {
            break;
        }
        if active_request.as_ref() != Some(&request) {
            state.search_cache.remove(&request);
        }
    }
}

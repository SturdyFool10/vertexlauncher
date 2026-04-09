use super::*;

#[derive(Clone, Debug)]
pub(crate) enum ContentInstallRequest {
    Latest {
        entry: BrowserProjectEntry,
        game_version: String,
        loader: BrowserLoader,
    },
    Exact {
        entry: BrowserProjectEntry,
        version: BrowserVersionEntry,
        game_version: String,
        loader: BrowserLoader,
    },
}

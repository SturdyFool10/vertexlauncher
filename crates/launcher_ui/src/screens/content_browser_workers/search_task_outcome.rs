use super::*;

#[derive(Default)]
pub(super) struct SearchTaskOutcome {
    pub(super) entries: Vec<ProviderSearchEntry>,
    pub(super) warnings: Vec<String>,
}

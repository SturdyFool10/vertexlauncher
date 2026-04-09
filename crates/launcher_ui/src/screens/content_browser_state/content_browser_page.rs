#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContentBrowserPage {
    #[default]
    Browse,
    Detail,
}

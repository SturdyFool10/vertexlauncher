#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VersionRowAction {
    Download,
    Installed,
    Switch,
}

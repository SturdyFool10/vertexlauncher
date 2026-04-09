#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeAction {
    None,
    LaunchRequested,
    StopRequested,
    DeleteRequested,
    OpenFolderRequested,
    CopyCommandRequested,
    CopySteamOptionsRequested,
    OpenInstanceRequested,
}

use super::*;

/// Actions emitted by the library screen for the app shell to process.
#[derive(Debug, Default, Clone)]
pub struct LibraryOutput {
    pub selected_instance_id: Option<String>,
    pub requested_screen: Option<AppScreen>,
}

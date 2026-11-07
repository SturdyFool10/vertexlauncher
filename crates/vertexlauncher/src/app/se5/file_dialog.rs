//! Native file/folder pickers that never block the UI thread.
//!
//! The engine opens the dialog and returns at once: call [`FileDialogs::open`] when the user clicks, then poll
//! [`FileDialogs::take`] every frame from the same place. While a dialog is open `take` keeps frames coming.

use std::collections::HashMap;
use std::path::PathBuf;

use vertex_shell::sturdy::desktop::{
    FileDialog, FileDialogKind, FileDialogResult, FileFilter, PendingFileDialog,
};

/// What the dialog picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    File,
    Files,
    Folder,
    /// A path to write; the file need not exist.
    SaveFile,
}

/// Dialog appearance and starting point.
#[derive(Clone, Debug, Default)]
pub struct Dialog {
    title: Option<String>,
    filters: Vec<(String, Vec<String>)>,
    file_name: Option<String>,
}

impl Dialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push((
            name.into(),
            extensions.iter().map(|ext| (*ext).to_owned()).collect(),
        ));
        self
    }

    pub fn file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    fn to_engine(&self, pick: Pick) -> FileDialog {
        let kind = match pick {
            Pick::File | Pick::Files => FileDialogKind::OpenFile,
            Pick::Folder => FileDialogKind::OpenFolder,
            Pick::SaveFile => FileDialogKind::SaveFile,
        };
        // The engine takes one location: a folder, or a file path to preselect.
        let default_location = self.file_name.clone().unwrap_or_default();
        FileDialog {
            kind,
            title: self.title.clone().unwrap_or_default(),
            default_location,
            filters: self
                .filters
                .iter()
                .map(|(name, extensions)| FileFilter {
                    name: name.clone(),
                    pattern: extensions.join(";"),
                })
                .collect(),
            allow_multiple: pick == Pick::Files,
            ..Default::default()
        }
    }
}

#[derive(Default)]
struct SlotState {
    pending: Option<PendingFileDialog>,
}

/// Named dialog slots. Keep one per UI.
#[derive(Default)]
pub struct FileDialogs {
    slots: HashMap<String, SlotState>,
}

impl FileDialogs {
    /// Opens a dialog for `slot`; does nothing (returns false) while that slot's dialog is already open or the system
    /// cannot show one.
    pub fn open(&mut self, slot: &str, pick: Pick, dialog: Dialog) -> bool {
        let state = self.slots.entry(slot.to_owned()).or_default();
        if state.pending.is_some() {
            return false;
        }
        match dialog.to_engine(pick).open() {
            Ok(pending) => {
                state.pending = Some(pending);
                true
            }
            Err(err) => {
                tracing::warn!(target: "vertexlauncher/file_dialog", error = %err, "Could not open a file dialog.");
                false
            }
        }
    }

    /// The finished dialog's result: `Some(paths)` once, then `None` until opened again. `Some(empty)` is a cancel.
    pub fn take(&mut self, slot: &str) -> Option<Vec<PathBuf>> {
        let state = self.slots.get_mut(slot)?;
        let result = state.pending.as_ref()?.poll();
        let Some(result) = result else {
            return None;
        };
        state.pending = None;
        Some(match result {
            FileDialogResult::Chosen { paths, .. } => paths,
            FileDialogResult::Cancelled => Vec::new(),
            FileDialogResult::Failed(err) => {
                tracing::warn!(target: "vertexlauncher/file_dialog", error = %err, "A file dialog failed.");
                Vec::new()
            }
        })
    }
}

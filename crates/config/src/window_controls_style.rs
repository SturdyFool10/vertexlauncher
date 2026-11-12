use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// Which look the title bar's minimize, maximize and restore buttons have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowControlsStyle {
    /// A dash, a browser-window outline and a pair of overlapping squares.
    Windows,
    /// A down arrow, an up arrow and a diamond.
    Linux,
}

impl WindowControlsStyle {
    pub const ALL: [WindowControlsStyle; 2] = [WindowControlsStyle::Windows, WindowControlsStyle::Linux];

    pub const fn label(self) -> &'static str {
        match self {
            WindowControlsStyle::Windows => src!("Windows"),
            WindowControlsStyle::Linux => src!("Linux"),
        }
    }
}

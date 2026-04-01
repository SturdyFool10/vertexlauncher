use egui::{Context, Id, Pos2};

use crate::{
    assets,
    ui::context_menu::{self, ContextMenuItem, ContextMenuRequest},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Actions that can be invoked from an instance context menu.
pub enum InstanceContextAction {
    OpenInstance,
    OpenFolder,
    CopyLaunchCommand,
    CopySteamLaunchOptions,
    Delete,
}

impl InstanceContextAction {
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::OpenInstance => "open_instance_screen",
            Self::OpenFolder => "open_instance_folder",
            Self::CopyLaunchCommand => "copy_instance_launch_command",
            Self::CopySteamLaunchOptions => "copy_instance_steam_launch_options",
            Self::Delete => "delete_instance",
        }
    }

    /// Converts a context-menu action id back into its strongly typed action.
    ///
    /// This is intentionally not `const` because matching on `&str` in a const
    /// context is not yet stable on the toolchain used by the project.
    pub fn from_action_id(action_id: &str) -> Option<Self> {
        match action_id {
            "open_instance_screen" => Some(Self::OpenInstance),
            "open_instance_folder" => Some(Self::OpenFolder),
            "copy_instance_launch_command" => Some(Self::CopyLaunchCommand),
            "copy_instance_steam_launch_options" => Some(Self::CopySteamLaunchOptions),
            "delete_instance" => Some(Self::Delete),
            _ => None,
        }
    }
}

pub fn items(include_delete: bool) -> Vec<ContextMenuItem> {
    let mut items = vec![
        ContextMenuItem::new_with_icon(
            InstanceContextAction::OpenInstance.action_id(),
            "Open instance menu",
            assets::LIBRARY_SVG,
        ),
        ContextMenuItem::new_with_icon(
            InstanceContextAction::OpenFolder.action_id(),
            "Open folder",
            assets::FOLDER_SVG,
        ),
        ContextMenuItem::new_with_icon(
            InstanceContextAction::CopyLaunchCommand.action_id(),
            "Copy command line",
            assets::TERMINAL_SVG,
        ),
        ContextMenuItem::new_with_icon(
            InstanceContextAction::CopySteamLaunchOptions.action_id(),
            "Copy Steam launch options",
            assets::STEAM_SVG,
        ),
    ];

    if include_delete {
        items.push(
            ContextMenuItem::new_with_icon(
                InstanceContextAction::Delete.action_id(),
                "Delete instance",
                assets::TRASH_X_SVG,
            )
            .danger(),
        );
    }

    items
}

pub fn request_for_instance(ctx: &Context, source_id: Id, anchor_pos: Pos2, include_delete: bool) {
    context_menu::request(
        ctx,
        ContextMenuRequest::new(source_id, anchor_pos, items(include_delete)),
    );
}

pub fn take(ctx: &Context, source_id: Id) -> Option<InstanceContextAction> {
    context_menu::take_invocation(ctx, source_id)
        .as_deref()
        .and_then(InstanceContextAction::from_action_id)
}

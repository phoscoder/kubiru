//! Actions and their default key bindings.
//!
//! `secondary` is Cmd on macOS and Ctrl elsewhere, so each binding follows
//! the platform's convention.

use gpui_kit::{App, KeyBinding, actions};

actions!(
    kubiru,
    [
        Quit,
        Refresh,
        FocusFilter,
        ToggleSidebar,
        OpenSettings,
        OpenClusters,
        AddCluster,
        GoOverview,
        GoPods,
        GoDeployments,
        GoServices,
        GoNodes,
        /// Open the object selected in a resource table.
        OpenSelected,
        /// Delete the object selected in a resource table.
        DeleteSelected,
        /// Copy the name of the object selected in a resource table.
        CopySelectedName,
        /// Leave the manifest view.
        Back,
        /// Apply the edited manifest.
        ApplyManifest,
    ]
);

/// Key context of the main window content.
pub const WORKSPACE: &str = "Workspace";
/// Key context of the manifest view.
pub const MANIFEST: &str = "Manifest";
/// Key context of a resource table (set by the table component).
const DATA_TABLE: &str = "DataTable";

pub fn bind(cx: &mut App) {
    let workspace = Some(WORKSPACE);
    cx.bind_keys([
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-r", Refresh, workspace),
        KeyBinding::new("f5", Refresh, workspace),
        KeyBinding::new("secondary-f", FocusFilter, workspace),
        KeyBinding::new("secondary-b", ToggleSidebar, workspace),
        KeyBinding::new("secondary-,", OpenSettings, workspace),
        KeyBinding::new("secondary-shift-k", OpenClusters, workspace),
        KeyBinding::new("secondary-n", AddCluster, workspace),
        KeyBinding::new("secondary-1", GoOverview, workspace),
        KeyBinding::new("secondary-2", GoPods, workspace),
        KeyBinding::new("secondary-3", GoDeployments, workspace),
        KeyBinding::new("secondary-4", GoServices, workspace),
        KeyBinding::new("secondary-5", GoNodes, workspace),
        KeyBinding::new("enter", OpenSelected, Some(DATA_TABLE)),
        KeyBinding::new("delete", DeleteSelected, Some(DATA_TABLE)),
        KeyBinding::new("secondary-c", CopySelectedName, Some(DATA_TABLE)),
        KeyBinding::new("escape", Back, Some(MANIFEST)),
        KeyBinding::new("secondary-s", ApplyManifest, Some(MANIFEST)),
    ]);
}

use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};
use std::path::PathBuf;

pub struct GoState {
    pub picker: FuzzyPicker,
    pub workspace_paths: Vec<PathBuf>, // parallel to all_items
}

impl GoState {
    pub fn new(workspaces: &[crate::core::workspace::Workspace]) -> Self {
        let items: Vec<PickerItem> = workspaces
            .iter()
            .map(|ws| PickerItem {
                name: ws.name.clone(),
                parent: "workspaces".to_string(),
                full_path: ws.path.clone(),
            })
            .collect();
        let workspace_paths = workspaces.iter().map(|ws| ws.path.clone()).collect();
        GoState {
            picker: FuzzyPicker::new(
                "Go to workspace  ENTER=go  ESC=cancel",
                items,
                false,
            ),
            workspace_paths,
        }
    }
}

impl std::fmt::Debug for GoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoState")
            .field("workspace_paths", &self.workspace_paths)
            .finish()
    }
}

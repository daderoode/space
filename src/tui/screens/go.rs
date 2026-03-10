use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};
use std::path::PathBuf;

pub struct GoState {
    pub picker: FuzzyPicker,
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
        GoState {
            picker: FuzzyPicker::new(
                "Go to workspace  ENTER=go  ESC=cancel",
                items,
                false,
            ),
        }
    }
}

impl std::fmt::Debug for GoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoState").finish()
    }
}

use crate::core::workspace::BranchStrategy;
use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum AddStage {
    PickRepos,
    PickBranchStrategy,
    Creating,
}

pub struct AddState {
    pub stage: AddStage,
    pub workspace_name: String,
    pub workspace_path: PathBuf,
    pub picker: FuzzyPicker,
    pub selected_repos: Vec<PathBuf>,
    pub branch_strategy_idx: usize,
    pub progress: Vec<String>,
    pub error: Option<String>,
}

impl AddState {
    pub fn new(ws_name: String, ws_path: PathBuf, available_repos: Vec<PathBuf>) -> Self {
        let items: Vec<PickerItem> = available_repos
            .into_iter()
            .map(PickerItem::from_path)
            .collect();
        let picker = FuzzyPicker::new(
            "Add repos  TAB=toggle  ENTER=confirm  ESC=cancel",
            items,
            true,
        );
        Self {
            stage: AddStage::PickRepos,
            workspace_name: ws_name,
            workspace_path: ws_path,
            picker,
            selected_repos: vec![],
            branch_strategy_idx: 0,
            progress: vec![],
            error: None,
        }
    }

    pub fn branch_strategy(&self) -> BranchStrategy {
        match self.branch_strategy_idx {
            1 => BranchStrategy::ExistingBranch(self.workspace_name.clone()),
            2 => BranchStrategy::DetachedHead,
            _ => BranchStrategy::NewBranch(self.workspace_name.clone()),
        }
    }
}

impl std::fmt::Debug for AddState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddState")
            .field("stage", &self.stage)
            .field("workspace_name", &self.workspace_name)
            .finish()
    }
}

use crate::core::workspace::BranchStrategy;
use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};
use std::path::PathBuf;
use tui_input::Input;

#[derive(Debug, Clone, PartialEq)]
pub enum CreateStage {
    PickRepos,
    NameWorkspace,
    PickBranchStrategy,
    PickBranch,
    Creating,
}

pub struct CreateState {
    pub stage: CreateStage,
    pub picker: FuzzyPicker,
    pub ws_name: Input,
    pub selected_repos: Vec<PathBuf>,
    pub branch_strategy_idx: usize, // 0=new branch, 1=existing, 2=detached, 3=pick branch
    pub branch_picker: Option<FuzzyPicker>, // populated when entering PickBranch stage
    pub picked_branch: Option<String>, // branch name chosen via branch_picker
    pub progress: Vec<String>,      // log lines shown during Creating stage
    pub error: Option<String>,
}

impl std::fmt::Debug for CreateState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateState")
            .field("stage", &self.stage)
            .field("ws_name", &self.ws_name.value())
            .field("selected_repos", &self.selected_repos)
            .field("branch_strategy_idx", &self.branch_strategy_idx)
            .field("picked_branch", &self.picked_branch)
            .field("progress", &self.progress)
            .field("error", &self.error)
            .finish()
    }
}

impl CreateState {
    pub fn new(all_repos: Vec<PathBuf>, initial_queries: Vec<String>) -> Self {
        let items: Vec<PickerItem> = all_repos.into_iter().map(PickerItem::from_path).collect();
        let mut picker = FuzzyPicker::new(
            "Select repos  TAB=toggle  ENTER=confirm  ESC=cancel",
            items,
            true,
        );
        // Pre-populate query if args were passed
        if !initial_queries.is_empty() {
            picker.input = picker.input.with_value(initial_queries.join(" "));
            picker.refilter();
        }
        Self {
            stage: CreateStage::PickRepos,
            picker,
            ws_name: Input::default(),
            selected_repos: vec![],
            branch_strategy_idx: 0,
            branch_picker: None,
            picked_branch: None,
            progress: vec![],
            error: None,
        }
    }

    pub fn branch_strategy(&self) -> BranchStrategy {
        match self.branch_strategy_idx {
            1 => BranchStrategy::ExistingBranch(self.ws_name.value().to_string()),
            2 => BranchStrategy::DetachedHead,
            3 => BranchStrategy::ExistingBranch(
                self.picked_branch
                    .clone()
                    .unwrap_or_else(|| self.ws_name.value().to_string()),
            ),
            _ => BranchStrategy::NewBranch(self.ws_name.value().to_string()),
        }
    }
}

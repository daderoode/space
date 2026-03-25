use crate::core::workspace::BranchStrategy;
use crate::tui::actions::{ScreenAction, ScreenContext, WorktreeParams};
use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum AddStage {
    PickRepos,
    PickBranchStrategy,
    PickBranch,
    Creating,
}

pub struct AddState {
    pub stage: AddStage,
    pub workspace_name: String,
    pub picker: FuzzyPicker,
    pub selected_repos: Vec<PathBuf>,
    pub branch_strategy_idx: usize,
    pub branch_picker: Option<FuzzyPicker>,
    pub picked_branch: Option<String>,
    pub progress: Vec<String>,
    pub error: Option<String>,
}

impl AddState {
    pub fn new(
        ws_name: String,
        available_repos: Vec<PathBuf>,
        initial_queries: Vec<String>,
    ) -> Self {
        let items: Vec<PickerItem> = available_repos
            .into_iter()
            .map(PickerItem::from_path)
            .collect();
        let mut picker = FuzzyPicker::new(
            "Add repos  TAB=toggle  ENTER=confirm  ESC=cancel",
            items,
            true,
        );
        // Pre-populate query if args were passed
        if !initial_queries.is_empty() {
            picker.input = picker.input.with_value(initial_queries.join(" "));
            picker.refilter();
        }
        Self {
            stage: AddStage::PickRepos,
            workspace_name: ws_name,
            picker,
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
            1 => BranchStrategy::ExistingBranch(self.workspace_name.clone()),
            2 => BranchStrategy::DetachedHead,
            3 => BranchStrategy::ExistingBranch(
                self.picked_branch
                    .clone()
                    .unwrap_or_else(|| self.workspace_name.clone()),
            ),
            _ => BranchStrategy::NewBranch(self.workspace_name.clone()),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        match self.stage {
            AddStage::PickRepos => self.handle_pick_repos(key),
            AddStage::PickBranchStrategy => self.handle_branch_strategy(key, ctx),
            AddStage::PickBranch => self.handle_pick_branch(key, ctx),
            AddStage::Creating => self.handle_creating(key),
        }
    }

    fn handle_pick_repos(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Esc => ScreenAction::Back,
            KeyCode::Enter => {
                let confirmed: Vec<PathBuf> = self
                    .picker
                    .confirmed_items()
                    .into_iter()
                    .map(|i| i.full_path.clone())
                    .collect();
                if confirmed.is_empty() {
                    self.error = Some("Select at least one repo".to_string());
                    return ScreenAction::Continue;
                }
                self.selected_repos = confirmed;
                self.error = None;
                self.stage = AddStage::PickBranchStrategy;
                ScreenAction::Continue
            }
            KeyCode::Tab => {
                self.picker.toggle_highlighted();
                ScreenAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.picker.move_up();
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.picker.move_down();
                ScreenAction::Continue
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.picker.cycle_scope();
                ScreenAction::Continue
            }
            _ => {
                if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                    self.picker.input.handle(req);
                }
                self.picker.refilter();
                ScreenAction::Continue
            }
        }
    }

    fn handle_branch_strategy(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Esc => {
                self.error = None;
                self.stage = AddStage::PickRepos;
                ScreenAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.error = None;
                if self.branch_strategy_idx > 0 {
                    self.branch_strategy_idx -= 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.error = None;
                if self.branch_strategy_idx < 3 {
                    self.branch_strategy_idx += 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                if self.branch_strategy_idx == 3 {
                    // Build branch picker from the first selected repo
                    let repo_path = self.selected_repos.first().cloned();
                    if let Some(repo_path) = repo_path {
                        let repo_name = repo_path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        match crate::tui::app::build_branch_picker(&repo_path, &repo_name) {
                            Some(picker) => {
                                self.picked_branch = None;
                                self.error = None;
                                self.branch_picker = Some(picker);
                                self.stage = AddStage::PickBranch;
                            }
                            None => {
                                self.error =
                                    Some(format!("Could not list branches for {}", repo_name));
                            }
                        }
                    }
                    ScreenAction::Continue
                } else {
                    self.stage = AddStage::Creating;
                    ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                        workspace_name: self.workspace_name.clone(),
                        workspace_dir: ctx.config.workspaces.dir.clone(),
                        repos: self.selected_repos.clone(),
                        branch_strategy: self.branch_strategy(),
                        is_new: false,
                    })
                }
            }
            _ => ScreenAction::Continue,
        }
    }

    fn handle_pick_branch(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Esc => {
                self.stage = AddStage::PickBranchStrategy;
                ScreenAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(ref mut bp) = self.branch_picker {
                    bp.move_up();
                }
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(ref mut bp) = self.branch_picker {
                    bp.move_down();
                }
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                let picked = self
                    .branch_picker
                    .as_ref()
                    .and_then(|bp| bp.confirmed_items().into_iter().next())
                    .map(|item| item.name.clone());
                let Some(branch) = picked else {
                    self.error = Some("Select a branch".to_string());
                    return ScreenAction::Continue;
                };
                self.error = None;
                self.picked_branch = Some(branch);
                self.stage = AddStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.workspace_name.clone(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: self.branch_strategy(),
                    is_new: false,
                })
            }
            _ => {
                if let Some(ref mut bp) = self.branch_picker {
                    if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                        bp.input.handle(req);
                    }
                    bp.refilter();
                }
                ScreenAction::Continue
            }
        }
    }

    fn handle_creating(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                let error_msg = self.error.clone();
                if let Some(err) = error_msg {
                    ScreenAction::BackWithStatus(format!("Add failed: {}", err))
                } else {
                    ScreenAction::Back
                }
            }
            _ => ScreenAction::Continue,
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

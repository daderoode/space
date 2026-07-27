use crate::core::git::BranchInfo;
use crate::tui::actions::{ScreenAction, ScreenContext};
use crate::tui::widgets::fuzzy_picker::FuzzyPicker;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;
use tui_input::Input;

#[derive(Debug, Clone, PartialEq)]
pub enum SwitchBranchStage {
    PickStrategy,
    EnterBranchName,
    PickBranch,
}

pub struct SwitchBranchState {
    pub stage: SwitchBranchStage,
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub strategy_idx: usize,
    pub branch_name_input: Input,
    pub branch_picker: Option<FuzzyPicker>,
    pub recent_branches: Vec<BranchInfo>,
    pub error: Option<String>,
}

impl std::fmt::Debug for SwitchBranchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwitchBranchState")
            .field("stage", &self.stage)
            .field("repo_name", &self.repo_name)
            .field("strategy_idx", &self.strategy_idx)
            .finish()
    }
}

impl SwitchBranchState {
    pub fn new(repo_name: String, repo_path: PathBuf) -> Self {
        let recent_branches = crate::core::git::recent_branches(&repo_path, 5);
        Self {
            stage: SwitchBranchStage::PickStrategy,
            repo_name,
            repo_path,
            strategy_idx: 0,
            branch_name_input: Input::default(),
            branch_picker: None,
            recent_branches,
            error: None,
        }
    }

    /// Maximum selectable index:
    /// - 0: New branch
    /// - 1..=n: Recent branches (n = recent_branches.len())
    /// - n+1: Show more / Pick a branch
    pub fn max_idx(&self) -> usize {
        1 + self.recent_branches.len()
    }

    pub fn handle_key(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        match self.stage {
            SwitchBranchStage::PickStrategy => self.handle_pick_strategy(key, ctx),
            SwitchBranchStage::EnterBranchName => self.handle_enter_branch_name(key, ctx),
            SwitchBranchStage::PickBranch => self.handle_pick_branch(key, ctx),
        }
    }

    fn handle_pick_strategy(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        let max = self.max_idx();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ScreenAction::Back,
            KeyCode::Up | KeyCode::Char('k') => {
                self.error = None;
                if self.strategy_idx > 0 {
                    self.strategy_idx -= 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.error = None;
                if self.strategy_idx < max {
                    self.strategy_idx += 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                if self.strategy_idx == 0 {
                    self.error = None;
                    self.stage = SwitchBranchStage::EnterBranchName;
                    ScreenAction::Continue
                } else if self.strategy_idx == max {
                    match crate::tui::app::build_branch_picker(
                        &self.repo_path,
                        &self.repo_name,
                        "Branch",
                    ) {
                        Some(picker) => {
                            self.error = None;
                            self.branch_picker = Some(picker);
                            self.stage = SwitchBranchStage::PickBranch;
                        }
                        None => {
                            self.error =
                                Some(format!("Could not list branches for {}", self.repo_name));
                        }
                    }
                    ScreenAction::Continue
                } else {
                    let branch = self.recent_branches[self.strategy_idx - 1].name.clone();
                    ScreenAction::SwitchRepoBranch {
                        repo_path: self.repo_path.clone(),
                        branch,
                        new_branch: false,
                    }
                }
            }
            _ => ScreenAction::Continue,
        }
    }

    fn handle_enter_branch_name(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Char('q') => ScreenAction::Back,
            KeyCode::Esc => {
                self.error = None;
                self.stage = SwitchBranchStage::PickStrategy;
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                let name = self.branch_name_input.value().trim().to_string();
                if name.is_empty() {
                    self.error = Some("Branch name cannot be empty".to_string());
                    return ScreenAction::Continue;
                }
                self.error = None;
                ScreenAction::SwitchRepoBranch {
                    repo_path: self.repo_path.clone(),
                    branch: name,
                    new_branch: true,
                }
            }
            _ => {
                if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                    self.branch_name_input.handle(req);
                }
                self.error = None;
                ScreenAction::Continue
            }
        }
    }

    fn handle_pick_branch(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Char('q') => ScreenAction::Back,
            KeyCode::Esc => {
                self.error = None;
                self.stage = SwitchBranchStage::PickStrategy;
                ScreenAction::Continue
            }
            KeyCode::Up => {
                if let Some(ref mut bp) = self.branch_picker {
                    bp.move_up();
                }
                ScreenAction::Continue
            }
            KeyCode::Char('k')
                if self
                    .branch_picker
                    .as_ref()
                    .is_some_and(|bp| bp.input.value().is_empty()) =>
            {
                if let Some(ref mut bp) = self.branch_picker {
                    bp.move_up();
                }
                ScreenAction::Continue
            }
            KeyCode::Down => {
                if let Some(ref mut bp) = self.branch_picker {
                    bp.move_down();
                }
                ScreenAction::Continue
            }
            KeyCode::Char('j')
                if self
                    .branch_picker
                    .as_ref()
                    .is_some_and(|bp| bp.input.value().is_empty()) =>
            {
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
                match picked {
                    None => ScreenAction::Continue,
                    Some(branch) => {
                        self.error = None;
                        ScreenAction::SwitchRepoBranch {
                            repo_path: self.repo_path.clone(),
                            branch,
                            new_branch: false,
                        }
                    }
                }
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
}

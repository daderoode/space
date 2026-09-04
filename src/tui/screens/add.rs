use crate::core::workspace::BranchStrategy;
use crate::tui::actions::{ScreenAction, ScreenContext, WorktreeParams};
use crate::tui::screens::sync_report::{LogView, SyncReport};
use crate::tui::widgets::fuzzy_picker::FuzzyPicker;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use tui_input::Input;

#[derive(Debug, Clone, PartialEq)]
pub enum AddStage {
    PickRepos,
    Syncing,
    PickBranchStrategy,
    EnterBranchName, // edit the new-branch name
    PickBranch,
    Creating,
}

pub struct AddState {
    pub stage: AddStage,
    pub workspace_name: String,
    pub picker: FuzzyPicker,
    pub branch_name_input: Input,
    pub selected_repos: Vec<PathBuf>,
    pub branch_strategy_idx: usize,
    pub branch_picker: Option<FuzzyPicker>,
    pub picked_branch: Option<String>,
    pub recent_branches: Vec<crate::core::git::BranchInfo>,
    pub progress: Vec<String>,
    pub report: SyncReport, // per-repo sync outcomes shown during Syncing stage
    pub log_view: LogView,  // scroll state of the Creating log
    pub error: Option<String>,
}

impl AddState {
    pub fn new(
        ws_name: String,
        available_repos: Vec<PathBuf>,
        initial_queries: Vec<String>,
    ) -> Self {
        let items = super::repo_items(available_repos);
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
            branch_name_input: Input::default(),
            selected_repos: vec![],
            branch_strategy_idx: 0,
            branch_picker: None,
            picked_branch: None,
            recent_branches: vec![],
            progress: vec![],
            report: SyncReport::empty(),
            log_view: LogView::new(),
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
            // idx 0 — New Branch; name comes from the EnterBranchName stage input.
            // Fall back to workspace_name if branch_name_input is empty (direct callers
            // before the stage gate has run).
            _ => {
                let name = self.branch_name_input.value().trim().to_string();
                BranchStrategy::NewBranch(if name.is_empty() {
                    self.workspace_name.clone()
                } else {
                    name
                })
            }
        }
    }

    /// Rebuild the repo picker from a rescanned repo list (already minus the
    /// repos in the space), keeping the user's place (see
    /// `FuzzyPicker::replace_items`). Returns how many toggled repos are no
    /// longer in the list.
    pub fn replace_repo_list(&mut self, repos: Vec<PathBuf>) -> usize {
        self.picker.replace_items(super::repo_items(repos))
    }

    pub fn handle_key(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        if super::opens_help(
            key.code,
            matches!(
                self.stage,
                AddStage::Syncing | AddStage::PickBranchStrategy | AddStage::Creating
            ),
        ) {
            return ScreenAction::OpenHelp;
        }
        match self.stage {
            AddStage::PickRepos => self.handle_pick_repos(key),
            AddStage::Syncing => self.handle_syncing(key),
            AddStage::PickBranchStrategy => self.handle_branch_strategy(key, ctx),
            AddStage::EnterBranchName => self.handle_enter_branch_name(key, ctx),
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
                self.progress.clear();
                self.report = SyncReport::new(&self.selected_repos);
                self.stage = AddStage::Syncing;
                ScreenAction::ExecuteSyncFlow(self.selected_repos.clone())
            }
            KeyCode::Tab => {
                self.picker.toggle_highlighted();
                ScreenAction::Continue
            }
            KeyCode::Up => {
                self.picker.move_up();
                ScreenAction::Continue
            }
            KeyCode::Down => {
                self.picker.move_down();
                ScreenAction::Continue
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.picker.cycle_scope();
                ScreenAction::Continue
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                ScreenAction::RescanRepoList
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

    /// The sync report; mirrors `CreateState::handle_syncing`.
    fn handle_syncing(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Esc => {
                self.progress.clear();
                self.stage = AddStage::PickRepos;
                ScreenAction::Continue
            }
            KeyCode::Enter if self.report.done => ScreenAction::ContinueFromSyncReport,
            _ => {
                self.report.handle_key(key);
                ScreenAction::Continue
            }
        }
    }

    fn handle_branch_strategy(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        let n = self.recent_branches.len();
        let max_idx = 3 + n;

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
                if self.branch_strategy_idx < max_idx {
                    self.branch_strategy_idx += 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                if self.branch_strategy_idx == max_idx {
                    // "Show more..." / "Pick a branch..." — open fuzzy picker
                    let repo_path = self.selected_repos.first().cloned();
                    if let Some(repo_path) = repo_path {
                        let repo_name = repo_path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        match crate::tui::app::build_branch_picker(&repo_path, &repo_name, "Branch")
                        {
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
                } else if self.branch_strategy_idx >= 3 && n > 0 {
                    // Selected a recent branch directly
                    let branch_name = self.recent_branches[self.branch_strategy_idx - 3]
                        .name
                        .clone();
                    self.stage = AddStage::Creating;
                    ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                        workspace_name: self.workspace_name.clone(),
                        workspace_dir: ctx.config.workspaces.dir.clone(),
                        repos: self.selected_repos.clone(),
                        branch_strategy: BranchStrategy::ExistingBranch(branch_name),
                        is_new: false,
                    })
                } else if self.branch_strategy_idx == 0 {
                    // New branch — open branch name editing stage.
                    // Only pre-fill when the field is empty; preserve whatever the
                    // user typed if they Esc'd back and re-selected "New branch".
                    if self.branch_name_input.value().is_empty() {
                        self.branch_name_input =
                            Input::default().with_value(self.workspace_name.clone());
                    }
                    self.error = None;
                    self.stage = AddStage::EnterBranchName;
                    ScreenAction::Continue
                } else {
                    // idx 1 (ExistingBranch) or idx 2 (DetachedHead)
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

    fn handle_enter_branch_name(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Esc => {
                self.error = None;
                self.stage = AddStage::PickBranchStrategy;
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                let name = self.branch_name_input.value().trim().to_string();
                if name.is_empty() {
                    self.error = Some("Branch name cannot be empty".to_string());
                    return ScreenAction::Continue;
                }
                self.error = None;
                self.stage = AddStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.workspace_name.clone(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: BranchStrategy::NewBranch(name),
                    is_new: false,
                })
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

    fn handle_pick_branch(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Esc => {
                self.stage = AddStage::PickBranchStrategy;
                ScreenAction::Continue
            }
            KeyCode::Up => {
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
                self.picked_branch = Some(branch.clone());
                self.stage = AddStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.workspace_name.clone(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: BranchStrategy::ExistingBranch(branch),
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
                    ScreenAction::BackWithStatus(
                        format!("Add failed: {}", err),
                        crate::tui::actions::StatusKind::Error,
                    )
                } else {
                    ScreenAction::Back
                }
            }
            _ => {
                self.log_view.handle_key(key, self.progress.len());
                ScreenAction::Continue
            }
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

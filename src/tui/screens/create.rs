use crate::core::workspace::BranchStrategy;
use crate::tui::screens::sync_report::{LogView, SyncReport};
use crate::tui::widgets::fuzzy_picker::FuzzyPicker;
use std::path::PathBuf;
use tui_input::Input;

#[derive(Debug, Clone, PartialEq)]
pub enum CreateStage {
    EnterName,
    PickRepos,
    Syncing, // running git fetch + fast-forward before showing the branch picker
    PickBranchStrategy,
    EnterBranchName, // edit the new-branch name
    PickBranch,
    Creating,
}

pub struct CreateState {
    pub stage: CreateStage,
    pub picker: FuzzyPicker,
    pub ws_name: Input,
    pub branch_name_input: Input,
    pub selected_repos: Vec<PathBuf>,
    pub branch_strategy_idx: usize, // 0=new branch, 1=existing, 2=detached, 3=pick branch
    pub branch_picker: Option<FuzzyPicker>, // populated when entering PickBranch stage
    pub picked_branch: Option<String>, // branch name chosen via branch_picker
    pub recent_branches: Vec<crate::core::git::BranchInfo>,
    pub progress: Vec<String>, // log lines shown during Creating stage
    pub report: SyncReport,    // per-repo sync outcomes shown during Syncing stage
    pub log_view: LogView,     // scroll state of the Creating log
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
        let items = super::repo_items(all_repos);
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
            stage: CreateStage::EnterName,
            picker,
            ws_name: Input::default(),
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

    /// Rebuild the repo picker from a rescanned repo list, keeping the user's
    /// place (see `FuzzyPicker::replace_items`). Returns how many toggled repos
    /// are no longer in the list.
    pub fn replace_repo_list(&mut self, repos: Vec<PathBuf>) -> usize {
        self.picker.replace_items(super::repo_items(repos))
    }

    pub fn handle_key(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        ctx: &crate::tui::actions::ScreenContext,
    ) -> crate::tui::actions::ScreenAction {
        match self.stage {
            CreateStage::EnterName => self.handle_enter_name(key),
            CreateStage::PickRepos => self.handle_pick_repos(key),
            CreateStage::Syncing => self.handle_syncing(key),
            CreateStage::PickBranchStrategy => self.handle_branch_strategy(key, ctx),
            CreateStage::EnterBranchName => self.handle_enter_branch_name(key, ctx),
            CreateStage::PickBranch => self.handle_pick_branch(key, ctx),
            CreateStage::Creating => self.handle_creating(key),
        }
    }

    fn handle_pick_repos(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
    ) -> crate::tui::actions::ScreenAction {
        use crate::tui::actions::ScreenAction;
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        match key.code {
            KeyCode::Esc => {
                self.stage = CreateStage::EnterName;
                ScreenAction::Continue
            }
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
                self.stage = CreateStage::Syncing;
                ScreenAction::ExecuteSyncFlow(self.selected_repos.clone())
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

    /// The sync report. Esc always returns to PickRepos with the picker's
    /// selection and query intact (running: cancels the worker at its next
    /// boundary). Enter continues only once the run is done; the cursor keys
    /// are handled by the report, which ignores them until then.
    fn handle_syncing(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
    ) -> crate::tui::actions::ScreenAction {
        use crate::tui::actions::ScreenAction;
        use ratatui::crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => {
                self.progress.clear();
                self.stage = CreateStage::PickRepos;
                ScreenAction::Continue
            }
            KeyCode::Enter if self.report.done => ScreenAction::ContinueFromSyncReport,
            _ => {
                self.report.handle_key(key);
                ScreenAction::Continue
            }
        }
    }

    fn handle_enter_name(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
    ) -> crate::tui::actions::ScreenAction {
        use crate::tui::actions::ScreenAction;
        use ratatui::crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc => ScreenAction::Back,
            KeyCode::Enter => {
                let name = self.ws_name.value().trim().to_string();
                if name.is_empty() {
                    self.error = Some("Workspace name cannot be empty".to_string());
                    return ScreenAction::Continue;
                }
                // Normalize: write trimmed value back so all downstream uses
                // (WorktreeParams, branch_strategy) get the clean name.
                self.ws_name = self.ws_name.clone().with_value(name);
                self.error = None;
                self.stage = CreateStage::PickRepos;
                ScreenAction::Continue
            }
            _ => {
                if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                    self.ws_name.handle(req);
                }
                self.error = None;
                ScreenAction::Continue
            }
        }
    }

    fn handle_branch_strategy(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        ctx: &crate::tui::actions::ScreenContext,
    ) -> crate::tui::actions::ScreenAction {
        use crate::tui::actions::{ScreenAction, WorktreeParams};
        use ratatui::crossterm::event::KeyCode;

        let n = self.recent_branches.len();
        let max_idx = 3 + n;

        match key.code {
            KeyCode::Esc => {
                self.error = None;
                self.stage = CreateStage::PickRepos;
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
                                self.stage = CreateStage::PickBranch;
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
                    self.stage = CreateStage::Creating;
                    ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                        workspace_name: self.ws_name.value().to_string(),
                        workspace_dir: ctx.config.workspaces.dir.clone(),
                        repos: self.selected_repos.clone(),
                        branch_strategy: BranchStrategy::ExistingBranch(branch_name),
                        is_new: true,
                    })
                } else if self.branch_strategy_idx == 0 {
                    // New branch — open branch name editing stage.
                    // Only pre-fill when the field is empty; preserve whatever the
                    // user typed if they Esc'd back and re-selected "New branch".
                    if self.branch_name_input.value().is_empty() {
                        let ws_name = self.ws_name.value().to_string();
                        self.branch_name_input = Input::default().with_value(ws_name);
                    }
                    self.error = None;
                    self.stage = CreateStage::EnterBranchName;
                    ScreenAction::Continue
                } else {
                    // idx 1 (ExistingBranch) or idx 2 (DetachedHead)
                    self.stage = CreateStage::Creating;
                    ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                        workspace_name: self.ws_name.value().to_string(),
                        workspace_dir: ctx.config.workspaces.dir.clone(),
                        repos: self.selected_repos.clone(),
                        branch_strategy: self.branch_strategy(),
                        is_new: true,
                    })
                }
            }
            _ => ScreenAction::Continue,
        }
    }

    fn handle_enter_branch_name(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        ctx: &crate::tui::actions::ScreenContext,
    ) -> crate::tui::actions::ScreenAction {
        use crate::tui::actions::{ScreenAction, WorktreeParams};
        use ratatui::crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc => {
                self.error = None;
                self.stage = CreateStage::PickBranchStrategy;
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                let name = self.branch_name_input.value().trim().to_string();
                if name.is_empty() {
                    self.error = Some("Branch name cannot be empty".to_string());
                    return ScreenAction::Continue;
                }
                self.error = None;
                self.stage = CreateStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.ws_name.value().to_string(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: BranchStrategy::NewBranch(name),
                    is_new: true,
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

    fn handle_pick_branch(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        ctx: &crate::tui::actions::ScreenContext,
    ) -> crate::tui::actions::ScreenAction {
        use crate::tui::actions::{ScreenAction, WorktreeParams};
        use ratatui::crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc => {
                self.stage = CreateStage::PickBranchStrategy;
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
                self.picked_branch = Some(branch.clone());
                self.stage = CreateStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.ws_name.value().to_string(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: BranchStrategy::ExistingBranch(branch),
                    is_new: true,
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

    fn handle_creating(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
    ) -> crate::tui::actions::ScreenAction {
        use crate::tui::actions::ScreenAction;
        use ratatui::crossterm::event::KeyCode;

        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                let error_msg = self.error.clone();
                if let Some(err) = error_msg {
                    ScreenAction::BackWithStatus(
                        format!("Create failed: {}", err),
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

    pub fn branch_strategy(&self) -> BranchStrategy {
        match self.branch_strategy_idx {
            1 => BranchStrategy::ExistingBranch(self.ws_name.value().to_string()),
            2 => BranchStrategy::DetachedHead,
            3 => BranchStrategy::ExistingBranch(
                self.picked_branch
                    .clone()
                    .unwrap_or_else(|| self.ws_name.value().to_string()),
            ),
            // idx 0 — New Branch; name comes from the EnterBranchName stage input.
            // Fall back to ws_name if branch_name_input is empty (direct callers,
            // e.g. tests or future MCP tools, before the stage gate has run).
            _ => {
                let name = self.branch_name_input.value().trim().to_string();
                BranchStrategy::NewBranch(if name.is_empty() {
                    self.ws_name.value().to_string()
                } else {
                    name
                })
            }
        }
    }
}

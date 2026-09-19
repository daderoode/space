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
    /// The space name the branch name stage was last opened with. A field
    /// that still reads it, or nothing, when the stage is opened again is not
    /// the user's and follows a renamed space.
    branch_name_default: String,
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
            .field("branch_name_input", &self.branch_name_input.value())
            .field("branch_name_default", &self.branch_name_default)
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
            branch_name_default: String::new(),
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
        if super::opens_help(
            key.code,
            matches!(
                self.stage,
                CreateStage::Syncing | CreateStage::PickBranchStrategy | CreateStage::Creating
            ),
        ) {
            return crate::tui::actions::ScreenAction::OpenHelp;
        }
        match self.stage {
            CreateStage::EnterName => self.handle_enter_name(key),
            CreateStage::PickRepos => self.handle_pick_repos(key),
            CreateStage::Syncing => self.handle_syncing(key),
            CreateStage::PickBranchStrategy => self.handle_branch_strategy(key, ctx),
            CreateStage::EnterBranchName => self.handle_enter_branch_name(key, ctx),
            CreateStage::PickBranch => self.handle_pick_branch(key, ctx),
            CreateStage::Creating => self.handle_creating(key, ctx),
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
                // The creation rule (`validate_space_name`): the name becomes
                // a directory under `workspaces.dir` and the default branch.
                // The field keeps what was typed so the user can fix it.
                if let Err(e) = crate::core::workspace::validate_space_name(&name) {
                    self.error = Some(e.to_string());
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
                        fresh_repos: self.report.fetched_ok_paths(),
                        slow_fetch_repos: self.report.slow_fetch_paths(),
                    })
                } else if self.branch_strategy_idx == 0 {
                    // New branch: open the branch name stage. A field left
                    // reading the space name it was opened with, or nothing,
                    // is not the user's and follows the space name, so going
                    // back to rename the space renames the branch too;
                    // anything else they typed is kept. It is read trimmed,
                    // as Enter reads it, and replaced only when the name
                    // changes, so the cursor stays where it was left.
                    let ws_name = self.ws_name.value().to_string();
                    let field = self.branch_name_input.value().trim();
                    let follows = field.is_empty() || field == self.branch_name_default;
                    if follows && field != ws_name {
                        self.branch_name_input = Input::default().with_value(ws_name.clone());
                    }
                    self.branch_name_default = ws_name;
                    self.error = None;
                    self.stage = CreateStage::EnterBranchName;
                    ScreenAction::Continue
                } else {
                    // idx 1 (ExistingBranch) or idx 2 (DetachedHead)
                    // idx 1 reuses the space name as the branch, and the
                    // creation rule accepts names git does not (`my space`,
                    // `v1..v2`), so ask git here as the branch stage does.
                    let strategy = self.branch_strategy();
                    if let Some(branch) = crate::core::workspace::branch_slot_name(&strategy) {
                        if let Err(e) = crate::core::workspace::check_branch_name(branch) {
                            self.error = Some(e.to_string());
                            return ScreenAction::Continue;
                        }
                    }
                    // The bounce reason (ticket 12) stays on the picker here;
                    // the dispatch clears it, as it did before this check.
                    self.stage = CreateStage::Creating;
                    ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                        workspace_name: self.ws_name.value().to_string(),
                        workspace_dir: ctx.config.workspaces.dir.clone(),
                        repos: self.selected_repos.clone(),
                        branch_strategy: strategy,
                        is_new: true,
                        fresh_repos: self.report.fetched_ok_paths(),
                        slow_fetch_repos: self.report.slow_fetch_paths(),
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
                // Git's verdict on the name, before the flow starts: one
                // spawn on Enter, as `build_branch_picker` already does at
                // this stage. Names from the recent list or the picker are
                // git's own and are not re-checked.
                if let Err(e) = crate::core::workspace::check_branch_name(&name) {
                    self.error = Some(e.to_string());
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
                    fresh_repos: self.report.fetched_ok_paths(),
                    slow_fetch_repos: self.report.slow_fetch_paths(),
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
                self.stage = CreateStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.ws_name.value().to_string(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: BranchStrategy::ExistingBranch(branch),
                    is_new: true,
                    fresh_repos: self.report.fetched_ok_paths(),
                    slow_fetch_repos: self.report.slow_fetch_paths(),
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
        ctx: &crate::tui::actions::ScreenContext,
    ) -> crate::tui::actions::ScreenAction {
        use crate::tui::actions::ScreenAction;
        use ratatui::crossterm::event::KeyCode;

        match key.code {
            // Stopping a run in flight and leaving a finished one are different
            // acts, so the same keys mean different things either side of it.
            KeyCode::Esc | KeyCode::Char('q') if ctx.creating_in_flight => {
                ScreenAction::CancelCreating
            }
            // Ignored while the worker runs, the sync report's rule for Enter.
            KeyCode::Enter if ctx.creating_in_flight => ScreenAction::Continue,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::SpaceConfig;
    use crate::tui::actions::{ScreenAction, ScreenContext};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn ctx(creating_in_flight: bool) -> ScreenContext<'static> {
        static CFG: std::sync::OnceLock<SpaceConfig> = std::sync::OnceLock::new();
        ScreenContext {
            config: CFG.get_or_init(SpaceConfig::default),
            creating_in_flight,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn creating_state() -> CreateState {
        let mut st = CreateState::new(vec![], vec![]);
        st.stage = CreateStage::Creating;
        st.progress = (1..=30).map(|i| format!("step {:02}", i)).collect();
        st
    }

    #[test]
    fn enter_is_ignored_while_the_worker_runs() {
        let mut st = creating_state();
        let action = st.handle_key(key(KeyCode::Enter), &ctx(true));
        assert!(
            matches!(action, ScreenAction::Continue),
            "Enter must not leave a run that is still creating worktrees"
        );
        assert_eq!(st.stage, CreateStage::Creating, "the stage is unchanged");
    }

    #[test]
    fn esc_and_q_cancel_while_the_worker_runs() {
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            let mut st = creating_state();
            let action = st.handle_key(key(code), &ctx(true));
            assert!(
                matches!(action, ScreenAction::CancelCreating),
                "{:?} must stop the run rather than just leaving",
                code
            );
        }
    }

    #[test]
    fn scroll_keys_still_reach_the_log_while_the_worker_runs() {
        let mut st = creating_state();
        let action = st.handle_key(key(KeyCode::Up), &ctx(true));
        assert!(matches!(action, ScreenAction::Continue));
        assert!(
            !st.log_view.follow,
            "Up must detach the log from the tail while the worker runs"
        );
    }

    #[test]
    fn enter_esc_and_q_leave_once_the_run_is_over() {
        for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Char('q')] {
            let mut st = creating_state();
            let action = st.handle_key(key(code), &ctx(false));
            assert!(
                matches!(action, ScreenAction::Back),
                "{:?} leaves a finished run for the dashboard",
                code
            );
        }
    }

    #[test]
    fn a_finished_run_that_failed_leaves_with_the_error() {
        let mut st = creating_state();
        st.error = Some("boom".to_string());
        match st.handle_key(key(KeyCode::Esc), &ctx(false)) {
            ScreenAction::BackWithStatus(msg, kind) => {
                assert_eq!(msg, "Create failed: boom");
                assert_eq!(kind, crate::tui::actions::StatusKind::Error);
            }
            _ => panic!("a failed run must report why on the way out"),
        }
    }

    // The branch-name field across a rename of the space (ticket 26). Each
    // test drives the screen by keys from the name stage to the branch-name
    // stage, back to the name stage, and forward again.

    fn press(st: &mut CreateState, code: KeyCode) -> ScreenAction {
        st.handle_key(key(code), &ctx(false))
    }

    fn type_text(st: &mut CreateState, text: &str) {
        for c in text.chars() {
            press(st, KeyCode::Char(c));
        }
    }

    /// Ctrl-U: empty the field the stage is editing.
    fn clear_field(st: &mut CreateState) {
        st.handle_key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &ctx(false),
        );
    }

    /// Name stage and back again, with the space renamed on the way.
    fn rename_space(st: &mut CreateState, name: &str) {
        back_to_name(st);
        clear_field(st);
        type_text(st, name);
        forward_to_branch_name(st);
    }

    /// A create screen on its first stage with one repo to pick.
    fn naming_state() -> CreateState {
        CreateState::new(vec![PathBuf::from("/nonexistent/ticket-26/repo")], vec![])
    }

    /// Name stage to branch-name stage. The sync report and the move past it
    /// belong to the app; this finishes the report and then does what
    /// `App::advance_to_branch_strategy` does, with no recent branches.
    fn forward_to_branch_name(st: &mut CreateState) {
        assert_eq!(st.stage, CreateStage::EnterName);
        press(st, KeyCode::Enter);
        assert_eq!(
            st.stage,
            CreateStage::PickRepos,
            "the name must be accepted"
        );
        assert!(matches!(
            press(st, KeyCode::Enter),
            ScreenAction::ExecuteSyncFlow(_)
        ));
        st.report.done = true;
        assert!(matches!(
            press(st, KeyCode::Enter),
            ScreenAction::ContinueFromSyncReport
        ));
        st.recent_branches = vec![];
        st.branch_strategy_idx = 0;
        st.progress.clear();
        st.stage = CreateStage::PickBranchStrategy;
        press(st, KeyCode::Enter);
        assert_eq!(st.stage, CreateStage::EnterBranchName);
    }

    /// Branch-name stage back to the name stage, one Esc per stage.
    fn back_to_name(st: &mut CreateState) {
        for stage in [
            CreateStage::PickBranchStrategy,
            CreateStage::PickRepos,
            CreateStage::EnterName,
        ] {
            press(st, KeyCode::Esc);
            assert_eq!(st.stage, stage);
        }
    }

    /// Enter on the branch-name stage: the new-branch name it creates, and
    /// the space it creates it in.
    fn confirm_branch_name(st: &mut CreateState) -> (String, String) {
        match press(st, KeyCode::Enter) {
            ScreenAction::ExecuteWorktreeFlow(p) => match p.branch_strategy {
                BranchStrategy::NewBranch(name) => (name, p.workspace_name),
                other => panic!("expected a new branch, got {:?}", other),
            },
            _ => panic!("Enter on the branch name must start the create"),
        }
    }

    #[test]
    fn renaming_the_space_renames_the_branch_it_filled_in() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        assert_eq!(st.branch_name_input.value(), "a");

        rename_space(&mut st, "b");

        assert_eq!(
            st.branch_name_input.value(),
            "b",
            "the field must follow the new space name, not keep the old one"
        );
        assert_eq!(
            confirm_branch_name(&mut st),
            ("b".to_string(), "b".to_string()),
            "the branch is created under the new name"
        );
    }

    #[test]
    fn a_branch_name_the_user_typed_survives_renaming_the_space() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        clear_field(&mut st);
        type_text(&mut st, "feat");

        for name in ["b", "c"] {
            rename_space(&mut st, name);
            assert_eq!(
                st.branch_name_input.value(),
                "feat",
                "a name the user typed is theirs and must be kept (space '{}')",
                name
            );
        }
        assert_eq!(
            confirm_branch_name(&mut st),
            ("feat".to_string(), "c".to_string())
        );
    }

    #[test]
    fn a_typed_name_the_space_is_renamed_to_follows_the_space_from_then_on() {
        // The accepted limit: a field that reads the space name when the
        // user leaves it cannot be told from one they never touched.
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        clear_field(&mut st);
        type_text(&mut st, "feat");
        rename_space(&mut st, "feat");
        assert_eq!(st.branch_name_input.value(), "feat");

        rename_space(&mut st, "c");

        assert_eq!(st.branch_name_input.value(), "c");
    }

    #[test]
    fn a_typed_name_that_matches_an_old_space_name_is_still_the_users() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        clear_field(&mut st);
        type_text(&mut st, "x");
        rename_space(&mut st, "b");
        assert_eq!(st.branch_name_input.value(), "x");

        // In space "b" the user asks for branch "a", the first space name.
        clear_field(&mut st);
        type_text(&mut st, "a");
        rename_space(&mut st, "c");

        assert_eq!(
            st.branch_name_input.value(),
            "a",
            "the field said something other than 'b' when the user left it"
        );
    }

    #[test]
    fn an_emptied_branch_field_is_filled_with_the_new_space_name() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        clear_field(&mut st);
        assert_eq!(st.branch_name_input.value(), "");

        rename_space(&mut st, "b");

        assert_eq!(st.branch_name_input.value(), "b");
    }

    #[test]
    fn a_branch_field_of_only_spaces_counts_as_emptied() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        clear_field(&mut st);
        type_text(&mut st, "  ");

        rename_space(&mut st, "b");

        assert_eq!(
            st.branch_name_input.value(),
            "b",
            "Enter reads the field trimmed, so this field names no branch"
        );
    }

    #[test]
    fn a_trailing_space_does_not_make_the_filled_in_name_the_users() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        type_text(&mut st, " ");

        rename_space(&mut st, "b");

        assert_eq!(
            st.branch_name_input.value(),
            "b",
            "Enter would read 'a', the old space name, so the field must follow"
        );
    }

    #[test]
    fn a_leading_space_does_not_make_the_filled_in_name_the_users() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        press(&mut st, KeyCode::Home);
        type_text(&mut st, " ");
        assert_eq!(st.branch_name_input.value(), " a");

        rename_space(&mut st, "b");

        assert_eq!(
            st.branch_name_input.value(),
            "b",
            "Enter would read 'a', the old space name, so the field must follow"
        );
    }

    #[test]
    fn a_branch_field_edited_back_to_what_was_filled_in_still_follows_the_space() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        type_text(&mut st, "x");
        press(&mut st, KeyCode::Backspace);
        assert_eq!(st.branch_name_input.value(), "a");

        rename_space(&mut st, "b");

        assert_eq!(
            st.branch_name_input.value(),
            "b",
            "the field reads what was filled in, so it is not the user's name"
        );
    }

    #[test]
    fn choosing_new_branch_again_leaves_the_cursor_where_it_was() {
        let mut st = naming_state();
        type_text(&mut st, "a");
        forward_to_branch_name(&mut st);
        press(&mut st, KeyCode::Home);

        press(&mut st, KeyCode::Esc);
        assert_eq!(st.stage, CreateStage::PickBranchStrategy);
        press(&mut st, KeyCode::Enter);
        assert_eq!(st.stage, CreateStage::EnterBranchName);
        type_text(&mut st, "x");

        assert_eq!(
            st.branch_name_input.value(),
            "xa",
            "a field with nothing to change must not be rebuilt"
        );
    }
}

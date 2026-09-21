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
    /// `preselected` are the repos named on the command line, resolved to
    /// cached paths; they open the picker toggled on, with an empty query.
    pub fn new(ws_name: String, available_repos: Vec<PathBuf>, preselected: Vec<PathBuf>) -> Self {
        let items = super::repo_items(available_repos);
        let mut picker = FuzzyPicker::new(
            "Add repos  TAB=toggle  ENTER=confirm  ESC=cancel",
            items,
            true,
        );
        picker.toggle_paths(&preselected);
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

    /// The branch the "New branch" option creates: the space name while the
    /// field is empty (read trimmed, as Enter reads it), and the user's name,
    /// trimmed, once they have typed one. The strategy picker's row renders
    /// it and the branch name stage opens on it, so the row names what Enter
    /// there creates. This is the create flow's rule (ticket 26) without its
    /// rename clause: the space name here is fixed when the flow opens, so a
    /// field reading the space name and an empty one open on the same name
    /// (ticket 32).
    pub fn new_branch_name(&self) -> &str {
        let typed = self.branch_name_input.value().trim();
        if typed.is_empty() {
            &self.workspace_name
        } else {
            typed
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
            // idx 0, New Branch: the name the picker's row shows and its
            // stage opens on. The picker never asks for idx 0 here (Enter on
            // that row opens the branch name stage instead), so only a
            // direct caller reaches this arm.
            _ => BranchStrategy::NewBranch(self.new_branch_name().to_string()),
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
            AddStage::Creating => self.handle_creating(key, ctx),
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
                        fresh_repos: self.report.fetched_ok_paths(),
                        slow_fetch_repos: self.report.slow_fetch_paths(),
                    })
                } else if self.branch_strategy_idx == 0 {
                    // New branch: open the branch name stage on
                    // `new_branch_name`, the name the picker's row shows. A
                    // field left empty, or holding only whitespace, is filled
                    // in with the space name; anything the user typed is
                    // kept. The field is replaced only when its trimmed value
                    // differs, so the cursor stays where it was left.
                    let opens_with = self.new_branch_name().to_string();
                    if self.branch_name_input.value().trim() != opens_with {
                        self.branch_name_input = Input::default().with_value(opens_with);
                    }
                    self.error = None;
                    self.stage = AddStage::EnterBranchName;
                    ScreenAction::Continue
                } else {
                    // idx 1 (ExistingBranch) or idx 2 (DetachedHead); see
                    // the create flow for why git is asked here.
                    let strategy = self.branch_strategy();
                    if let Some(branch) = crate::core::workspace::branch_slot_name(&strategy, &[]) {
                        if let Err(e) = crate::core::workspace::check_branch_name(branch) {
                            self.error = Some(e.to_string());
                            return ScreenAction::Continue;
                        }
                    }
                    // The bounce reason (ticket 12) stays on the picker here;
                    // the dispatch clears it, as it did before this check.
                    self.stage = AddStage::Creating;
                    ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                        workspace_name: self.workspace_name.clone(),
                        workspace_dir: ctx.config.workspaces.dir.clone(),
                        repos: self.selected_repos.clone(),
                        branch_strategy: strategy,
                        is_new: false,
                        fresh_repos: self.report.fetched_ok_paths(),
                        slow_fetch_repos: self.report.slow_fetch_paths(),
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
                // Git's verdict on the name, before the flow starts (see the
                // create flow's branch stage).
                if let Err(e) = crate::core::workspace::check_branch_name(&name) {
                    self.error = Some(e.to_string());
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

    fn handle_creating(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::SpaceConfig;

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

    fn creating_state() -> AddState {
        let mut st = AddState::new("ws".to_string(), vec![], vec![]);
        st.stage = AddStage::Creating;
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
        assert_eq!(st.stage, AddStage::Creating, "the stage is unchanged");
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
                assert_eq!(msg, "Add failed: boom");
                assert_eq!(kind, crate::tui::actions::StatusKind::Error);
            }
            _ => panic!("a failed run must report why on the way out"),
        }
    }

    // -----------------------------------------------------------------------
    // The New branch field (ticket 32): the space name while the field is
    // empty, the typed name once there is one, and the label agrees.
    // -----------------------------------------------------------------------

    fn press(st: &mut AddState, code: KeyCode) -> ScreenAction {
        st.handle_key(key(code), &ctx(false))
    }

    fn type_text(st: &mut AddState, text: &str) {
        for c in text.chars() {
            press(st, KeyCode::Char(c));
        }
    }

    /// Ctrl-U: empty the field the stage is editing.
    fn clear_field(st: &mut AddState) {
        st.handle_key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &ctx(false),
        );
    }

    /// An add screen for space `ws` parked on the strategy picker with one
    /// repo picked and no recent branches, where
    /// `App::advance_to_branch_strategy` leaves it once the sync report is
    /// done.
    fn at_branch_strategy() -> AddState {
        let mut st = AddState::new("ws".to_string(), vec![], vec![]);
        st.selected_repos = vec![PathBuf::from("/nonexistent/ticket-32/repo")];
        st.recent_branches = vec![];
        st.branch_strategy_idx = 0;
        st.stage = AddStage::PickBranchStrategy;
        st
    }

    /// Enter on the New branch row: the stage must open.
    fn open_branch_name(st: &mut AddState) {
        assert_eq!(st.stage, AddStage::PickBranchStrategy);
        assert_eq!(st.branch_strategy_idx, 0);
        press(st, KeyCode::Enter);
        assert_eq!(st.stage, AddStage::EnterBranchName);
    }

    /// Esc from the branch name stage back to the picker.
    fn back_to_picker(st: &mut AddState) {
        assert_eq!(st.stage, AddStage::EnterBranchName);
        press(st, KeyCode::Esc);
        assert_eq!(st.stage, AddStage::PickBranchStrategy);
    }

    /// Enter on the branch-name stage: the new-branch name it creates.
    fn confirm_branch_name(st: &mut AddState) -> String {
        match press(st, KeyCode::Enter) {
            ScreenAction::ExecuteWorktreeFlow(p) => match p.branch_strategy {
                BranchStrategy::NewBranch(name) => name,
                other => panic!("expected a new branch, got {:?}", other),
            },
            _ => panic!("Enter on the branch name must start the add"),
        }
    }

    #[test]
    fn the_branch_field_opens_with_the_space_name() {
        let mut st = at_branch_strategy();
        assert_eq!(st.new_branch_name(), "ws", "the row names the space");
        open_branch_name(&mut st);
        assert_eq!(st.branch_name_input.value(), "ws");
        assert_eq!(confirm_branch_name(&mut st), "ws");
    }

    #[test]
    fn a_typed_name_survives_esc_and_choosing_new_branch_again() {
        let mut st = at_branch_strategy();
        open_branch_name(&mut st);
        clear_field(&mut st);
        type_text(&mut st, "feat");
        back_to_picker(&mut st);
        assert_eq!(
            st.new_branch_name(),
            "feat",
            "the row must name the branch the stage will open with"
        );

        open_branch_name(&mut st);

        assert_eq!(
            st.branch_name_input.value(),
            "feat",
            "a name the user typed is theirs and must be kept"
        );
        assert_eq!(confirm_branch_name(&mut st), "feat");
    }

    #[test]
    fn an_emptied_branch_field_is_filled_in_again() {
        let mut st = at_branch_strategy();
        open_branch_name(&mut st);
        type_text(&mut st, "x");
        clear_field(&mut st);
        assert_eq!(st.branch_name_input.value(), "");
        back_to_picker(&mut st);
        assert_eq!(st.new_branch_name(), "ws");

        open_branch_name(&mut st);

        assert_eq!(
            st.branch_name_input.value(),
            "ws",
            "an emptied field names no branch, so it follows the space again"
        );
        assert_eq!(confirm_branch_name(&mut st), "ws");
    }

    #[test]
    fn a_branch_field_of_only_spaces_is_filled_in_again() {
        let mut st = at_branch_strategy();
        open_branch_name(&mut st);
        clear_field(&mut st);
        type_text(&mut st, "  ");
        assert_eq!(st.branch_name_input.value(), "  ");
        back_to_picker(&mut st);
        assert_eq!(st.new_branch_name(), "ws");

        open_branch_name(&mut st);

        assert_eq!(
            st.branch_name_input.value(),
            "ws",
            "Enter reads the field trimmed, so this field names no branch"
        );
        assert_eq!(confirm_branch_name(&mut st), "ws");
    }

    #[test]
    fn a_padded_typed_name_is_kept_and_created_trimmed() {
        let mut st = at_branch_strategy();
        open_branch_name(&mut st);
        clear_field(&mut st);
        type_text(&mut st, " feat ");
        back_to_picker(&mut st);
        assert_eq!(
            st.new_branch_name(),
            "feat",
            "the row names what Enter creates, which is the field trimmed"
        );

        open_branch_name(&mut st);

        assert_eq!(
            st.branch_name_input.value(),
            " feat ",
            "the field is the user's and is left as they typed it"
        );
        assert_eq!(confirm_branch_name(&mut st), "feat");
    }

    #[test]
    fn choosing_new_branch_clears_an_error_left_by_another_row() {
        // The Existing branch row can leave a refusal on the picker; the
        // branch name stage renders the same error slot, so choosing New
        // branch must not carry it in.
        let mut st = at_branch_strategy();
        st.error = Some("refused".to_string());

        open_branch_name(&mut st);

        assert_eq!(st.error, None, "the stage opens with no error showing");
    }

    #[test]
    fn choosing_new_branch_again_leaves_the_cursor_where_it_was() {
        let mut st = at_branch_strategy();
        open_branch_name(&mut st);
        press(&mut st, KeyCode::Home);

        back_to_picker(&mut st);
        open_branch_name(&mut st);
        type_text(&mut st, "x");

        assert_eq!(
            st.branch_name_input.value(),
            "xws",
            "a field with nothing to change must not be rebuilt"
        );
    }
}

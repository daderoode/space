use super::default_no_confirm;
use crate::core::git::CommitInfo;
use crate::tui::actions::{GitOp, ScreenAction, ScreenContext};
use crate::tui::widgets::fuzzy_picker::FuzzyPicker;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;
use tui_input::Input;

/// The git-ops menu items in display order: (hotkey, label). Indices match
/// `GitOpsState::fire` and `item_enabled`. Hotkeys are case-sensitive.
pub const MENU: [(char, &str); 6] = [
    ('f', "fetch"),
    ('p', "pull"),
    ('P', "push"),
    ('c', "commit"),
    ('l', "log"),
    ('r', "rebase"),
];

/// Stage of the git-operations overlay: the action menu plus one stage per
/// sub-flow (commit entry, log view, a running network op, push confirmation).
#[derive(Debug, Clone, PartialEq)]
pub enum GitOpsStage {
    Menu,
    /// Single-line commit-message entry for the staged changes.
    Committing,
    /// Read-only scrollable list of recent commits.
    Log,
    /// A network op (fetch / pull / push / rebase) is running, showing live
    /// output lines.
    Running,
    /// Confirm publishing a branch that has no upstream yet (push -u origin).
    ConfirmPush,
    /// Rebase pre-flight: shows the branch state and either a blocking reason
    /// (detached HEAD / dirty tree) or a ready-to-continue prompt.
    RebasePreflight,
    /// Pick the branch to rebase onto (reuses the shared fuzzy branch picker).
    RebasePickTarget,
    /// Confirm the rebase with an ahead/behind preview before executing.
    RebaseConfirm,
}

pub struct GitOpsState {
    pub stage: GitOpsStage,
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub branch: String,
    pub selected: usize,
    pub has_staged: bool,
    /// Paths of the currently staged files, listed above the commit-message
    /// input in the Committing stage. Captured in `new()` so the renderer needs
    /// no git call.
    pub staged_files: Vec<String>,
    /// Single-line commit message entered in the Committing stage.
    pub message_input: Input,
    /// Whether the current branch already has a configured upstream. Drives the
    /// push routing: plain push when true, ConfirmPush (set upstream) when false.
    pub has_upstream: bool,
    pub status: Option<String>,
    /// The network op currently running (fetch / pull / push), used by the
    /// Running-stage header. `None` when no op has run.
    pub running_op: Option<GitOp>,
    /// Live output lines captured while a Running network op streams.
    pub output: Vec<String>,
    /// None while running; `Some(success)` once the op completes.
    pub finished: Option<bool>,
    /// When set (success only), the overlay auto-closes at this instant.
    pub close_at: Option<std::time::Instant>,
    /// Recent commits loaded synchronously when the Log stage opens.
    pub commits: Vec<CommitInfo>,
    /// Scroll offset (top row index) for the Log stage. The renderer applies a
    /// viewport-aware upper clamp so it can never scroll past the last full
    /// screenful.
    pub log_scroll: u16,
    /// `Some(reason)` when the rebase pre-flight failed (detached HEAD / dirty
    /// tree); the RebasePreflight stage shows it and Enter cannot proceed.
    pub rebase_block: Option<String>,
    /// The branch picker for the RebasePickTarget stage.
    pub rebase_picker: Option<FuzzyPicker>,
    /// The target branch chosen to rebase onto, carried into the confirm stage
    /// and the `GitOp::Rebase` dispatch.
    pub rebase_onto: Option<String>,
    /// `(ahead, behind)` of HEAD vs the chosen target, shown as a preview on the
    /// RebaseConfirm stage. `ahead` = commits to replay; `behind` = commits the
    /// target has that HEAD does not.
    pub rebase_ahead_behind: Option<(usize, usize)>,
}

impl std::fmt::Debug for GitOpsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitOpsState")
            .field("stage", &self.stage)
            .field("repo_name", &self.repo_name)
            .field("branch", &self.branch)
            .field("selected", &self.selected)
            .finish()
    }
}

impl GitOpsState {
    /// Build the menu state for a repo. Panic-safe on non-existent paths:
    /// `current_branch` and `file_diff` failures degrade to "?" / no-staged.
    pub fn new(repo_name: String, repo_path: PathBuf) -> Self {
        let branch =
            crate::core::git::current_branch(&repo_path).unwrap_or_else(|_| "?".to_string());
        let staged_files: Vec<String> = crate::core::git::file_diff(&repo_path)
            .map(|v| {
                v.iter()
                    .filter(|e| e.staged)
                    .map(|e| e.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let has_staged = !staged_files.is_empty();
        let has_upstream = crate::core::git::has_upstream(&repo_path);
        Self {
            stage: GitOpsStage::Menu,
            repo_name,
            repo_path,
            branch,
            selected: 0,
            has_staged,
            staged_files,
            message_input: Input::default(),
            has_upstream,
            status: None,
            running_op: None,
            output: Vec::new(),
            finished: None,
            close_at: None,
            commits: Vec::new(),
            log_scroll: 0,
            rebase_block: None,
            rebase_picker: None,
            rebase_onto: None,
            rebase_ahead_behind: None,
        }
    }

    /// Highest selectable menu index (six items, 0..=5).
    const MAX_IDX: usize = 5;

    /// Whether the menu item at `idx` is enabled: commit requires staged
    /// files; everything else is always on.
    pub fn item_enabled(&self, idx: usize) -> bool {
        match idx {
            3 => self.has_staged,
            _ => true,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match self.stage {
            GitOpsStage::Running => match key.code {
                // Close early; while running other keys are no-ops.
                KeyCode::Esc | KeyCode::Char('q') => ScreenAction::Back,
                _ => ScreenAction::Continue,
            },
            GitOpsStage::ConfirmPush => self.handle_confirm_push_key(key),
            GitOpsStage::Committing => self.handle_committing_key(key),
            GitOpsStage::Log => self.handle_log_key(key),
            GitOpsStage::RebasePreflight => self.handle_rebase_preflight_key(key),
            GitOpsStage::RebasePickTarget => self.handle_rebase_pick_target_key(key),
            GitOpsStage::RebaseConfirm => self.handle_rebase_confirm_key(key),
            GitOpsStage::Menu => self.handle_menu_key(key),
        }
    }

    /// Handle keys in the read-only Log stage. `Esc`/`q` return to the menu;
    /// the arrow / `j`/`k` / PageUp/PageDown / Home / End keys scroll
    /// `log_scroll` with saturating add/sub. The down/end direction is clamped
    /// to the last commit index here; the renderer applies the viewport-aware
    /// clamp so no blank screenful is shown.
    fn handle_log_key(&mut self, key: KeyEvent) -> ScreenAction {
        let max = self.commits.len().saturating_sub(1) as u16;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.stage = GitOpsStage::Menu;
                ScreenAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.log_scroll = self.log_scroll.saturating_add(1).min(max);
                ScreenAction::Continue
            }
            KeyCode::PageUp => {
                self.log_scroll = self.log_scroll.saturating_sub(10);
                ScreenAction::Continue
            }
            KeyCode::PageDown => {
                self.log_scroll = self.log_scroll.saturating_add(10).min(max);
                ScreenAction::Continue
            }
            KeyCode::Home => {
                self.log_scroll = 0;
                ScreenAction::Continue
            }
            KeyCode::End => {
                self.log_scroll = max;
                ScreenAction::Continue
            }
            _ => ScreenAction::Continue,
        }
    }

    /// Handle keys in the Committing stage (single-line commit-message entry).
    /// `Esc` returns to the menu; `Enter` commits a non-empty message; every
    /// other key (including `q`) feeds the text input.
    fn handle_committing_key(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Esc => {
                self.status = None;
                self.stage = GitOpsStage::Menu;
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                let message = self.message_input.value().trim().to_string();
                if message.is_empty() {
                    self.status = Some("Commit message cannot be empty".to_string());
                    return ScreenAction::Continue;
                }
                self.status = None;
                ScreenAction::CommitRepo {
                    repo_path: self.repo_path.clone(),
                    message,
                }
            }
            _ => {
                if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                    self.message_input.handle(req);
                }
                self.status = None;
                ScreenAction::Continue
            }
        }
    }

    /// Handle keys in the ConfirmPush stage (branch has no upstream).
    /// `y`/Enter confirms and pushes with `-u origin <branch>`.
    fn handle_confirm_push_key(&mut self, key: KeyEvent) -> ScreenAction {
        // Only an explicit `y` confirms publishing the branch; Enter
        // declines, matching the [y/N] prompt (default No) so the remote
        // is never mutated by a reflexive keypress.
        match default_no_confirm(key.code) {
            Some(true) => self.start_network_op(GitOp::Push { set_upstream: true }),
            // Decline: back to the menu without touching the remote.
            Some(false) => {
                self.stage = GitOpsStage::Menu;
                ScreenAction::Continue
            }
            None => ScreenAction::Continue,
        }
    }

    fn handle_menu_key(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ScreenAction::Back,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected < Self::MAX_IDX {
                    self.selected += 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Enter => self.fire(self.selected),
            // Case-sensitive hotkey lookup over the menu table ('p' pull vs
            // 'P' push).
            KeyCode::Char(c) => {
                if let Some(idx) = MENU.iter().position(|(k, _)| *k == c) {
                    return self.fire(idx);
                }
                ScreenAction::Continue
            }
            _ => ScreenAction::Continue,
        }
    }

    /// Fire the menu item at `idx`, moving the highlight to it. Fetch/pull/push
    /// run through the async Running stage (push confirms first when the branch
    /// has no upstream); commit and log open their own stages synchronously;
    /// rebase opens the guarded pre-flight/target/confirm sub-flow.
    fn fire(&mut self, idx: usize) -> ScreenAction {
        self.selected = idx;
        match idx {
            0 => self.start_network_op(GitOp::Fetch),
            1 => self.start_network_op(GitOp::Pull),
            2 => {
                // With an upstream, push straight away; otherwise confirm before
                // publishing the branch (push -u origin <branch>).
                if self.has_upstream {
                    self.start_network_op(GitOp::Push {
                        set_upstream: false,
                    })
                } else {
                    self.stage = GitOpsStage::ConfirmPush;
                    self.status = None;
                    ScreenAction::Continue
                }
            }
            3 if !self.has_staged => {
                self.status = Some("Stage files first with s/S".to_string());
                ScreenAction::Continue
            }
            3 => {
                // Enter the commit-message stage with a fresh input.
                self.stage = GitOpsStage::Committing;
                self.message_input = Input::default();
                self.status = None;
                ScreenAction::Continue
            }
            4 => {
                // Log is a local read-only op: load the commits synchronously.
                self.commits = crate::core::git::recent_commits(&self.repo_path, 50);
                self.log_scroll = 0;
                self.status = None;
                self.stage = GitOpsStage::Log;
                ScreenAction::Continue
            }
            5 => self.start_rebase(),
            _ => ScreenAction::Continue,
        }
    }

    /// Reset the run buffers, enter the Running stage, and dispatch `op` to the
    /// background git-ops worker.
    fn start_network_op(&mut self, op: GitOp) -> ScreenAction {
        self.running_op = Some(op.clone());
        self.stage = GitOpsStage::Running;
        self.output.clear();
        self.finished = None;
        self.close_at = None;
        self.status = None;
        ScreenAction::ExecuteGitOp {
            repo_path: self.repo_path.clone(),
            op,
        }
    }

    /// Enter the rebase sub-flow: run the synchronous pre-flight (detached HEAD
    /// / dirty working tree) and open the RebasePreflight summary. A blocking
    /// state is recorded in `rebase_block`; the stage renders it and Enter
    /// cannot proceed until the user clears it (commit/stash) and re-enters.
    fn start_rebase(&mut self) -> ScreenAction {
        self.rebase_onto = None;
        self.rebase_ahead_behind = None;
        self.rebase_picker = None;
        self.status = None;

        // Block when HEAD is not on a branch. `current_branch` reports a
        // detached HEAD as `Ok("(<sha>)")`, so a plain `.is_err()` would let a
        // genuinely detached HEAD slip past the pre-flight; `is_on_branch`
        // gates on the branch ref itself.
        let detached = !crate::core::git::is_on_branch(&self.repo_path);
        // Untracked files do not block a rebase, so only tracked modifications,
        // staged changes, and conflicts count as "dirty" here (matching what
        // `git rebase` itself refuses).
        let dirty = crate::core::git::repo_status(&self.repo_path)
            .map(|s| s.modified > 0 || s.staged > 0 || s.conflicted > 0)
            .unwrap_or(false);

        self.rebase_block = if detached {
            Some("Detached HEAD: checkout a branch before rebasing.".to_string())
        } else if dirty {
            Some(
                "Working tree has uncommitted changes. Commit them (stage with s/S on the \
                 dashboard, then c in this menu) or stash outside space."
                    .to_string(),
            )
        } else {
            None
        };
        self.stage = GitOpsStage::RebasePreflight;
        ScreenAction::Continue
    }

    /// Pre-flight stage keys. `Esc`/`q` return to the menu; `Enter` proceeds to
    /// the target picker only when the working tree is clean (no `rebase_block`).
    fn handle_rebase_preflight_key(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.stage = GitOpsStage::Menu;
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                if self.rebase_block.is_some() {
                    // Blocked: stay put until the user clears the tree.
                    return ScreenAction::Continue;
                }
                match crate::tui::app::build_branch_picker(
                    &self.repo_path,
                    &self.repo_name,
                    "Rebase onto",
                ) {
                    Some(picker) => {
                        self.rebase_picker = Some(picker);
                        self.stage = GitOpsStage::RebasePickTarget;
                    }
                    None => {
                        self.status =
                            Some(format!("Could not list branches for {}", self.repo_name));
                    }
                }
                ScreenAction::Continue
            }
            _ => ScreenAction::Continue,
        }
    }

    /// Target-picker stage keys. Follows the switch-branch picker's shape with
    /// one deliberate difference: `q` types into the fuzzy filter (branch names
    /// can contain it) instead of closing. `Esc` steps back to the pre-flight;
    /// the arrows move the highlight (letters are text in a typed picker, so
    /// there is no `j`/`k` navigation here); `Enter` selects the target,
    /// computes the ahead/behind preview, and advances to the confirm stage;
    /// anything else edits the fuzzy filter.
    fn handle_rebase_pick_target_key(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Esc => {
                self.stage = GitOpsStage::RebasePreflight;
                ScreenAction::Continue
            }
            KeyCode::Up => {
                if let Some(ref mut bp) = self.rebase_picker {
                    bp.move_up();
                }
                ScreenAction::Continue
            }
            KeyCode::Down => {
                if let Some(ref mut bp) = self.rebase_picker {
                    bp.move_down();
                }
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                let picked = self
                    .rebase_picker
                    .as_ref()
                    .and_then(|bp| bp.confirmed_items().into_iter().next())
                    .map(|item| item.name.clone());
                match picked {
                    None => ScreenAction::Continue,
                    Some(onto) => {
                        self.rebase_ahead_behind =
                            crate::core::git::ahead_behind_vs(&self.repo_path, &onto);
                        self.rebase_onto = Some(onto);
                        self.stage = GitOpsStage::RebaseConfirm;
                        ScreenAction::Continue
                    }
                }
            }
            _ => {
                if let Some(ref mut bp) = self.rebase_picker {
                    if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                        bp.input.handle(req);
                    }
                    bp.refilter();
                }
                ScreenAction::Continue
            }
        }
    }

    /// Confirm-stage keys. `y` executes the rebase on the git-ops worker;
    /// `n`/`Enter`/`Esc`/`q` decline back to the target picker. Enter declines
    /// (default No) so a reflexive keypress never rewrites history.
    fn handle_rebase_confirm_key(&mut self, key: KeyEvent) -> ScreenAction {
        match default_no_confirm(key.code) {
            Some(true) => match self.rebase_onto.clone() {
                Some(onto) => self.start_network_op(GitOp::Rebase { onto }),
                None => {
                    self.stage = GitOpsStage::RebasePickTarget;
                    ScreenAction::Continue
                }
            },
            Some(false) => {
                self.stage = GitOpsStage::RebasePickTarget;
                ScreenAction::Continue
            }
            None => ScreenAction::Continue,
        }
    }
}
